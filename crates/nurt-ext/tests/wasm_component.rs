// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-ext/tests/wasm_component.rs
// Responsibility:   The WebAssembly host on components assembled from their text form.
// Dependencies:     nurt_catalog, nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The WebAssembly host on components assembled from their text form.
//!
//! The component `reader` implements `nurt:ext/stage@1.0.0`: its `run`
//! takes the arguments `{"path":"<file>"}`, reads the file through the host
//! call `read-file` and returns the content as one record, or the message of
//! the failed host call. The component `spinner` never returns.

#![cfg(feature = "wasm")]
// Test assertions fail through unwrap, expect and panic by design.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;
use std::sync::Arc;

use nurt_catalog::Registry;
use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, Event, LogLevel, StageData, StageImpl, StageInput,
};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_ext::locations::ExtLocations;
use nurt_ext::manifest::{MANIFEST_FILE, Permissions};
use nurt_ext::policy::{PathPolicy, PlaceholderRoots};
use nurt_ext::register::{core_names, register};
use nurt_ext::stage::{ExtRuntime, GuestCall, PlannedWrite};
use nurt_ext::store::ExtStore;
use nurt_ext::wasm::WasmHost;
use nurt_testkit::ctx::TestContext;
use nurt_testkit::vfs::test_path;

/// Body of `run` that reads the file named by the arguments. The path
/// starts after the 9 bytes of `{"path":"` and ends before the 2 bytes of
/// `"}`. The host stores the `result<list<u8>, failure>` at offset 16
/// (discriminant, then the list or the record at offset 20); the function
/// returns the `result<list<string>, string>` stored at offset 48, with the
/// one-element list at offset 64.
const READER: &str = r"
      (call $read-file
        (i32.add (local.get $args) (i32.const 9))
        (i32.sub (local.get $args-len) (i32.const 11))
        (i32.const 16))
      (if (i32.eqz (i32.load8_u (i32.const 16)))
        (then
          (i32.store (i32.const 64) (i32.load (i32.const 20)))
          (i32.store (i32.const 68) (i32.load (i32.const 24)))
          (i32.store8 (i32.const 48) (i32.const 0))
          (i32.store (i32.const 52) (i32.const 64))
          (i32.store (i32.const 56) (i32.const 1)))
        (else
          (i32.store8 (i32.const 48) (i32.const 1))
          (i32.store (i32.const 52) (i32.load (i32.const 28)))
          (i32.store (i32.const 56) (i32.load (i32.const 32)))))
      (i32.const 48)";

/// Body of `run` that loops until the execution budget ends.
const SPINNER: &str = r"
      (loop $spin (br $spin))
      (unreachable)";

/// Component text around the body of the core function `run`. The core
/// module `libc` owns the memory and a bump allocator that starts above the
/// fixed areas used by the bodies.
fn component(body: &str) -> String {
    format!(
        r#"(component
  (import "nurt:ext/host@1.0.0" (instance $host
    (type $failure (record (field "code" string) (field "message" string)))
    (export "failure" (type $failure-type (eq $failure)))
    (export "read-file" (func (param "path" string) (result (result (list u8) (error $failure-type)))))))
  (core module $libc-module
    (memory (export "memory") 1)
    (global $heap (mut i32) (i32.const 1024))
    (func (export "realloc") (param $old i32) (param $old-size i32) (param $align i32) (param $size i32) (result i32)
      (local $ptr i32)
      (local.set $ptr
        (i32.and
          (i32.add (global.get $heap) (i32.sub (local.get $align) (i32.const 1)))
          (i32.sub (i32.const 0) (local.get $align))))
      (global.set $heap (i32.add (local.get $ptr) (local.get $size)))
      (block $enough
        (br_if $enough (i32.le_u (global.get $heap) (i32.shl (memory.size) (i32.const 16))))
        (br_if $enough
          (i32.ne
            (memory.grow
              (i32.sub
                (i32.add (i32.shr_u (global.get $heap) (i32.const 16)) (i32.const 1))
                (memory.size)))
            (i32.const -1)))
        (unreachable))
      (local.get $ptr)))
  (core instance $libc (instantiate $libc-module))
  (alias core export $libc "memory" (core memory $memory))
  (alias core export $libc "realloc" (core func $realloc))
  (core func $read-file (canon lower (func $host "read-file") (memory $memory) (realloc $realloc)))
  (core instance $imports (export "read-file" (func $read-file)))
  (core module $main-module
    (import "libc" "memory" (memory 1))
    (import "host" "read-file" (func $read-file (param i32 i32 i32)))
    (func (export "run")
      (param $command i32) (param $command-len i32)
      (param $args i32) (param $args-len i32)
      (param $input i32) (param $input-len i32)
      (result i32)
      {body}))
  (core instance $main (instantiate $main-module
    (with "libc" (instance $libc))
    (with "host" (instance $imports))))
  (func $run
    (param "command" string) (param "args" string) (param "input" (list string))
    (result (result (list string) (error string)))
    (canon lift (core func $main "run") (memory $memory) (realloc $realloc)))
  (instance $stage (export "run" (func $run)))
  (export "nurt:ext/stage@1.0.0" (instance $stage)))
"#
    )
}

/// Manifest of the extension; the granted directory is `/project` with the
/// root of `nurt_testkit::paths`.
fn manifest() -> String {
    MANIFEST.replace(
        "\"/project\"",
        &format!("\"{}\"", nurt_testkit::paths::abs("/project")),
    )
}

const MANIFEST: &str = r#"[ext]
name = "acme"
version = "1.0.0"
api = "nurt:ext@1.0"
kind = "wasm"
wasm = "acme.wasm"

[[stage]]
name = "show"
summary = "Reads one record from a file."
source = true

[permissions]
read = ["/project"]
"#;

const RECORD: &str = r#"{"kind":"item","fields":{"name":"a"}}"#;

fn source_dir(root: &Path) -> std::path::PathBuf {
    let dir = root.join("acme-src");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(MANIFEST_FILE), manifest()).unwrap();
    fs::write(dir.join("acme.wasm"), component(READER)).unwrap();
    dir
}

fn installed_stage(root: &Path) -> Arc<dyn StageImpl> {
    let mut registry = Registry::load_embedded().unwrap();
    let store = ExtStore::new(ExtLocations::new(root.join("ext")), core_names(&registry));
    let extension = store.install(&source_dir(root), false).unwrap();
    let runtime = WasmHost::new().unwrap().load(&extension).unwrap();
    assert_eq!(runtime.name(), "acme");
    let commands = register(&mut registry, &extension, Arc::new(runtime)).unwrap();
    assert_eq!(commands, ["acme.show"]);
    registry.resolve_source("acme.show").unwrap()
}

fn context() -> TestContext {
    let context = TestContext::new();
    context
        .fs()
        .write_file(
            &test_path("/project/record.json").unwrap(),
            RECORD.as_bytes(),
        )
        .unwrap();
    context
        .fs()
        .write_file(&test_path("/secret/key").unwrap(), RECORD.as_bytes())
        .unwrap();
    context
}

fn show(stage: &dyn StageImpl, context: &TestContext, path: &str) -> Result<Vec<Record>> {
    let args = BoundArgs::new("acme.show").with("path", ArgValue::Value(Value::from(path)), false);
    let ctx = context.ctx();
    let output = stage
        .as_read()
        .expect("a command without writes is a read stage")
        .execute(&args, StageInput::none(), &ctx)?;
    let StageData::Records(stream) = output.data else {
        panic!("the command returns records");
    };
    stream.collect()
}

#[test]
fn component_commands_return_records_read_through_the_host() {
    let temp = tempfile::tempdir().unwrap();
    let stage = installed_stage(temp.path());
    let path = nurt_testkit::paths::abs("/project/record.json");
    let records = show(stage.as_ref(), &context(), &path).unwrap();
    assert_eq!(
        records,
        [Record::new(RecordKind::Item).with("name", Value::from("a"))]
    );
}

#[test]
fn paths_outside_the_manifest_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let stage = installed_stage(temp.path());
    let secret = nurt_testkit::paths::abs("/secret/key");
    let error = show(stage.as_ref(), &context(), &secret).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Permission);
    assert!(
        error.message.contains("permissions.read"),
        "{}",
        error.message
    );
    let absent = nurt_testkit::paths::abs("/project/missing.json");
    let missing = show(stage.as_ref(), &context(), &absent).unwrap_err();
    assert_eq!(missing.code(), ErrorCode::StageFailed);
}

#[test]
fn execution_budget_is_enforced() {
    let host = WasmHost::new().unwrap().with_fuel(100_000);
    let spinner = host.compile("acme", component(SPINNER).as_bytes()).unwrap();
    let policy = PathPolicy::new(
        "acme",
        &Permissions::default(),
        &PlaceholderRoots::default(),
    );
    let context = TestContext::new();
    let call = GuestCall {
        command: "acme.spin",
        args: "{}".to_owned(),
        input: Vec::new(),
        policy: &policy,
        writes: false,
    };
    let error = spinner.run(call, &context.ctx()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Limit);
}

#[test]
fn unusable_components_are_rejected() {
    let host = WasmHost::new().unwrap();
    let invalid = host.compile("acme", b"not a component").unwrap_err();
    assert_eq!(invalid.code(), ErrorCode::Catalog);
    let foreign = host
        .compile(
            "acme",
            br#"(component (import "other:x/y@1.0.0" (instance (export "f" (func)))))"#,
        )
        .unwrap_err();
    assert_eq!(foreign.code(), ErrorCode::Catalog);

    let empty = host.compile("acme", b"(component)").unwrap();
    let policy = PathPolicy::new(
        "acme",
        &Permissions::default(),
        &PlaceholderRoots::default(),
    );
    let context = TestContext::new();
    let call = GuestCall {
        command: "acme.show",
        args: "{}".to_owned(),
        input: Vec::new(),
        policy: &policy,
        writes: false,
    };
    let missing = empty.run(call, &context.ctx()).unwrap_err();
    assert_eq!(missing.code(), ErrorCode::Catalog);
    assert!(missing.message.contains("nurt:ext/stage@1.0.0"));
}

/// Component text of a command that logs, plans the write of
/// `/project/out.txt` with the content `abc`, lists `/project` and returns
/// one record; when the write is refused it returns the message of the
/// refusal.
fn worker_component() -> String {
    let record = RECORD.replace('"', "\\\"");
    // The paths carry the root of `nurt_testkit::paths`; their data segments
    // leave room for the drive.
    let out = nurt_testkit::paths::abs("/project/out.txt");
    let project = nurt_testkit::paths::abs("/project");
    format!(
        r#"(component
  (import "nurt:ext/host@1.0.0" (instance $host
    (type $failure (record (field "code" string) (field "message" string)))
    (export "failure" (type $failure-type (eq $failure)))
    (type $level (enum "error" "warn" "info" "debug" "trace"))
    (export "log-level" (type $level-type (eq $level)))
    (export "write-file" (func (param "path" string) (param "content" (list u8)) (result (result (error $failure-type)))))
    (export "list-dir" (func (param "path" string) (result (result (list string) (error $failure-type)))))
    (export "log" (func (param "level" $level-type) (param "message" string)))))
  (core module $libc-module
    (memory (export "memory") 1)
    (data (i32.const 100) "hello")
    (data (i32.const 112) "{out}")
    (data (i32.const 160) "abc")
    (data (i32.const 176) "{project}")
    (data (i32.const 200) "{record}")
    (global $heap (mut i32) (i32.const 1024))
    (func (export "realloc") (param $old i32) (param $old-size i32) (param $align i32) (param $size i32) (result i32)
      (local $ptr i32)
      (local.set $ptr
        (i32.and
          (i32.add (global.get $heap) (i32.sub (local.get $align) (i32.const 1)))
          (i32.sub (i32.const 0) (local.get $align))))
      (global.set $heap (i32.add (local.get $ptr) (local.get $size)))
      (block $enough
        (br_if $enough (i32.le_u (global.get $heap) (i32.shl (memory.size) (i32.const 16))))
        (br_if $enough
          (i32.ne
            (memory.grow
              (i32.sub
                (i32.add (i32.shr_u (global.get $heap) (i32.const 16)) (i32.const 1))
                (memory.size)))
            (i32.const -1)))
        (unreachable))
      (local.get $ptr)))
  (core instance $libc (instantiate $libc-module))
  (alias core export $libc "memory" (core memory $memory))
  (alias core export $libc "realloc" (core func $realloc))
  (core func $write-file (canon lower (func $host "write-file") (memory $memory) (realloc $realloc)))
  (core func $list-dir (canon lower (func $host "list-dir") (memory $memory) (realloc $realloc)))
  (core func $log (canon lower (func $host "log") (memory $memory) (realloc $realloc)))
  (core instance $imports
    (export "write-file" (func $write-file))
    (export "list-dir" (func $list-dir))
    (export "log" (func $log)))
  (core module $main-module
    (import "libc" "memory" (memory 1))
    (import "host" "write-file" (func $write-file (param i32 i32 i32 i32 i32)))
    (import "host" "list-dir" (func $list-dir (param i32 i32 i32)))
    (import "host" "log" (func $log (param i32 i32 i32)))
    (func (export "run")
      (param $command i32) (param $command-len i32)
      (param $args i32) (param $args-len i32)
      (param $input i32) (param $input-len i32)
      (result i32)
      (call $log (i32.const 2) (i32.const 100) (i32.const 5))
      (call $write-file (i32.const 112) (i32.const {out_len}) (i32.const 160) (i32.const 3) (i32.const 16))
      (if (i32.load8_u (i32.const 16))
        (then
          (i32.store8 (i32.const 80) (i32.const 1))
          (i32.store (i32.const 84) (i32.load (i32.const 28)))
          (i32.store (i32.const 88) (i32.load (i32.const 32)))
          (return (i32.const 80))))
      (call $list-dir (i32.const 176) (i32.const {project_len}) (i32.const 32))
      (i32.store (i32.const 256) (i32.const 200))
      (i32.store (i32.const 260) (i32.const {length}))
      (i32.store8 (i32.const 80) (i32.const 0))
      (i32.store (i32.const 84) (i32.const 256))
      (i32.store (i32.const 88) (i32.const 1))
      (i32.const 80)))
  (core instance $main (instantiate $main-module
    (with "libc" (instance $libc))
    (with "host" (instance $imports))))
  (func $run
    (param "command" string) (param "args" string) (param "input" (list string))
    (result (result (list string) (error string)))
    (canon lift (core func $main "run") (memory $memory) (realloc $realloc)))
  (instance $stage (export "run" (func $run)))
  (export "nurt:ext/stage@1.0.0" (instance $stage)))
"#,
        length = RECORD.len(),
        out_len = out.len(),
        project_len = project.len()
    )
}

fn worker_policy(write: bool) -> PathPolicy {
    let permissions = Permissions {
        read: vec![nurt_testkit::paths::abs("/project")],
        write: if write {
            vec![nurt_testkit::paths::abs("/project")]
        } else {
            Vec::new()
        },
        ..Permissions::default()
    };
    PathPolicy::new("acme", &permissions, &PlaceholderRoots::default())
}

fn run_worker(writes: bool, granted: bool) -> (Result<nurt_ext::stage::GuestOutput>, TestContext) {
    let host = WasmHost::new().unwrap();
    let worker = host.compile("acme", worker_component().as_bytes()).unwrap();
    let context = context();
    let policy = worker_policy(granted);
    let call = GuestCall {
        command: "acme.put",
        args: "{}".to_owned(),
        input: Vec::new(),
        policy: &policy,
        writes,
    };
    let output = worker.run(call, &context.ctx());
    (output, context)
}

#[test]
fn write_requests_are_planned_and_messages_reach_the_event_stream() {
    let (output, context) = run_worker(true, true);
    let output = output.unwrap();
    assert_eq!(output.records, [RECORD]);
    assert_eq!(output.writes.len(), 1);
    let PlannedWrite { path, content } = &output.writes[0];
    assert_eq!(path, &test_path("/project/out.txt").unwrap());
    assert_eq!(content, b"abc");
    assert!(
        !context.fs().exists(&test_path("/project/out.txt").unwrap()),
        "the host plans the write; it does not perform it"
    );
    let logged = context.events().events().into_iter().any(|event| {
        matches!(
            event,
            Event::Log(log) if log.message == "hello"
                && log.level == LogLevel::Info
                && log.source.as_deref() == Some("acme.put")
        )
    });
    assert!(logged);
}

#[test]
fn the_bytes_a_guest_plans_to_write_are_limited() {
    let host = WasmHost::new().unwrap();
    let worker = host.compile("acme", worker_component().as_bytes()).unwrap();
    let context = context();
    let policy = worker_policy(true);
    let call = GuestCall {
        command: "acme.put",
        args: "{}".to_owned(),
        input: Vec::new(),
        policy: &policy,
        writes: true,
    };
    let limits = nurt_contracts::stage::Limits {
        max_file_bytes_text: 2,
        ..nurt_contracts::stage::Limits::default()
    };
    let error = worker
        .run(call, &context.builder().limits(limits).build())
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::StageFailed);
    assert!(
        error.message.contains("limits.max_file_bytes_text"),
        "{}",
        error.message
    );
}

#[test]
fn writes_are_refused_without_the_declaration_or_the_grant() {
    let (undeclared, _) = run_worker(false, true);
    let error = undeclared.unwrap_err();
    assert_eq!(error.code(), ErrorCode::Permission);
    assert!(error.message.contains("writes = true"), "{}", error.message);
    let (ungranted, _) = run_worker(true, false);
    assert_eq!(ungranted.unwrap_err().code(), ErrorCode::Permission);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
