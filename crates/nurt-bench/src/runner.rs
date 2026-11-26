// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Execution of tasks: one run per task and seed, each in its own
//! temporary directory, recorded as a run log.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::agent::{AgentSetup, run_agent};
use crate::condition::Condition;
use crate::error::{BenchError, Result};
use crate::model::{ModelClient, ModelSpec, RunKey, ToolSpec};
use crate::oracle::{Observation, Oracle};
use crate::process;
use crate::protocol::Protocol;
use crate::rpc::ServeLauncher;
use crate::runlog::{
    EndEvent, Event, OracleEvent, Outcome, ProtocolInfo, RunHeader, RunLog, TaskInfo, ToolInfo,
};
use crate::snapshot::{self, Snapshot};
use crate::task::{HostOs, TaskFile};
use crate::tool::{NurtTool, Shell, ShellTool, ToolExecutor, run_environment};

/// Time limit of the `setup` and `teardown` scripts of a task.
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(120);

/// SHA-256 of a text in hexadecimal.
#[must_use]
pub fn sha256_hex(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

/// Replaces characters that are not safe in a file name.
#[must_use]
pub fn path_label(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

/// Settings of a series of runs.
pub struct RunSettings {
    /// Protocol files.
    pub protocol: Protocol,
    /// Condition of every run of the series.
    pub condition: Condition,
    /// Model.
    pub model: ModelSpec,
    /// Seeds; every task runs once per seed.
    pub seeds: Vec<u64>,
    /// Output directory of the run logs.
    pub out: PathBuf,
    /// Directory of the shared initial trees (`bench/fixtures`).
    pub fixtures: PathBuf,
    /// Operating system of the runs.
    pub host: HostOs,
    /// Number of runs executed in parallel.
    pub jobs: usize,
}

/// Result of one run as reported to the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    /// Identifier of the run.
    pub run_id: String,
    /// What happened.
    pub status: RunStatus,
}

/// Status of one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunStatus {
    /// The run was executed and logged.
    Logged {
        /// How the agent loop ended.
        outcome: Outcome,
        /// Whether the oracle accepted the run.
        passed: bool,
    },
    /// A complete log already exists; the run was not repeated.
    Existing,
    /// The run could not be prepared; nothing was logged.
    Failed(String),
}

/// Counts of a series of runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SeriesSummary {
    /// Runs executed and logged.
    pub logged: usize,
    /// Logged runs accepted by the oracle.
    pub passed: usize,
    /// Logged runs with outcome `error`.
    pub errors: usize,
    /// Runs skipped because a complete log exists.
    pub existing: usize,
    /// Runs that could not be prepared.
    pub failed: usize,
    /// Tasks skipped because they do not run on this system.
    pub other_platform: usize,
}

/// Executes runs of tasks.
pub struct Runner<'a> {
    settings: &'a RunSettings,
    oracle: &'a dyn Oracle,
    launcher: Option<&'a dyn ServeLauncher>,
}

/// Prepared tool of a run.
struct PreparedTool {
    executor: Box<dyn ToolExecutor>,
    spec: ToolSpec,
}

impl<'a> Runner<'a> {
    /// Creates a runner. A launcher of `nurt serve` is required for the
    /// Danaco Nurt conditions.
    ///
    /// # Errors
    ///
    /// Returns [`BenchError::Argument`] when a Danaco Nurt condition has no
    /// launcher.
    pub fn new(
        settings: &'a RunSettings,
        oracle: &'a dyn Oracle,
        launcher: Option<&'a dyn ServeLauncher>,
    ) -> Result<Self> {
        if settings.condition.is_nurt() && launcher.is_none() {
            return Err(BenchError::Argument(format!(
                "condition {} needs the server command (--nurt)",
                settings.condition
            )));
        }
        Ok(Self {
            settings,
            oracle,
            launcher,
        })
    }

    /// Identifier of a run: `<model>/<condition>/<task>/s<seed>`.
    #[must_use]
    pub fn run_id(&self, task: &str, seed: u64) -> String {
        format!(
            "{}/{}/{task}/s{seed}",
            self.settings.model.info().id,
            self.settings.condition
        )
    }

    /// Path of the log of a run.
    #[must_use]
    pub fn log_path(&self, task: &str, seed: u64) -> PathBuf {
        self.settings
            .out
            .join(path_label(&self.settings.model.info().id))
            .join(self.settings.condition.as_str())
            .join(format!("{}.s{seed}.jsonl", path_label(task)))
    }

    /// Executes every task that runs on the system once per seed, skipping
    /// runs with a complete log, and calls `progress` after every run.
    #[must_use]
    pub fn run_series(
        &self,
        tasks: &[TaskFile],
        progress: &(dyn Fn(&RunReport) + Sync),
    ) -> SeriesSummary {
        let mut summary = SeriesSummary::default();
        let mut queue = VecDeque::new();
        for task in tasks {
            if !task.task.platform.runs_on(self.settings.host) {
                summary.other_platform += 1;
                continue;
            }
            for seed in &self.settings.seeds {
                queue.push_back((task, *seed));
            }
        }
        let queue = Mutex::new(queue);
        let totals = Mutex::new(summary);
        std::thread::scope(|scope| {
            for _ in 0..self.settings.jobs.max(1) {
                scope.spawn(|| {
                    loop {
                        let next = queue
                            .lock()
                            .map(|mut queue| queue.pop_front())
                            .ok()
                            .flatten();
                        let Some((task, seed)) = next else { break };
                        let report = self.run_once(task, seed);
                        if let Ok(mut totals) = totals.lock() {
                            count(&mut totals, &report.status);
                        }
                        progress(&report);
                    }
                });
            }
        });
        totals.into_inner().unwrap_or_default()
    }

    /// Executes one run unless a complete log of a run without an error
    /// exists.
    #[must_use]
    pub fn run_once(&self, task: &TaskFile, seed: u64) -> RunReport {
        let run_id = self.run_id(&task.task.id, seed);
        let path = self.log_path(&task.task.id, seed);
        let status = if crate::runlog::is_final(&path) {
            RunStatus::Existing
        } else {
            match self.execute(task, seed, &run_id, &path) {
                Ok((outcome, passed)) => RunStatus::Logged { outcome, passed },
                Err(error) => RunStatus::Failed(error.to_string()),
            }
        };
        RunReport { run_id, status }
    }

    fn prepare_tool(
        &self,
        root: &Path,
        scratch: &Path,
        env: &[(String, String)],
    ) -> Result<PreparedTool> {
        let definition = self.settings.protocol.tool(self.settings.condition)?;
        let (executor, description): (Box<dyn ToolExecutor>, String) = match self.settings.condition
        {
            Condition::Bash | Condition::Pwsh => {
                let shell = if self.settings.condition == Condition::Bash {
                    Shell::Bash
                } else {
                    Shell::Pwsh
                };
                let executor = ShellTool::new(
                    shell,
                    root.to_path_buf(),
                    env.to_vec(),
                    definition.argument.clone(),
                );
                (
                    Box::new(executor),
                    definition.description.clone().unwrap_or_default(),
                )
            }
            Condition::Nurt | Condition::NurtGroups => {
                let launcher = self
                    .launcher
                    .ok_or_else(|| BenchError::Argument("no server command".to_owned()))?;
                let mut client = launcher.launch(scratch)?;
                let description = client.describe(None)?;
                let executor = NurtTool::new(
                    client,
                    root,
                    self.settings.condition == Condition::NurtGroups,
                );
                (Box::new(executor), description)
            }
        };
        Ok(PreparedTool {
            executor,
            spec: ToolSpec {
                name: definition.name.clone(),
                description,
                input_schema: definition.input_schema.clone(),
            },
        })
    }

    fn header(
        &self,
        task: &TaskFile,
        seed: u64,
        run_id: &str,
        system: &str,
        tool: &ToolSpec,
        model: &mut dyn ModelClient,
    ) -> RunHeader {
        RunHeader {
            format: crate::runlog::RUN_FORMAT.to_owned(),
            run_id: run_id.to_owned(),
            task: TaskInfo {
                id: task.task.id.clone(),
                domain: task.task.domain,
                platform: task.task.platform,
                split: task.task.split.clone(),
                provocation: task.task.provocation.clone(),
            },
            condition: self.settings.condition,
            model: self.settings.model.info(),
            seed,
            protocol: ProtocolInfo {
                version: self.settings.protocol.settings.version.clone(),
                system_message_sha256: sha256_hex(system),
            },
            tool: ToolInfo {
                name: tool.name.clone(),
                description_sha256: sha256_hex(&tool.description),
                description_chars: u64::try_from(tool.description.chars().count())
                    .unwrap_or(u64::MAX),
                description_tokens: model.count_tokens(&tool.description).unwrap_or(None),
            },
            limits: task.task.limits,
            host: self.settings.host,
            started: jiff::Timestamp::now().to_string(),
            runner: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }

    fn run_script(
        &self,
        script: Option<&str>,
        root: &Path,
        env: &[(String, String)],
        stage: &str,
    ) -> Result<()> {
        let Some(script) = script else { return Ok(()) };
        let spec = Shell::of_host(self.settings.host).spec(script, root, env, SCRIPT_TIMEOUT);
        let output = process::run(&spec)?;
        if output.success() {
            Ok(())
        } else {
            Err(BenchError::Process(format!(
                "{stage} failed with {:?}: {}",
                output.code,
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }

    fn execute(
        &self,
        task: &TaskFile,
        seed: u64,
        run_id: &str,
        path: &Path,
    ) -> Result<(Outcome, bool)> {
        let scratch =
            tempfile::tempdir().map_err(|error| BenchError::io(&std::env::temp_dir(), error))?;
        let root = scratch.path().join("task");
        let home = scratch.path().join("home");
        let serve = scratch.path().join("serve");
        for directory in [&root, &home, &serve] {
            std::fs::create_dir_all(directory).map_err(|error| BenchError::io(directory, error))?;
        }
        let tree = snapshot::resolve_fs_before(&task.task.fs_before, &self.settings.fixtures)?;
        snapshot::materialize(&tree, &root)?;
        let env = run_environment(&home);
        let host = self.settings.host;
        self.run_script(
            task.task.setup.as_ref().and_then(|s| s.for_host(host)),
            &root,
            &env,
            "setup",
        )?;
        let before = Snapshot::from_tree(&snapshot::capture(&root)?);
        // The model is opened first: a missing key must not start a server.
        let key = RunKey {
            task: task.task.id.clone(),
            condition: self.settings.condition,
            seed,
        };
        let mut model = self.settings.model.open(&key)?;
        let mut tool = self.prepare_tool(&root, &serve, &env)?;
        let system = self
            .settings
            .protocol
            .system_message(host, task.task.limits.steps);
        let header = self.header(task, seed, run_id, &system, &tool.spec, model.as_mut());
        let mut log = RunLog::create(path)?;
        log.write(&Event::Run(Box::new(header)))?;
        let setup = AgentSetup {
            system: &system,
            tool: &tool.spec,
            task: &task.task.nl,
            limits: task.task.limits,
            seed,
            max_output_chars: self.settings.protocol.settings.max_tool_output_chars,
        };
        let result = run_agent(
            setup,
            model.as_mut(),
            tool.executor.as_mut(),
            &mut |event| log.write(&event),
        );
        // The server is ended on every path, also when the agent failed.
        tool.executor.finish();
        let result = result?;
        let mut end = EndEvent {
            outcome: result.outcome,
            steps: result.steps,
            tool_calls: result.tool_calls,
            tokens: result.tokens,
            wall_ms: result.wall_ms,
            error: result.error,
        };
        let mut passed = false;
        if end.outcome != Outcome::Error {
            match self.judge(task, &before, &root, &env, result.answer) {
                Ok(event) => {
                    passed = event.verdict.passed;
                    log.write(&Event::Oracle(Box::new(event)))?;
                }
                Err(error) => {
                    end.outcome = Outcome::Error;
                    end.error = Some(error.to_string());
                }
            }
        }
        let outcome = end.outcome;
        log.write(&Event::End(end))?;
        log.finish()?;
        Ok((outcome, passed))
    }

    fn judge(
        &self,
        task: &TaskFile,
        before: &Snapshot,
        root: &Path,
        env: &[(String, String)],
        answer: String,
    ) -> Result<OracleEvent> {
        let mut observation = Observation {
            answer,
            probes: std::collections::BTreeMap::new(),
        };
        for probe in &task.task.probes {
            let value = crate::probe::collect(probe, root, env, self.settings.host)?;
            observation.probes.insert(probe.id.clone(), value);
        }
        let host = self.settings.host;
        self.run_script(
            task.task.teardown.as_ref().and_then(|s| s.for_host(host)),
            root,
            env,
            "teardown",
        )?;
        let after = Snapshot::from_tree(&snapshot::capture(root)?);
        let verdict = self
            .oracle
            .judge(&task.document, before, &after, &observation)?;
        Ok(OracleEvent {
            observation,
            verdict,
        })
    }
}

fn count(summary: &mut SeriesSummary, status: &RunStatus) {
    match status {
        RunStatus::Logged { outcome, passed } => {
            summary.logged += 1;
            summary.passed += usize::from(*passed);
            summary.errors += usize::from(*outcome == Outcome::Error);
        }
        RunStatus::Existing => summary.existing += 1,
        RunStatus::Failed(_) => summary.failed += 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_hashes() {
        assert_eq!(path_label("org/model:1"), "org-model-1");
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn counts_statuses() {
        let mut summary = SeriesSummary::default();
        count(
            &mut summary,
            &RunStatus::Logged {
                outcome: Outcome::Answered,
                passed: true,
            },
        );
        count(
            &mut summary,
            &RunStatus::Logged {
                outcome: Outcome::Error,
                passed: false,
            },
        );
        count(&mut summary, &RunStatus::Existing);
        count(&mut summary, &RunStatus::Failed("x".into()));
        assert_eq!(
            summary,
            SeriesSummary {
                logged: 2,
                passed: 1,
                errors: 1,
                existing: 1,
                failed: 1,
                other_platform: 0
            }
        );
    }
}
