// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Mock model: scripted or recorded responses without a provider.
//!
//! `--model mock:<file>` reads either a script `nurt-bench-mock/1` (JSON) or
//! a run log `nurt-bench-run/1` (JSON Lines), whose model responses are
//! replayed in order. Token counts that a script does not give are
//! estimated as one token per [`CHARS_PER_TOKEN`] characters.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    ChatRequest, ChatResponse, ModelClient, ModelInfo, Part, Provider, RunKey, StopReason,
    ToolCall, Usage, Weights,
};
use crate::error::{BenchError, Result};
use crate::runlog::{self, Event};
use crate::task::matches_filter;

/// Format identifier of a mock script.
pub const MOCK_FORMAT: &str = "nurt-bench-mock/1";

/// Characters per token of the token estimate of the mock model.
pub const CHARS_PER_TOKEN: u64 = 4;

/// Tool call of a scripted step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptedCall {
    /// Identifier; generated when absent.
    #[serde(default)]
    pub id: Option<String>,
    /// Arguments of the call.
    pub input: Value,
}

/// One scripted response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Text of the response.
    #[serde(default)]
    pub text: String,
    /// Tool calls of the response.
    #[serde(default)]
    pub tool_calls: Vec<ScriptedCall>,
    /// Token counts; estimated when absent.
    #[serde(default)]
    pub usage: Option<Usage>,
}

/// Responses for the runs that match a task filter, a condition and a seed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Script {
    /// Task identifier or pattern with `*`, see
    /// [`matches_filter`](crate::task::matches_filter); `*` matches every task.
    pub task: String,
    /// Condition; any condition when absent.
    #[serde(default)]
    pub condition: Option<String>,
    /// Seed; any seed when absent.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Responses in order.
    pub steps: Vec<Step>,
}

/// Mock model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MockModel {
    /// Format identifier, [`MOCK_FORMAT`].
    pub format: String,
    /// Label of the model in run logs.
    #[serde(default = "default_id")]
    pub id: String,
    /// Scripts, tried in order.
    #[serde(default)]
    pub scripts: Vec<Script>,
    /// Responses for runs without a matching script.
    #[serde(default)]
    pub default: Vec<Step>,
}

fn default_id() -> String {
    "mock".to_owned()
}

impl MockModel {
    /// Reads a mock script or a run log to replay.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read or is neither a script
    /// nor a run log.
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).map_err(|error| BenchError::io(path, error))?;
        if let Ok(model) = serde_json::from_str::<Self>(&text) {
            if model.format != MOCK_FORMAT {
                return Err(BenchError::format(
                    path,
                    "mock script",
                    format!("format must be {MOCK_FORMAT}"),
                ));
            }
            return Ok(model);
        }
        let events = runlog::parse(&text)
            .map_err(|message| BenchError::format(path, "mock script", message))?;
        Ok(Self::from_events(&events))
    }

    /// Builds a mock model that replays the model responses of a run log.
    #[must_use]
    pub fn from_events(events: &[Event]) -> Self {
        let steps = events
            .iter()
            .filter_map(|event| match event {
                Event::Model(model) => Some(Step {
                    text: model.response.text.clone(),
                    tool_calls: model
                        .response
                        .tool_calls
                        .iter()
                        .map(|call| ScriptedCall {
                            id: Some(call.id.clone()),
                            input: call.input.clone(),
                        })
                        .collect(),
                    usage: Some(model.response.usage),
                }),
                _ => None,
            })
            .collect();
        Self {
            format: MOCK_FORMAT.to_owned(),
            id: default_id(),
            scripts: Vec::new(),
            default: steps,
        }
    }

    /// Description of the model for run logs.
    #[must_use]
    pub fn info(&self) -> ModelInfo {
        ModelInfo {
            id: self.id.clone(),
            provider: Provider::Mock,
            model: "mock".to_owned(),
            producer: "none".to_owned(),
            weights: Weights::Open,
            temperature: 0.0,
            max_output_tokens: 0,
        }
    }

    /// Creates the client of one run with the steps of the first matching
    /// script.
    #[must_use]
    pub fn client(&self, key: &RunKey) -> MockClient {
        let steps = self
            .scripts
            .iter()
            .find(|script| {
                (script.task == "*" || matches_filter(&key.task, &script.task))
                    && script
                        .condition
                        .as_deref()
                        .is_none_or(|condition| condition == key.condition.as_str())
                    && script.seed.is_none_or(|seed| seed == key.seed)
            })
            .map_or_else(|| self.default.clone(), |script| script.steps.clone());
        MockClient {
            steps,
            position: 0,
            calls: 0,
        }
    }
}

/// Client of the mock model for one run.
#[derive(Debug, Clone)]
pub struct MockClient {
    steps: Vec<Step>,
    position: usize,
    calls: u64,
}

fn estimate(chars: usize) -> u64 {
    u64::try_from(chars)
        .unwrap_or(u64::MAX)
        .div_ceil(CHARS_PER_TOKEN)
}

fn chars(text: &str) -> usize {
    text.chars().count()
}

fn request_chars(request: &ChatRequest<'_>) -> usize {
    let parts: usize = request
        .messages
        .iter()
        .flat_map(|message| &message.parts)
        .map(|part| match part {
            Part::Text { text } => chars(text),
            Part::ToolCall { input, .. } => chars(&input.to_string()),
            Part::ToolResult { output, .. } => chars(output),
        })
        .sum();
    chars(request.system) + chars(&request.tool.description) + parts
}

impl ModelClient for MockClient {
    fn complete(&mut self, request: &ChatRequest<'_>) -> Result<ChatResponse> {
        let step = self.steps.get(self.position).cloned().unwrap_or_default();
        self.position += 1;
        let tool_calls: Vec<ToolCall> = step
            .tool_calls
            .into_iter()
            .map(|call| {
                self.calls += 1;
                ToolCall {
                    id: call.id.unwrap_or_else(|| format!("mock-{}", self.calls)),
                    input: call.input,
                }
            })
            .collect();
        let output_chars = chars(&step.text)
            + tool_calls
                .iter()
                .map(|call| chars(&call.input.to_string()))
                .sum::<usize>();
        let usage = step.usage.unwrap_or(Usage {
            input: estimate(request_chars(request)),
            output: estimate(output_chars),
        });
        let stop = if tool_calls.is_empty() {
            StopReason::End
        } else {
            StopReason::ToolUse
        };
        Ok(ChatResponse {
            text: step.text,
            tool_calls,
            usage,
            stop,
        })
    }

    fn count_tokens(&mut self, text: &str) -> Result<Option<u64>> {
        Ok(Some(estimate(chars(text))))
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Message, ToolSpec};
    use super::*;
    use crate::condition::Condition;
    use serde_json::json;

    fn key(task: &str, condition: Condition, seed: u64) -> RunKey {
        RunKey {
            task: task.into(),
            condition,
            seed,
        }
    }

    fn model() -> MockModel {
        serde_json::from_value(json!({
            "format": "nurt-bench-mock/1",
            "id": "scripted",
            "scripts": [
                {"task": "edit-*", "condition": "nurt", "steps": [
                    {"text": "Writing.", "tool_calls": [{"input": {"program": "write"}}]},
                    {"text": "Done.", "usage": {"input": 7, "output": 3}}
                ]},
                {"task": "edit-001", "seed": 2, "steps": [{"text": "Seed two."}]}
            ],
            "default": [{"text": "No script."}]
        }))
        .unwrap()
    }

    fn ask(client: &mut MockClient) -> ChatResponse {
        let tool = ToolSpec {
            name: "nurt".into(),
            description: "abcd".into(),
            input_schema: json!({}),
        };
        let messages = vec![Message::user_text("12345678")];
        client
            .complete(&ChatRequest {
                system: "abcd",
                tool: &tool,
                messages: &messages,
                seed: 1,
            })
            .unwrap()
    }

    #[test]
    fn plays_the_matching_script_in_order() {
        let mut client = model().client(&key("edit-001", Condition::Nurt, 1));
        let first = ask(&mut client);
        assert_eq!(first.text, "Writing.");
        assert_eq!(first.tool_calls[0].id, "mock-1");
        assert_eq!(first.stop, StopReason::ToolUse);
        assert_eq!(first.usage.input, 4);
        let second = ask(&mut client);
        assert_eq!(
            second.usage,
            Usage {
                input: 7,
                output: 3
            }
        );
        assert_eq!(second.stop, StopReason::End);
        let after = ask(&mut client);
        assert!(after.text.is_empty() && after.tool_calls.is_empty());
    }

    #[test]
    fn selects_scripts_by_condition_and_seed() {
        let mut bash = model().client(&key("edit-001", Condition::Bash, 2));
        assert_eq!(ask(&mut bash).text, "Seed two.");
        let mut other = model().client(&key("git-001", Condition::Nurt, 1));
        assert_eq!(ask(&mut other).text, "No script.");
        assert_eq!(model().info().id, "scripted");
        assert_eq!(other.count_tokens("12345").unwrap(), Some(2));
    }

    #[test]
    fn token_estimates_count_characters_not_bytes() {
        let mut client = model().client(&key("git-001", Condition::Nurt, 1));
        // Four characters of two bytes each are one token, not two.
        assert_eq!(client.count_tokens("żółw").unwrap(), Some(1));
        assert_eq!(client.count_tokens("żółwia").unwrap(), Some(2));
        assert_eq!(client.count_tokens("").unwrap(), Some(0));
    }

    #[test]
    fn loads_scripts_and_rejects_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("mock.json");
        fs::write(&script, serde_json::to_string(&model()).unwrap()).unwrap();
        assert_eq!(MockModel::load(&script).unwrap(), model());
        let wrong = dir.path().join("wrong.json");
        fs::write(&wrong, r#"{"format": "other/1"}"#).unwrap();
        assert!(MockModel::load(&wrong).is_err());
        let garbage = dir.path().join("garbage.json");
        fs::write(&garbage, "[1, 2]").unwrap();
        assert!(MockModel::load(&garbage).is_err());
        assert!(MockModel::load(&dir.path().join("absent.json")).is_err());
    }
}
