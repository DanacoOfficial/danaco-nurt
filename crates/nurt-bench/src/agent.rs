// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-bench::agent
// Responsibility:   The agent loop of the benchmark, identical in every condition.
// Dependencies:     serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The agent loop of the benchmark, identical in every condition.
//!
//! The loop sends the conversation to the model, executes the requested
//! tool calls in order and returns their results, until the model answers
//! without a tool call or a limit is reached.

use std::time::{Duration, Instant};

use crate::error::Result;
use crate::model::{ChatRequest, Message, ModelClient, Part, Role, ToolSpec};
use crate::protocol::clip;
use crate::runlog::{Event, GroupEvent, ModelEvent, Outcome, Tokens, ToolEvent};
use crate::task::Limits;
use crate::tool::ToolExecutor;

/// Inputs of one run of the loop.
#[derive(Debug, Clone, Copy)]
pub struct AgentSetup<'a> {
    /// System message.
    pub system: &'a str,
    /// The tool.
    pub tool: &'a ToolSpec,
    /// Task description, the first user message.
    pub task: &'a str,
    /// Limits of the run.
    pub limits: Limits,
    /// Seed passed to the model.
    pub seed: u64,
    /// Largest number of characters of a tool result shown to the model.
    pub max_output_chars: usize,
}

/// Result of the loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentResult {
    /// How the loop ended.
    pub outcome: Outcome,
    /// Model calls.
    pub steps: u32,
    /// Tool calls.
    pub tool_calls: u32,
    /// Token totals.
    pub tokens: Tokens,
    /// Wall clock time in milliseconds.
    pub wall_ms: u64,
    /// Text of the last model response.
    pub answer: String,
    /// Failure of a run with outcome [`Outcome::Error`].
    pub error: Option<String>,
}

/// Longest time of one tool call; a call that hangs must not use the whole
/// time of the run, so that the model can still react to it.
pub const TOOL_CALL_LIMIT: Duration = Duration::from_secs(120);

/// Time limit of a tool call when `remaining` of the run is left.
fn tool_call_limit(remaining: Duration) -> Duration {
    remaining.min(TOOL_CALL_LIMIT)
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// State of a running loop.
struct Loop<'a, 'b> {
    setup: AgentSetup<'a>,
    model: &'b mut dyn ModelClient,
    tool: &'b mut dyn ToolExecutor,
    log: &'b mut dyn FnMut(Event) -> Result<()>,
    started: Instant,
    limit: Duration,
    result: AgentResult,
}

impl Loop<'_, '_> {
    fn remaining(&self) -> Duration {
        self.limit.saturating_sub(self.started.elapsed())
    }

    fn fail(&mut self, message: String) {
        self.result.outcome = Outcome::Error;
        self.result.error = Some(message);
    }

    /// Executes the tool calls of one response; returns `false` when the
    /// run must stop.
    fn execute(
        &mut self,
        step: u32,
        calls: &[crate::model::ToolCall],
        messages: &mut Vec<Message>,
    ) -> Result<bool> {
        let mut results = Vec::new();
        for call in calls {
            if self.remaining().is_zero() {
                self.result.outcome = Outcome::TimeLimit;
                return Ok(false);
            }
            let called = Instant::now();
            let outcome = match self
                .tool
                .call(&call.input, tool_call_limit(self.remaining()))
            {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.fail(error.to_string());
                    return Ok(false);
                }
            };
            let mut output = clip(&outcome.output, self.setup.max_output_chars);
            for (group, text) in &outcome.loaded_groups {
                output.push_str(&format!("\n\n[description of the group {group}]\n{text}"));
                let tokens = self.model.count_tokens(text).unwrap_or(None);
                (self.log)(Event::Group(GroupEvent {
                    step,
                    group: group.clone(),
                    chars: u64::try_from(text.chars().count()).unwrap_or(u64::MAX),
                    tokens,
                }))?;
            }
            // Only the call of the model counts: the `check` before a `run` with groups on demand stays out so that the conditions remain comparable.
            self.result.tool_calls += 1;
            (self.log)(Event::Tool(Box::new(ToolEvent {
                step,
                call_id: call.id.clone(),
                t_ms: millis(self.started.elapsed()),
                duration_ms: millis(called.elapsed()),
                input: call.input.clone(),
                output: output.clone(),
                is_error: outcome.is_error,
                exit: outcome.exit,
                timed_out: outcome.timed_out,
                nurt: outcome.nurt,
            })))?;
            results.push(Part::ToolResult {
                id: call.id.clone(),
                output,
                is_error: outcome.is_error,
            });
        }
        messages.push(Message {
            role: Role::User,
            parts: results,
        });
        Ok(true)
    }

    fn run(mut self) -> Result<AgentResult> {
        let mut messages = vec![Message::user_text(self.setup.task)];
        for step in 1..=self.setup.limits.steps {
            if self.remaining().is_zero() {
                self.result.outcome = Outcome::TimeLimit;
                break;
            }
            let request = ChatRequest {
                system: self.setup.system,
                tool: self.setup.tool,
                messages: &messages,
                seed: self.setup.seed,
            };
            let called = Instant::now();
            self.model.set_time_limit(self.remaining());
            let response = match self.model.complete(&request) {
                Ok(response) => response,
                Err(error) => {
                    if self.remaining().is_zero() {
                        self.result.outcome = Outcome::TimeLimit;
                    } else {
                        self.fail(error.to_string());
                    }
                    break;
                }
            };
            self.result.steps = step;
            self.result.tokens.input += response.usage.input;
            self.result.tokens.output += response.usage.output;
            self.result.answer.clone_from(&response.text);
            (self.log)(Event::Model(Box::new(ModelEvent {
                step,
                t_ms: millis(self.started.elapsed()),
                latency_ms: millis(called.elapsed()),
                response: response.clone(),
            })))?;
            messages.push(response.to_message());
            if response.tool_calls.is_empty() {
                self.result.outcome = Outcome::Answered;
                break;
            }
            if !self.execute(step, &response.tool_calls, &mut messages)? {
                break;
            }
            if step == self.setup.limits.steps {
                self.result.outcome = Outcome::StepLimit;
            }
        }
        self.result.wall_ms = millis(self.started.elapsed());
        Ok(self.result)
    }
}

/// Runs the agent loop and reports every model call, tool call and loaded
/// group description to `log`.
///
/// Failures of the provider or of the tool executor end the loop with
/// [`Outcome::Error`].
///
/// # Errors
///
/// Returns an error only when `log` fails.
pub fn run_agent(
    setup: AgentSetup<'_>,
    model: &mut dyn ModelClient,
    tool: &mut dyn ToolExecutor,
    log: &mut dyn FnMut(Event) -> Result<()>,
) -> Result<AgentResult> {
    Loop {
        setup,
        model,
        tool,
        log,
        started: Instant::now(),
        limit: Duration::from_secs(u64::from(setup.limits.seconds)),
        result: AgentResult {
            outcome: Outcome::StepLimit,
            steps: 0,
            tool_calls: 0,
            tokens: Tokens::default(),
            wall_ms: 0,
            answer: String::new(),
            error: None,
        },
    }
    .run()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::condition::Condition;
    use crate::error::BenchError;
    use crate::model::mock::MockModel;
    use crate::model::{ChatResponse, RunKey};
    use crate::tool::ToolOutcome;

    struct EchoTool {
        fail_on: Option<String>,
    }

    impl ToolExecutor for EchoTool {
        fn call(&mut self, input: &Value, _timeout: Duration) -> Result<ToolOutcome> {
            let command = input["command"].as_str().unwrap_or_default().to_owned();
            if self.fail_on.as_deref() == Some(command.as_str()) {
                return Err(BenchError::Serve("the server ended".into()));
            }
            Ok(ToolOutcome {
                output: format!("ran {command}"),
                loaded_groups: if command == "git" {
                    vec![("git".into(), "git group".into())]
                } else {
                    Vec::new()
                },
                ..ToolOutcome::default()
            })
        }

        fn finish(self: Box<Self>) {}
    }

    fn mock(steps: Value) -> MockModel {
        serde_json::from_value(json!({"format": "nurt-bench-mock/1", "default": steps})).unwrap()
    }

    fn run(steps: Value, limits: Limits, fail_on: Option<&str>) -> (AgentResult, Vec<Event>) {
        let tool_spec = ToolSpec {
            name: "bash".into(),
            description: "Runs commands.".into(),
            input_schema: json!({}),
        };
        let key = RunKey {
            task: "t".into(),
            condition: Condition::Bash,
            seed: 1,
        };
        let mut model = mock(steps).client(&key);
        let mut tool = EchoTool {
            fail_on: fail_on.map(str::to_owned),
        };
        let mut events = Vec::new();
        let result = run_agent(
            AgentSetup {
                system: "System.",
                tool: &tool_spec,
                task: "Do it.",
                limits,
                seed: 1,
                max_output_chars: 100,
            },
            &mut model,
            &mut tool,
            &mut |event| {
                events.push(event);
                Ok(())
            },
        )
        .unwrap();
        (result, events)
    }

    const LIMITS: Limits = Limits {
        steps: 15,
        seconds: 60,
    };

    #[test]
    fn ends_when_the_model_answers() {
        let steps = json!([
            {"text": "Step.", "tool_calls": [{"input": {"command": "a"}}, {"input": {"command": "git"}}]},
            {"text": "Finished.", "usage": {"input": 5, "output": 2}}
        ]);
        let (result, events) = run(steps, LIMITS, None);
        assert_eq!(result.outcome, Outcome::Answered);
        assert_eq!(result.steps, 2);
        assert_eq!(result.tool_calls, 2);
        assert_eq!(result.answer, "Finished.");
        assert!(result.tokens.input > 5);
        let kinds: Vec<&str> = events
            .iter()
            .map(|event| match event {
                Event::Model(_) => "model",
                Event::Tool(_) => "tool",
                Event::Group(_) => "group",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["model", "tool", "group", "tool", "model"]);
        let Event::Tool(tool) = &events[3] else {
            panic!()
        };
        assert!(
            tool.output
                .ends_with("[description of the group git]\ngit group")
        );
        let Event::Group(group) = &events[2] else {
            panic!()
        };
        assert_eq!(group.tokens, Some(3));
    }

    #[test]
    fn stops_at_the_step_limit() {
        let step = json!({"text": "Again.", "tool_calls": [{"input": {"command": "a"}}]});
        let limits = Limits {
            steps: 3,
            seconds: 60,
        };
        let (result, _) = run(json!([step, step, step, step]), limits, None);
        assert_eq!(result.outcome, Outcome::StepLimit);
        assert_eq!(result.steps, 3);
        assert_eq!(result.tool_calls, 3);
    }

    /// Model that notes the time limit it was given and then fails after the
    /// time of the run has passed.
    struct SlowModel {
        limits: Vec<Duration>,
    }

    impl ModelClient for SlowModel {
        fn complete(&mut self, _request: &ChatRequest<'_>) -> Result<ChatResponse> {
            std::thread::sleep(Duration::from_millis(1200));
            Err(BenchError::Provider {
                provider: "slow".into(),
                message: "timed out".into(),
                retryable: true,
            })
        }

        fn count_tokens(&mut self, _text: &str) -> Result<Option<u64>> {
            Ok(None)
        }

        fn set_time_limit(&mut self, remaining: Duration) {
            self.limits.push(remaining);
        }
    }

    #[test]
    fn gives_the_model_the_time_left_and_reports_a_call_that_outlasts_it_as_the_time_limit() {
        let tool_spec = ToolSpec {
            name: "bash".into(),
            description: "Runs commands.".into(),
            input_schema: json!({}),
        };
        let mut model = SlowModel { limits: Vec::new() };
        let mut tool = EchoTool { fail_on: None };
        let result = run_agent(
            AgentSetup {
                system: "System.",
                tool: &tool_spec,
                task: "Do it.",
                limits: Limits {
                    steps: 3,
                    seconds: 1,
                },
                seed: 1,
                max_output_chars: 100,
            },
            &mut model,
            &mut tool,
            &mut |_| Ok(()),
        )
        .unwrap();
        assert_eq!(model.limits.len(), 1);
        assert!(model.limits[0] <= Duration::from_secs(1));
        assert!(!model.limits[0].is_zero());
        assert_eq!(result.outcome, Outcome::TimeLimit);
        assert_eq!(result.error, None);
    }

    #[test]
    fn ends_with_an_error_when_the_tool_executor_fails() {
        let steps = json!([{"tool_calls": [{"input": {"command": "boom"}}]}]);
        let (result, events) = run(steps, LIMITS, Some("boom"));
        assert_eq!(result.outcome, Outcome::Error);
        assert!(result.error.unwrap().contains("server ended"));
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn one_tool_call_never_gets_more_than_its_own_limit() {
        let hour = Duration::from_secs(3600);
        assert_eq!(tool_call_limit(hour), TOOL_CALL_LIMIT);
        let short = Duration::from_secs(5);
        assert_eq!(tool_call_limit(short), short);
        assert_eq!(tool_call_limit(Duration::ZERO), Duration::ZERO);
    }

    #[test]
    fn clips_long_tool_results() {
        let long = "x".repeat(500);
        let steps = json!([{"tool_calls": [{"input": {"command": long}}]}, {"text": "ok"}]);
        let (_, events) = run(steps, LIMITS, None);
        let Event::Tool(tool) = &events[1] else {
            panic!()
        };
        assert!(tool.output.contains("characters omitted"));
    }

    #[test]
    fn check_before_run_with_groups_on_demand_is_not_a_tool_call() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        use crate::rpc::testing::connect;
        use crate::tool::NurtTool;

        let checks = Arc::new(AtomicU32::new(0));
        let runs = Arc::new(AtomicU32::new(0));
        let (seen_checks, seen_runs) = (Arc::clone(&checks), Arc::clone(&runs));
        let client = connect(Arc::new(move |method: &str, params: &Value| match method {
            "check" => {
                seen_checks.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"groups": ["core", {"name": "git"}]}))
            }
            "describe" => Ok(json!({"text": format!("about {}", params["name"])})),
            "run" => {
                seen_runs.fetch_add(1, Ordering::SeqCst);
                Ok(
                    json!({"nurt": "1.0", "report": {"nurt": "1.0", "status": "ok", "kind": "list"}}),
                )
            }
            _ => Err((-32601, "unknown".to_owned())),
        }));
        let mut tool = NurtTool::new(client, std::path::Path::new("/task"), true);
        let tool_spec = ToolSpec {
            name: "nurt".into(),
            description: "Runs programs.".into(),
            input_schema: json!({}),
        };
        let key = RunKey {
            task: "t".into(),
            condition: Condition::NurtGroups,
            seed: 1,
        };
        let steps = json!([
            {"text": "Step.", "tool_calls": [{"input": {"program": "list ."}}]},
            {"text": "Done."}
        ]);
        let mut model = mock(steps).client(&key);
        let result = run_agent(
            AgentSetup {
                system: "System.",
                tool: &tool_spec,
                task: "Do it.",
                limits: LIMITS,
                seed: 1,
                max_output_chars: 100,
            },
            &mut model,
            &mut tool,
            &mut |_| Ok(()),
        )
        .unwrap();
        assert_eq!(checks.load(Ordering::SeqCst), 1);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(result.tool_calls, 1);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
