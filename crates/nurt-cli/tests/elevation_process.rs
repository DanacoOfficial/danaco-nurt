// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Privilege elevation through the `nurt` executable: the call parameters on
//! the command line and on a stage, and the hidden subcommand
//! `elevated-exec`. Tests that need root or a program that elevates without a
//! password are ignored with the reason and run on the pool machine.

// Test failures report through panics.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use nurt_contracts::envelope::Response;
use nurt_contracts::report::ReportStatus;

const BIN: &str = env!("CARGO_BIN_EXE_nurt");

/// Runs the executable in `home` with `stdin`, a clean environment that
/// keeps `PATH`, and the configuration file `config` when given.
fn nurt_in(home: &Path, args: &[&str], stdin: &str, config: Option<&str>) -> Output {
    let mut command = Command::new(BIN);
    command
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("XDG_STATE_HOME", home.join("state"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(text) = config {
        let file = home.join("nurt.toml");
        std::fs::write(&file, text).unwrap();
        command.env("NURT_CONFIG", &file);
    }
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// Like [`nurt_in`] in a new home directory, which is returned.
fn nurt(args: &[&str], stdin: &str, config: Option<&str>) -> (Output, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let output = nurt_in(home.path(), args, stdin, config);
    (output, home)
}

/// Whether a file `name` exists below `dir`.
fn contains_file(dir: &Path, name: &str) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            let path = entry.path();
            path.file_name().is_some_and(|found| found == name)
                || (path.is_dir() && contains_file(&path, name))
        })
    })
}

fn quoted(path: &Path) -> String {
    format!("\"{}\"", path.display().to_string().replace('\\', "/"))
}

fn response(output: &Output) -> Response {
    let text = String::from_utf8(output.stdout.clone()).unwrap();
    let response: Response = serde_json::from_str(&text).unwrap_or_else(|error| {
        panic!(
            "{error}: {text} / {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    response.validate().unwrap();
    response
}

fn request_for(program: &str, privilege: &str) -> String {
    serde_json::json!({
        "nurt": "1.0",
        "program": program,
        "options": {"privilege": privilege, "format": "json"}
    })
    .to_string()
}

#[test]
fn stage_parameter_account_is_accepted_by_the_language() {
    let dir = tempfile::tempdir().unwrap();
    let program = format!("find {} privilege: account", quoted(dir.path()));
    let (output, _home) = nurt(&["--format", "json", &program], "", None);
    assert_eq!(output.status.code(), Some(0));
    let response = response(&output);
    assert_eq!(response.report.status, ReportStatus::Ok);
    assert!(!response.report.elevated);
}

#[test]
fn command_line_option_accepts_the_three_values_and_rejects_others() {
    let dir = tempfile::tempdir().unwrap();
    let program = format!("find {}", quoted(dir.path()));
    let (output, _home) = nurt(&["--privilege", "account", "--json", &program], "", None);
    assert_eq!(output.status.code(), Some(0));
    let (output, _home) = nurt(&["--privilege", "root", &program], "", None);
    assert_ne!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a privilege"));
}

#[test]
fn configuration_can_forbid_elevation_for_a_call() {
    let dir = tempfile::tempdir().unwrap();
    let program = format!("find {} privilege: elevated", quoted(dir.path()));
    let (output, _home) = nurt(
        &["--format", "json", &program],
        "",
        Some("[privilege]\nallowed = false\n"),
    );
    let response = response(&output);
    let error = response.report.error.unwrap();
    assert_eq!(error.code().as_str(), "E_PRIVILEGE");
    assert_eq!(
        error.details.get("reason"),
        Some(&serde_json::json!("disabled_by_configuration"))
    );
}

#[test]
fn without_a_backend_the_call_ends_with_the_refusal_of_every_backend() {
    let dir = tempfile::tempdir().unwrap();
    let program = format!("find {}", quoted(dir.path()));
    let config = "[privilege]\nbackends = [\"doas\"]\n";
    let (output, _home) = nurt(
        &["--format", "json", "--privilege", "elevated", &program],
        "",
        Some(config),
    );
    let response = response(&output);
    let error = response.report.error.unwrap();
    assert_eq!(error.code().as_str(), "E_PRIVILEGE");
    assert!(error.details.get("backend").is_some());
    assert!(!error.next_steps.is_empty());
}

#[cfg(unix)]
fn running_as_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "0")
}

#[cfg(unix)]
#[test]
fn elevated_exec_refuses_a_process_that_is_not_the_administrator() {
    if running_as_root() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let request = request_for(&format!("find {}", quoted(dir.path())), "elevated");
    let (output, _home) = nurt(&["elevated-exec"], &request, None);
    assert_eq!(output.status.code(), Some(126));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("administrator"));
}

#[test]
fn elevated_exec_rejects_a_request_that_is_not_valid() {
    let (output, _home) = nurt(&["elevated-exec"], "not json", None);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
}

#[test]
fn elevated_exec_is_not_listed_among_the_subcommands() {
    let (output, _home) = nurt(&["--help"], "", None);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("elevated-exec"));
}

#[cfg(unix)]
#[test]
#[ignore = "needs root"]
fn elevated_exec_as_the_administrator_answers_with_a_marked_report() {
    let dir = tempfile::tempdir().unwrap();
    let request = request_for(&format!("find {}", quoted(dir.path())), "elevated");
    let (output, _home) = nurt(&["elevated-exec"], &request, None);
    assert_eq!(output.status.code(), Some(0));
    let response = response(&output);
    assert!(response.report.elevated);
    assert_eq!(response.report.backend.as_deref(), Some("self"));
}

#[cfg(unix)]
#[test]
#[ignore = "needs sudo without a password"]
fn sudo_runs_an_elevated_read() {
    let dir = tempfile::tempdir().unwrap();
    let program = format!("find {} privilege: elevated", quoted(dir.path()));
    let config = "[privilege]\nbackends = [\"sudo\"]\n";
    let (output, _home) = nurt(&["--format", "json", &program], "", Some(config));
    let response = response(&output);
    assert_eq!(response.report.status, ReportStatus::Ok, "{response:?}");
    assert!(response.report.elevated);
    assert_eq!(response.report.backend.as_deref(), Some("sudo"));
}

#[cfg(unix)]
#[test]
#[ignore = "needs sudo without a password"]
fn sudo_writes_a_system_file_and_undo_removes_it() {
    let target = "/etc/nurt-test-priv.conf";
    let _ = Command::new("sudo")
        .args(["-n", "rm", "-f", target])
        .status();
    let config = "[privilege]\nbackends = [\"sudo\"]\n";
    let write = format!("write \"{target}\" \"a=1\\n\" privilege: elevated");
    let home = tempfile::tempdir().unwrap();
    let output = nurt_in(home.path(), &["--format", "json", &write], "", Some(config));
    let written = response(&output);
    assert_eq!(written.report.status, ReportStatus::Ok, "{written:?}");
    assert!(written.report.elevated);
    let op = written.report.op.expect("an entry of the system journal");
    assert!(Path::new(target).exists());
    assert!(contains_file(
        &home.path().join("state"),
        "elevated-operations.jsonl"
    ));
    let undo = format!("undo {op}");
    let output = nurt_in(home.path(), &["--format", "json", &undo], "", Some(config));
    let undone = response(&output);
    assert_eq!(undone.report.status, ReportStatus::Ok, "{undone:?}");
    assert!(undone.report.elevated);
    assert!(!Path::new(target).exists());
    let _ = Command::new("sudo")
        .args(["-n", "rm", "-f", target])
        .status();
}

#[cfg(unix)]
#[test]
#[ignore = "needs doas"]
fn doas_runs_an_elevated_read() {
    let dir = tempfile::tempdir().unwrap();
    let program = format!("find {} privilege: elevated", quoted(dir.path()));
    let config = "[privilege]\nbackends = [\"doas\"]\n";
    let (output, _home) = nurt(&["--format", "json", &program], "", Some(config));
    let response = response(&output);
    assert_eq!(response.report.backend.as_deref(), Some("doas"));
}

#[cfg(unix)]
#[test]
#[ignore = "needs run0 with a polkit rule that allows it without a password"]
fn run0_runs_an_elevated_read() {
    let dir = tempfile::tempdir().unwrap();
    let program = format!("find {} privilege: elevated", quoted(dir.path()));
    let config = "[privilege]\nbackends = [\"run0\"]\n";
    let (output, _home) = nurt(&["--format", "json", &program], "", Some(config));
    let response = response(&output);
    assert_eq!(response.report.backend.as_deref(), Some("run0"));
}
