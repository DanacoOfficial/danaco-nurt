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
#[cfg(feature = "mcp")]
use std::io::{BufRead, BufReader};
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
fn help_option_after_a_subcommand_prints_its_usage_and_succeeds() {
    for name in nurt_cli::args::SUBCOMMANDS {
        for option in ["-h", "--help"] {
            let output = nurt(&[name, option], "");
            assert_eq!(output.status.code(), Some(0), "{name} {option}");
            let text = stdout(&output);
            assert!(
                text.contains(&format!("Usage: nurt {name}")),
                "{name}: {text}"
            );
            assert!(stderr(&output).is_empty(), "{name}: {}", stderr(&output));
        }
    }
}

#[test]
fn program_with_the_front_end_prints_a_valid_json_response() {
    let output = nurt(&["--format", "json", "find", "\"/srv\""], "");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(!stdout(&output).contains("E_UNSUPPORTED"));
    let response: Response = serde_json::from_str(&stdout(&output)).unwrap();
    response.validate().unwrap();
    let report: serde_json::Value = serde_json::to_value(&response.report).unwrap();
    let schema: serde_json::Value =
        serde_json::from_str(nurt_contracts::schema::by_name("report").unwrap()).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    assert!(validator.is_valid(&report));
}

#[test]
fn check_with_the_front_end_accepts_a_valid_program() {
    let output = nurt(&["check", "-e", "find \"/srv\""], "");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(!stderr(&output).contains("E_UNSUPPORTED"));
}

const SIZE_AGAINST_TEXT: &str = "find \"/tmp\" |> filter size > \"abc\"";

/// The document written to the standard output, and the exit code.
fn json_outcome(args: &[&str], stdin: &str) -> (serde_json::Value, Option<i32>) {
    let output = nurt(args, stdin);
    assert_eq!(stderr(&output), "", "{args:?}");
    (
        serde_json::from_str(&stdout(&output)).unwrap(),
        output.status.code(),
    )
}

#[test]
fn check_rejects_a_literal_that_is_not_a_value_of_the_field() {
    let output = nurt(&["check", "-e", SIZE_AGAINST_TEXT], "");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    assert!(stderr(&output).contains("E_INVALID_VALUE"));
    assert!(stderr(&output).contains("invalid size `abc`"));
}

#[test]
fn running_reports_the_error_of_check_before_it_reads_anything() {
    let (checked, _) = json_outcome(&["check", "--json", "-e", SIZE_AGAINST_TEXT], "");
    let (response, code) = json_outcome(&["exec", "--json", SIZE_AGAINST_TEXT], "");
    assert_eq!(code, Some(1));
    let error = &response["report"]["error"];
    for member in ["code", "message", "found", "expected"] {
        assert_eq!(error[member], checked["error"][member], "{member}");
    }
    assert_eq!(error["at"]["line"], checked["error"]["at"]["line"]);
    assert_eq!(error["at"]["column"], checked["error"]["at"]["column"]);
    assert_eq!(error["at"]["stage"], "1");
}

/// `check` and a run report the same error of the binding, and `check`
/// writes it as a document with the exit code 1.
fn assert_rejected_before_running(program: &str, code: &str) {
    let (checked, check_code) = json_outcome(&["check", "--json", "-e", program], "");
    assert_eq!(check_code, Some(1), "{program}");
    assert_eq!(checked["error"]["code"], code, "{program}");
    assert_eq!(checked["error"]["at"]["stage"], "1", "{program}");
    let (response, run_code) = json_outcome(&["exec", "--json", program], "");
    assert_eq!(run_code, Some(1), "{program}");
    let error = &response["report"]["error"];
    for member in ["code", "message", "expected"] {
        assert_eq!(
            error[member], checked["error"][member],
            "{program}: {member}"
        );
    }
    assert_eq!(error["at"], checked["error"]["at"], "{program}");
}

#[test]
fn check_rejects_membership_of_a_field_that_cannot_hold() {
    assert_rejected_before_running("find \"/tmp\" |> filter size in 1..5", "E_TYPE");
}

#[test]
fn check_rejects_arithmetic_with_a_literal_of_another_type() {
    assert_rejected_before_running("find \"/tmp\" |> filter size + \"abc\" > 1mb", "E_TYPE");
    assert_rejected_before_running("find \"/tmp\" |> filter modified - 5 > 1h", "E_TYPE");
}

#[test]
fn check_rejects_an_argument_that_the_function_does_not_take() {
    assert_rejected_before_running("find \"/tmp\" |> filter lower(size) == \"a\"", "E_TYPE");
    assert_rejected_before_running(
        "find \"/tmp\" |> filter modified > ago(\"abc\")",
        "E_INVALID_VALUE",
    );
}

#[test]
fn check_rejects_a_literal_that_does_not_fit_a_list_a_map_bytes_or_a_range() {
    assert_rejected_before_running("find \"/tmp\" |> filter flags > 5", "E_TYPE");
    assert_rejected_before_running("find \"/tmp\" |> filter raw_name > \"abc\"", "E_TYPE");
}

#[test]
fn check_with_json_writes_a_language_error_as_a_document() {
    let text = nurt(&["check", "-e", SIZE_AGAINST_TEXT], "");
    let (document, code) =
        json_outcome(&["check", "--format", "json", "-e", SIZE_AGAINST_TEXT], "");
    assert_eq!(code, text.status.code());
    assert_eq!(document["diagnostics"], serde_json::json!([]));
    let error = &document["error"];
    assert_eq!(error["code"], "E_INVALID_VALUE");
    assert_eq!(error["expected"], serde_json::json!(["size"]));
    assert_eq!(error["found"], "abc");
    assert_eq!(error["at"]["line"], 1);
    assert_eq!(error["at"]["column"], 30);
    let (short, _) = json_outcome(&["check", "--json", "-e", SIZE_AGAINST_TEXT], "");
    assert_eq!(short, document);
}

#[test]
fn check_with_json_writes_a_syntax_error_as_a_document() {
    let (document, code) = json_outcome(&["check", "--json", "-"], "find \"/srv\" |>\n");
    assert_eq!(code, Some(1));
    assert_eq!(document["diagnostics"], serde_json::json!([]));
    assert!(document["error"]["code"].as_str().is_some());
}

#[test]
fn check_with_json_writes_the_check_document_of_a_valid_program() {
    let (document, code) = json_outcome(&["check", "--json", "-e", "find \"/srv\""], "");
    assert_eq!(code, Some(0));
    assert_eq!(document["effect"], "read");
    assert!(document.get("error").is_none());
}

#[test]
fn failures_that_are_not_language_errors_stay_on_the_error_output() {
    let output = nurt(&["check", "--json", "/no/such/program.nurt"], "");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    assert!(stderr(&output).contains("cannot read"));
    let output = nurt(&["check", "--format", "xml", "-e", "find \"/srv\""], "");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(stdout(&output), "");
}

#[test]
fn fmt_ast_and_schema_with_json_write_a_language_error_as_a_document() {
    let invalid = "find \"/srv\" |>\n";
    let cases: [(&[&str], &str); 3] = [
        (&["fmt", "--format", "json", "-"], invalid),
        (&["ast", "--format", "json", "--from", "a", "-"], invalid),
        (&["schema", "--json", "nothing"], ""),
    ];
    for (args, stdin) in cases {
        let plain: Vec<&str> = args
            .iter()
            .copied()
            .filter(|arg| !matches!(*arg, "--json" | "--format" | "json"))
            .collect();
        let text = nurt(&plain, stdin);
        assert_eq!(stdout(&text), "", "{args:?}");
        let (document, code) = json_outcome(args, stdin);
        assert_eq!(code, text.status.code(), "{args:?}");
        assert!(document["error"]["code"].as_str().is_some(), "{args:?}");
        assert!(document.get("diagnostics").is_none(), "{args:?}");
    }
}

#[test]
fn fmt_ast_and_schema_with_json_keep_their_output_on_success() {
    for (args, stdin) in [
        (&["fmt", "--json", "-"][..], "find \"/srv\"\n"),
        (
            &["ast", "--json", "--from", "a", "-"][..],
            "find \"/srv\"\n",
        ),
        (&["schema", "--format", "json", "request"][..], ""),
    ] {
        let plain: Vec<&str> = args
            .iter()
            .copied()
            .filter(|arg| !matches!(*arg, "--json" | "--format" | "json"))
            .collect();
        let json = nurt(args, stdin);
        assert_eq!(json.status.code(), Some(0), "{args:?}");
        assert_eq!(stdout(&json), stdout(&nurt(&plain, stdin)), "{args:?}");
    }
}

#[test]
fn unavailable_subcommands_fail_with_a_message() {
    for args in [
        &["catalog", "check-distance"][..],
        #[cfg(not(feature = "mcp-http"))]
        &["mcp", "--http", "127.0.0.1:0"],
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

#[test]
fn doctor_lists_the_features_of_the_linked_components() {
    let output = nurt(&["doctor"], "");
    let text = stdout(&output);
    let row = text
        .lines()
        .find(|line| line.trim_start().starts_with("features"))
        .unwrap();
    for (feature, linked) in [
        ("mcp", cfg!(feature = "mcp")),
        ("lsp", cfg!(feature = "lsp")),
        ("migrate", cfg!(feature = "migrate")),
        ("service", cfg!(feature = "service")),
        ("journal", cfg!(feature = "journal")),
    ] {
        assert_eq!(row.contains(feature), linked, "{feature}: {row}");
    }
}

#[test]
fn doctor_lists_the_services_registered_by_the_engine() {
    let output = nurt(&["doctor"], "");
    let text = stdout(&output);
    let row = text
        .lines()
        .find(|line| line.trim_start().starts_with("services"))
        .unwrap();
    assert!(!row.contains("unavailable"), "{row}");
    for service in ["doc", "code", "process_host", "text_codec", "file_holders"] {
        assert!(row.contains(service), "{service}: {row}");
    }
}

#[test]
fn read_of_a_key_uses_the_document_service() {
    let home = tempfile::tempdir().unwrap();
    let document = home.path().join("settings.json");
    std::fs::write(&document, "{\"server\": {\"port\": 8080}}\n").unwrap();
    let program = format!("read \"{}\" key: \"server.port\"", document.display());
    let output = nurt(&[program.as_str()], "");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("8080"), "{text}");
    assert!(!text.contains("semantic_fallback"), "{text}");
}

#[cfg(feature = "mcp")]
#[test]
fn mcp_server_ends_at_the_end_of_the_input() {
    let output = nurt(&["mcp", "--stdio"], "");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(output.stdout.is_empty());
}

#[cfg(feature = "lsp")]
#[test]
fn language_server_ends_at_the_end_of_the_input() {
    let output = nurt(&["lsp"], "");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
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
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn protocol_server_answers_requests_on_the_standard_streams() {
    let requests = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocol":"nurt-rpc/1"}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#,
        "\n",
    );
    let output = nurt(&["serve", "--stdio"], requests);
    assert_eq!(output.status.code(), Some(0));
    let replies: Vec<serde_json::Value> = stdout(&output)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["result"]["protocol"], "nurt-rpc/1");
    assert_eq!(replies[1]["id"], 2);
}

#[test]
fn protocol_server_on_a_socket_is_not_available() {
    let output = nurt(&["serve", "--socket", "/run/nurt.sock"], "");
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("serve --socket is not available in this build"));
}

#[test]
fn protocol_server_rejects_invalid_arguments() {
    let output = nurt(&["serve", "--stdio", "--framing", "xml"], "");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
}

#[test]
fn session_reads_programs_from_a_pipe() {
    let output = nurt(&["repl"], "find \"/srv\"\nexit\n");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(!stdout(&output).contains("E_UNSUPPORTED"));
    assert!(!stderr(&output).contains("E_UNSUPPORTED"));
}

/// Command of the executable with the state and configuration directories
/// in `home` and without the variables of the configuration.
fn command(args: &[&str], home: &std::path::Path) -> Command {
    let mut command = Command::new(BIN);
    command
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("XDG_STATE_HOME", home.join("state"))
        .env("XDG_CONFIG_HOME", home.join("config"));
    command
}

/// Message with the framing of the language server protocol.
#[cfg(feature = "lsp")]
fn framed(body: &str) -> String {
    format!("Content-Length: {}\r\n\r\n{body}", body.len())
}

/// Next reply with the identifier `id` in the output of a server that writes
/// one JSON-RPC message per line.
#[cfg(feature = "mcp")]
fn reply_of(
    lines: &mut std::io::Lines<BufReader<std::process::ChildStdout>>,
    id: u64,
) -> serde_json::Value {
    loop {
        let line = lines.next().expect("the server closed its output").unwrap();
        let message: serde_json::Value = serde_json::from_str(&line).unwrap();
        if message["id"] == id {
            return message;
        }
    }
}

#[cfg(feature = "mcp")]
#[test]
fn mcp_server_answers_initialize_and_tools_list_and_ends_at_the_end_of_the_input() {
    let home = tempfile::tempdir().unwrap();
    let mut child = command(&["mcp", "--stdio"], home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();

    let initialize = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0.0"}
        }
    });
    writeln!(input, "{initialize}").unwrap();
    input.flush().unwrap();
    let reply = reply_of(&mut lines, 1);
    assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(reply["result"]["serverInfo"]["name"], "nurt");

    writeln!(
        input,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    writeln!(input, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}"#).unwrap();
    input.flush().unwrap();
    let reply = reply_of(&mut lines, 2);
    let names: Vec<&str> = reply["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert_eq!(names, ["nurt", "nurt_help", "nurt_check"]);

    let printed: serde_json::Value =
        serde_json::from_str(&stdout(&nurt(&["schema", "tool", "--format", "mcp"], ""))).unwrap();
    let listed = &reply["result"]["tools"][0];
    let members = printed.as_object().unwrap();
    for member in ["name", "description", "inputSchema", "_meta"] {
        assert!(members.contains_key(member), "{member} is not printed");
    }
    for (member, value) in members {
        assert_eq!(&listed[member.as_str()], value, "{member}");
    }
    let listed_members = listed.as_object().unwrap();
    for member in listed_members.keys() {
        assert!(
            members.contains_key(member) || ["title", "annotations"].contains(&member.as_str()),
            "{member} is listed but not printed"
        );
    }
    assert!(
        !printed["description"].as_str().unwrap().starts_with('{'),
        "the description is text, not a JSON document"
    );

    drop(input);
    assert!(lines.next().is_none(), "output after the end of the input");
    assert_eq!(child.wait().unwrap().code(), Some(0));
}

/// The input of a client that writes its whole session and closes the pipe
/// at once, as `nurt mcp --stdio < file` does: the requests that were read
/// before the end of the input are still answered.
#[cfg(feature = "mcp")]
#[test]
fn mcp_server_answers_the_requests_read_before_the_end_of_the_input() {
    let session = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1.0.0"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    ]
    .join("\n")
        + "\n";
    let output = nurt(&["mcp", "--stdio"], &session);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let replies: Vec<serde_json::Value> = stdout(&output)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let list = replies
        .iter()
        .find(|reply| reply["id"] == 2)
        .unwrap_or_else(|| panic!("no answer to tools/list in {replies:?}"));
    assert_eq!(list["result"]["tools"].as_array().map(Vec::len), Some(3));
}

#[cfg(feature = "lsp")]
#[test]
fn language_server_answers_a_whole_session_on_the_standard_streams() {
    let session = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#,
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#,
        r#"{"jsonrpc":"2.0","method":"exit"}"#,
    ]
    .map(framed)
    .concat();
    let output = nurt(&["lsp"], &session);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.starts_with("Content-Length: "), "{text}");
    assert!(text.contains(r#""serverInfo":{"name":"nurt""#), "{text}");
    assert!(text.contains(r#""id":2"#), "{text}");
}

#[test]
fn check_reads_the_program_from_the_standard_input() {
    let output = nurt(&["check", "-"], "find \"/srv\"\n");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stdout(&output).starts_with("ok"));
}

#[test]
fn fmt_reads_the_program_from_the_standard_input() {
    let output = nurt(&["fmt", "-"], "find \"/srv\"\n");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stdout(&output).contains("find"));
}

#[test]
fn fmt_prints_a_program_of_the_configured_variant_in_the_requested_one() {
    let output = nurt(
        &["fmt", "--variant", "c", "-"],
        "find \"/srv\" kind: \"file\"\n",
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stdout(&output).contains("kind="), "{}", stdout(&output));
}

#[test]
fn fmt_keeps_the_comments_of_the_program() {
    let output = nurt(
        &["fmt", "-"],
        "# all files\nfind \"/srv\"   |> count # total\n",
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "# all files\nfind \"/srv\"\n|> count # total\n"
    );
}

#[test]
fn fmt_keeps_the_comments_when_it_changes_the_variant() {
    let output = nurt(&["fmt", "--variant", "b", "-"], "find \"/srv\" # all\n");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "find(\"/srv\") # all\n");
}

fn tool_definition(arguments: &[&str]) -> serde_json::Value {
    let mut command = vec!["schema", "tool"];
    command.extend_from_slice(arguments);
    let output = nurt(&command, "");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    serde_json::from_str(&stdout(&output)).unwrap()
}

#[test]
fn schema_tool_prints_the_definition_in_the_requested_format() {
    let anthropic = tool_definition(&["--format", "anthropic"]);
    assert_eq!(anthropic["name"], "nurt");
    assert_eq!(anthropic["input_schema"]["type"], "object");
    assert!(!anthropic["description"].as_str().unwrap().is_empty());
    let responses = tool_definition(&["--format=openai-responses"]);
    assert_eq!(responses["type"], "function");
    assert_eq!(responses["strict"], false);
    let neutral = tool_definition(&["--format", "json-schema"]);
    assert_eq!(neutral["name"], "nurt");
    assert!(neutral["input_schema"]["$schema"].is_string());
}

#[test]
fn schema_tool_describes_the_selected_groups_in_the_selected_language() {
    let core = tool_definition(&["--format", "mcp"]);
    let all = tool_definition(&["--format", "mcp", "--groups", "all"]);
    let polish = tool_definition(&["--format", "mcp", "--language", "pl"]);
    let length = |definition: &serde_json::Value| definition["description"].as_str().unwrap().len();
    assert!(length(&all) > length(&core));
    assert_ne!(polish["description"], core["description"]);
    assert_eq!(
        polish["inputSchema"]["properties"]["language"]["default"],
        "pl"
    );
}

#[test]
fn schema_tool_rejects_an_unknown_format_and_an_unknown_option() {
    for arguments in [
        &["schema", "tool", "--format", "yaml"][..],
        &["schema", "tool", "--output", "x"][..],
        &["schema", "tool", "--language", "de"][..],
        &["schema", "tool", "--format"][..],
    ] {
        let output = nurt(arguments, "");
        assert_eq!(
            output.status.code(),
            Some(2),
            "{arguments:?}: {}",
            stderr(&output)
        );
    }
}

#[cfg(unix)]
#[test]
fn argument_that_is_not_utf8_is_a_usage_error() {
    use std::os::unix::ffi::OsStrExt;

    let home = tempfile::tempdir().unwrap();
    let output = command(&[], home.path())
        .arg(std::ffi::OsStr::from_bytes(b"find \xff"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("not valid UTF-8"));
}

fn combined(output: &Output) -> String {
    format!("{}{}", stdout(output), stderr(output))
}

#[test]
fn check_translates_a_bash_habit_that_reads_as_a_nurt_program() {
    let removed = nurt(&["check", "-e", "rm -rf /srv/app/build"], "");
    assert_eq!(removed.status.code(), Some(0), "{}", combined(&removed));
    let text = stdout(&removed);
    assert!(text.contains("corrections: 2"), "{text}");
    assert!(text.contains("paths: [\"/srv/app/build\"]"), "{text}");
    let fetched = nurt(&["check", "-e", "curl -s https://example.com/health"], "");
    assert_eq!(fetched.status.code(), Some(0), "{}", combined(&fetched));
    let text = stdout(&fetched);
    assert!(text.contains("corrections: 1"), "{text}");
    assert!(text.contains("\"command\":\"http.request\""), "{text}");
}

#[test]
fn a_misspelt_changing_command_is_offered_and_not_run() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "x").unwrap();
    let program = format!("remvoe \"{}\"", file.display());
    let output = nurt(&[&program], "");
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    let text = combined(&output);
    assert!(text.contains("E_AMBIGUOUS"), "{text}");
    assert!(
        text.contains(&format!("remove \"{}\"", file.display())),
        "{text}"
    );
    assert!(file.exists());
    let checked = nurt(&["check", "-e", &program], "");
    assert!(combined(&checked).contains("E_AMBIGUOUS"));
}

#[test]
fn a_misspelt_reading_command_is_corrected_and_run() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "x").unwrap();
    let program = format!("lsit \"{}\"", dir.path().display());
    let output = nurt(&[&program], "");
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(
        stdout(&output).contains("corrected: 1"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn a_text_with_an_unknown_escape_is_read_as_a_regular_expression_or_rejected_with_a_fix() {
    let read = nurt(&["check", "-e", "search \"\\d+\" \"/srv\""], "");
    assert_eq!(read.status.code(), Some(0), "{}", combined(&read));
    assert!(
        stdout(&read).contains("corrections: 1"),
        "{}",
        stdout(&read)
    );
    let write = nurt(
        &["check", "-e", "write \"/srv/a.txt\" \"a\\d\" mode: append"],
        "",
    );
    assert_eq!(write.status.code(), Some(1), "{}", combined(&write));
    let text = combined(&write);
    assert!(text.contains("E_ESCAPE"), "{text}");
    assert!(
        text.contains("write \"/srv/a.txt\" r\"a\\d\" mode: append"),
        "{text}"
    );
}

#[cfg(feature = "mcp")]
#[test]
fn mcp_tool_offers_a_misspelt_changing_command_and_runs_a_misspelt_reading_command() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "x").unwrap();
    let mut child = command(&["mcp", "--stdio"], home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let initialize = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0.0"}
        }
    });
    writeln!(input, "{initialize}").unwrap();
    reply_of(&mut lines, 1);
    writeln!(
        input,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    let call = |id: u64, program: String| {
        serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": "nurt", "arguments": {"program": program}}
        })
    };
    writeln!(input, "{}", call(2, format!("remvoe \"{}\"", file.display()))).unwrap();
    writeln!(input, "{}", call(3, format!("lsit \"{}\"", dir.path().display()))).unwrap();
    input.flush().unwrap();
    let offered = reply_of(&mut lines, 2).to_string();
    assert!(offered.contains("E_AMBIGUOUS"), "{offered}");
    let corrected = reply_of(&mut lines, 3).to_string();
    assert!(!corrected.contains("E_AMBIGUOUS"), "{corrected}");
    assert!(corrected.contains("a.txt"), "{corrected}");
    assert!(file.exists());
    drop(input);
    assert_eq!(child.wait().unwrap().code(), Some(0));
}
