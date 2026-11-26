// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Run log `nurt-bench-run/1`: one JSON object per line.
//!
//! A log records one run: the header, every model call and tool call, the
//! group descriptions loaded on demand, the verdict of the oracle and the
//! end of the run. The JSON Schema of an event is
//! `bench/protocol/schema/nurt-bench-run-1.schema.json`.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::condition::Condition;
use crate::error::{BenchError, Result};
use crate::model::{ChatResponse, ModelInfo};
use crate::oracle::{Observation, Verdict};
use crate::task::{HostOs, Limits, Platform};

/// Format identifier of a run log.
pub const RUN_FORMAT: &str = "nurt-bench-run/1";

/// Task of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskInfo {
    /// Identifier.
    pub id: String,
    /// Domain from 1 to 12.
    pub domain: u8,
    /// Systems the task runs on.
    pub platform: Platform,
    /// `public` or `holdout`.
    pub split: String,
    /// Shell failure class of a task of domain 12.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provocation: Option<String>,
}

/// Protocol files of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolInfo {
    /// Version of the protocol.
    pub version: String,
    /// SHA-256 of the system message as sent.
    pub system_message_sha256: String,
}

/// Tool of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInfo {
    /// Name of the tool.
    pub name: String,
    /// SHA-256 of the description.
    pub description_sha256: String,
    /// Characters of the description.
    pub description_chars: u64,
    /// Tokens of the description, when the provider counts tokens.
    #[serde(default)]
    pub description_tokens: Option<u64>,
}

/// First event of a log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunHeader {
    /// Format identifier, [`RUN_FORMAT`].
    pub format: String,
    /// Identifier `<model>/<condition>/<task>/s<seed>`.
    pub run_id: String,
    /// Task.
    pub task: TaskInfo,
    /// Condition.
    pub condition: Condition,
    /// Model.
    pub model: ModelInfo,
    /// Seed.
    pub seed: u64,
    /// Protocol.
    pub protocol: ProtocolInfo,
    /// Tool.
    pub tool: ToolInfo,
    /// Limits.
    pub limits: Limits,
    /// Operating system of the run.
    pub host: HostOs,
    /// Start time, RFC 3339.
    pub started: String,
    /// Version of `nurt-bench`.
    pub runner: String,
}

/// Model call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelEvent {
    /// Step number from 1.
    pub step: u32,
    /// Milliseconds since the start of the run when the response arrived.
    pub t_ms: u64,
    /// Duration of the call in milliseconds.
    pub latency_ms: u64,
    /// Response of the model.
    pub response: ChatResponse,
}

/// Danaco Nurt fields of a tool call, taken from the report.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NurtCall {
    /// `status` of the report.
    #[serde(default)]
    pub status: Option<String>,
    /// Code of the language error.
    #[serde(default)]
    pub error_code: Option<String>,
    /// Rules of the corrections applied to the program, for example `B03`.
    #[serde(default)]
    pub corrections: Vec<String>,
    /// Paths named by the changes of the report.
    #[serde(default)]
    pub changes: Vec<String>,
}

/// Tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolEvent {
    /// Step of the model call that requested the tool call.
    pub step: u32,
    /// Identifier of the call.
    pub call_id: String,
    /// Milliseconds since the start of the run when the call finished.
    pub t_ms: u64,
    /// Duration of the call in milliseconds.
    pub duration_ms: u64,
    /// Arguments of the call.
    pub input: Value,
    /// Text returned to the model.
    pub output: String,
    /// Whether the call failed: a non-zero exit code, a time limit, an
    /// invalid argument or a report with an error.
    pub is_error: bool,
    /// Exit code of a shell call.
    #[serde(default)]
    pub exit: Option<i32>,
    /// Whether the call exceeded its time limit.
    #[serde(default)]
    pub timed_out: bool,
    /// Report fields of a Danaco Nurt call.
    #[serde(default)]
    pub nurt: Option<NurtCall>,
}

/// Group description loaded on demand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupEvent {
    /// Step after which the description was added.
    pub step: u32,
    /// Name of the group.
    pub group: String,
    /// Characters of the description.
    pub chars: u64,
    /// Tokens of the description, when the provider counts tokens.
    #[serde(default)]
    pub tokens: Option<u64>,
}

/// Verdict of the oracle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OracleEvent {
    /// Final answer and probe results passed to the oracle.
    pub observation: Observation,
    /// Verdict.
    pub verdict: Verdict,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The model answered without a tool call.
    Answered,
    /// The step limit was reached.
    StepLimit,
    /// The time limit was reached.
    TimeLimit,
    /// The run failed for a reason outside the model: provider, tool
    /// process or oracle. Such runs are excluded from the analysis.
    Error,
}

/// Token totals of a run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    /// Input tokens of all model calls.
    pub input: u64,
    /// Output tokens of all model calls.
    pub output: u64,
}

/// Last event of a log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndEvent {
    /// How the run ended.
    pub outcome: Outcome,
    /// Model calls.
    pub steps: u32,
    /// Tool calls.
    pub tool_calls: u32,
    /// Token totals.
    pub tokens: Tokens,
    /// Wall clock time of the agent loop in milliseconds.
    pub wall_ms: u64,
    /// Description of the failure of a run with outcome `error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Event of a run log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// Header.
    Run(Box<RunHeader>),
    /// Model call.
    Model(Box<ModelEvent>),
    /// Tool call.
    Tool(Box<ToolEvent>),
    /// Group description loaded on demand.
    Group(GroupEvent),
    /// Verdict of the oracle.
    Oracle(Box<OracleEvent>),
    /// End of the run.
    End(EndEvent),
}

/// Reads the events of a log and checks its structure: a header of format
/// [`RUN_FORMAT`] first, at most one end event, and the end event last.
///
/// # Errors
///
/// Returns a message naming the first invalid line.
pub fn parse(text: &str) -> std::result::Result<Vec<Event>, String> {
    let mut events = Vec::new();
    for (number, line) in text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
    {
        let event: Event =
            serde_json::from_str(line).map_err(|error| format!("line {}: {error}", number + 1))?;
        events.push(event);
    }
    match events.first() {
        Some(Event::Run(header)) if header.format == RUN_FORMAT => {}
        Some(Event::Run(header)) => {
            return Err(format!("format {} is not {RUN_FORMAT}", header.format));
        }
        _ => return Err("the first event is not a run header".to_owned()),
    }
    let ends = events
        .iter()
        .filter(|event| matches!(event, Event::End(_)))
        .count();
    if ends > 1 || (ends == 1 && !matches!(events.last(), Some(Event::End(_)))) {
        return Err("the end event must be the last event".to_owned());
    }
    Ok(events)
}

/// Reads a log file.
///
/// # Errors
///
/// Returns an error when the file cannot be read or is invalid.
pub fn read(path: &Path) -> Result<Vec<Event>> {
    let text = fs::read_to_string(path).map_err(|error| BenchError::io(path, error))?;
    parse(&text).map_err(|message| BenchError::format(path, "run log", message))
}

/// Whether a log file exists and ends with an end event.
#[must_use]
pub fn is_complete(path: &Path) -> bool {
    read(path).is_ok_and(|events| matches!(events.last(), Some(Event::End(_))))
}

/// Whether a log is complete and its run did not end with an error: only
/// such a log is kept when a series is repeated; a run that failed is run
/// again.
#[must_use]
pub fn is_final(path: &Path) -> bool {
    read(path).is_ok_and(
        |events| matches!(events.last(), Some(Event::End(end)) if end.outcome != Outcome::Error),
    )
}

/// Writer of a run log. Events are written to `<path>.partial` and the file
/// is renamed to its final name by [`RunLog::finish`], so that an
/// interrupted run leaves no log that looks complete.
pub struct RunLog {
    path: PathBuf,
    partial: PathBuf,
    writer: BufWriter<File>,
}

impl RunLog {
    /// Creates the log file and its directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be created.
    pub fn create(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| BenchError::io(parent, error))?;
        }
        let mut partial = path.as_os_str().to_owned();
        partial.push(".partial");
        let partial = PathBuf::from(partial);
        let file = File::create(&partial).map_err(|error| BenchError::io(&partial, error))?;
        Ok(Self {
            path: path.to_path_buf(),
            partial,
            writer: BufWriter::new(file),
        })
    }

    /// Appends an event.
    ///
    /// # Errors
    ///
    /// Returns an error when the event cannot be written.
    pub fn write(&mut self, event: &Event) -> Result<()> {
        let line = serde_json::to_string(event)
            .map_err(|error| BenchError::format(&self.partial, "run log", error.to_string()))?;
        writeln!(self.writer, "{line}").map_err(|error| BenchError::io(&self.partial, error))
    }

    /// Flushes the log and gives it its final name.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be written or renamed.
    pub fn finish(mut self) -> Result<()> {
        self.writer
            .flush()
            .map_err(|error| BenchError::io(&self.partial, error))?;
        fs::rename(&self.partial, &self.path).map_err(|error| BenchError::io(&self.path, error))
    }
}

/// Paths of every complete log below a directory, sorted.
///
/// # Errors
///
/// Returns an error when a directory cannot be read.
pub fn find_logs(root: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let listing =
            fs::read_dir(&directory).map_err(|error| BenchError::io(&directory, error))?;
        for item in listing {
            let item = item.map_err(|error| BenchError::io(&directory, error))?;
            let path = item.path();
            // A link to a directory is not followed, so that a cycle ends.
            let is_directory = item
                .file_type()
                .map_err(|error| BenchError::io(&path, error))?
                .is_dir();
            if is_directory {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "jsonl")
            {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{Provider, StopReason, ToolCall, Usage, Weights};
    use serde_json::json;

    pub(crate) fn header(condition: Condition) -> RunHeader {
        RunHeader {
            format: RUN_FORMAT.to_owned(),
            run_id: format!("mock/{condition}/edit-001/s1"),
            task: TaskInfo {
                id: "edit-001".into(),
                domain: 3,
                platform: Platform::Linux,
                split: "public".into(),
                provocation: None,
            },
            condition,
            model: ModelInfo {
                id: "mock".into(),
                provider: Provider::Mock,
                model: "mock".into(),
                producer: "none".into(),
                weights: Weights::Open,
                temperature: 0.0,
                max_output_tokens: 0,
            },
            seed: 1,
            protocol: ProtocolInfo {
                version: "1.0".into(),
                system_message_sha256: "0".repeat(64),
            },
            tool: ToolInfo {
                name: "bash".into(),
                description_sha256: "0".repeat(64),
                description_chars: 10,
                description_tokens: Some(3),
            },
            limits: Limits {
                steps: 15,
                seconds: 60,
            },
            host: HostOs::Linux,
            started: "2026-01-01T00:00:00Z".into(),
            runner: "0.1.0".into(),
        }
    }

    pub(crate) fn model_event(step: u32, text: &str, command: Option<&str>) -> Event {
        Event::Model(Box::new(ModelEvent {
            step,
            t_ms: 10,
            latency_ms: 5,
            response: ChatResponse {
                text: text.into(),
                tool_calls: command
                    .map(|command| ToolCall {
                        id: format!("c{step}"),
                        input: json!({ "command": command }),
                    })
                    .into_iter()
                    .collect(),
                usage: Usage {
                    input: 100,
                    output: 10,
                },
                stop: if command.is_some() {
                    StopReason::ToolUse
                } else {
                    StopReason::End
                },
            },
        }))
    }

    pub(crate) fn end_event(outcome: Outcome) -> Event {
        Event::End(EndEvent {
            outcome,
            steps: 2,
            tool_calls: 1,
            tokens: Tokens {
                input: 200,
                output: 20,
            },
            wall_ms: 50,
            error: None,
        })
    }

    #[test]
    fn writes_and_reads_a_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b/run.jsonl");
        let mut log = RunLog::create(&path).unwrap();
        log.write(&Event::Run(Box::new(header(Condition::Bash))))
            .unwrap();
        log.write(&model_event(1, "", Some("ls"))).unwrap();
        log.write(&end_event(Outcome::Answered)).unwrap();
        assert!(!path.exists());
        log.finish().unwrap();
        assert!(is_complete(&path));
        let events = read(&path).unwrap();
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[0], Event::Run(h) if h.condition == Condition::Bash));
        assert_eq!(find_logs(dir.path()).unwrap(), vec![path.clone()]);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path(), dir.path().join("loop")).unwrap();
            assert_eq!(find_logs(dir.path()).unwrap(), vec![path]);
        }
    }

    #[test]
    fn rejects_logs_without_header_or_with_misplaced_end() {
        let end = serde_json::to_string(&end_event(Outcome::Answered)).unwrap();
        let run = serde_json::to_string(&Event::Run(Box::new(header(Condition::Nurt)))).unwrap();
        let model = serde_json::to_string(&model_event(1, "x", None)).unwrap();
        assert!(parse(&end).is_err());
        assert!(parse(&format!("{run}\n{end}\n{model}")).is_err());
        assert!(parse(&format!("{run}\n{end}\n{end}")).is_err());
        assert!(parse(&format!("{run}\nnot json")).is_err());
        assert!(parse(&format!("{run}\n\n{model}\n")).is_ok());
        let mut foreign = header(Condition::Nurt);
        foreign.format = "x/1".into();
        let foreign = serde_json::to_string(&Event::Run(Box::new(foreign))).unwrap();
        assert!(parse(&foreign).is_err());
        assert!(!is_complete(Path::new("/nonexistent/run.jsonl")));
    }

    #[test]
    fn a_log_of_a_run_that_ended_with_an_error_is_not_final() {
        let dir = tempfile::tempdir().unwrap();
        for (name, outcome) in [("answered", Outcome::Answered), ("error", Outcome::Error)] {
            let path = dir.path().join(format!("{name}.jsonl"));
            let mut log = RunLog::create(&path).unwrap();
            log.write(&Event::Run(Box::new(header(Condition::Bash))))
                .unwrap();
            log.write(&end_event(outcome)).unwrap();
            log.finish().unwrap();
            assert!(is_complete(&path), "{name}");
            assert_eq!(is_final(&path), outcome != Outcome::Error, "{name}");
        }
        assert!(!is_final(&dir.path().join("absent.jsonl")));
    }

    #[test]
    fn events_are_tagged() {
        let value = serde_json::to_value(end_event(Outcome::StepLimit)).unwrap();
        assert_eq!(value["event"], "end");
        assert_eq!(value["outcome"], "step_limit");
    }
}
