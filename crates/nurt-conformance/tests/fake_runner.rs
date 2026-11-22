// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! The runner against the fake server: every mandatory case passes on the
//! conforming fake server, and every deviation of the fake server makes a
//! mandatory case fail.
//!
//! A fake server process started for one case parses only the case files that
//! name it, but the probe process of every run loads the whole suite before it
//! answers `initialize`, which takes long on a loaded machine. The limits of
//! the start and of the cases are therefore large; only the hanging fake server
//! runs with a short case limit, because reaching that limit is the checked
//! result. The runs of the deviations are independent and run in parallel.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use nurt_conformance::Error;
use nurt_conformance::case::Kind;
use nurt_conformance::fake::Break;
use nurt_conformance::report::ConformanceReport;
use nurt_conformance::runner::{self, RunOptions};
use nurt_conformance::schemas::SchemaSet;

const BIN: &str = env!("CARGO_BIN_EXE_nurt-conformance");

/// Time limit of the start of a fake server process and of `initialize`.
const STARTUP_LIMIT: Duration = Duration::from_secs(300);

/// Time limit of a case whose result does not depend on the limit.
const CASE_LIMIT: Duration = Duration::from_secs(120);

/// Time limit of a case against the hanging fake server.
const HANG_LIMIT: Duration = Duration::from_secs(2);

/// Values of `--startup-timeout` and `--timeout` for the program.
const STARTUP_ARG: &str = "300s";
const CASE_ARG: &str = "120s";

/// Number of cases run in parallel by the complete run.
const JOBS: usize = 4;

/// Failure message of a call that reached the case limit.
const TIME_LIMIT: &str = "time limit reached";

fn conformance() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

fn command(deviation: Option<Break>) -> String {
    let mut command = format!("'{BIN}' fake-serve --cases '{}'", conformance().display());
    if let Some(deviation) = deviation {
        command.push_str(&format!(" --break {}", deviation.name()));
    }
    command
}

fn run(
    kinds: Vec<Kind>,
    filters: Vec<String>,
    deviation: Option<Break>,
    jobs: usize,
    timeout: Duration,
) -> Result<ConformanceReport, Error> {
    runner::run(&RunOptions {
        command: command(deviation),
        cases: conformance(),
        filters,
        kinds,
        jobs,
        startup_timeout: STARTUP_LIMIT,
        timeout,
        second_volume: None,
    })
}

#[test]
fn every_mandatory_case_passes_on_the_fake_server() {
    let report = run(
        vec![Kind::Golden, Kind::Bind, Kind::Error],
        Vec::new(),
        None,
        JOBS,
        CASE_LIMIT,
    )
    .unwrap();
    let failed: Vec<String> = report
        .results
        .iter()
        .filter(|result| !result.failures.is_empty())
        .map(|result| format!("{}: {:?}", result.id, result.failures))
        .collect();
    assert!(failed.is_empty(), "{}", failed.join("\n"));
    assert!(
        report.summary.mandatory.passed > 100,
        "{:?}",
        report.summary
    );
    assert_eq!(report.summary.optional.failed, 0);
    assert!(
        report.summary.optional.passed > 0,
        "testkit cases run on the fake server"
    );
    assert!(
        !report.summary.conformant,
        "a filtered run is never conformant"
    );
    assert!(report.implementation.executable_digest.is_some());
    let value = serde_json::to_value(&report).unwrap();
    let schemas = SchemaSet::new().unwrap();
    assert_eq!(schemas.validate("conformance-report", &value), Ok(()));
}

#[test]
fn every_deviation_of_the_fake_server_is_detected() {
    let probes = [
        (Break::Tree, "golden/task-01-"),
        (Break::Bound, "golden/task-01-"),
        (Break::Hash, "golden/task-01-"),
        (Break::ErrorCode, "errors/execution/read-missing-file"),
        (Break::Report, "errors/execution/read-missing-file"),
        (Break::Schema, "golden/schema/report"),
        (Break::Effect, "bind/check/no-effect-of-a-write"),
        (Break::Protocol, "golden/task-02-"),
        (Break::Crash, "golden/task-03-"),
        (Break::Hang, "golden/task-04-"),
    ];
    let outcomes: Vec<(Break, &str, Result<ConformanceReport, Error>)> =
        std::thread::scope(|scope| {
            let runs: Vec<_> = probes
                .iter()
                .map(|&(deviation, filter)| {
                    let limit = if deviation == Break::Hang {
                        HANG_LIMIT
                    } else {
                        CASE_LIMIT
                    };
                    scope.spawn(move || {
                        run(
                            Vec::new(),
                            vec![filter.to_owned()],
                            Some(deviation),
                            1,
                            limit,
                        )
                    })
                })
                .collect();
            probes
                .iter()
                .zip(runs)
                .map(|(&(deviation, filter), run)| (deviation, filter, run.join().unwrap()))
                .collect()
        });
    for (deviation, filter, outcome) in outcomes {
        let name = deviation.name();
        let hangs = deviation == Break::Hang;
        match outcome {
            Ok(report) => {
                assert_eq!(report.results.len(), 1, "{name}: {filter}");
                assert!(
                    report.summary.mandatory.failed == 1 && !report.mandatory_passed(),
                    "{name} not detected: {:?}",
                    report.results
                );
                let timed_out = report.results[0]
                    .failures
                    .iter()
                    .any(|failure| failure.message.contains(TIME_LIMIT));
                assert_eq!(
                    timed_out, hangs,
                    "{name}: the case limit decides only the hanging server: {:?}",
                    report.results
                );
            }
            Err(error) => assert!(
                deviation == Break::Protocol && error.to_string().contains("protocol violation"),
                "{name}: unexpected error {error}"
            ),
        }
    }
}

#[test]
fn the_program_reports_through_its_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let report = dir.path().join("report.json");
    let cases = conformance();
    let pass = Command::new(BIN)
        .args([
            "run",
            "--impl",
            &command(None),
            "--filter",
            "golden/task-01-",
            "--jobs",
            "1",
            "--startup-timeout",
            STARTUP_ARG,
            "--timeout",
            CASE_ARG,
            "--quiet",
        ])
        .arg("--cases")
        .arg(&cases)
        .arg("--report")
        .arg(&report)
        .output()
        .unwrap();
    assert_eq!(
        pass.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&pass.stdout)
    );
    assert!(String::from_utf8_lossy(&pass.stdout).contains("mandatory: 1 passed"));
    assert!(report.is_file());

    let fail = Command::new(BIN)
        .args([
            "run",
            "--impl",
            &command(Some(Break::Tree)),
            "--filter",
            "golden/task-01-",
            "--startup-timeout",
            STARTUP_ARG,
            "--timeout",
            CASE_ARG,
        ])
        .arg("--cases")
        .arg(&cases)
        .output()
        .unwrap();
    assert_eq!(fail.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&fail.stdout).contains("FAIL golden/task-01-"));

    for args in [
        vec!["run", "--impl", "x", "--kind", "other"],
        vec!["run", "--impl", "x", "--timeout", "soon"],
        vec!["run", "--impl", "x", "--startup-timeout", "soon"],
        vec!["fake-serve", "--break", "everything"],
    ] {
        let output = Command::new(BIN)
            .args(&args)
            .arg("--cases")
            .arg(&cases)
            .env_remove("NURT_CONFORMANCE_CASES")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(String::from_utf8_lossy(&output.stderr).starts_with("nurt-conformance: "));
    }
    let missing = Command::new(BIN)
        .args(["serve", "--stdio"])
        .env_remove("NURT_CONFORMANCE_CASES")
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
}

#[test]
fn serve_reads_the_cases_from_the_environment() {
    use std::io::Write as _;
    let mut child = Command::new(BIN)
        .args(["serve", "--stdio"])
        .env("NURT_CONFORMANCE_CASES", conformance())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, r#"{{"jsonrpc": "2.0", "id": 1, "method": "parse", "params": {{"program": "find(\"bindings\")", "variant": "b"}}}}"#).unwrap();
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    let line = String::from_utf8(output.stdout).unwrap();
    let message: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(message["result"]["ast"]["nurt"], "1.0");
}
