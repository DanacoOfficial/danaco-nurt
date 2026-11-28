// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-difftest::runner
// Responsibility:   Runner of scenarios: preparation, the two sides, comparison, undo and the summary of a run.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Runner of scenarios: preparation, the two sides, comparison, undo and the
//! summary of a run.
//!
//! A case directory holds `base` (the prepared repository with its
//! siblings), the copies `a` (the `git` side) and `b` (the Nurt side),
//! `home` (the empty global configuration of both sides) and `journal` (the
//! previous content kept by the journal). The placeholder `$SERVER` in setup
//! steps, `git` arguments and Nurt parameters is the address of the case
//! directory of the side through the transport of the scenario: a
//! `file://` address, `http://127.0.0.1:<port>/<side>` for the HTTP test
//! server or `ssh://nurt@127.0.0.1:<port>/<side>` for the SSH test server.
//! Addresses written during the preparation are moved to the side in the
//! configuration and `FETCH_HEAD` files of each copy.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nurt_contracts::env::Host;
use nurt_contracts::services::Services;
use nurt_contracts::stage::{ExecCtx, StageImpl};

use crate::bind::{Catalog, ScenarioHost};
use crate::compare::{DifferenceReport, Sides, compare};
use crate::corpus::{self, Corpus, REPO_DIR};
use crate::error::{DifftestError, Result, io_error, scenario_error};
use crate::fsutil::{self, Entry};
use crate::git::{GLOBAL_CONFIG, GitEnv, GitProgram, expect_exit, join_args};
use crate::journal::{self, RecordingJournal, UndoResult};
use crate::normalize::{Normalizer, RepoState, SERVER_PLACEHOLDER};
use crate::process::ScenarioProcessHost;
use crate::scenario::{
    Expectation, NurtAction, Platform, Scenario, Section, SetupStep, StateSelector, Transport,
};
use crate::server::HttpServer;
use crate::ssh::{SshServer, local_ssh_command};
use crate::stage::{CommandTable, StageResult, run_stage};
use crate::time::FixedClock;

/// Files whose remote addresses are moved from `base` to the side after
/// copying.
const ADDRESS_FILES: &[&str] = &["config", "config.worktree", "FETCH_HEAD"];

/// Files that hold the absolute location of a working tree or of a
/// repository; the location of `base` is moved to the side after copying.
const LOCATION_FILES: &[&str] = &["config", "config.worktree", "gitdir", ".git"];

/// Options of a run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunOptions {
    /// A scenario whose command has no implementation in the table fails
    /// instead of running the `git` side only.
    pub require_nurt: bool,
    /// Run scenarios on heavy corpus repositories.
    pub heavy: bool,
    /// Keep the case directories of failed scenarios.
    pub keep_failed: bool,
}

/// Result of one scenario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Both sides agree, undo restored the initial state and `fsck` passed.
    Passed,
    /// The `git` side and the checks of the state passed; the command has no
    /// implementation in the table yet.
    GitOnly,
    /// The scenario does not run here; the reason is given.
    Skipped(String),
    /// The scenario failed; every finding is listed.
    Failed(Vec<String>),
}

/// Outcome of one scenario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Scenario identifier.
    pub id: String,
    /// Result.
    pub status: Status,
    /// Remarks that do not fail the scenario.
    pub notes: Vec<String>,
}

/// Directory of the case, next to the repository, that a test server serves
/// as `$SERVER/upstream`; the server process writes it, not the Nurt side.
const SERVED_DIR: &str = "upstream";

/// Text of a panic payload.
fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no message".to_owned())
}

/// Runs scenarios against the `git` program and the command table.
#[derive(Debug)]
pub struct Runner {
    git: GitProgram,
    catalog: Catalog,
    corpus: Corpus,
    commands: CommandTable,
    options: RunOptions,
    work: tempfile::TempDir,
}

/// Directories of one case.
#[derive(Debug)]
struct CaseDirs {
    root: PathBuf,
    home: PathBuf,
    base: PathBuf,
    a: PathBuf,
    b: PathBuf,
    store: PathBuf,
}

impl CaseDirs {
    fn create(root: &Path) -> Result<Self> {
        let dirs = Self {
            root: root.to_path_buf(),
            home: root.join("home"),
            base: root.join("base"),
            a: root.join("a"),
            b: root.join("b"),
            store: root.join("journal"),
        };
        for dir in [&dirs.home, &dirs.base, &dirs.store] {
            std::fs::create_dir_all(dir).map_err(io_error("create directory", dir))?;
        }
        let config = dirs.home.join(".gitconfig");
        std::fs::write(&config, GLOBAL_CONFIG).map_err(io_error("write", &config))?;
        Ok(dirs)
    }
}

/// Addresses of the case directories through the transport of a scenario.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Addresses {
    /// Base address of the test server; `None` for `file://` addresses.
    server: Option<String>,
}

impl Addresses {
    /// Value of `$SERVER` for the case directory `side`.
    fn side(&self, side: &Path) -> String {
        match &self.server {
            Some(base) => format!(
                "{base}/{}",
                side.file_name()
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_default()
            ),
            None => file_url(side),
        }
    }
}

/// `file://` address of a directory.
fn file_url(path: &Path) -> String {
    let forward = path.to_string_lossy().replace('\\', "/");
    if forward.starts_with('/') {
        format!("file://{forward}")
    } else {
        format!("file:///{forward}")
    }
}

/// `args` with `$SERVER` replaced by `address`.
fn with_address(args: &[String], address: &str) -> Vec<String> {
    args.iter()
        .map(|arg| arg.replace(SERVER_PLACEHOLDER, address))
        .collect()
}

/// `table` with `$SERVER` replaced by `address` in every text value.
fn params_with_address(table: &toml::Table, address: &str) -> toml::Table {
    fn value(item: &toml::Value, address: &str) -> toml::Value {
        match item {
            toml::Value::String(text) => {
                toml::Value::String(text.replace(SERVER_PLACEHOLDER, address))
            }
            toml::Value::Array(items) => {
                toml::Value::Array(items.iter().map(|item| value(item, address)).collect())
            }
            toml::Value::Table(table) => toml::Value::Table(params_with_address(table, address)),
            other => other.clone(),
        }
    }
    table
        .iter()
        .map(|(key, item)| (key.clone(), value(item, address)))
        .collect()
}

/// Moves the addresses `<from>/…` to `<to>/…` in the configuration and
/// `FETCH_HEAD` files below `side`, and the directory `base` to `side` in the
/// files that record the location of linked working trees.
fn move_addresses(side: &Path, base: &Path, from: &str, to: &str) -> Result<()> {
    let skip = |path: &Path| path.file_name().is_some_and(|name| name == "objects");
    let (from, to) = (format!("{from}/"), format!("{to}/"));
    let (base, side_dir) = (base.to_string_lossy(), side.to_string_lossy());
    for (key, entry) in fsutil::walk(side, &skip)? {
        let Entry::File { bytes, .. } = entry else {
            continue;
        };
        let name = key.rsplit('/').next().unwrap_or_default();
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        let mut moved = text.clone();
        if from != to && ADDRESS_FILES.contains(&name) {
            moved = moved.replace(&from, &to);
        }
        if LOCATION_FILES.contains(&name) {
            moved = moved.replace(base.as_ref(), side_dir.as_ref());
            // Git for Windows records the locations with forward slashes.
            moved = moved.replace(&base.replace('\\', "/"), &side_dir.replace('\\', "/"));
        }
        if moved != text {
            let path = side.join(&key);
            std::fs::write(&path, moved).map_err(io_error("write", &path))?;
        }
    }
    Ok(())
}

/// Everything found while running one scenario.
#[derive(Debug, Default)]
struct Findings {
    failures: Vec<String>,
    notes: Vec<String>,
}

impl Findings {
    fn fail(&mut self, message: impl Into<String>) {
        self.failures.push(message.into());
    }
}

/// Test server of the transport of a scenario.
#[derive(Debug)]
enum TestServer {
    Http(HttpServer),
    Ssh(SshServer),
}

impl TestServer {
    fn base_url(&self) -> String {
        match self {
            Self::Http(server) => server.base_url(),
            Self::Ssh(server) => server.base_url(),
        }
    }
}

/// What the preparation of a case produced.
#[derive(Debug)]
struct Prepared {
    /// Environment of the Nurt host and of the normalizer.
    env: GitEnv,
    /// Environment of the setup steps and of the `git` side: `env` with the
    /// local replacement of `ssh` for SSH scenarios.
    git_env: GitEnv,
    addresses: Addresses,
    server: Option<TestServer>,
}

impl Runner {
    /// Creates a runner with a fresh work directory.
    ///
    /// # Errors
    ///
    /// Fails with [`DifftestError`] when the catalog is invalid or the work
    /// directory cannot be created.
    pub fn new(git: GitProgram, commands: CommandTable, options: RunOptions) -> Result<Self> {
        let work = tempfile::Builder::new()
            .prefix("nurt-difftest-")
            .tempdir()
            .map_err(io_error("create directory", &std::env::temp_dir()))?;
        let corpus = Corpus::new(git.clone(), work.path().join("corpus"));
        Ok(Self {
            git,
            catalog: Catalog::git()?,
            corpus,
            commands,
            options,
            work,
        })
    }

    /// Runs `scenarios` on `threads` threads; outcomes are in input order.
    #[must_use]
    pub fn run_all(&self, scenarios: &[Scenario], threads: usize) -> Vec<Outcome> {
        let next = AtomicUsize::new(0);
        let mut outcomes: Vec<Option<Outcome>> = vec![None; scenarios.len()];
        let slots = std::sync::Mutex::new(&mut outcomes);
        std::thread::scope(|scope| {
            for _ in 0..threads.max(1) {
                scope.spawn(|| {
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(scenario) = scenarios.get(index) else {
                            break;
                        };
                        let outcome = self.run(scenario, index);
                        let mut slots = slots
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        slots[index] = Some(outcome);
                    }
                });
            }
        });
        outcomes.into_iter().flatten().collect()
    }

    /// Runs one scenario; `ordinal` names its case directory.
    #[must_use]
    pub fn run(&self, scenario: &Scenario, ordinal: usize) -> Outcome {
        if let Some(reason) = self.skip_reason(scenario) {
            return Outcome {
                id: scenario.id.clone(),
                status: Status::Skipped(reason),
                notes: Vec::new(),
            };
        }
        let case_root = self.work.path().join(format!("case-{ordinal}"));
        let mut findings = Findings::default();
        let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.run_case(scenario, &case_root, &mut findings)
        }));
        let git_only = match attempt {
            Ok(Ok(git_only)) => git_only,
            Ok(Err(error)) => {
                findings.fail(format!("preparation failed: {error}"));
                false
            }
            Err(payload) => {
                findings.fail(format!("the Nurt side panicked: {}", panic_text(&payload)));
                false
            }
        };
        if !findings.failures.is_empty() && self.options.keep_failed {
            findings
                .notes
                .push(format!("case directory kept at {}", case_root.display()));
        } else {
            let _ = fsutil::remove_entry(&case_root);
        }
        let status = if !findings.failures.is_empty() {
            Status::Failed(findings.failures)
        } else if git_only {
            Status::GitOnly
        } else {
            Status::Passed
        };
        Outcome {
            id: scenario.id.clone(),
            status,
            notes: findings.notes,
        }
    }

    fn skip_reason(&self, scenario: &Scenario) -> Option<String> {
        let platform = Platform::current();
        if !scenario.runs_on(platform) {
            return Some(format!("not run on {}", platform.as_str()));
        }
        match corpus::find(&scenario.corpus) {
            Some(repo) if repo.heavy && !self.options.heavy => Some(format!(
                "corpus repository `{}` is heavy, run with --heavy",
                repo.name
            )),
            Some(repo) if !repo.platforms.contains(&platform) => Some(format!(
                "corpus repository `{}` cannot exist on {}",
                repo.name,
                platform.as_str()
            )),
            _ => None,
        }
    }

    /// Runs the case; returns `true` when only the `git` side ran.
    fn run_case(
        &self,
        scenario: &Scenario,
        case_root: &Path,
        findings: &mut Findings,
    ) -> Result<bool> {
        let dirs = CaseDirs::create(case_root)?;
        let prepared = self.prepare(scenario, &dirs)?;
        let env = &prepared.env;
        let repo_a = dirs.a.join(REPO_DIR);
        let repo_b = dirs.b.join(REPO_DIR);
        let normalizer = Normalizer::new(&self.git, env, scenario.compare.strict)
            .with_server(prepared.addresses.server.as_deref());
        let mut compared = scenario.compare.state.clone();
        let fsck = StateSelector::main(Section::Fsck);
        compared.push(fsck.clone());
        let standard: Vec<StateSelector> = Section::STANDARD
            .into_iter()
            .map(StateSelector::main)
            .collect();
        let all: Vec<StateSelector> = standard.iter().chain(&compared).cloned().collect();
        let baseline = normalizer.baseline(&repo_b, &all)?;
        let initial = normalizer.snapshot(&dirs.b, &repo_b, &standard, &baseline)?;
        let address_a = prepared.addresses.side(&dirs.a);
        self.run_git_side(scenario, &repo_a, &prepared.git_env, &address_a, findings)?;
        let git_output =
            self.git_output(scenario, &dirs.a, &prepared.git_env, &address_a, findings)?;
        let state_a = normalizer.snapshot(&dirs.a, &repo_a, &compared, &baseline)?;
        check_fsck(&state_a, &fsck, "git", findings);
        let Some(stage) = self.commands.get(&scenario.nurt.command) else {
            if self.options.require_nurt {
                findings.fail(format!(
                    "command `{}` has no implementation in the table",
                    scenario.nurt.command
                ));
            }
            return Ok(true);
        };
        let journal = RecordingJournal::new(dirs.store.clone());
        let address_b = prepared.addresses.side(&dirs.b);
        let result = self.run_nurt_side(
            scenario,
            stage.as_ref(),
            &dirs,
            env,
            &address_b,
            &journal,
            findings,
        )?;
        check_output(scenario, git_output.as_deref(), &result, &dirs.b, findings);
        let state_b = normalizer.snapshot(&dirs.b, &repo_b, &compared, &baseline)?;
        check_fsck(&state_b, &fsck, "nurt", findings);
        let differences = compare(
            &state_a,
            &state_b,
            &scenario.compare.state,
            &scenario.compare.ignore,
        );
        if !differences.is_empty() {
            findings.fail(format!(
                "states differ after the operation:\n{}",
                DifferenceReport {
                    differences: &differences,
                    sides: Sides::GIT_NURT
                }
            ));
        }
        let check_initial = |label: &str, findings: &mut Findings| -> Result<()> {
            let state = normalizer.snapshot(&dirs.b, &repo_b, &standard, &baseline)?;
            let differences = compare(&initial, &state, &standard, &[]);
            if !differences.is_empty() {
                findings.fail(format!(
                    "{label}:\n{}",
                    DifferenceReport {
                        differences: &differences,
                        sides: Sides::INITIAL_UNDONE
                    }
                ));
            }
            Ok(())
        };
        if result.code.is_some() && result.committed.is_none() {
            check_initial("the refused operation changed the repository", findings)?;
        } else if scenario.compare.undo && result.committed.is_some() {
            match journal.undo_last() {
                Ok(UndoResult::Undone(_)) => {
                    check_initial("undo did not restore the initial state", findings)?
                }
                Ok(UndoResult::Nothing) => findings.fail("undo found no committed operation"),
                Err(message) => findings.fail(format!("undo failed: {message}")),
            }
        }
        drop(prepared.server);
        Ok(false)
    }

    /// Builds `base`, starts the test server of the transport and makes the
    /// two copies.
    fn prepare(&self, scenario: &Scenario, dirs: &CaseDirs) -> Result<Prepared> {
        let env = GitEnv::new(&dirs.home, scenario.clock);
        let mut git_env = env.clone();
        let server = match scenario.transport {
            Transport::Http => Some(TestServer::Http(HttpServer::start(
                &self.git, &env, &dirs.root,
            )?)),
            Transport::Ssh => {
                git_env = git_env
                    .with_var("GIT_SSH_COMMAND", local_ssh_command(&dirs.root))
                    .with_var("GIT_SSH_VARIANT", "ssh");
                Some(TestServer::Ssh(SshServer::start(
                    &self.git, &env, &dirs.root, &dirs.home,
                )?))
            }
            Transport::File => None,
        };
        let addresses = Addresses {
            server: server.as_ref().map(TestServer::base_url),
        };
        self.corpus.materialize(&scenario.corpus, &dirs.base)?;
        let repo = dirs.base.join(REPO_DIR);
        let base_address = addresses.side(&dirs.base);
        let mut setup_env = git_env.clone();
        for step in &scenario.setup {
            match step {
                SetupStep::Git { args, cwd } => {
                    let dir = cwd
                        .as_ref()
                        .map_or_else(|| repo.clone(), |cwd| repo.join(cwd));
                    std::fs::create_dir_all(&dir).map_err(io_error("create directory", &dir))?;
                    self.git
                        .run_ok(&setup_env, &dir, &with_address(args, &base_address))?;
                }
                SetupStep::Write(write) => fsutil::apply_write(&repo, write)?,
                SetupStep::Clock(clock) => setup_env = setup_env.with_clock(*clock),
            }
        }
        for write in &scenario.files {
            fsutil::apply_write(&repo, write)?;
        }
        for side in [&dirs.a, &dirs.b] {
            fsutil::copy_tree(&dirs.base, side)?;
            move_addresses(side, &dirs.base, &base_address, &addresses.side(side))?;
            let copy = side.join(REPO_DIR);
            if copy.join(".git").exists() {
                self.git
                    .run(&env, &copy, &["update-index", "-q", "--refresh"])?;
            }
        }
        Ok(Prepared {
            env,
            git_env,
            addresses,
            server,
        })
    }

    /// Lines of the output command of the scenario, run on copy A after the
    /// operation; `None` without an output comparison or when the command
    /// failed, which is a finding.
    fn git_output(
        &self,
        scenario: &Scenario,
        side: &Path,
        env: &GitEnv,
        address: &str,
        findings: &mut Findings,
    ) -> Result<Option<Vec<String>>> {
        let Some(spec) = &scenario.compare.output else {
            return Ok(None);
        };
        let dir = git_dir(scenario, &side.join(REPO_DIR));
        let args = with_address(&spec.git, address);
        let output = self.git.run(env, &dir, &args)?;
        if let Err(error) = expect_exit(&output, 0, &dir, &args) {
            findings.fail(format!("output command: {error}"));
            return Ok(None);
        }
        Ok(Some(spec.git_lines(&output.stdout, side)))
    }

    fn run_git_side(
        &self,
        scenario: &Scenario,
        repo: &Path,
        env: &GitEnv,
        address: &str,
        findings: &mut Findings,
    ) -> Result<()> {
        let dir = git_dir(scenario, repo);
        let last = scenario.git.steps.len().saturating_sub(1);
        for (index, args) in scenario.git.steps.iter().enumerate() {
            let args = with_address(args, address);
            let output = self.git.run(env, &dir, &args)?;
            let expected = if index == last {
                scenario.git.expect_exit
            } else {
                0
            };
            if let Err(error) = expect_exit(&output, expected, &dir, &args) {
                findings.fail(format!("git side: {error}"));
                return Ok(());
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // The case parts are borrowed separately from the runner state.
    fn run_nurt_side(
        &self,
        scenario: &Scenario,
        stage: &dyn StageImpl,
        dirs: &CaseDirs,
        env: &GitEnv,
        address: &str,
        journal: &RecordingJournal,
        findings: &mut Findings,
    ) -> Result<StageResult> {
        let repo = dirs.b.join(REPO_DIR);
        let params = params_with_address(&scenario.nurt.params, address);
        let binding = self
            .catalog
            .bind(&scenario.nurt.command, &params, &repo)
            .map_err(|message| scenario_error(&scenario.source, message))?;
        if !binding.unknown.is_empty() {
            return Err(scenario_error(
                &scenario.source,
                format!(
                    "parameters missing from catalog/git.toml: {}",
                    binding.unknown.join(", ")
                ),
            ));
        }
        let spec = self
            .catalog
            .command(&scenario.nurt.command)
            .ok_or_else(|| {
                DifftestError::Catalog(format!("`{}` disappeared", scenario.nurt.command))
            })?;
        let host = ScenarioHost::new(env, &dirs.home, &dirs.root, scenario.clock);
        let services =
            Services::new().with_process_host(Arc::new(ScenarioProcessHost::new(host.env_vars())));
        let ctx = ExecCtx::builder(Arc::new(FixedClock::new(scenario.clock)), Arc::new(host))
            .services(Arc::new(services))
            .build();
        let before = journal::snapshot(&dirs.b)?;
        let result = run_stage(stage, spec, &binding.args, &ctx, journal);
        let after = journal::snapshot(&dirs.b)?;
        let served = (scenario.transport != Transport::File).then(|| dirs.b.join(SERVED_DIR));
        let undeclared = journal::undeclared_changes(&before, &after, &journal.declared());
        for path in undeclared
            .into_iter()
            .filter(|path| served.as_ref().is_none_or(|dir| !path.starts_with(dir)))
        {
            findings.fail(format!(
                "write outside the transaction: {}",
                fsutil::relative_key(&dirs.b, &path)
            ));
        }
        for violation in journal.violations() {
            findings.fail(format!("journal protocol violation: {violation}"));
        }
        check_expectation(scenario.nurt.expect, &result, findings);
        check_records(&scenario.nurt, &result, &dirs.b, findings);
        Ok(result)
    }
}

/// Working directory of the `git` commands of the scenario on `repo`.
fn git_dir(scenario: &Scenario, repo: &Path) -> PathBuf {
    scenario
        .git
        .cwd
        .as_ref()
        .map_or_else(|| repo.to_path_buf(), |cwd| repo.join(cwd))
}

/// Compares the output records of a successful command with the output of
/// the `git` command of the scenario.
fn check_output(
    scenario: &Scenario,
    git_lines: Option<&[String]>,
    result: &StageResult,
    case_root: &Path,
    findings: &mut Findings,
) {
    let (Some(spec), Some(git_lines)) = (&scenario.compare.output, git_lines) else {
        return;
    };
    if result.code.is_some() {
        return;
    }
    let nurt_lines = spec.record_lines(&result.records, case_root);
    if let Some(difference) = spec.compare(git_lines, &nurt_lines) {
        findings.fail(format!(
            "output differs from `git {}`:\n{difference}",
            join_args(&spec.git)
        ));
    }
}

fn check_fsck(state: &RepoState, fsck: &StateSelector, side: &str, findings: &mut Findings) {
    let selectors = std::iter::once(fsck).chain(
        state
            .selectors()
            .filter(|selector| selector.section == Section::Fsck && *selector != fsck),
    );
    for selector in selectors {
        let place = if selector == fsck {
            format!("on the {side} side")
        } else {
            format!("for {selector} on the {side} side")
        };
        let result = state
            .section(selector)
            .and_then(|section| section.get("result"));
        match result {
            Some(result) if result == "ok" => {}
            Some(result) => findings.fail(format!("git fsck --strict failed {place}: {result}")),
            None => findings.fail(format!(
                "git fsck --strict did not run {place}: the repository is missing"
            )),
        }
    }
}

fn check_expectation(expect: Expectation, result: &StageResult, findings: &mut Findings) {
    let actual = match result.code {
        None => Expectation::Ok,
        Some(code) => Expectation::Error(code),
    };
    if actual != expect {
        let message = result.message.as_deref().unwrap_or("no message");
        findings.fail(format!(
            "nurt side returned {actual} ({message}), the scenario expects {expect}"
        ));
    }
}

/// Checks the output records against the record patterns of the scenario;
/// each pattern needs a record of its own.
fn check_records(
    nurt: &NurtAction,
    result: &StageResult,
    case_root: &Path,
    findings: &mut Findings,
) {
    if let Some(count) = nurt.record_count
        && result.records.len() != count
    {
        findings.fail(format!(
            "nurt side output {} records, the scenario expects {count}",
            result.records.len()
        ));
    }
    let mut used = vec![false; result.records.len()];
    for pattern in &nurt.records {
        let found = result
            .records
            .iter()
            .enumerate()
            .position(|(index, record)| !used[index] && pattern.matches(record, case_root));
        match found {
            Some(index) => used[index] = true,
            None => findings.fail(format!("nurt side output no record matching {pattern}")),
        }
    }
}

/// Validates the parts of a scenario that do not need the `git` program:
/// the corpus repository and the binding of the parameters.
///
/// # Errors
///
/// Returns a message naming the first problem.
pub fn validate(scenario: &Scenario, catalog: &Catalog) -> Result<(), String> {
    if corpus::find(&scenario.corpus).is_none() {
        let names: Vec<&str> = corpus::CORPUS.iter().map(|repo| repo.name).collect();
        return Err(format!(
            "corpus repository `{}` does not exist, use one of: {}",
            scenario.corpus,
            names.join(", ")
        ));
    }
    let root = std::env::temp_dir().join(REPO_DIR);
    catalog
        .bind(&scenario.nurt.command, &scenario.nurt.params, &root)
        .map(drop)
}

/// Summary of a run.
#[derive(Debug, Clone)]
pub struct Summary<'a> {
    /// Outcomes of the scenarios.
    pub outcomes: &'a [Outcome],
    /// Scenario files that could not be loaded.
    pub invalid: &'a [String],
}

impl Summary<'_> {
    /// Number of outcomes with a status matching `wanted`.
    #[must_use]
    pub fn count(&self, wanted: fn(&Status) -> bool) -> usize {
        self.outcomes
            .iter()
            .filter(|outcome| wanted(&outcome.status))
            .count()
    }

    /// Returns `true` when no scenario failed and every file was valid.
    #[must_use]
    pub fn success(&self) -> bool {
        self.invalid.is_empty() && self.count(|status| matches!(status, Status::Failed(_))) == 0
    }
}

impl fmt::Display for Summary<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for message in self.invalid {
            writeln!(f, "invalid scenario {message}")?;
        }
        for outcome in self.outcomes {
            if let Status::Failed(findings) = &outcome.status {
                writeln!(f, "FAILED {}", outcome.id)?;
                for finding in findings {
                    for line in finding.lines() {
                        writeln!(f, "  {line}")?;
                    }
                }
            }
            for note in &outcome.notes {
                writeln!(f, "note {}: {note}", outcome.id)?;
            }
        }
        writeln!(
            f,
            "scenarios: {} total, {} passed, {} git side only, {} skipped, {} failed, {} invalid",
            self.outcomes.len() + self.invalid.len(),
            self.count(|status| *status == Status::Passed),
            self.count(|status| *status == Status::GitOnly),
            self.count(|status| matches!(status, Status::Skipped(_))),
            self.count(|status| matches!(status, Status::Failed(_))),
            self.invalid.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::value::{Record, RecordKind, Value};

    fn outcome(id: &str, status: Status) -> Outcome {
        Outcome {
            id: id.into(),
            status,
            notes: vec![],
        }
    }

    fn result(code: Option<ErrorCode>, records: Vec<Record>) -> StageResult {
        StageResult {
            code,
            message: code.map(|_| "local changes".to_owned()),
            committed: None,
            records,
            value: None,
            effect: true,
        }
    }

    fn scenario(extra: &str) -> Scenario {
        let text = format!(
            "id = \"x/y\"\ndescription = \"d.\"\ncorpus = \"empty\"\n[git]\nargs = [\"status\"]\n[nurt]\ncommand = \"git.review\"\n{extra}"
        );
        Scenario::parse(&text, Path::new("x/y.toml"), "x/y").unwrap()
    }

    #[test]
    fn summary_counts_statuses_and_lists_failures() {
        let outcomes = [
            outcome("a/pass", Status::Passed),
            outcome("a/only", Status::GitOnly),
            outcome("a/skip", Status::Skipped("x".into())),
            outcome("a/fail", Status::Failed(vec!["first\nsecond".into()])),
        ];
        let summary = Summary {
            outcomes: &outcomes,
            invalid: &[],
        };
        let text = summary.to_string();
        assert!(text.contains("FAILED a/fail\n  first\n  second"));
        assert!(
            text.contains("4 total, 1 passed, 1 git side only, 1 skipped, 1 failed, 0 invalid")
        );
        assert!(!summary.success());
        let passing = Summary {
            outcomes: &outcomes[..3],
            invalid: &[],
        };
        assert!(passing.success());
    }

    fn fsck_state(selector: &StateSelector, pairs: &[(&str, &str)]) -> RepoState {
        let mut state = RepoState::default();
        state.insert(
            selector.clone(),
            pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        );
        state
    }

    #[test]
    fn fsck_must_report_ok_for_every_fsck_section() {
        let main = StateSelector::main(Section::Fsck);
        let mut findings = Findings::default();
        check_fsck(
            &fsck_state(&main, &[("result", "ok")]),
            &main,
            "git",
            &mut findings,
        );
        assert!(findings.failures.is_empty(), "{:?}", findings.failures);
        check_fsck(
            &fsck_state(&main, &[("result", "exit 2: broken")]),
            &main,
            "git",
            &mut findings,
        );
        assert_eq!(findings.failures.len(), 1);
        assert!(findings.failures[0].contains("failed on the git side"));
        let upstream: StateSelector = "fsck@../upstream".parse().unwrap();
        let mut state = fsck_state(&main, &[("result", "ok")]);
        state.insert(
            upstream,
            [("result".to_owned(), "exit 1: x".to_owned())].into(),
        );
        let mut findings = Findings::default();
        check_fsck(&state, &main, "nurt", &mut findings);
        assert_eq!(findings.failures.len(), 1);
        assert!(findings.failures[0].contains("fsck@../upstream"));
    }

    #[test]
    fn fsck_of_a_missing_repository_is_a_failure() {
        let main = StateSelector::main(Section::Fsck);
        let mut findings = Findings::default();
        check_fsck(
            &fsck_state(&main, &[("repository", "missing")]),
            &main,
            "nurt",
            &mut findings,
        );
        check_fsck(&RepoState::default(), &main, "git", &mut findings);
        assert_eq!(findings.failures.len(), 2);
        assert!(findings.failures[0].contains("did not run on the nurt side"));
    }

    #[test]
    fn expectation_mismatch_is_a_failure() {
        let mut findings = Findings::default();
        check_expectation(
            Expectation::Ok,
            &result(Some(ErrorCode::GitDirty), vec![]),
            &mut findings,
        );
        assert_eq!(findings.failures.len(), 1);
        assert!(findings.failures[0].contains("E_GIT_DIRTY"));
    }

    #[test]
    fn each_record_pattern_needs_its_own_record() {
        let scenario = scenario(
            "record_count = 2\n[[nurt.records]]\nkind = \"finding\"\ncheck = \"whitespace\"\n[[nurt.records]]\nkind = \"finding\"\ncheck = \"whitespace\"\n",
        );
        let finding = Record::new(RecordKind::Finding).with("check", Value::from("whitespace"));
        let mut findings = Findings::default();
        let root = Path::new("/case/b");
        check_records(
            &scenario.nurt,
            &result(None, vec![finding.clone(), finding.clone()]),
            root,
            &mut findings,
        );
        assert!(findings.failures.is_empty(), "{:?}", findings.failures);
        check_records(
            &scenario.nurt,
            &result(None, vec![finding]),
            root,
            &mut findings,
        );
        assert_eq!(findings.failures.len(), 2);
        assert!(findings.failures[0].contains("output 1 records"));
        assert!(findings.failures[1].contains("no record matching finding"));
    }

    #[test]
    fn replaces_the_server_placeholder() {
        let args = vec!["fetch".to_owned(), "$SERVER/up.git".to_owned()];
        assert_eq!(
            with_address(&args, "http://h:1/a")[1],
            "http://h:1/a/up.git"
        );
        let params: toml::Table =
            toml::from_str("url = \"$SERVER/x\"\nlist = [\"$SERVER\"]\nn = 1").unwrap();
        let replaced = params_with_address(&params, "file:///c/b");
        assert_eq!(replaced["url"].as_str(), Some("file:///c/b/x"));
        assert_eq!(replaced["list"][0].as_str(), Some("file:///c/b"));
        assert_eq!(replaced["n"].as_integer(), Some(1));
    }

    #[test]
    fn addresses_name_the_side_directory() {
        let http = Addresses {
            server: Some("http://127.0.0.1:9".into()),
        };
        assert_eq!(http.side(Path::new("/w/case-1/a")), "http://127.0.0.1:9/a");
        let file = Addresses { server: None };
        assert_eq!(file.side(Path::new("/w/case-1/b")), "file:///w/case-1/b");
        assert_eq!(file_url(Path::new("C:\\w\\b")), "file:///C:/w/b");
    }

    #[test]
    fn moves_addresses_in_configuration_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path().join("repo/.git");
        std::fs::create_dir_all(git_dir.join("objects")).unwrap();
        std::fs::write(git_dir.join("config"), "url = http://h/base/up.git\n").unwrap();
        std::fs::write(git_dir.join("objects/config"), "http://h/base/x").unwrap();
        std::fs::write(dir.path().join("repo/notes.txt"), "http://h/base/up.git").unwrap();
        move_addresses(
            dir.path(),
            Path::new("/w/base"),
            "http://h/base",
            "http://h/a",
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(git_dir.join("config")).unwrap(),
            "url = http://h/a/up.git\n"
        );
        assert_eq!(
            std::fs::read_to_string(git_dir.join("objects/config")).unwrap(),
            "http://h/base/x"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("repo/notes.txt")).unwrap(),
            "http://h/base/up.git"
        );
    }

    #[test]
    fn moves_the_location_of_linked_working_trees() {
        let dir = tempfile::tempdir().unwrap();
        let registration = dir.path().join("repo/.git/worktrees/wt");
        std::fs::create_dir_all(&registration).unwrap();
        std::fs::write(registration.join("gitdir"), "/w/base/wt/.git\n").unwrap();
        std::fs::create_dir_all(dir.path().join("wt")).unwrap();
        std::fs::write(
            dir.path().join("wt/.git"),
            "gitdir: /w/base/repo/.git/worktrees/wt\n",
        )
        .unwrap();
        let side = dir.path().to_string_lossy().into_owned();
        move_addresses(
            dir.path(),
            Path::new("/w/base"),
            "http://h/base",
            "http://h/base",
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(registration.join("gitdir")).unwrap(),
            format!("{side}/wt/.git\n")
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("wt/.git")).unwrap(),
            format!("gitdir: {side}/repo/.git/worktrees/wt\n")
        );
    }

    #[test]
    fn validation_rejects_unknown_corpus_repositories() {
        let text = "id = \"x/y\"\ndescription = \"d.\"\ncorpus = \"nowhere\"\n[git]\nargs = [\"status\"]\n[nurt]\ncommand = \"git.status\"\n";
        let scenario = Scenario::parse(text, Path::new("x/y.toml"), "x/y").unwrap();
        let catalog = Catalog::git().unwrap();
        assert!(
            validate(&scenario, &catalog)
                .unwrap_err()
                .contains("does not exist")
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
