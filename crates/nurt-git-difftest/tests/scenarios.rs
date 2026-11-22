// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Runs the differential scenarios below `scenarios/`.
//!
//! Arguments after `--` of `cargo test`: substrings of scenario identifiers
//! select scenarios (`--exact` requires equality, `--skip <text>` excludes);
//! `--heavy` (or `--include-ignored`) runs scenarios on heavy corpus
//! repositories; `--require-nurt` fails scenarios whose command has no
//! implementation; `--keep-failed` keeps case directories of failures;
//! `--test-threads <n>` sets the parallelism; `--list` prints the scenarios.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use nurt_git_difftest::bind::Catalog;
use nurt_git_difftest::error::DifftestError;
use nurt_git_difftest::git::{Availability, GitProgram, MINIMUM_VERSION};
use nurt_git_difftest::runner::{RunOptions, Runner, Summary, validate};
use nurt_git_difftest::scenario::{Scenario, load_all, scenario_id};
use nurt_git_difftest::stage::CommandTable;

/// Options of the command line that take a value and do not affect the run.
const IGNORED_WITH_VALUE: &[&str] = &["--format", "--color", "--logfile", "-Z"];

#[derive(Debug, Default)]
struct Args {
    filters: Vec<String>,
    skips: Vec<String>,
    exact: bool,
    list: bool,
    threads: Option<usize>,
    options: RunOptions,
}

impl Args {
    fn parse(mut raw: impl Iterator<Item = String>) -> Self {
        let mut args = Self::default();
        while let Some(arg) = raw.next() {
            match arg.as_str() {
                "--exact" => args.exact = true,
                "--list" => args.list = true,
                "--heavy" | "--include-ignored" | "--ignored" => args.options.heavy = true,
                "--require-nurt" => args.options.require_nurt = true,
                "--keep-failed" => args.options.keep_failed = true,
                "--skip" => args.skips.extend(raw.next()),
                "--test-threads" => args.threads = raw.next().and_then(|value| value.parse().ok()),
                flag if IGNORED_WITH_VALUE.contains(&flag) => {
                    raw.next();
                }
                flag if flag.starts_with('-') => {}
                filter => args.filters.push(filter.to_owned()),
            }
        }
        args
    }

    /// Returns `true` when an invalid file belongs to a selected scenario.
    fn selects_error(&self, root: &Path, error: &DifftestError) -> bool {
        match error {
            DifftestError::Scenario { path, .. } => {
                scenario_id(root, path).is_ok_and(|id| self.selects(&id))
            }
            _ => true,
        }
    }

    fn selects(&self, id: &str) -> bool {
        let matches = |filter: &String| {
            if self.exact {
                id == filter
            } else {
                id.contains(filter.as_str())
            }
        };
        (self.filters.is_empty() || self.filters.iter().any(matches))
            && !self.skips.iter().any(|skip| id.contains(skip.as_str()))
    }
}

/// Implementations of the commands under test, registered the way the
/// executor registers them. `git.pull` needs a fetch engine from the caller
/// and is not in the table.
fn commands() -> nurt_contracts::error::Result<CommandTable> {
    let mut table = CommandTable::new();
    nurt_git::register(&mut table)?;
    nurt_git_ops::register(&mut table)?;
    Ok(table)
}

fn main() -> ExitCode {
    let args = Args::parse(std::env::args().skip(1));
    let mut out = std::io::stdout().lock();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scenarios");
    let (scenarios, errors) = match load_all(&root) {
        Ok(loaded) => loaded,
        Err(error) => {
            let _ = writeln!(out, "cannot list scenarios: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut invalid: Vec<String> = errors
        .iter()
        .filter(|error| args.selects_error(&root, error))
        .map(ToString::to_string)
        .collect();
    let catalog = match Catalog::git() {
        Ok(catalog) => catalog,
        Err(error) => {
            let _ = writeln!(out, "{error}");
            return ExitCode::FAILURE;
        }
    };
    let mut selected: Vec<Scenario> = Vec::new();
    for scenario in scenarios
        .into_iter()
        .filter(|scenario| args.selects(&scenario.id))
    {
        match validate(&scenario, &catalog) {
            Ok(()) => selected.push(scenario),
            Err(message) => invalid.push(format!("{}: {message}", scenario.source.display())),
        }
    }
    if args.list {
        for scenario in &selected {
            let _ = writeln!(out, "{}: test", scenario.id);
        }
        return ExitCode::SUCCESS;
    }
    let git = match GitProgram::locate() {
        Availability::Ready(git) => git,
        Availability::Missing => {
            let _ = writeln!(
                out,
                "skipping {} differential scenarios: git is not on the search path",
                selected.len()
            );
            return report(&mut out, &[], &invalid);
        }
        Availability::TooOld(version) => {
            let _ = writeln!(
                out,
                "skipping {} differential scenarios: git {version} is older than {MINIMUM_VERSION}",
                selected.len()
            );
            return report(&mut out, &[], &invalid);
        }
    };
    let table = match commands() {
        Ok(table) => table,
        Err(error) => {
            let _ = writeln!(out, "cannot register the stage implementations: {error}");
            return ExitCode::FAILURE;
        }
    };
    let _ = writeln!(
        out,
        "running {} differential scenarios with git {}; nurt side: {}",
        selected.len(),
        git.version(),
        table.names().collect::<Vec<_>>().join(", ")
    );
    let runner = match Runner::new(git, table, args.options) {
        Ok(runner) => runner,
        Err(error) => {
            let _ = writeln!(out, "{error}");
            return ExitCode::FAILURE;
        }
    };
    let threads = args
        .threads
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, usize::from));
    let outcomes = runner.run_all(&selected, threads);
    report(&mut out, &outcomes, &invalid)
}

fn report(
    out: &mut impl std::io::Write,
    outcomes: &[nurt_git_difftest::runner::Outcome],
    invalid: &[String],
) -> ExitCode {
    let summary = Summary { outcomes, invalid };
    let _ = write!(out, "{summary}");
    if summary.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
