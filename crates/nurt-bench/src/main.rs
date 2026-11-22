// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Command line of the Danaco Nurt benchmark.

#![forbid(unsafe_code)]

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use nurt_bench::analyze;
use nurt_bench::dataset;
use nurt_bench::model::ModelSpec;
use nurt_bench::oracle::{CommandOracle, DEFAULT_ORACLE_COMMAND};
use nurt_bench::protocol::Protocol;
use nurt_bench::rpc::{CommandLauncher, ServeLauncher};
use nurt_bench::runner::{RunReport, RunSettings, RunStatus, Runner};
use nurt_bench::stats::Bootstrap;
use nurt_bench::task::{self, HostOs, TaskFile};
use nurt_bench::{Condition, Result, variants};

/// Confidence level of the intervals.
const LEVEL: f64 = 0.95;

#[derive(Parser)]
#[command(
    name = "nurt-bench",
    version,
    about = "Run, analyse and publish the Danaco Nurt benchmark."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run tasks under one condition with one model.
    Run(RunArgs),
    /// Analyse run logs and write result tables.
    Analyze(AnalyzeArgs),
    /// Generate syntax variants B and C of the reference programs.
    Variants(VariantsArgs),
    /// Generate and validate the pair data set.
    Pairs(PairsArgs),
}

#[derive(clap::Args)]
struct RunArgs {
    /// Directory of the task set.
    #[arg(long, default_value = "bench/tasks")]
    tasks: PathBuf,
    /// Condition: bash, pwsh, nurt or nurt-groups.
    #[arg(long)]
    condition: Condition,
    /// Model configuration file, or mock:<file> for the mock model.
    #[arg(long)]
    model: String,
    /// Number of runs per task, with seeds 1 to N.
    #[arg(long, default_value_t = 3)]
    seeds: u64,
    /// Output directory of the run logs.
    #[arg(long, default_value = "runs")]
    out: PathBuf,
    /// Directory of the protocol files.
    #[arg(long, default_value = "bench/protocol")]
    protocol: PathBuf,
    /// Directory of the shared initial trees.
    #[arg(long, default_value = "bench/fixtures")]
    fixtures: PathBuf,
    /// Working directory of the oracle command.
    #[arg(long, default_value = "bench/oracles")]
    oracles: PathBuf,
    /// Oracle command; reads the run on standard input, writes the verdict.
    #[arg(long, default_value = DEFAULT_ORACLE_COMMAND)]
    oracle_command: String,
    /// Command that starts the server, for the Danaco Nurt conditions.
    #[arg(long)]
    nurt: Option<String>,
    /// Run only tasks whose identifier contains the text or matches the
    /// pattern with `*`; repeatable.
    #[arg(long)]
    filter: Vec<String>,
    /// Number of runs executed in parallel.
    #[arg(long, default_value_t = 1)]
    jobs: usize,
}

#[derive(clap::Args)]
struct AnalyzeArgs {
    /// Directory of the run logs.
    runs: PathBuf,
    /// Output directory of the results.
    #[arg(long, default_value = "results")]
    out: PathBuf,
    /// Number of bootstrap resamples.
    #[arg(long, default_value_t = 10_000)]
    resamples: usize,
    /// Seed of the bootstrap.
    #[arg(long, default_value_t = 1)]
    seed: u64,
}

#[derive(clap::Args)]
struct VariantsArgs {
    /// Directory of the task set.
    #[arg(long, default_value = "bench/tasks")]
    tasks: PathBuf,
    /// Command that starts the server.
    #[arg(long)]
    nurt: String,
    /// Output file (JSON Lines).
    #[arg(long)]
    out: PathBuf,
}

#[derive(clap::Args)]
struct PairsArgs {
    /// Directory of the task set.
    #[arg(long, default_value = "bench/tasks")]
    tasks: PathBuf,
    /// Directory of the shared initial trees.
    #[arg(long, default_value = "bench/fixtures")]
    fixtures: PathBuf,
    /// Directory of the command documentation.
    #[arg(long)]
    docs: Option<PathBuf>,
    /// Habit table `nurt-bench-habits/1`; repeatable.
    #[arg(long)]
    habits: Vec<PathBuf>,
    /// Directory of run logs whose passed Danaco Nurt runs give pairs.
    #[arg(long)]
    runs: Option<PathBuf>,
    /// Write the candidates without validation.
    #[arg(long)]
    list_only: bool,
    /// Output directory.
    #[arg(long)]
    out: PathBuf,
}

fn say(text: &str) {
    let _ = writeln!(std::io::stdout().lock(), "{text}");
}

fn note(text: &str) {
    let _ = writeln!(std::io::stderr().lock(), "{text}");
}

fn select(tasks: Vec<TaskFile>, filters: &[String]) -> Vec<TaskFile> {
    if filters.is_empty() {
        return tasks;
    }
    tasks
        .into_iter()
        .filter(|file| {
            filters
                .iter()
                .any(|filter| task::matches_filter(&file.task.id, filter))
        })
        .collect()
}

fn progress(report: &RunReport) {
    let status = match &report.status {
        RunStatus::Logged { outcome, passed } => {
            let outcome = serde_json::to_value(outcome)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default();
            format!(
                "{outcome} {}",
                if *passed { "passed" } else { "not passed" }
            )
        }
        RunStatus::Existing => "complete log exists".to_owned(),
        RunStatus::Failed(message) => format!("not run: {message}"),
    };
    note(&format!("{} {status}", report.run_id));
}

fn run(args: RunArgs) -> Result<bool> {
    let tasks = select(task::load_tasks(&args.tasks)?, &args.filter);
    let settings = RunSettings {
        protocol: Protocol::load(&args.protocol)?,
        condition: args.condition,
        model: ModelSpec::from_argument(&args.model)?,
        seeds: (1..=args.seeds).collect(),
        out: args.out,
        fixtures: args.fixtures,
        host: HostOs::current(),
        jobs: args.jobs,
    };
    let oracle = CommandOracle::new(&args.oracle_command, args.oracles)?;
    let launcher = args.nurt.as_deref().map(CommandLauncher::new).transpose()?;
    let runner = Runner::new(
        &settings,
        &oracle,
        launcher.as_ref().map(|l| l as &dyn ServeLauncher),
    )?;
    let summary = runner.run_series(&tasks, &progress);
    say(&format!(
        "logged {} (passed {}, errors {}), existing {}, not run {}, other platform {}",
        summary.logged,
        summary.passed,
        summary.errors,
        summary.existing,
        summary.failed,
        summary.other_platform
    ));
    Ok(summary.failed == 0)
}

fn analyze(args: &AnalyzeArgs) -> Result<bool> {
    let bootstrap = Bootstrap {
        resamples: args.resamples,
        seed: args.seed,
        level: LEVEL,
    };
    let analysis = analyze::analyse_directory(&args.runs, &args.out, &bootstrap)?;
    say(&format!(
        "analysed {} runs, excluded {}; results in {}",
        analysis.runs,
        analysis.excluded_errors,
        args.out.display()
    ));
    Ok(true)
}

fn launch(command: &str) -> Result<(tempfile::TempDir, nurt_bench::rpc::RpcClient)> {
    let scratch = tempfile::tempdir()
        .map_err(|error| nurt_bench::BenchError::io(&std::env::temp_dir(), error))?;
    let client = CommandLauncher::new(command)?.launch(scratch.path())?;
    Ok((scratch, client))
}

fn variants(args: &VariantsArgs) -> Result<bool> {
    let tasks = task::load_tasks(&args.tasks)?;
    let (_scratch, mut client) = launch(&args.nurt)?;
    let (generated, failed) = variants::generate_file(&mut client, &tasks, &args.out)?;
    client.shutdown();
    say(&format!(
        "variants of {generated} tasks, {failed} tasks with errors"
    ));
    Ok(failed == 0)
}

fn pairs(args: &PairsArgs) -> Result<bool> {
    let tasks = task::load_tasks(&args.tasks)?;
    let mut candidates = dataset::from_tasks(&tasks, &args.fixtures)?;
    if let Some(docs) = &args.docs {
        candidates.extend(dataset::from_docs(docs)?);
    }
    for habits in &args.habits {
        candidates.extend(dataset::from_habits(habits)?);
    }
    if let Some(runs) = &args.runs {
        candidates.extend(dataset::from_runs(runs, &tasks, &args.fixtures)?);
    }
    let summary = if args.list_only {
        dataset::build(candidates, None, &args.out)?
    } else {
        let mut validator = dataset::EngineValidator::new(dataset::open_engine(None)?);
        dataset::build(candidates, Some(&mut validator), &args.out)?
    };
    for count in &summary.sources {
        let source = serde_json::to_value(count.source)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        say(&format!(
            "{source}: {} candidates, {} accepted, {} rejected",
            count.candidates, count.accepted, count.rejected
        ));
    }
    say(&format!("accepted pairs: {}", summary.accepted));
    Ok(true)
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Command::Run(args) => run(args),
        Command::Analyze(args) => analyze(&args),
        Command::Variants(args) => variants(&args),
        Command::Pairs(args) => pairs(&args),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            note(&format!("nurt-bench: {error}"));
            ExitCode::FAILURE
        }
    }
}
