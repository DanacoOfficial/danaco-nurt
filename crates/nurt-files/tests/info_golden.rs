// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Examples of `docs/commands/info.md`, run on the virtual file system.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use nurt_contracts::fs::{AbsPath, Follow, Fs};
use nurt_contracts::record::{Entry, EntryType, LineEnding, LinkType};
use nurt_contracts::stage::{ArgValue, BoundArgs, StageData, StageImpl, StageInput};
use nurt_contracts::value::{Size, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::vfs::test_path;
use nurt_text::hash::{HashAlgorithm, hash_reader};

/// Content of `/home/user/notes.txt` in every example.
const NOTES: &[u8] = b"line one\nline two\n";

fn abs(text: &str) -> AbsPath {
    test_path(text).expect("example paths are absolute")
}

fn stage() -> Arc<dyn StageImpl> {
    nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == "info")
        .expect("`info` is registered")
}

/// File system of the examples: one text file and a link to it.
fn home() -> TestContext {
    let test = TestContext::new();
    test.fs()
        .write_file(&abs("/home/user/notes.txt"), NOTES)
        .unwrap();
    test.fs()
        .symlink(
            abs("/home/user/notes.txt").as_path(),
            &abs("/home/user/current"),
        )
        .unwrap();
    test
}

fn call(paths: &[&str], params: &[(&str, Value)]) -> BoundArgs {
    let list = paths
        .iter()
        .map(|path| Value::Path(abs(path).to_language_string()))
        .collect();
    let mut args = BoundArgs::new("info")
        .with("path", ArgValue::Value(Value::List(list)), false)
        .with(
            "hash",
            ArgValue::Value(Value::Enum("none".to_owned())),
            true,
        )
        .with("details", ArgValue::Value(Value::List(Vec::new())), true);
    for (name, value) in params {
        args.insert(*name, ArgValue::Value(value.clone()), false);
    }
    args
}

fn run(test: &TestContext, args: &BoundArgs) -> Vec<Entry> {
    let stage = stage();
    let ctx = test.ctx();
    let output = stage
        .as_read()
        .expect("`info` reads")
        .execute(args, StageInput::none(), &ctx)
        .unwrap();
    let StageData::Records(records) = output.data else {
        panic!("`info` returns records");
    };
    records
        .map(|record| Entry::try_from(record.unwrap()).unwrap())
        .collect()
}

/// Fields every listing shares, for the example file.
fn assert_shared_fields(test: &TestContext, entry: &Entry) {
    assert_eq!(
        entry.path.as_str(),
        abs("/home/user/notes.txt").to_language_string()
    );
    assert_eq!(entry.name, "notes.txt");
    assert_eq!(entry.extension.as_deref(), Some("txt"));
    assert_eq!(entry.kind, EntryType::File);
    assert_eq!(entry.exists, Some(true));
    assert_eq!(entry.size, Size::from_bytes(18));
    assert_eq!(entry.mode.as_deref(), Some("0644"));
    assert_eq!(entry.readonly, Some(false));
    let meta = test
        .fs()
        .metadata(&abs("/home/user/notes.txt"), Follow::NoFollow)
        .unwrap();
    assert_eq!(entry.links, Some(meta.links));
    assert!(entry.etag.is_some());
}

/// Fields recognised from the content of the example file.
fn assert_content_fields(entry: &Entry) {
    assert_eq!(entry.mime.as_deref(), Some("text/plain"));
    assert_eq!(entry.binary, Some(false));
    assert_eq!(entry.encoding.as_deref(), Some("utf-8"));
    assert_eq!(entry.line_ending, Some(LineEnding::Lf));
    assert_eq!(entry.lines, None);
}

#[test]
fn describes_one_file() {
    // The example program is `info "/home/user/notes.txt"`.
    let test = home();
    let entries = run(&test, &call(&["/home/user/notes.txt"], &[]));
    assert_eq!(entries.len(), 1);
    assert_shared_fields(&test, &entries[0]);
    assert_content_fields(&entries[0]);
}

#[test]
fn reports_a_missing_path_without_an_error() {
    // The example program is `info "/home/user/notes.txt" "/home/user/draft.txt"`.
    let test = home();
    let entries = run(
        &test,
        &call(&["/home/user/notes.txt", "/home/user/draft.txt"], &[]),
    );
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].exists, Some(true));
    assert_eq!(entries[1].exists, Some(false));
    assert_eq!(entries[1].name, "draft.txt");
    assert_eq!(entries[1].size, Size::from_bytes(0));
    assert_eq!(entries[1].etag, None);
}

#[test]
fn adds_the_hash_and_the_line_count() {
    // The example program is `info "/home/user/notes.txt" hash: sha256 details: [text]`.
    let test = home();
    let args = call(
        &["/home/user/notes.txt"],
        &[
            ("hash", Value::Enum("sha256".to_owned())),
            ("details", Value::List(vec![Value::Enum("text".to_owned())])),
        ],
    );
    let entry = run(&test, &args).remove(0);
    let expected = hash_reader(HashAlgorithm::Sha256, NOTES)
        .unwrap()
        .to_string();
    assert_eq!(entry.hash, Some(expected));
    assert_eq!(entry.lines, Some(2));
    let text = match entry
        .details
        .and_then(|details| details.get("text").cloned())
    {
        Some(Value::Map(fields)) => fields,
        other => panic!("no text details: {other:?}"),
    };
    assert_eq!(text.get("lines"), Some(&Value::Int(2)));
    assert_eq!(text.get("encoding"), Some(&Value::Text("utf-8".to_owned())));
    assert_eq!(text.get("bom"), Some(&Value::Bool(false)));
}

#[test]
fn describes_a_link_and_its_target() {
    // The example program is `info "/home/user/current"`.
    let test = home();
    let entry = run(&test, &call(&["/home/user/current"], &[])).remove(0);
    let target = abs("/home/user/notes.txt").to_language_string();
    assert_eq!(entry.kind, EntryType::Link);
    assert_eq!(entry.link_type, Some(LinkType::Symbolic));
    assert_eq!(
        entry.link_target.map(|path| path.as_str().to_owned()),
        Some(target.clone())
    );
    assert_eq!(
        entry.real_path.map(|path| path.as_str().to_owned()),
        Some(target)
    );
    assert_eq!(entry.mime.as_deref(), Some("text/plain"));
}
