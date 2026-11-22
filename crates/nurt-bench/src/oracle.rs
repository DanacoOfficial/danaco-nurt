// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Evaluation of a run by the oracle of the task set.
//!
//! The oracle of the task set (`bench/oracles/`) compares the snapshots
//! before and after a run, the final answer and the probe results with the
//! expectation of the task. The runner starts it as a process: the process
//! reads one JSON object `{"task", "before", "after", "observation"}` on
//! standard input and writes the verdict `nurt-bench-verdict/1` on standard
//! output. The exit code is used only when the output is not a verdict: an
//! oracle may report a failed run with a non-zero code.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{BenchError, Result};
use crate::process::{self, ProcessSpec};
use crate::snapshot::Snapshot;

/// Format identifier of a verdict.
pub const VERDICT_FORMAT: &str = "nurt-bench-verdict/1";

/// Default command of the oracle, run in the oracle directory.
pub const DEFAULT_ORACLE_COMMAND: &str = "python3 -m nurt_bench_oracle";

/// Time limit of one oracle evaluation.
const ORACLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Unintended changes found by the oracle.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Harm {
    /// Entries changed outside the permitted set.
    #[serde(default)]
    pub unexpected: Vec<String>,
    /// Entries of the initial tree that no longer exist.
    #[serde(default)]
    pub lost: Vec<String>,
    /// Files whose encoding changed.
    #[serde(default)]
    pub encoding: Vec<String>,
    /// Files whose line ending style changed.
    #[serde(default)]
    pub line_endings: Vec<String>,
    /// Changes matching a `forbidden` rule of the task.
    #[serde(default)]
    pub forbidden: Vec<Value>,
}

/// Verdict of the oracle on one run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    /// Format identifier, [`VERDICT_FORMAT`].
    #[serde(default)]
    pub format: String,
    /// Identifier of the task.
    #[serde(default)]
    pub task: Option<String>,
    /// Whether the expectation holds.
    pub completed: bool,
    /// Whether an unintended or forbidden change happened.
    pub harmed: bool,
    /// Whether the run is completed and not harmed.
    #[serde(default)]
    pub passed: bool,
    /// Index of the alternative expectation that holds.
    #[serde(default)]
    pub alternative: Option<i64>,
    /// Failed conditions of the expectation.
    #[serde(default)]
    pub failures: Vec<String>,
    /// Details of the harm.
    #[serde(default)]
    pub harm: Harm,
    /// Changed paths with the kind of change.
    #[serde(default)]
    pub changes: BTreeMap<String, String>,
}

/// State outside the file tree observed after a run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    /// Final answer of the model: the text of its last message.
    pub answer: String,
    /// Results of the probes by probe identifier.
    pub probes: BTreeMap<String, Value>,
}

/// Evaluator of runs.
pub trait Oracle: Send + Sync {
    /// Evaluates a run of a task.
    ///
    /// # Errors
    ///
    /// Returns an error when the run cannot be evaluated.
    fn judge(
        &self,
        task: &Value,
        before: &Snapshot,
        after: &Snapshot,
        observation: &Observation,
    ) -> Result<Verdict>;
}

/// Oracle started as a process for every evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOracle {
    argv: Vec<String>,
    directory: PathBuf,
}

impl CommandOracle {
    /// Creates an oracle from a command line run in `directory`.
    ///
    /// # Errors
    ///
    /// Returns an error when the command line is empty or malformed.
    pub fn new(command: &str, directory: PathBuf) -> Result<Self> {
        Ok(Self {
            argv: process::split_command(command)?,
            directory,
        })
    }
}

impl Oracle for CommandOracle {
    fn judge(
        &self,
        task: &Value,
        before: &Snapshot,
        after: &Snapshot,
        observation: &Observation,
    ) -> Result<Verdict> {
        let input = json!({
            "task": task,
            "before": before,
            "after": after,
            "observation": observation,
        });
        let mut spec = ProcessSpec::new(
            &self.argv[0],
            self.argv[1..].to_vec(),
            self.directory.clone(),
            ORACLE_TIMEOUT,
        );
        spec.stdin = Some(input.to_string().into_bytes());
        let output = process::run(&spec)?;
        match parse_verdict(&output.stdout) {
            Err(_) if !output.success() => Err(BenchError::Oracle(format!(
                "the oracle exited with {:?}: {}",
                output.code,
                String::from_utf8_lossy(&output.stderr).trim()
            ))),
            result => result,
        }
    }
}

/// Reads a verdict from the output of an oracle.
///
/// # Errors
///
/// Returns [`BenchError::Oracle`] when the output is not a verdict.
pub fn parse_verdict(output: &[u8]) -> Result<Verdict> {
    let verdict: Verdict = serde_json::from_slice(output)
        .map_err(|error| BenchError::Oracle(format!("the output is not a verdict: {error}")))?;
    if verdict.format != VERDICT_FORMAT {
        return Err(BenchError::Oracle(format!(
            "the verdict format is {:?}, expected {VERDICT_FORMAT}",
            verdict.format
        )));
    }
    Ok(verdict)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERDICT: &str = r#"{"format":"nurt-bench-verdict/1","task":"t","completed":true,
        "harmed":true,"passed":false,"alternative":null,"failures":[],
        "harm":{"unexpected":["x"],"lost":[],"encoding":[],"line_endings":[],"forbidden":[]},
        "changes":{"x":"create"}}"#;

    fn empty() -> Snapshot {
        Snapshot {
            format: None,
            exhaustive: None,
            entries: Vec::new(),
        }
    }

    #[test]
    fn parses_a_verdict() {
        let verdict = parse_verdict(VERDICT.as_bytes()).unwrap();
        assert!(verdict.completed);
        assert!(verdict.harmed);
        assert_eq!(verdict.harm.unexpected, ["x"]);
        assert_eq!(verdict.changes["x"], "create");
    }

    #[test]
    fn rejects_output_that_is_not_a_verdict() {
        assert!(parse_verdict(b"not json").is_err());
        assert!(parse_verdict(br#"{"format":"other","completed":true,"harmed":false}"#).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn command_oracle_sends_the_run_on_standard_input() {
        let script = r#"read -r line; case "$line" in *'"answer":"done"'*) cat verdict.json;; *'"answer":"failed"'*) cat verdict.json; exit 1;; *) exit 4;; esac"#;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("oracle.sh"), script).unwrap();
        std::fs::write(dir.path().join("verdict.json"), VERDICT).unwrap();
        let oracle = CommandOracle::new("sh oracle.sh", dir.path().to_path_buf()).unwrap();
        let observation = Observation {
            answer: "done".into(),
            probes: BTreeMap::new(),
        };
        let verdict = oracle
            .judge(&json!({"id": "t"}), &empty(), &empty(), &observation)
            .unwrap();
        assert!(verdict.completed);
        let failed = Observation {
            answer: "failed".into(),
            probes: BTreeMap::new(),
        };
        let verdict = oracle
            .judge(&json!({"id": "t"}), &empty(), &empty(), &failed)
            .unwrap();
        assert!(verdict.harmed);
        let other = Observation::default();
        let error = oracle
            .judge(&json!({}), &empty(), &empty(), &other)
            .unwrap_err();
        assert!(error.to_string().contains("exited"));
    }
}
