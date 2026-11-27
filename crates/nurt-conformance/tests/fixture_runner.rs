// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-conformance/tests/fixture_runner.rs
// Responsibility:   The runner against the fake server on a small suite that uses the test.
// Dependencies:     Status
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The runner against the fake server on a small suite that uses the test
//! clock, absent members, events, the configuration placeholder
//! `${work_mount}`, the local HTTP server and the second volume. The
//! conforming fake server passes every applicable case; the deviations
//! `events` and `absent` make the matching case fail; `no-clock` makes the
//! case of the test clock skipped, not failed.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use nurt_conformance::fake::Break;
use nurt_conformance::report::{ConformanceReport, Status};
use nurt_conformance::runner::{self, RunOptions};
use nurt_conformance::schemas::SchemaSet;

const BIN: &str = env!("CARGO_BIN_EXE_nurt-conformance");

/// Time limit of the start of a fake server process and of `initialize`.
const STARTUP_LIMIT: Duration = Duration::from_secs(300);

/// Time limit of a case.
const CASE_LIMIT: Duration = Duration::from_secs(120);

/// Number of cases run in parallel.
const JOBS: usize = 3;

/// Cases of the suite; the comments of each case name the checked
/// capability of the runner.
const CASES: &str = r#"format = "nurt-conformance-cases/1"

# Events: exec.started and exec.finished arrive in this order.
[[case]]
id = "exec/events"
kind = "exec"
summary = "The events of a run arrive in order before its response."
program = "count"
expect = { report = { status = "ok" } }
events = [{ t = "exec.started" }, { t = "exec.finished", status = "ok" }]

# Absent members, and a configuration with the mount point of work.
[[case]]
id = "exec/absent"
kind = "exec"
summary = "A successful run reports no error and nothing irreversible."
config = "mount.toml"
program = "sort"
expect = { report = { status = "ok" } }
absent = ["/report/error", "/report/irreversible"]

# Test clock of the feature testkit.
[[case]]
id = "journal/clock"
kind = "journal"
class = "optional:testkit"
summary = "The test clock moves between two calls."

[[case.step]]
program = "history"

[[case.step]]
method = "testkit.clock.advance"
params = { by = "2h" }

[[case.step]]
program = "history last: 1"
expect = { report = { status = "ok" } }

# Local HTTP server.
[[case]]
id = "exec/http"
kind = "exec"
requires = ["fixture:http"]
summary = "A request to the local server names its address."
http_files = { entries = [{ path = "a.txt", type = "file", content_utf8 = "alpha\n" }] }
program = 'http.request "${http_base}/files/a.txt"'
expect = { report = { status = "ok", url = "${http_base}/files/a.txt" } }

# Second volume.
[[case]]
id = "exec/second-volume"
kind = "exec"
requires = ["fixture:second-volume"]
summary = "A file moves from the working directory to the second volume."
fs_before = { entries = [{ path = "a.txt", type = "file", content_utf8 = "a" }] }
second_volume_before = { entries = [{ path = "kept.txt", type = "file", content_utf8 = "k" }] }
program = 'move "a.txt" "${second_volume}/a.txt"'
expect = { report = { status = "ok", target = "${second_volume}/a.txt" } }
fs_after = { entries = [] }
second_volume_after = { entries = [
    { path = "a.txt", type = "file", content_utf8 = "a" },
    { path = "kept.txt", type = "file", content_utf8 = "k" },
] }
"#;

/// Configuration of `exec/absent`; the runner replaces the placeholder.
const MOUNT_CONFIG: &str = "[journal.volume.\"${work_mount}\"]\nmax_size = \"1mib\"\n";

fn suite() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("VERSION"), "1.0.0\n").unwrap();
    std::fs::write(root.path().join("fixtures.cases.toml"), CASES).unwrap();
    std::fs::write(root.path().join("mount.toml"), MOUNT_CONFIG).unwrap();
    root
}

fn run(
    cases: &Path,
    deviation: Option<Break>,
    second_volume: Option<PathBuf>,
) -> ConformanceReport {
    let mut command = format!("'{BIN}' fake-serve --cases '{}'", cases.display());
    if let Some(deviation) = deviation {
        command.push_str(&format!(" --break {}", deviation.name()));
    }
    runner::run(&RunOptions {
        command,
        cases: cases.to_path_buf(),
        filters: Vec::new(),
        kinds: Vec::new(),
        jobs: JOBS,
        startup_timeout: STARTUP_LIMIT,
        timeout: CASE_LIMIT,
        second_volume,
    })
    .unwrap()
}

fn statuses(report: &ConformanceReport) -> Vec<(&str, Status)> {
    report
        .results
        .iter()
        .map(|result| (result.id.as_str(), result.status))
        .collect()
}

fn failed_checks(report: &ConformanceReport, id: &str) -> Vec<String> {
    report
        .results
        .iter()
        .filter(|result| result.id == id)
        .flat_map(|result| &result.failures)
        .filter_map(|failure| failure.check.clone())
        .collect()
}

#[test]
fn the_runner_provides_the_fixtures_and_detects_their_deviations() {
    let root = suite();
    let cases = root.path();
    let volume = tempfile::tempdir().unwrap();
    let volume_dir = volume.path().to_path_buf();
    let [plain, with_volume, events, absent, no_clock] = std::thread::scope(|scope| {
        let runs = [
            scope.spawn(|| run(cases, None, None)),
            scope.spawn(|| run(cases, None, Some(volume_dir.clone()))),
            scope.spawn(|| run(cases, Some(Break::Events), None)),
            scope.spawn(|| run(cases, Some(Break::Absent), None)),
            scope.spawn(|| run(cases, Some(Break::NoClock), None)),
        ];
        runs.map(|run| run.join().unwrap())
    });

    check_conforming(&plain);
    check_second_volume(&with_volume, volume.path());
    check_deviations(&events, &absent, &no_clock);
}

fn check_conforming(plain: &ConformanceReport) {
    use Status::{Pass, Skip};
    assert_eq!(
        statuses(plain),
        [
            ("exec/absent", Pass),
            ("exec/events", Pass),
            ("exec/http", Pass),
            ("exec/second-volume", Skip),
            ("journal/clock", Pass),
        ],
        "{:?}",
        plain.results
    );
    assert_eq!(
        plain.results[3].reason.as_deref(),
        Some("requires fixture:second-volume")
    );
    assert!(plain.summary.conformant, "{:?}", plain.summary);
    let value = serde_json::to_value(plain).unwrap();
    let schemas = SchemaSet::new().unwrap();
    assert_eq!(schemas.validate("conformance-report", &value), Ok(()));
}

fn check_second_volume(with_volume: &ConformanceReport, volume: &Path) {
    assert!(
        with_volume
            .results
            .iter()
            .all(|result| result.status == Status::Pass),
        "{:?}",
        with_volume.results
    );
    assert_eq!(std::fs::read_dir(volume).unwrap().count(), 0);
}

fn check_deviations(
    events: &ConformanceReport,
    absent: &ConformanceReport,
    no_clock: &ConformanceReport,
) {
    assert_eq!(failed_checks(events, "exec/events"), ["events"]);
    assert!(!events.mandatory_passed());
    assert_eq!(events.summary.mandatory.failed, 1, "{:?}", events.results);

    assert_eq!(
        failed_checks(absent, "exec/absent"),
        ["absent", "absent"],
        "{:?}",
        absent.results
    );
    assert_eq!(absent.summary.mandatory.failed, 1, "{:?}", absent.results);

    let clock = no_clock
        .results
        .iter()
        .find(|result| result.id == "journal/clock")
        .unwrap();
    assert_eq!(clock.status, Status::Skip, "{clock:?}");
    assert_eq!(
        clock.reason.as_deref(),
        Some("the implementation does not offer testkit.clock.advance")
    );
    assert!(clock.failures.is_empty());
    assert!(no_clock.mandatory_passed());
}

#[test]
fn a_second_volume_on_the_temporary_volume_is_rejected() {
    let root = suite();
    let volume = tempfile::tempdir().unwrap();
    for (args, env) in [
        (
            vec![
                "--second-volume".to_owned(),
                volume.path().display().to_string(),
            ],
            None,
        ),
        (Vec::new(), Some(volume.path())),
    ] {
        let mut command = Command::new(BIN);
        command
            .args(["run", "--impl", "x", "--cases"])
            .arg(root.path())
            .args(&args);
        match env {
            Some(dir) => command.env("NURT_CONFORMANCE_SECOND_VOLUME", dir),
            None => command.env_remove("NURT_CONFORMANCE_SECOND_VOLUME"),
        };
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("another file system"), "{stderr}");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
