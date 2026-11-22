// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Runner of scenarios: preparation, the two sides, comparison, undo and the
//! summary of a run.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nurt_contracts::stage::ExecCtx;

use crate::bind::{Catalog, ScenarioHost};
use crate::compare::{DifferenceReport, Sides, compare};
use crate::corpus::{self, Corpus, REPO_DIR};
use crate::error::{HarnessError, Result, io_error};
use crate::fsutil;
use crate::git::{GitEnv, GitProgram, expect_exit, join_args};
use crate::journal::{self, RecordingJournal, UndoResult};
use crate::normalize::{Baseline, Normalizer, RepoState};
use crate::scenario::{Expectation, Platform, Scenario, Section, SetupStep, StateSelector, Transport};
use crate::stage::{CommandTable, StageResult, run_stage};
use crate::time::FixedClock;

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
        std::fs::write(&config, b"").map_err(io_error("write", &config))?;
        Ok(dirs)
    }
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

impl Runner {
    /// Creates a runner with a fresh work directory.
    ///
    /// # Errors
    ///
    /// Fails with [`HarnessError`] when the catalog is invalid or the work
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
                        let mut slots = slots.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let status_of = |findings: Findings, git_only: bool| {
            let status = if !findings.failures.is_empty() {
                Status::Failed(findings.failures)
            } else if git_only {
                Status::GitOnly
            } else {
                Status::Passed
            };
            (status, findings.notes)
        };
        if let Some(reason) = self.skip_reason(scenario) {
            return Outcome {
                id: scenario.id.clone(),
                status: Status::Skipped(reason),
                notes: Vec::new(),
            };
        }
        let case_root = self.work.path().join(format!("case-{ordinal}"));
        let mut findings = Findings::default();
        let git_only = match self.run_case(scenario, &case_root, &mut findings) {
            Ok(git_only) => git_only,
            Err(error) => {
                findings.fail(format!("preparation failed: {error}"));
                false
            }
        };
        let failed = !findings.failures.is_empty();
        if failed && self.options.keep_failed {
            findings.notes.push(format!("case directory kept at {}", case_root.display()));
        } else {
            let _ = fsutil::remove_entry(&case_root);
        }
        let (status, notes) = status_of(findings, git_only);
        Outcome {
            id: scenario.id.clone(),
            status,
            notes,
        }
    }

    fn skip_reason(&self, scenario: &Scenario) -> Option<String> {
        let platform = Platform::current();
        if !scenario.runs_on(platform) {
            return Some(format!("not run on {}", platform.as_str()));
        }
        if scenario.transport != Transport::File {
            return Some("the http and ssh test servers are not available yet".to_owned());
        }
        match corpus::find(&scenario.corpus) {
            Some(repo) if repo.heavy && !self.options.heavy => {
                Some(format!("corpus repository `{}` is heavy, run with --heavy", repo.name))
            }
            Some(repo) if !repo.platforms.contains(&platform) => Some(format!(
                "corpus repository `{}` cannot exist on {}",
                repo.name,
                platform.as_str()
            )),
            _ => None,
        }
    }

    /// Runs the case; returns `true` when only the `git` side ran.
    fn run_case(&self, scenario: &Scenario, case_root: &Path, findings: &mut Findings) -> Result<bool> {
        let dirs = CaseDirs::create(case_root)?;
        let env = GitEnv::new(&dirs.home, scenario.clock);
        self.prepare(scenario, &dirs, &env)?;
        let repo_a = dirs.a.join(REPO_DIR);
        let repo_b = dirs.b.join(REPO_DIR);
        let normalizer = Normalizer::new(&self.git, &env, scenario.compare.strict);
        let mut compared = scenario.compare.state.clone();
        let fsck = StateSelector::main(Section::Fsck);
        compared.push(fsck.clone());
        let standard: Vec<StateSelector> = Section::STANDARD.into_iter().map(StateSelector::main).collect();
        let all: Vec<StateSelector> = standard.iter().chain(&compared).cloned().collect();
        let baseline = normalizer.baseline(&repo_b, &all)?;
        let initial = normalizer.snapshot(&dirs.b, &repo_b, &standard, &baseline)?;
        self.run_git_side(scenario, &repo_a, &env, findings)?;
        let state_a = normalizer.snapshot(&dirs.a, &repo_a, &compared, &baseline)?;
        check_fsck(&state_a, &fsck, "git", findings);
        let Some(stage) = self.commands.get(&scenario.nurt.command) else {
            if self.options.require_nurt {
                findings.fail(format!("command `{}` has no implementation in the table", scenario.nurt.command));
            }
            return Ok(true);
        };
        let journal = RecordingJournal::new(dirs.store.clone());
        let result = self.run_nurt_side(scenario, stage.as_ref(), &dirs, &env, &journal, findings)?;
        let state_b = normalizer.snapshot(&dirs.b, &repo_b, &compared, &baseline)?;
        check_fsck(&state_b, &fsck, "nurt", findings);
        let differences = compare(&state_a, &state_b, &scenario.compare.state, &scenario.compare.ignore);
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
                Ok(UndoResult::Undone(_)) => check_initial("undo did not restore the initial state", findings)?,
                Ok(UndoResult::Nothing) => findings.fail("undo found no committed operation"),
                Err(message) => findings.fail(format!("undo failed: {message}")),
            }
        }
        Ok(false)
    }

    fn prepare(&self, scenario: &Scenario, dirs: &CaseDirs, env: &GitEnv) -> Result<()> {
        self.corpus.materialize(&scenario.corpus, &dirs.base)?;
        let repo = dirs.base.join(REPO_DIR);
        for step in &scenario.setup {
            match step {
                SetupStep::Git { args, cwd } => {
                    let dir = cwd.as_ref().map_or_else(|| repo.clone(), |cwd| repo.join(cwd));
                    std::fs::create_dir_all(&dir).map_err(io_error("create directory", &dir))?;
                    self.git.run_ok(env, &dir, args)?;
                }
                SetupStep::Write(write) => fsutil::apply_write(&repo, write)?,
            }
        }
        for write in &scenario.files {
            fsutil::apply_write(&repo, write)?;
        }
        fsutil::copy_tree(&dirs.base, &dirs.a)?;
        fsutil::copy_tree(&dirs.base, &dirs.b)?;
        for side in [&dirs.a, &dirs.b] {
            let copy = side.join(REPO_DIR);
            if copy.join(".git").exists() {
                self.git.run(env, &copy, &["update-index", "-q", "--refresh"])?;
            }
        }
        Ok(())
    }

    fn run_git_side(&self, scenario: &Scenario, repo: &Path, env: &GitEnv, findings: &mut Findings) -> Result<()> {
        let dir = scenario.git.cwd.as_ref().map_or_else(|| repo.to_path_buf(), |cwd| repo.join(cwd));
        let last = scenario.git.steps.len().saturating_sub(1);
        for (index, args) in scenario.git.steps.iter().enumerate() {
            let output = self.git.run(env, &dir, args)?;
            let expected = if index == last { scenario.git.expect_exit } else { 0 };
            if let Err(error) = expect_exit(&output, expected, &dir, args) {
                findings.fail(format!("git side: {error}"));
                return Ok(());
            }
        }
        Ok(())
    }

    fn run_nurt_side(
        &self,
        scenario: &Scenario,
        stage: &dyn nurt_contracts::stage::StageImpl,
        dirs: &CaseDirs,
        env: &GitEnv,
        journal: &RecordingJournal,
        findings: &mut Findings,
    ) -> Result<StageResult> {
        let repo = dirs.b.join(REPO_DIR);
        let binding = self
            .catalog
            .bind(&scenario.nurt.command, &scenario.nurt.params, &repo)
            .map_err(|message| crate::error::scenario_error(&scenario.source, message))?;
        if !binding.unknown.is_empty() {
            findings.notes.push(format!(
                "parameters missing from catalog/git.toml: {}",
                binding.unknown.join(", ")
            ));
        }
        let spec = self
            .catalog
            .command(&scenario.nurt.command)
            .ok_or_else(|| HarnessError::Catalog(format!("`{}` disappeared", scenario.nurt.command)))?;
        let host = ScenarioHost::new(env, &dirs.home, &dirs.root, scenario.clock);
        let ctx = ExecCtx::builder(Arc::new(FixedClock::new(scenario.clock)), Arc::new(host)).build();
        let before = journal::snapshot(&dirs.b)?;
        let result = run_stage(stage, spec, &binding.args, &ctx, journal);
        let after = journal::snapshot(&dirs.b)?;
        for path in journal::undeclared_changes(&before, &after, &journal.declared()) {
            findings.fail(format!(
                "write outside the transaction: {}",
                fsutil::relative_key(&dirs.b, &path)
            ));
        }
        for violation in journal.violations() {
            findings.fail(format!("journal protocol violation: {violation}"));
        }
        check_expectation(scenario.nurt.expect, &result, findings);
        Ok(result)
    }
}

fn check_fsck(state: &RepoState, fsck: &StateSelector, side: &str, findings: &mut Findings) {
    let result = state.section(fsck).and_then(|section| section.get("result"));
    if let Some(result) = result.filter(|result| result.as_str() != "ok") {
        findings.fail(format!("git fsck --strict failed on the {side} side: {result}"));
    }
    for (selector, section) in state.selectors().filter_map(|selector| Some((selector, state.section(selector)?))) {
        if selector.section == Section::Fsck && selector != fsck {
            if let Some(result) = section.get("result").filter(|result| result.as_str() != "ok") {
                findings.fail(format!("git fsck --strict failed for {selector} on the {side} side: {result}"));
            }
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
        findings.fail(format!("nurt side returned {actual} ({message}), the scenario expects {expect}"));
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
    let root = std::env::temp_dir().join("repo");
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
        self.outcomes.iter().filter(|outcome| wanted(&outcome.status)).count()
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

/// Text of a list of arguments, for messages of callers.
#[must_use]
pub fn describe_args(args: &[String]) -> String {
    join_args(args)
}

/// Placeholder for unused baseline reads, keeping the type exported.
#[doc(hidden)]
pub type ScenarioBaseline = Baseline;

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(id: &str, status: Status) -> Outcome {
        Outcome {
            id: id.into(),
            status,
            notes: vec![],
        }
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
        assert!(text.contains("4 total, 1 passed, 1 git side only, 1 skipped, 1 failed, 0 invalid"));
        assert!(!summary.success());
        let passing = Summary {
            outcomes: &outcomes[..3],
            invalid: &[],
        };
        assert!(passing.success());
    }

    #[test]
    fn expectation_mismatch_is_a_failure() {
        let mut findings = Findings::default();
        let result = StageResult {
            code: Some(nurt_contracts::error::ErrorCode::GitDirty),
            message: Some("local changes".into()),
            committed: None,
            records: 0,
            effect: true,
        };
        check_expectation(Expectation::Ok, &result, &mut findings);
        assert_eq!(findings.failures.len(), 1);
        assert!(findings.failures[0].contains("E_GIT_DIRTY"));
    }

    #[test]
    fn validation_rejects_unknown_corpus_repositories() {
        let text = "id = \"x/y\"\ndescription = \"d.\"\ncorpus = \"nowhere\"\n[git]\nargs = [\"status\"]\n[nurt]\ncommand = \"git.status\"\n";
        let scenario = Scenario::parse(text, Path::new("x/y.toml"), "x/y").unwrap();
        let catalog = Catalog::git().unwrap();
        assert!(validate(&scenario, &catalog).unwrap_err().contains("does not exist"));
    }
}
