// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-cli/tests/server_command.rs
// Responsibility:   Tests of the `serve` subcommand on streams other than those of the process.
// Dependencies:     nurt_contracts, nurt_core, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests of the `serve` subcommand on streams other than those of the process.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "assertions of a test file"
)]

use std::io::Cursor;
use std::sync::Arc;

use nurt_cli::integrations::serve::start;
use nurt_cli::{CliError, EngineSource};
use nurt_contracts::env::Host;
use nurt_core::config::ConfigSources;
use nurt_core::{Engine, EngineConfig, EngineError};
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::host::TestHost;

struct VirtualSource;

impl EngineSource for VirtualSource {
    fn open(&self) -> Result<Engine, EngineError> {
        Engine::open(
            EngineConfig::new(Arc::new(VirtualClock::new()), self.host())
                .with_sources(ConfigSources::isolated())
                .without_state_dir(),
        )
    }

    fn host(&self) -> Arc<dyn Host> {
        Arc::new(TestHost::linux())
    }
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

#[test]
fn content_length_framing_is_used_when_requested() {
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"shutdown"}"#;
    let input = format!("Content-Length: {}\r\n\r\n{body}", body.len());
    let mut output = Vec::new();
    let code = start(
        &words("--stdio --framing content-length"),
        &VirtualSource,
        Cursor::new(input),
        &mut output,
    );
    assert!(code.is_ok());
    let text = String::from_utf8(output).unwrap();
    assert!(text.starts_with("Content-Length: "), "{text}");
    assert!(text.contains(r#""id":1"#), "{text}");
}

#[test]
fn ndjson_is_the_default_framing() {
    let mut output = Vec::new();
    let input = "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"shutdown\"}\n";
    assert!(
        start(
            &words("--stdio"),
            &VirtualSource,
            Cursor::new(input),
            &mut output
        )
        .is_ok()
    );
    let text = String::from_utf8(output).unwrap();
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains(r#""id":7"#));
}

#[test]
fn socket_attachment_is_not_available() {
    let mut output = Vec::new();
    let outcome = start(
        &words("--socket /run/nurt.sock"),
        &VirtualSource,
        Cursor::new(""),
        &mut output,
    );
    assert_eq!(
        outcome.err(),
        Some(CliError::Unavailable("serve --socket".to_owned()))
    );
    assert!(output.is_empty());
}

#[cfg(not(feature = "stub"))]
#[test]
fn recorded_cases_need_the_stub_feature() {
    let mut output = Vec::new();
    let outcome = start(
        &words("--stdio --stub /cases"),
        &VirtualSource,
        Cursor::new(""),
        &mut output,
    );
    assert_eq!(
        outcome.err(),
        Some(CliError::Unavailable("serve --stub".to_owned()))
    );
}

#[cfg(feature = "stub")]
#[test]
fn missing_recorded_cases_are_reported() {
    let mut output = Vec::new();
    let outcome = start(
        &words("--stdio --stub /nonexistent/cases"),
        &VirtualSource,
        Cursor::new(""),
        &mut output,
    );
    assert!(matches!(outcome, Err(CliError::Failure(_))));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
