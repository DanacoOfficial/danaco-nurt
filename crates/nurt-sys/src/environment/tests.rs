// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::environment::tests
// Responsibility:   Implements the module nurt-sys::environment::tests.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use std::ffi::OsString;

use nurt_contracts::env::Os;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::{EnvironmentScope, EnvironmentVariable};
use nurt_contracts::report::Notable;
use nurt_contracts::value::{Regex as RegexValue, Value};

use super::{COMMAND, EnvironmentStage, NAME, NameFilter, SCOPE, persistent_location};
use crate::host::test_support::{self, TestHost};

fn variables(records: Vec<nurt_contracts::value::Record>) -> Vec<EnvironmentVariable> {
    records
        .into_iter()
        .map(|record| EnvironmentVariable::try_from(record).unwrap())
        .collect()
}

#[cfg(unix)]
fn invalid_utf8() -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(vec![b'a', 0xFF, b'b'])
}

#[cfg(windows)]
fn invalid_utf8() -> OsString {
    use std::os::windows::ffi::OsStringExt;
    OsString::from_wide(&[u16::from(b'a'), 0xD800, u16::from(b'b')])
}

#[test]
fn environment_regex_filter_and_non_utf8_warning() {
    let dir = tempfile::tempdir().unwrap();
    let host = TestHost::new(dir.path())
        .with_var("CARGO_HOME", "/opt/cargo")
        .with_var("CARGO_TARGET_DIR", invalid_utf8())
        .with_var("PATH", "/usr/bin")
        .with_var("SECRET_TOKEN", "abc123");
    let ctx = test_support::context(host);
    let pattern = RegexValue::new("^cargo_", "i").unwrap();
    let args = test_support::args(COMMAND, vec![(NAME, Value::Regex(pattern))]);
    let found = variables(test_support::run(&EnvironmentStage::new(), &args, &ctx).unwrap());
    let names: Vec<&str> = found
        .iter()
        .map(|variable| variable.name.as_str())
        .collect();
    assert_eq!(names, ["CARGO_HOME", "CARGO_TARGET_DIR"]);
    assert_eq!(found[1].value, "a\u{FFFD}b");
    assert!(
        found
            .iter()
            .all(|variable| variable.scope == EnvironmentScope::Process)
    );
    let notes = ctx.take_notes();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].code, Notable::EncodingChanged);
    assert_eq!(notes[0].path.as_deref(), Some("env:/CARGO_TARGET_DIR"));

    let args = test_support::args(
        COMMAND,
        vec![(NAME, Value::Text("SECRET_TOKEN".to_owned()))],
    );
    let found = variables(test_support::run(&EnvironmentStage::new(), &args, &ctx).unwrap());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].value, "abc123");
}

#[test]
fn environment_rejects_invalid_pattern() {
    let pattern = RegexValue {
        pattern: "(".to_owned(),
        flags: Default::default(),
    };
    let args = test_support::args(COMMAND, vec![(NAME, Value::Regex(pattern))]);
    let error = NameFilter::from_args(&args, Os::Linux).unwrap_err();
    assert_eq!(error.code, ErrorCode::Regex);
    let args = test_support::args(COMMAND, vec![(NAME, Value::Int(1))]);
    assert_eq!(
        NameFilter::from_args(&args, Os::Linux).unwrap_err().code,
        ErrorCode::Type
    );
}

#[test]
fn environment_null_parameters_select_all_process_variables() {
    let dir = tempfile::tempdir().unwrap();
    let host = TestHost::new(dir.path())
        .with_var("PATH", "/usr/bin")
        .with_var("HOME", "/home/user");
    let ctx = test_support::context(host);
    let args = test_support::args(COMMAND, vec![(NAME, Value::Null), (SCOPE, Value::Null)]);
    let found = variables(test_support::run(&EnvironmentStage::new(), &args, &ctx).unwrap());
    let mut names: Vec<&str> = found
        .iter()
        .map(|variable| variable.name.as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["HOME", "PATH"]);
    assert!(
        found
            .iter()
            .all(|variable| variable.scope == EnvironmentScope::Process)
    );
}

#[test]
fn environment_filtered_out_non_utf8_variable_adds_no_note() {
    let dir = tempfile::tempdir().unwrap();
    let host = TestHost::new(dir.path())
        .with_var("PATH", "/usr/bin")
        .with_var("BROKEN", invalid_utf8());
    let ctx = test_support::context(host);
    let args = test_support::args(COMMAND, vec![(NAME, Value::Text("PATH".to_owned()))]);
    let found = variables(test_support::run(&EnvironmentStage::new(), &args, &ctx).unwrap());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].value, "/usr/bin");
    assert!(ctx.take_notes().is_empty());
}

#[test]
fn environment_exact_name_ignores_case_on_windows() {
    let args = test_support::args(COMMAND, vec![(NAME, Value::Text("Path".to_owned()))]);
    assert!(
        NameFilter::from_args(&args, Os::Windows)
            .unwrap()
            .matches("PATH")
    );
    assert!(
        !NameFilter::from_args(&args, Os::Linux)
            .unwrap()
            .matches("PATH")
    );
    assert!(
        NameFilter::from_args(&args, Os::Linux)
            .unwrap()
            .matches("Path")
    );
}

#[test]
fn environment_scope_user_on_linux_os_only() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = test_support::context(TestHost::new(dir.path()).with_os(Os::Linux));
    for scope in ["user", "machine"] {
        let args = test_support::args(COMMAND, vec![(SCOPE, Value::Enum(scope.to_owned()))]);
        let error = test_support::run(&EnvironmentStage::new(), &args, &ctx).unwrap_err();
        assert_eq!(error.code, ErrorCode::OsOnly);
        assert_eq!(error.fix, ["environment scope: process"]);
        assert!(error.details.contains_key("profiles"));
    }
    let args = test_support::args(COMMAND, vec![(SCOPE, Value::Enum("galaxy".to_owned()))]);
    let error = test_support::run(&EnvironmentStage::new(), &args, &ctx).unwrap_err();
    assert_eq!(error.code, ErrorCode::Type);
}

#[test]
fn environment_persistent_location_paths() {
    let user = persistent_location(EnvironmentScope::User).unwrap();
    assert_eq!(user.scheme.0, "registry");
    assert_eq!(user.segments, ["HKCU", "Environment"]);
    assert_eq!(user.value, None);
    let machine = persistent_location(EnvironmentScope::Machine).unwrap();
    assert_eq!(
        machine.segments.join("/"),
        "HKLM/SYSTEM/CurrentControlSet/Control/Session Manager/Environment"
    );
    assert_eq!(persistent_location(EnvironmentScope::Process), None);
}

#[cfg(windows)]
#[test]
#[ignore = "requires the registry of a Windows account"]
fn it_environment_windows_user_scope_reads_registry() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = test_support::context(TestHost::new(dir.path()));
    let args = test_support::args(COMMAND, vec![(SCOPE, Value::Enum("user".to_owned()))]);
    let found = variables(test_support::run(&EnvironmentStage::new(), &args, &ctx).unwrap());
    assert!(
        found
            .iter()
            .all(|variable| variable.scope == EnvironmentScope::User)
    );
    assert!(
        found
            .iter()
            .any(|variable| variable.name.eq_ignore_ascii_case("TEMP"))
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
