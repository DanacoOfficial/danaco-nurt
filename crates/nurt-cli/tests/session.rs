// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Invocations of the command line on an engine with a front end of recorded
//! trees and virtual components.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "assertions of a test file"
)]

use std::path::PathBuf;
use std::sync::Arc;

use nurt_cli::exit;
use nurt_cli::repl::{Input, LineSource, PlainLines, Session, banner_for};
use nurt_cli::{EngineSource, Io, Terminal, run};
use nurt_contracts::env::Host;
use nurt_contracts::envelope::Response;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_core::config::ConfigSources;
use nurt_core::fixtures::FixtureFrontend;
use nurt_core::{Engine, EngineConfig, EngineError};
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::fixtures::empty_program;
use nurt_testkit::host::TestHost;

struct TestSource {
    frontend: FixtureFrontend,
    state: PathBuf,
}

impl EngineSource for TestSource {
    fn open(&self) -> Result<Engine, EngineError> {
        Engine::open(
            EngineConfig::new(Arc::new(VirtualClock::new()), self.host())
                .with_sources(ConfigSources::isolated())
                .with_state_dir(&self.state)
                .with_frontend(Arc::new(self.frontend.clone())),
        )
    }

    fn host(&self) -> Arc<dyn Host> {
        Arc::new(TestHost::linux())
    }
}

struct Outcome {
    code: u8,
    out: String,
    err: String,
}

fn source(temp: &tempfile::TempDir) -> TestSource {
    TestSource {
        frontend: FixtureFrontend::new()
            .with_bound("find \"/srv\"", empty_program())
            .with_bound("history limit: 5", empty_program())
            .with_error(
                "remvoe \"/tmp/a\"",
                Error::new(ErrorCode::UnknownCommand, "unknown command `remvoe`")
                    .with_fix("remove \"/tmp/a\""),
            ),
        state: temp.path().join("state"),
    }
}

fn invoke(source: &TestSource, args: &[&str], stdin: &str, terminal: Terminal) -> Outcome {
    let words: Vec<String> = args.iter().map(|word| (*word).to_owned()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut input = stdin.as_bytes();
    let code = run(
        &words,
        &mut Io {
            out: &mut out,
            err: &mut err,
            input: &mut input,
            terminal,
        },
        source,
    );
    Outcome {
        code,
        out: String::from_utf8(out).unwrap(),
        err: String::from_utf8(err).unwrap(),
    }
}

fn call(args: &[&str]) -> Outcome {
    let temp = tempfile::tempdir().unwrap();
    invoke(&source(&temp), args, "", Terminal::default())
}

#[test]
fn version_options_print_the_fixed_lines() {
    let version = env!("CARGO_PKG_VERSION");
    let short = call(&["-V"]);
    assert_eq!(
        (short.code, short.out.as_str()),
        (0, format!("nurt {version}\n").as_str())
    );
    let full = call(&["--version"]);
    assert_eq!(full.code, 0);
    let lines: Vec<&str> = full.out.lines().collect();
    assert!(lines[0].starts_with(&format!("nurt {version}")));
    assert_eq!(
        lines[1..],
        [
            "Copyright (c) 2026 Danaco Holding Group Sp. z o.o. All rights reserved.",
            "License: Apache-2.0"
        ]
    );
    let sub = call(&["version"]);
    assert_eq!((sub.code, sub.out), (0, full.out));
}

#[test]
fn help_option_starts_with_the_description() {
    let help = call(&["--help"]);
    let lines: Vec<&str> = help.out.lines().collect();
    assert_eq!(
        lines[0],
        "Command shell and language where every change can be undone."
    );
    assert!(lines[2].starts_with("Usage:"));
}

#[test]
fn help_subcommand_starts_with_the_introduction() {
    let help = call(&["help"]);
    assert!(help.out.starts_with(&format!(
        "Danaco Nurt {} - command shell and language where every change can be undone\n",
        env!("CARGO_PKG_VERSION")
    )));
    assert_eq!(help.code, 0);
}

#[test]
fn program_with_a_language_error_exits_nonzero_and_names_the_fix() {
    let outcome = call(&["remvoe", "\"/tmp/a\""]);
    assert_eq!(outcome.code, exit::FAILURE);
    assert!(outcome.out.contains("E_UNKNOWN_COMMAND"));
    assert!(outcome.out.starts_with("failed program"));
}

#[test]
fn program_words_are_joined_and_run_through_the_engine() {
    let outcome = call(&["find", "\"/srv\""]);
    let unsupported = exit::for_error_code(ErrorCode::Unsupported);
    assert_eq!(outcome.code, unsupported);
    assert!(outcome.out.contains("E_UNSUPPORTED"));
    let same = call(&["exec", "find \"/srv\""]);
    assert_eq!(same.out, outcome.out);
}

#[test]
fn json_format_prints_a_valid_response_document() {
    let outcome = call(&["exec", "--format", "json", "find \"/srv\""]);
    let response: Response = serde_json::from_str(&outcome.out).unwrap();
    response.validate().unwrap();
    assert!(outcome.out.contains("\"E_UNSUPPORTED\""));
}

#[test]
fn history_shortcut_runs_the_journal_command() {
    let outcome = call(&["history", "limit:", "5"]);
    assert!(outcome.out.contains("E_UNSUPPORTED"));
    assert!(!outcome.out.contains("E_UNKNOWN_COMMAND"));
}

#[test]
fn script_runs_the_file_in_script_mode() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("find.nurt");
    std::fs::write(&file, "#!/usr/bin/env nurt\nfind \"/srv\"\n").unwrap();
    let outcome = invoke(
        &source(&temp),
        &["script", file.to_str().unwrap()],
        "",
        Terminal::default(),
    );
    assert!(outcome.out.contains("E_UNSUPPORTED"));
    let missing = call(&["script", "/absent/file.nurt"]);
    assert_eq!(missing.code, exit::FAILURE);
    assert!(missing.err.contains("cannot read"));
}

#[test]
fn usage_errors_exit_with_two() {
    for args in [
        &["exec"][..],
        &["exec", "--bogus", "find"],
        &["exec", "--limit", "x", "find"],
        &["schema"],
        &["version", "extra"],
        &["doctor", "extra"],
        &["repl", "extra"],
        &["fmt"],
        &["ast"],
        &["catalog", "a", "b"],
        &["check"],
        &["check", "-e", "find", "file"],
        &["exec", "--cursor", "c_1", "find"],
    ] {
        let outcome = call(args);
        assert_eq!(outcome.code, exit::USAGE, "{args:?}");
        assert!(outcome.err.starts_with("nurt: "), "{args:?}");
    }
}

#[test]
fn unavailable_subcommands_exit_with_three() {
    for args in [
        &["serve", "--stdio"][..],
        &["serve", "--socket", "/run/nurt.sock"],
        &["mcp"],
        &["lsp"],
        &["migrate", "x.sh"],
        &["service", "status"],
        &["journal", "salvage", "/"],
        &["ext", "list"],
        &["catalog", "check-distance"],
    ] {
        let outcome = call(args);
        assert_eq!(outcome.code, exit::UNAVAILABLE, "{args:?}");
        assert!(
            outcome.err.contains("is not available in this build"),
            "{args:?}"
        );
        assert!(outcome.out.is_empty(), "{args:?}");
    }
}

#[test]
fn schema_prints_the_published_schema() {
    let report = call(&["schema", "report"]);
    assert_eq!(report.code, 0);
    assert!(report.out.contains("\"$schema\""));
    let tool = call(&["schema", "tool"]);
    assert_eq!(tool.code, 0);
    let unknown = call(&["schema", "bogus"]);
    assert_eq!(unknown.code, exit::FAILURE);
    assert!(unknown.err.contains("E_UNKNOWN_VALUE"));
}

#[test]
fn check_reports_the_error_of_the_program() {
    let outcome = call(&["check", "-e", "remvoe \"/tmp/a\""]);
    assert_eq!(outcome.code, exit::FAILURE);
    assert!(outcome.err.contains("E_UNKNOWN_COMMAND"));
    assert!(outcome.err.contains("fix: remove \"/tmp/a\""));
}

#[test]
fn fmt_and_ast_report_the_error_of_the_front_end() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("a.nurt");
    std::fs::write(&file, "find \"/srv\"").unwrap();
    let fmt = invoke(
        &source(&temp),
        &["fmt", file.to_str().unwrap()],
        "",
        Terminal::default(),
    );
    assert_eq!(fmt.code, exit::for_error_code(ErrorCode::Unsupported));
    let ast = call(&["ast", "--from", "a", "--to", "json", "find \"/srv\""]);
    assert_eq!(ast.code, exit::for_error_code(ErrorCode::Unsupported));
    let bad = call(&["ast", "--from", "yaml", "x"]);
    assert_eq!(bad.code, exit::USAGE);
}

#[test]
fn ast_converts_a_tree_between_json_and_cbor() {
    let tree = r#"{"nurt":"1.0","program":{"statements":[]}}"#;
    let temp = tempfile::tempdir().unwrap();
    let json = invoke(&source(&temp), &["ast", "-"], tree, Terminal::default());
    assert_eq!(json.code, 0);
    let parsed: serde_json::Value = serde_json::from_str(&json.out).unwrap();
    assert_eq!(parsed["nurt"], "1.0");
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut input = tree.as_bytes();
    let words: Vec<String> = ["ast", "--to", "cbor", "-"].map(str::to_owned).to_vec();
    let code = run(
        &words,
        &mut Io {
            out: &mut out,
            err: &mut err,
            input: &mut input,
            terminal: Terminal::default(),
        },
        &source(&temp),
    );
    assert_eq!(code, 0);
    let decoded: serde_json::Value = ciborium::from_reader(out.as_slice()).unwrap();
    assert_eq!(decoded["nurt"], "1.0");
    let invalid = invoke(&source(&temp), &["ast", "-"], "{", Terminal::default());
    assert_eq!(invalid.code, exit::FAILURE);
}

#[test]
fn catalog_and_help_topics_report_the_missing_components() {
    let catalog = call(&["catalog"]);
    assert_eq!(catalog.code, exit::for_error_code(ErrorCode::Unsupported));
    let topic = call(&["help", "remove"]);
    assert!(topic.err.contains("E_UNSUPPORTED"));
}

#[test]
fn doctor_lists_the_checks_of_the_installation() {
    let outcome = call(&["doctor"]);
    assert_eq!(outcome.code, 0);
    for name in ["version", "state directory", "journal", "subcommand mcp"] {
        assert!(outcome.out.contains(name), "{name}");
    }
    assert!(outcome.out.contains("writable"));
}

#[test]
fn engine_open_failure_is_reported_with_its_exit_code() {
    let temp = tempfile::tempdir().unwrap();
    let blocker = temp.path().join("file");
    std::fs::write(&blocker, b"x").unwrap();
    let mut broken = source(&temp);
    broken.state = blocker.join("state");
    let outcome = invoke(&broken, &["doctor"], "", Terminal::default());
    assert_ne!(outcome.code, 0);
    assert!(!outcome.err.is_empty());
}

#[test]
fn banner_is_printed_only_on_an_interactive_terminal() {
    let interactive = Terminal {
        stdin: true,
        stdout: true,
        stderr: true,
    };
    let version = env!("CARGO_PKG_VERSION");
    assert_eq!(
        banner_for(interactive).as_deref(),
        Some(
            format!(
                "Danaco Nurt {version}\nCopyright (c) 2026 Danaco Holding Group Sp. z o.o. All rights reserved.\nType 'help' to get started."
            )
            .as_str()
        )
    );
    let script = Terminal {
        stdin: false,
        stdout: true,
        stderr: true,
    };
    let pipe = Terminal {
        stdin: true,
        stdout: false,
        stderr: true,
    };
    assert_eq!(banner_for(script), None);
    assert_eq!(banner_for(pipe), None);
    assert_eq!(banner_for(Terminal::default()), None);
}

#[test]
fn session_without_a_terminal_prints_no_banner() {
    let temp = tempfile::tempdir().unwrap();
    let outcome = invoke(&source(&temp), &["repl"], "exit\n", Terminal::default());
    assert_eq!(outcome.code, 0);
    assert!(outcome.out.is_empty());
    assert!(outcome.err.is_empty());
}

#[test]
fn session_runs_scripted_lines_and_controls_jobs() {
    let temp = tempfile::tempdir().unwrap();
    let script = "\nfind \"/srv\"\nhelp\nhelp find\nbg find \"/srv\"\njobs\nfg\nfg\nbg\nkill 9\nkill x\nbg find \"/srv\"\nkill\njobs\nexit\nfind \"/srv\"\n";
    let outcome = invoke(&source(&temp), &["repl"], script, Terminal::default());
    assert!(outcome.out.contains("Danaco Nurt"));
    assert!(outcome.out.contains("[1] started"));
    assert!(outcome.out.contains("[1] "));
    assert!(outcome.out.contains("[2] started"));
    assert!(outcome.out.contains("[2] cancelled"));
    assert!(outcome.err.contains("nurt: no such job"));
    assert!(outcome.err.contains("`x` is not a job number"));
    assert!(outcome.err.contains("`bg` needs a program"));
    assert!(outcome.err.contains("E_UNSUPPORTED"));
    assert_eq!(outcome.out.matches("E_UNSUPPORTED").count(), 2);
    assert_eq!(outcome.code, exit::for_error_code(ErrorCode::Unsupported));
}

struct Scripted(Vec<Input>);

impl LineSource for Scripted {
    fn read_line(&mut self) -> Input {
        if self.0.is_empty() {
            Input::End
        } else {
            self.0.remove(0)
        }
    }
}

#[test]
fn interrupted_lines_do_not_end_the_session() {
    let temp = tempfile::tempdir().unwrap();
    let engine = source(&temp).open().unwrap();
    let mut session = Session::new(&engine, nurt_cli::output::Style::plain());
    let mut lines = Scripted(vec![
        Input::Interrupted,
        Input::Line("quit".to_owned()),
        Input::Line("find \"/srv\"".to_owned()),
    ]);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    session.run(&mut lines, &mut out, &mut err).unwrap();
    assert!(out.is_empty());
    assert_eq!(session.last_exit_code(), 0);
    assert_eq!(lines.0.len(), 1);
}

#[test]
fn plain_lines_end_at_the_end_of_the_input() {
    let mut input: &[u8] = b"one\r\ntwo";
    let mut lines = PlainLines::new(&mut input);
    assert_eq!(lines.read_line(), Input::Line("one".to_owned()));
    assert_eq!(lines.read_line(), Input::Line("two".to_owned()));
    assert_eq!(lines.read_line(), Input::End);
}
