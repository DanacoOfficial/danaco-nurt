// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Task files in the format `nurt-bench-task/1`.
//!
//! The task set lives in `bench/tasks/`; its format is described in
//! `bench/tasks/README.md`. The runner reads the members it needs and passes
//! the whole task document to the oracle.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{BenchError, Result};

/// Format identifier of a task.
pub const TASK_FORMAT: &str = "nurt-bench-task/1";

/// Step limit of the protocol.
pub const DEFAULT_STEPS: u32 = 15;

/// Systems a task runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    /// Linux only.
    Linux,
    /// Windows only.
    Windows,
    /// Linux, macOS and Windows with one description and one expected state.
    Cross,
    /// macOS only.
    Macos,
}

/// Operating system of the machine that executes a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostOs {
    /// Linux.
    Linux,
    /// Windows.
    Windows,
    /// macOS.
    Macos,
}

impl HostOs {
    /// Operating system of the running program.
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Linux
        }
    }

    /// Name of the system as given to the model.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Linux => "Linux",
            Self::Windows => "Windows",
            Self::Macos => "macOS",
        }
    }
}

impl Platform {
    /// Whether a task of this platform runs on the given system.
    #[must_use]
    pub const fn runs_on(self, host: HostOs) -> bool {
        matches!(
            (self, host),
            (Self::Cross, _)
                | (Self::Linux, HostOs::Linux)
                | (Self::Windows, HostOs::Windows)
                | (Self::Macos, HostOs::Macos)
        )
    }
}

/// Scripts of `setup` or `teardown`, one per shell.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scripts {
    /// Description of the prepared environment.
    #[serde(default)]
    pub description: Option<String>,
    /// Script for bash.
    #[serde(default)]
    pub bash: Option<String>,
    /// Script for PowerShell 7.
    #[serde(default)]
    pub pwsh: Option<String>,
}

impl Scripts {
    /// Script for the shell of the given system.
    #[must_use]
    pub fn for_host(&self, host: HostOs) -> Option<&str> {
        match host {
            HostOs::Windows => self.pwsh.as_deref(),
            HostOs::Linux | HostOs::Macos => self.bash.as_deref(),
        }
    }
}

/// Probe of state outside the file tree, collected after the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Probe {
    /// Identifier referenced by `expect.probes`.
    pub id: String,
    /// Kind of the probe: `command`, `process`, `registry` or
    /// `scheduled_task`.
    pub kind: String,
    /// Program and arguments of a `command` probe.
    #[serde(default)]
    pub argv: Vec<String>,
    /// Text contained in the command line of a `process` probe.
    #[serde(default, rename = "match")]
    pub pattern: Option<String>,
    /// Registry key of a `registry` probe.
    #[serde(default)]
    pub key: Option<String>,
    /// Registry value of a `registry` probe.
    #[serde(default)]
    pub value: Option<String>,
    /// Task folder of a `scheduled_task` probe.
    #[serde(default)]
    pub path: Option<String>,
    /// Task name of a `scheduled_task` probe.
    #[serde(default)]
    pub name: Option<String>,
}

/// Limits of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    /// Largest number of model calls.
    pub steps: u32,
    /// Largest wall clock time of the run in seconds.
    pub seconds: u32,
}

/// Members of a task used by the runner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    /// Identifier, for example `search-007`.
    pub id: String,
    /// Domain from 1 to 12.
    pub domain: u8,
    /// Systems the task runs on.
    pub platform: Platform,
    /// `public` or `holdout`.
    pub split: String,
    /// Description given to the model.
    pub nl: String,
    /// Initial tree: a snapshot or a fixture reference.
    pub fs_before: Value,
    /// Preparation before the run.
    #[serde(default)]
    pub setup: Option<Scripts>,
    /// Scripts run after the probes, before the final snapshot.
    #[serde(default)]
    pub teardown: Option<Scripts>,
    /// Probes collected after the run.
    #[serde(default)]
    pub probes: Vec<Probe>,
    /// Limits of the run.
    pub limits: Limits,
    /// Reference program of Danaco Nurt in the canonical syntax.
    pub oracle_program: String,
    /// Reference solution in bash.
    #[serde(default)]
    pub reference_bash: Option<String>,
    /// Reference solution in PowerShell 7.
    #[serde(default)]
    pub reference_pwsh: Option<String>,
    /// Shell failure class of a task of domain 12.
    #[serde(default)]
    pub provocation: Option<String>,
    /// Classification labels.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Task with its source file and the full document.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskFile {
    /// Path of the task file.
    pub path: PathBuf,
    /// Members used by the runner.
    pub task: Task,
    /// The whole task document, passed to the oracle.
    pub document: Value,
}

impl TaskFile {
    /// Reads a task from a JSON document.
    ///
    /// # Errors
    ///
    /// Returns an error when the document is not a task.
    pub fn from_document(path: &Path, document: Value) -> Result<Self> {
        if document.get("format").and_then(Value::as_str) != Some(TASK_FORMAT) {
            return Err(BenchError::format(
                path,
                "task",
                format!("format is not {TASK_FORMAT}"),
            ));
        }
        let task: Task = serde_json::from_value(document.clone())
            .map_err(|error| BenchError::format(path, "task", error.to_string()))?;
        if task.limits.steps == 0 || task.limits.seconds == 0 {
            return Err(BenchError::format(path, "task", "limits must be positive"));
        }
        Ok(Self {
            path: path.to_path_buf(),
            task,
            document,
        })
    }

    /// Whether the task belongs to the held-out set.
    #[must_use]
    pub fn is_holdout(&self) -> bool {
        self.task.split == "holdout"
    }
}

/// Reads every task below a directory, sorted by identifier.
///
/// JSON files whose `format` is not [`TASK_FORMAT`] (schemas, the list of
/// held-out tasks) are skipped.
///
/// # Errors
///
/// Returns an error when a directory or a task file cannot be read, a task
/// is invalid, or two tasks share an identifier.
pub fn load_tasks(root: &Path) -> Result<Vec<TaskFile>> {
    let mut files = Vec::new();
    collect_json(root, &mut files)?;
    let mut tasks = Vec::new();
    for path in files {
        let document = crate::snapshot::read_json(&path)?;
        if document.get("format").and_then(Value::as_str) == Some(TASK_FORMAT) {
            tasks.push(TaskFile::from_document(&path, document)?);
        }
    }
    tasks.sort_by(|a, b| a.task.id.cmp(&b.task.id));
    if let Some(pair) = tasks
        .windows(2)
        .find(|pair| pair[0].task.id == pair[1].task.id)
    {
        return Err(BenchError::format(
            &pair[1].path,
            "task",
            format!("identifier {} is used twice", pair[1].task.id),
        ));
    }
    Ok(tasks)
}

fn collect_json(directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let listing = fs::read_dir(directory).map_err(|error| BenchError::io(directory, error))?;
    for item in listing {
        let path = item
            .map_err(|error| BenchError::io(directory, error))?
            .path();
        if path.is_dir() {
            collect_json(&path, files)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            files.push(path);
        }
    }
    Ok(())
}

/// Matches a task identifier against a filter: a text contained in the
/// identifier, or a pattern with `*` matching any sequence.
///
/// ```
/// use nurt_bench::task::matches_filter;
///
/// assert!(matches_filter("search-007", "search"));
/// assert!(matches_filter("search-007", "*-007"));
/// assert!(!matches_filter("search-007", "git-*"));
/// ```
#[must_use]
pub fn matches_filter(id: &str, filter: &str) -> bool {
    if !filter.contains('*') {
        return id.contains(filter);
    }
    let parts: Vec<&str> = filter.split('*').collect();
    let mut rest = id;
    for (index, part) in parts.iter().enumerate() {
        if index == 0 {
            let Some(stripped) = rest.strip_prefix(part) else {
                return false;
            };
            rest = stripped;
        } else if index == parts.len() - 1 {
            return rest.ends_with(part);
        } else if let Some(position) = rest.find(part) {
            rest = &rest[position + part.len()..];
        } else {
            return false;
        }
    }
    true
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn task_document(id: &str) -> Value {
        json!({
            "format": "nurt-bench-task/1",
            "id": id,
            "domain": 3,
            "platform": "linux",
            "split": "public",
            "nl": "Create out.txt containing hello.",
            "fs_before": {"format": "nurt-fs-snapshot/1", "entries": []},
            "expect": {"files": {"out.txt": {"text": "hello"}}},
            "forbidden": [],
            "limits": {"steps": 15, "seconds": 60},
            "oracle_program": "write \"out.txt\" \"hello\"",
            "reference_bash": "printf hello > out.txt",
            "tags": ["write"],
            "license": "CC-BY-4.0"
        })
    }

    #[test]
    fn reads_task_members() {
        let file = TaskFile::from_document(Path::new("t.json"), task_document("edit-001")).unwrap();
        assert_eq!(file.task.id, "edit-001");
        assert_eq!(file.task.limits.steps, DEFAULT_STEPS);
        assert!(!file.is_holdout());
        assert!(file.document.get("expect").is_some());
    }

    #[test]
    fn rejects_foreign_format_and_zero_limits() {
        let mut document = task_document("edit-001");
        document["format"] = json!("other/1");
        assert!(TaskFile::from_document(Path::new("t.json"), document).is_err());
        let mut document = task_document("edit-001");
        document["limits"]["steps"] = json!(0);
        assert!(TaskFile::from_document(Path::new("t.json"), document).is_err());
    }

    #[test]
    fn loads_tasks_sorted_and_skips_other_json() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("03-single-edit")).unwrap();
        for id in ["edit-002", "edit-001"] {
            fs::write(
                dir.path().join("03-single-edit").join(format!("{id}.json")),
                task_document(id).to_string(),
            )
            .unwrap();
        }
        fs::write(dir.path().join("holdout.json"), "[]").unwrap();
        fs::write(dir.path().join("README.md"), "text").unwrap();
        let tasks = load_tasks(dir.path()).unwrap();
        let ids: Vec<&str> = tasks.iter().map(|t| t.task.id.as_str()).collect();
        assert_eq!(ids, ["edit-001", "edit-002"]);
    }

    #[test]
    fn rejects_duplicated_identifiers() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a.json", "b.json"] {
            fs::write(dir.path().join(name), task_document("edit-001").to_string()).unwrap();
        }
        assert!(load_tasks(dir.path()).is_err());
    }

    #[test]
    fn platforms_run_on_matching_hosts() {
        assert!(Platform::Cross.runs_on(HostOs::Windows));
        assert!(Platform::Linux.runs_on(HostOs::Linux));
        assert!(!Platform::Windows.runs_on(HostOs::Linux));
        assert!(!Platform::Macos.runs_on(HostOs::Linux));
    }

    #[test]
    fn selects_script_of_the_host_shell() {
        let scripts = Scripts {
            description: None,
            bash: Some("b".into()),
            pwsh: Some("p".into()),
        };
        assert_eq!(scripts.for_host(HostOs::Linux), Some("b"));
        assert_eq!(scripts.for_host(HostOs::Windows), Some("p"));
    }

    #[test]
    fn filters_by_substring_and_pattern() {
        assert!(matches_filter("git-010", "git"));
        assert!(matches_filter("git-010", "git-*"));
        assert!(matches_filter("git-010", "*010"));
        assert!(matches_filter("git-010", "g*-0*0"));
        assert!(!matches_filter("git-010", "search-*"));
        assert!(!matches_filter("git-010", "*-011"));
        assert!(!matches_filter("git-010", "x*0"));
        assert!(!matches_filter("git-010", "g*z*0"));
    }
}
