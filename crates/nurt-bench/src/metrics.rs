// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Measures of one run, derived from its log.

use std::collections::BTreeSet;

use nurt_contracts::error::{ErrorCode, Stage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::condition::Condition;
use crate::oracle::Verdict;
use crate::runlog::{Event, NurtCall, Outcome, RunHeader, ToolEvent};
use crate::task::Platform;

/// Interpreters whose use counts as an intermediate script.
const INTERPRETERS: [&str; 8] = ["python", "py", "perl", "ruby", "node", "php", "deno", "bun"];

/// Shells whose use from a Danaco Nurt program counts as an intermediate
/// script.
const SHELLS: [&str; 6] = ["bash", "sh", "zsh", "pwsh", "powershell", "cmd"];

/// Characters that separate words of a shell command.
const SEPARATORS: [char; 10] = [';', '&', '|', '(', ')', '`', '"', '\'', '{', '}'];

/// Counts of the Danaco Nurt calls of a run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NurtCounts {
    /// Calls with a report.
    pub calls: u32,
    /// Calls written correctly: no correction and no notation error (FV).
    pub valid: u32,
    /// Calls that ran thanks to a correction (FC).
    pub corrected: u32,
    /// Corrected calls whose changes touched a path that the oracle found
    /// harmed (FCR).
    pub false_corrections: u32,
    /// Calls that ended with `E_AMBIGUOUS` (AMB).
    pub ambiguous: u32,
}

/// Measures of one run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunMeasures {
    /// Identifier of the run.
    pub run_id: String,
    /// Model label.
    pub model: String,
    /// Organization that trained the model.
    pub producer: String,
    /// Condition.
    pub condition: Condition,
    /// Task identifier.
    pub task: String,
    /// Domain of the task.
    pub domain: u8,
    /// Platform of the task.
    pub platform: Platform,
    /// Shell failure class of a task of domain 12.
    pub provocation: Option<String>,
    /// Seed.
    pub seed: u64,
    /// How the run ended.
    pub outcome: Outcome,
    /// Whether the expectation holds.
    pub completed: bool,
    /// Whether an unintended or forbidden change happened.
    pub harmed: bool,
    /// Whether the run is completed and not harmed.
    pub passed: bool,
    /// Entries changed outside the permitted set.
    pub unexpected: usize,
    /// Entries of the initial tree lost.
    pub lost: usize,
    /// Files whose encoding or line ending style changed.
    pub encoding: usize,
    /// Forbidden changes.
    pub forbidden: usize,
    /// Model calls.
    pub steps: u32,
    /// Tool calls.
    pub tool_calls: u32,
    /// Input tokens.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
    /// Tokens of the tool description and of the loaded group descriptions,
    /// when the provider counts tokens.
    pub description_tokens: Option<u64>,
    /// Characters of the tool description and of the loaded group
    /// descriptions.
    pub description_chars: u64,
    /// Wall clock time of the agent loop in milliseconds.
    pub wall_ms: u64,
    /// Whether the model used an intermediate script.
    pub script: bool,
    /// Tool calls that directly follow a failed tool call.
    pub retries: u32,
    /// Counts of the Danaco Nurt calls.
    pub nurt: NurtCounts,
}

fn word_name(word: &str) -> &str {
    let base = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let base = base.strip_suffix(".exe").unwrap_or(base);
    base.trim_end_matches(|character: char| character.is_ascii_digit() || character == '.')
}

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|character: char| character.is_whitespace() || SEPARATORS.contains(&character))
        .filter(|word| !word.is_empty())
}

/// Whether a shell command starts an interpreter of another language.
///
/// ```
/// use nurt_bench::metrics::shell_uses_script;
///
/// assert!(shell_uses_script("cat a | /usr/bin/python3.12 -c 'print(1)'"));
/// assert!(!shell_uses_script("grep -r python src"));
/// ```
#[must_use]
pub fn shell_uses_script(command: &str) -> bool {
    command
        .split(['\n', ';', '&', '|', '(', '`'])
        .filter_map(|segment| words(segment).next())
        .any(|first| INTERPRETERS.contains(&word_name(first)))
}

/// Whether a Danaco Nurt program runs an interpreter or a shell with `run`.
///
/// ```
/// use nurt_bench::metrics::nurt_uses_script;
///
/// assert!(nurt_uses_script(r#"run "python3" args: ["fix.py"]"#));
/// assert!(!nurt_uses_script(r#"replace "a" "b" in: ["run.py"]"#));
/// ```
#[must_use]
pub fn nurt_uses_script(program: &str) -> bool {
    let tokens: Vec<&str> = words(program).collect();
    tokens.windows(2).any(|pair| {
        pair[0] == "run" && {
            let name = word_name(pair[1]);
            INTERPRETERS.contains(&name) || SHELLS.contains(&name)
        }
    })
}

/// Whether an error code is a notation error: it occurs only while the
/// program is parsed or bound.
#[must_use]
pub fn is_notation_error(code: &str) -> bool {
    code.parse::<ErrorCode>().is_ok_and(|code| {
        code.stages()
            .iter()
            .all(|stage| matches!(stage, Stage::Parse | Stage::Bind))
    })
}

fn harmed_paths(verdict: &Verdict) -> BTreeSet<&str> {
    let harm = &verdict.harm;
    let forbidden = harm
        .forbidden
        .iter()
        .filter_map(|rule| rule.get("path").and_then(Value::as_str));
    harm.unexpected
        .iter()
        .chain(&harm.lost)
        .map(String::as_str)
        .chain(forbidden)
        .collect()
}

fn count_nurt(calls: &[&NurtCall], verdict: Option<&Verdict>) -> NurtCounts {
    let harmed = verdict.map(harmed_paths).unwrap_or_default();
    let mut counts = NurtCounts::default();
    for call in calls {
        counts.calls += 1;
        let notation = call.error_code.as_deref().is_some_and(is_notation_error);
        let ambiguous = call.error_code.as_deref() == Some(ErrorCode::Ambiguous.as_str());
        counts.ambiguous += u32::from(ambiguous);
        if notation {
            continue;
        }
        if call.corrections.is_empty() {
            counts.valid += 1;
        } else {
            counts.corrected += 1;
            let touched = call
                .changes
                .iter()
                .any(|path| harmed.contains(path.as_str()));
            counts.false_corrections += u32::from(touched);
        }
    }
    counts
}

fn uses_script(condition: Condition, tool: &ToolEvent) -> bool {
    let text = ["command", "program"]
        .iter()
        .find_map(|key| tool.input.get(*key).and_then(Value::as_str))
        .unwrap_or_default();
    if condition.is_nurt() {
        nurt_uses_script(text)
    } else {
        shell_uses_script(text)
    }
}

impl RunMeasures {
    /// Derives the measures of a complete log; `None` when the log has no
    /// header or no end event.
    #[must_use]
    pub fn from_events(events: &[Event]) -> Option<Self> {
        let Some(Event::Run(header)) = events.first() else {
            return None;
        };
        let Some(Event::End(end)) = events.last() else {
            return None;
        };
        let header: &RunHeader = header;
        let tools: Vec<&ToolEvent> = events
            .iter()
            .filter_map(|event| match event {
                Event::Tool(tool) => Some(tool.as_ref()),
                _ => None,
            })
            .collect();
        let verdict = events.iter().find_map(|event| match event {
            Event::Oracle(oracle) => Some(&oracle.verdict),
            _ => None,
        });
        let groups: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                Event::Group(group) => Some(group),
                _ => None,
            })
            .collect();
        let description_tokens = groups
            .iter()
            .try_fold(header.tool.description_tokens?, |total, group| {
                group.tokens.map(|tokens| total + tokens)
            });
        let nurt_calls: Vec<&NurtCall> =
            tools.iter().filter_map(|tool| tool.nurt.as_ref()).collect();
        let retries = tools.windows(2).filter(|pair| pair[0].is_error).count();
        let harm = verdict.map(|verdict| &verdict.harm);
        Some(Self {
            run_id: header.run_id.clone(),
            model: header.model.id.clone(),
            producer: header.model.producer.clone(),
            condition: header.condition,
            task: header.task.id.clone(),
            domain: header.task.domain,
            platform: header.task.platform,
            provocation: header.task.provocation.clone(),
            seed: header.seed,
            outcome: end.outcome,
            completed: verdict.is_some_and(|verdict| verdict.completed),
            harmed: verdict.is_some_and(|verdict| verdict.harmed),
            passed: verdict.is_some_and(|verdict| verdict.passed),
            unexpected: harm.map_or(0, |harm| harm.unexpected.len()),
            lost: harm.map_or(0, |harm| harm.lost.len()),
            encoding: harm.map_or(0, |harm| harm.encoding.len() + harm.line_endings.len()),
            forbidden: harm.map_or(0, |harm| harm.forbidden.len()),
            steps: end.steps,
            tool_calls: end.tool_calls,
            input_tokens: end.tokens.input,
            output_tokens: end.tokens.output,
            description_tokens,
            description_chars: header.tool.description_chars
                + groups.iter().map(|group| group.chars).sum::<u64>(),
            wall_ms: end.wall_ms,
            script: tools.iter().any(|tool| uses_script(header.condition, tool)),
            retries: u32::try_from(retries).unwrap_or(u32::MAX),
            nurt: count_nurt(&nurt_calls, verdict),
        })
    }

    /// Input and output tokens.
    #[must_use]
    pub const fn tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::oracle::{Harm, Observation};
    use crate::runlog::tests::{end_event, header, model_event};
    use crate::runlog::{GroupEvent, OracleEvent};

    fn tool(step: u32, input: Value, is_error: bool, nurt: Option<NurtCall>) -> Event {
        Event::Tool(Box::new(ToolEvent {
            step,
            call_id: format!("c{step}"),
            t_ms: 1,
            duration_ms: 1,
            input,
            output: String::new(),
            is_error,
            exit: None,
            timed_out: false,
            nurt,
        }))
    }

    fn oracle(unexpected: &[&str]) -> Event {
        Event::Oracle(Box::new(OracleEvent {
            observation: Observation::default(),
            verdict: Verdict {
                format: crate::oracle::VERDICT_FORMAT.into(),
                completed: true,
                harmed: !unexpected.is_empty(),
                passed: unexpected.is_empty(),
                harm: Harm {
                    unexpected: unexpected.iter().map(|path| (*path).to_owned()).collect(),
                    ..Harm::default()
                },
                ..Verdict::default()
            },
        }))
    }

    #[test]
    fn detects_scripts_in_shell_commands() {
        assert!(shell_uses_script("python3 fix.py"));
        assert!(shell_uses_script("cd src && perl -pi -e 's/a/b/' x"));
        assert!(shell_uses_script("py.exe -3 x.py"));
        assert!(!shell_uses_script("sed -i 's/python/py/' a.txt"));
        assert!(!shell_uses_script("ls python"));
        assert!(nurt_uses_script("run \"bash\" args: [\"-c\", \"ls\"]"));
        assert!(!nurt_uses_script("run \"cargo\" args: [\"test\"]"));
    }

    #[test]
    fn classifies_notation_errors() {
        assert!(is_notation_error("E_AMBIGUOUS"));
        assert!(!is_notation_error("E_NOT_FOUND"));
        assert!(!is_notation_error("E_NO_SUCH_CODE"));
    }

    #[test]
    fn derives_measures_of_a_shell_run() {
        let events = vec![
            Event::Run(Box::new(header(Condition::Bash))),
            model_event(1, "", Some("python3 x.py")),
            tool(1, json!({"command": "python3 x.py"}), true, None),
            tool(1, json!({"command": "ls"}), false, None),
            model_event(2, "done", None),
            oracle(&["junk.txt"]),
            end_event(Outcome::Answered),
        ];
        let measures = RunMeasures::from_events(&events).unwrap();
        assert!(measures.completed && measures.harmed && !measures.passed);
        assert_eq!(measures.unexpected, 1);
        assert!(measures.script);
        assert_eq!(measures.retries, 1);
        assert_eq!(measures.tokens(), 220);
        assert_eq!(measures.description_tokens, Some(3));
        assert!(RunMeasures::from_events(&events[..3]).is_none());
        assert!(RunMeasures::from_events(&events[1..]).is_none());
    }

    #[test]
    fn counts_nurt_calls() {
        let call = |code: Option<&str>, corrections: &[&str], changes: &[&str]| NurtCall {
            status: Some("ok".into()),
            error_code: code.map(str::to_owned),
            corrections: corrections.iter().map(|rule| (*rule).to_owned()).collect(),
            changes: changes.iter().map(|path| (*path).to_owned()).collect(),
        };
        let events = vec![
            Event::Run(Box::new(header(Condition::NurtGroups))),
            tool(
                1,
                json!({"program": "x"}),
                false,
                Some(call(None, &[], &["a"])),
            ),
            tool(
                1,
                json!({"program": "x"}),
                false,
                Some(call(None, &["B03"], &["junk.txt"])),
            ),
            tool(
                1,
                json!({"program": "x"}),
                false,
                Some(call(None, &["L19"], &["b"])),
            ),
            tool(
                1,
                json!({"program": "x"}),
                true,
                Some(call(Some("E_AMBIGUOUS"), &[], &[])),
            ),
            tool(
                1,
                json!({"program": "x"}),
                true,
                Some(call(Some("E_NOT_FOUND"), &[], &[])),
            ),
            Event::Group(GroupEvent {
                step: 1,
                group: "git".into(),
                chars: 100,
                tokens: Some(30),
            }),
            oracle(&["junk.txt"]),
            end_event(Outcome::Answered),
        ];
        let measures = RunMeasures::from_events(&events).unwrap();
        assert_eq!(
            measures.nurt,
            NurtCounts {
                calls: 5,
                valid: 2,
                corrected: 2,
                false_corrections: 1,
                ambiguous: 1
            }
        );
        assert_eq!(measures.description_tokens, Some(33));
        assert_eq!(measures.description_chars, 110);
        assert!(!measures.script);
    }
}
