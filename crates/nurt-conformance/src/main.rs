// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Command line of the conformance suite of Danaco Nurt.
//!
//! `nurt-conformance run` runs the suite against an implementation;
//! `nurt-conformance fake-serve` answers the protocol from the cases.

#![forbid(unsafe_code)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use nurt_conformance::Error;
use nurt_conformance::binding;
use nurt_conformance::case::{Kind, Suite};
use nurt_conformance::fake::{BREAK_ENV, Break, CASE_ENV, CASES_ENV, FakeServer};
use nurt_conformance::runner::{self, RunOptions};
use nurt_conformance::schemas::SchemaSet;

/// Exit code when a mandatory case failed.
const EXIT_FAILED: u8 = 1;

/// Exit code of an invalid invocation or an unreadable suite.
const EXIT_INVALID: u8 = 2;

/// Directory of the binding cases below the case directory.
const BINDINGS_DIR: &str = "bindings";

#[derive(Debug, Parser)]
#[command(
    name = "nurt-conformance",
    version,
    about = "Conformance suite of Danaco Nurt"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the suite against an implementation.
    Run {
        /// Command line that starts the server, for example "nurt serve --stdio".
        #[arg(long = "impl", value_name = "COMMAND")]
        implementation: String,
        /// Root directory of the cases.
        #[arg(long, default_value = "conformance")]
        cases: PathBuf,
        /// Run only cases whose identifier contains the text or matches the pattern.
        #[arg(long)]
        filter: Vec<String>,
        /// Run only cases of the kind.
        #[arg(long)]
        kind: Vec<String>,
        /// Number of cases run in parallel [default: number of processors].
        #[arg(long)]
        jobs: Option<usize>,
        /// Time limit of one case, for example 30s or 500ms.
        #[arg(long, default_value = "30s")]
        timeout: String,
        /// Write the report nurt-conformance-report/1 to the file.
        #[arg(long)]
        report: Option<PathBuf>,
        /// Print only failed cases and the summary.
        #[arg(long)]
        quiet: bool,
    },
    /// Answer nurt-rpc/1 on standard input and output from the cases.
    #[command(alias = "serve")]
    FakeServe {
        /// Root directory of the cases [default: NURT_CONFORMANCE_CASES].
        #[arg(long)]
        cases: Option<PathBuf>,
        /// Deviate from the expected behavior [default: NURT_CONFORMANCE_BREAK].
        #[arg(long = "break", value_name = "KIND")]
        deviation: Option<String>,
        /// Use standard input and output; the only transport.
        #[arg(long)]
        stdio: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let outcome = match cli.command {
        Command::Run {
            implementation,
            cases,
            filter,
            kind,
            jobs,
            timeout,
            report,
            quiet,
        } => run(RunArgs {
            implementation,
            cases,
            filter,
            kind,
            jobs,
            timeout,
            report,
            quiet,
        }),
        Command::FakeServe {
            cases, deviation, ..
        } => fake_serve(cases, deviation),
    };
    match outcome {
        Ok(code) => code,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "nurt-conformance: {error}");
            ExitCode::from(EXIT_INVALID)
        }
    }
}

struct RunArgs {
    implementation: String,
    cases: PathBuf,
    filter: Vec<String>,
    kind: Vec<String>,
    jobs: Option<usize>,
    timeout: String,
    report: Option<PathBuf>,
    quiet: bool,
}

fn run(args: RunArgs) -> Result<ExitCode, Error> {
    let kinds = args
        .kind
        .iter()
        .map(|name| {
            Kind::from_name(name).ok_or_else(|| {
                Error::Invalid(format!("unknown kind {name:?}; use golden, bind, error, exec, journal, correction or parity"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let options = RunOptions {
        command: args.implementation,
        cases: args.cases,
        filters: args.filter,
        kinds,
        jobs: args.jobs.unwrap_or_else(runner::default_jobs),
        timeout: runner::parse_duration(&args.timeout)?,
    };
    let report = runner::run(&options)?;
    if let Some(path) = &args.report {
        runner::write_report(&report, path)?;
    }
    let mut out = std::io::stdout().lock();
    out.write_all(report.to_text(args.quiet).as_bytes())
        .map_err(|e| Error::Invalid(format!("cannot write the summary: {e}")))?;
    Ok(if report.mandatory_passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(EXIT_FAILED)
    })
}

fn fake_serve(cases: Option<PathBuf>, deviation: Option<String>) -> Result<ExitCode, Error> {
    let cases = cases
        .or_else(|| std::env::var_os(CASES_ENV).map(PathBuf::from))
        .ok_or_else(|| Error::Invalid(format!("give --cases or set {CASES_ENV}")))?;
    let deviation = deviation.or_else(|| std::env::var(BREAK_ENV).ok());
    let deviation = deviation
        .map(|name| {
            Break::from_name(&name)
                .ok_or_else(|| Error::Invalid(format!("unknown kind of --break {name:?}")))
        })
        .transpose()?;
    let schemas = SchemaSet::new()?;
    let suite = Suite::load(&cases, &schemas)?;
    let bindings = load_bindings(&cases.join(BINDINGS_DIR), &schemas)?;
    let case_id = std::env::var(CASE_ENV).ok();
    let mut server = FakeServer::new(&suite, &bindings, case_id.as_deref(), deviation)?;
    let stdin = std::io::stdin().lock();
    let stdout = std::io::stdout().lock();
    server
        .serve(stdin, stdout)
        .map_err(|e| Error::Invalid(format!("standard input or output failed: {e}")))?;
    Ok(ExitCode::SUCCESS)
}

fn load_bindings(dir: &Path, schemas: &SchemaSet) -> Result<Vec<binding::BindingFile>, Error> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    Ok(binding::load_dir(dir, schemas)?
        .into_iter()
        .map(|(_, file)| file)
        .collect())
}
