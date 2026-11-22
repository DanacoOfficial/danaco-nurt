// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the `nurt` executable built by the `cli` feature.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "assertions of a test file"
)]

use std::process::{Command, Output, Stdio};

use nurt::Nurt;
use nurt_contracts::product::{TAGLINE, help_summary};

const BIN: &str = env!("CARGO_BIN_EXE_nurt");
const VERSION: &str = env!("CARGO_PKG_VERSION");
const USAGE_EXIT_CODE: i32 = 2;

fn nurt(args: &[&str]) -> (Output, String, String) {
    let home = tempfile::tempdir().unwrap();
    let output = Command::new(BIN)
        .args(args)
        .env_clear()
        .env("HOME", home.path())
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let out = String::from_utf8(output.stdout.clone()).unwrap();
    let err = String::from_utf8(output.stderr.clone()).unwrap();
    (output, out, err)
}

#[test]
fn package_description_is_the_canonical_sentence() {
    assert_eq!(env!("CARGO_PKG_DESCRIPTION"), TAGLINE);
}

#[test]
fn version_reports_the_package_version() {
    let (output, out, _) = nurt(&["--version"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(out.starts_with(&format!("nurt {VERSION}")), "{out}");
    assert!(out.contains("License: Apache-2.0"), "{out}");
    let (short, short_out, _) = nurt(&["-V"]);
    assert_eq!(short.status.code(), Some(0));
    assert_eq!(short_out.trim_end(), format!("nurt {VERSION}"));
}

#[test]
fn help_starts_with_the_canonical_sentence() {
    let (output, out, _) = nurt(&["--help"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(out.starts_with(&help_summary()), "{out}");
    assert!(out.contains("Usage: nurt"), "{out}");
}

#[test]
fn unknown_option_is_a_usage_error() {
    let (output, out, err) = nurt(&["--no-such-option"]);
    assert_eq!(output.status.code(), Some(USAGE_EXIT_CODE));
    assert!(out.is_empty(), "{out}");
    assert!(!err.is_empty());
}

#[test]
fn process_client_talks_to_the_built_server() {
    let client = Nurt::builder().executable(BIN).spawn().unwrap();
    let info = client.info().unwrap();
    assert!(info.is_object(), "{info}");
    client.close().unwrap();
}
