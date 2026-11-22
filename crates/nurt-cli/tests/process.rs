// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the `nurt` executable as a process.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "assertions of a test file"
)]

use std::io::Write;
use std::process::{Command, Output, Stdio};

use nurt_contracts::envelope::Response;

const BIN: &str = env!("CARGO_BIN_EXE_nurt");

/// Runs the executable with the state and configuration directories in a
/// temporary directory and without the variables of the configuration.
fn nurt(args: &[&str], stdin: &str) -> Output {
    let home = tempfile::tempdir().unwrap();
    let mut command = Command::new(BIN);
    command
        .args(args)
        .env_clear()
        .env("HOME", home.path())
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

#[test]
fn version_subcommand_prints_the_full_version() {
    let output = nurt(&["version"], "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), stdout(&nurt(&["--version"], "")));
}

#[test]
fn version_options_print_the_short_and_the_full_form() {
    let short = nurt(&["-V"], "");
    assert_eq!(
        stdout(&short),
        format!("nurt {}\n", env!("CARGO_PKG_VERSION"))
    );
    let full = nurt(&["--version"], "");
    assert_eq!(full.status.code(), Some(0));
    let text = stdout(&full);
    assert!(text.starts_with(&format!("nurt {}", env!("CARGO_PKG_VERSION"))));
    assert!(text.ends_with("License: Apache-2.0\n"));
    assert_eq!(text.lines().count(), 3);
}

#[test]
fn help_option_names_the_product_first() {
    let output = nurt(&["--help"], "");
    assert_eq!(output.status.code(), Some(0));
    assert!(
        stdout(&output)
            .starts_with("Command shell and language where every change can be undone.\n\nUsage:")
    );
}

#[test]
fn program_without_a_front_end_fails_with_a_valid_json_response() {
    let output = nurt(&["--format", "json", "find", "\"/srv\""], "");
    assert_ne!(output.status.code(), Some(0));
    let response: Response = serde_json::from_str(&stdout(&output)).unwrap();
    response.validate().unwrap();
    let report: serde_json::Value = serde_json::to_value(&response.report).unwrap();
    let schema: serde_json::Value =
        serde_json::from_str(nurt_contracts::schema::by_name("report").unwrap()).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    assert!(validator.is_valid(&report));
}

#[test]
fn check_without_a_front_end_fails() {
    let output = nurt(&["check", "-e", "find \"/srv\""], "");
    assert_ne!(output.status.code(), Some(0));
    assert!(stderr(&output).contains("E_UNSUPPORTED"));
}

#[test]
fn unavailable_subcommands_fail_with_a_message() {
    for args in [
        &["mcp", "--stdio"][..],
        &["lsp"],
        &["migrate", "build.sh"],
        &["service", "status"],
        &["journal", "salvage", "/"],
        &["ext", "list"],
    ] {
        let output = nurt(args, "");
        assert_eq!(output.status.code(), Some(3), "{args:?}");
        assert!(
            stderr(&output).contains("is not available in this build"),
            "{args:?}"
        );
    }
}

#[test]
fn usage_error_exits_with_two() {
    let output = nurt(&["exec", "--bogus", "find"], "");
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("unknown option `--bogus`"));
}

#[test]
fn schema_command_prints_a_schema() {
    let output = nurt(&["schema", "request"], "");
    assert_eq!(output.status.code(), Some(0));
    let schema: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert!(schema.is_object());
}

#[test]
fn doctor_succeeds_on_a_clean_state_directory() {
    let output = nurt(&["doctor"], "");
    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).contains("state directory"));
}

fn assert_no_banner(output: &Output) {
    for text in [stdout(output), stderr(output)] {
        assert!(!text.contains("Copyright"), "{text}");
        assert!(!text.contains("to get started"), "{text}");
    }
}

#[test]
fn banner_is_not_printed_to_a_script() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("find.nurt");
    std::fs::write(&file, "#!/usr/bin/env nurt\nfind \"/srv\"\n").unwrap();
    assert_no_banner(&nurt(&["script", file.to_str().unwrap()], ""));
}

#[test]
fn banner_is_not_printed_to_a_pipe() {
    let output = nurt(&["repl"], "exit\n");
    assert_no_banner(&output);
    assert!(output.stdout.is_empty());
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn banner_is_not_printed_to_the_protocol_server() {
    let output = nurt(&["serve", "--stdio"], "");
    assert_no_banner(&output);
    assert!(output.stdout.is_empty());
    assert_eq!(output.status.code(), Some(3));
}

#[test]
fn session_reads_programs_from_a_pipe() {
    let output = nurt(&["repl"], "find \"/srv\"\nexit\n");
    assert_ne!(output.status.code(), Some(0));
    assert!(stdout(&output).contains("E_UNSUPPORTED"));
}
