// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `copy` with the default `preserve: true` and `move` keep the times and the
//! permission bits of the files they place, also from one volume to another;
//! run on the virtual file system.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::fs::{AbsPath, Follow, Fs};
use nurt_contracts::journal::JournalStore;
use nurt_contracts::stage::{ArgValue, BoundArgs, StageData, StageImpl, StageInput};
use nurt_contracts::value::Value;
use nurt_testkit::ctx::TestContext;
use nurt_testkit::fixtures::begin_request;
use nurt_testkit::vfs::{VolumeConfig, test_path};

/// Modification time in nanoseconds, a whole number of seconds in the past.
const MODIFIED_NS: i64 = 1_600_000_000_000_000_000;

/// Permission bits of the test files.
const MODE: u32 = 0o640;

const PERMISSION_BITS: u32 = 0o7777;

fn stage(command: &str) -> Arc<dyn StageImpl> {
    nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == command)
        .expect("the command is registered")
}

fn abs(text: &str) -> AbsPath {
    test_path(text).expect("test paths are absolute")
}

/// Writes a file with the fixed time and mode.
fn write(test: &TestContext, path: &str, content: &[u8]) -> AbsPath {
    let path = abs(path);
    test.fs()
        .create_dir_all(&path.parent().expect("files have a parent"))
        .expect("parent created");
    test.fs().write_file(&path, content).expect("file written");
    test.fs()
        .set_modified(&path, MODIFIED_NS)
        .expect("the time is set");
    test.fs().set_mode(&path, MODE).expect("the mode is set");
    path
}

fn path_value(path: &AbsPath) -> ArgValue {
    ArgValue::Value(Value::Path(path.to_language_string()))
}

fn run(test: &TestContext, args: &BoundArgs) -> Result<()> {
    let stage = stage(args.command());
    let effect = stage.as_effect().expect("an effect stage");
    let ctx = test.ctx();
    let plan = effect.plan(args, StageInput::none(), &ctx)?;
    let mut tx = test.journal().begin(begin_request(args.command()));
    let outcome = effect.apply(args, plan, tx.as_mut(), &ctx)?;
    let StageData::Records(stream) = outcome.output.data else {
        panic!("the stage returns the effect record");
    };
    stream.collect::<Result<Vec<_>>>()?;
    tx.commit()?;
    Ok(())
}

fn assert_kept(test: &TestContext, path: &AbsPath) {
    let meta = test
        .fs()
        .metadata(path, Follow::NoFollow)
        .expect("the file exists");
    assert_eq!(meta.modified_ns, MODIFIED_NS, "modification time of {path:?}");
    assert_eq!(meta.mode & PERMISSION_BITS, MODE, "mode of {path:?}");
}

#[test]
fn a_copied_file_keeps_its_time_and_mode() {
    let test = TestContext::new();
    let source = write(&test, "/srv/app/config.toml", b"port = 8080\n");
    let target = abs("/srv/app/config.toml.bak");
    let args = BoundArgs::new("copy")
        .with("from", path_value(&source), false)
        .with("to", path_value(&target), false);
    run(&test, &args).unwrap();
    assert_kept(&test, &target);
    assert_kept(&test, &source);
}

#[test]
fn the_files_of_a_copied_tree_keep_their_times() {
    let test = TestContext::new();
    write(&test, "/srv/app/src/main.rs", b"fn main() {}\n");
    write(&test, "/srv/app/src/lib/mod.rs", b"pub mod a;\n");
    test.fs().create_dir_all(&abs("/backup")).unwrap();
    let args = BoundArgs::new("copy")
        .with("from", path_value(&abs("/srv/app/src")), false)
        .with("into", path_value(&abs("/backup/copy")), false);
    run(&test, &args).unwrap();
    assert_kept(&test, &abs("/backup/copy/src/main.rs"));
    assert_kept(&test, &abs("/backup/copy/src/lib/mod.rs"));
}

#[test]
fn a_moved_file_keeps_its_time_and_mode() {
    let test = TestContext::new();
    let source = write(&test, "/srv/a.log", b"a");
    let target = abs("/srv/old/a.log");
    let args = BoundArgs::new("move")
        .with(
            "from",
            ArgValue::Value(Value::List(vec![Value::Path(source.to_language_string())])),
            false,
        )
        .with("into", path_value(&abs("/srv/old")), false);
    run(&test, &args).unwrap();
    assert!(!test.fs().exists(&source));
    assert_kept(&test, &target);
}

#[test]
fn a_file_moved_to_another_volume_keeps_its_time_and_mode() {
    let test = TestContext::new();
    test.fs()
        .add_volume(&abs("/mnt/usb"), VolumeConfig::default())
        .unwrap();
    let source = write(&test, "/data/export.csv", b"id,name\n");
    let target = abs("/mnt/usb/export.csv");
    let args = BoundArgs::new("move")
        .with(
            "from",
            ArgValue::Value(Value::List(vec![Value::Path(source.to_language_string())])),
            false,
        )
        .with("to", path_value(&target), false);
    run(&test, &args).unwrap();
    assert!(!test.fs().exists(&source));
    assert_kept(&test, &target);
}
