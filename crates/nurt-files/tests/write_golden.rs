// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Examples of `docs/commands/write.md`, run on the virtual file system,
//! and the validation of written text.

// Test code unwraps results so that a failure aborts the test with the original error.
#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::sync::Arc;

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, Follow, Fs};
use nurt_contracts::journal::{JournalStore, RollbackReason};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::record::{Change, ChangeAction, LineEnding};
use nurt_contracts::report::Notable;
use nurt_contracts::services::{
    ByteRange, DocFormatId, DocFormatInfo, DocItem, DocService, IssueSeverity, SerializeOptions,
    Services, SyntaxIssue, TextPosition, TextSpan,
};
use nurt_contracts::stage::{ArgValue, BoundArgs, StageImpl, StageInput};
use nurt_contracts::value::{Fields, Record, Size, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::fixtures::begin_request;
use nurt_testkit::journal::JournalCall;
use nurt_testkit::vfs::test_path;

const COMMAND: &str = "write";

#[derive(Debug)]
struct Run {
    changes: Vec<Change>,
    notable: Vec<Notable>,
}

fn abs(path: &str) -> AbsPath {
    test_path(path).unwrap()
}

fn path_value(path: &str) -> Value {
    Value::Path(abs(path).to_language_string())
}

fn set(args: BoundArgs, name: &str, value: Value) -> BoundArgs {
    args.with(name, ArgValue::Value(value), false)
}

fn write_args(path: &str, text: &str, mode: &str) -> BoundArgs {
    let args = set(BoundArgs::new(COMMAND), "path", path_value(path));
    let args = set(args, "text", Value::Text(text.to_owned()));
    set(args, "mode", Value::Enum(mode.to_owned()))
}

fn stage() -> Arc<dyn StageImpl> {
    nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == COMMAND)
        .unwrap()
}

fn run(test: &TestContext, args: &BoundArgs) -> Result<Run> {
    let stage = stage();
    let effect = stage.as_effect().unwrap();
    let ctx = test.ctx();
    let plan = effect.plan(args, StageInput::none(), &ctx)?;
    let notable = plan.notable.iter().map(|entry| entry.code).collect();
    let mut tx = test.journal().begin(begin_request(COMMAND));
    let changes = effect.apply(args, plan, tx.as_mut(), &ctx)?.changes;
    tx.commit()?;
    Ok(Run { changes, notable })
}

fn app() -> TestContext {
    let test = TestContext::new();
    test.fs().create_dir_all(&abs("/home/user/app")).unwrap();
    test
}

fn assert_every_change_declared(test: &TestContext) {
    let calls = test.journal().calls();
    let declared: usize = calls
        .iter()
        .map(|call| match call {
            JournalCall::Preserve { targets, .. } => targets.len(),
            _ => 0,
        })
        .sum();
    let applied = calls
        .iter()
        .filter(|call| matches!(call, JournalCall::Apply { ok: true, .. }))
        .count();
    assert!(declared > 0);
    assert_eq!(declared, applied);
}

/// The Dockerfile of the first example: three lines, 56 bytes.
const DOCKERFILE: &str = "FROM alpine:3.20\nRUN apk add --no-cache curl\nCMD [\"sh\"]\n";

#[test]
fn creates_a_file_with_multiline_content() {
    let test = app();
    let created = run(
        &test,
        &write_args("/home/user/app/Dockerfile", DOCKERFILE, "create"),
    )
    .unwrap();
    assert_eq!(created.changes.len(), 1);
    assert_eq!(created.changes[0].action, ChangeAction::Create);
    assert_eq!(created.changes[0].size_before, None);
    assert_eq!(created.changes[0].size_after, Some(Size::from_bytes(56)));
    assert_eq!(
        test.fs()
            .read_file(&abs("/home/user/app/Dockerfile"))
            .unwrap(),
        DOCKERFILE.as_bytes()
    );
    assert_every_change_declared(&test);

    let again = run(
        &test,
        &write_args("/home/user/app/Dockerfile", DOCKERFILE, "create"),
    )
    .unwrap_err();
    assert_eq!(again.code(), ErrorCode::Exists);
}

#[test]
fn appends_a_line_to_an_existing_file() {
    let test = app();
    let ignore = abs("/home/user/app/.gitignore");
    let content: String = (1..=12).map(|n| format!("build-{n:02}/\n")).collect();
    test.fs().write_file(&ignore, content.as_bytes()).unwrap();
    let run = run(
        &test,
        &write_args("/home/user/app/.gitignore", "dist/\n", "append"),
    )
    .unwrap();
    assert_eq!(run.changes[0].action, ChangeAction::Modify);
    assert_eq!(run.changes[0].size_before, Some(Size::from_bytes(120)));
    assert_eq!(run.changes[0].size_after, Some(Size::from_bytes(126)));
    let written = test.fs().read_file(&ignore).unwrap();
    assert!(written.starts_with(content.as_bytes()));
    assert!(written.ends_with(b"build-12/\ndist/\n"));
    assert_every_change_declared(&test);
}

#[test]
fn overwrite_keeps_the_line_endings_of_the_file() {
    let test = app();
    let config = abs("/home/user/app/config.ini");
    test.fs()
        .write_file(&config, b"[main]\r\nport = 80\r\n")
        .unwrap();
    let run = run(
        &test,
        &write_args(
            "/home/user/app/config.ini",
            "[main]\nport = 8080\n",
            "overwrite",
        ),
    )
    .unwrap();
    assert_eq!(
        test.fs().read_file(&config).unwrap(),
        b"[main]\r\nport = 8080\r\n"
    );
    assert_eq!(run.changes[0].line_ending_after, Some(LineEnding::Crlf));
    assert!(run.notable.contains(&Notable::OverwroteExisting));
    assert!(run.notable.contains(&Notable::LineEndingPreserved));
    assert_every_change_declared(&test);
}

#[test]
fn writes_a_batch_in_one_transaction() {
    let test = app();
    let entry = |path: &str, text: &str| {
        let mut fields = Fields::new();
        fields.insert("path".to_owned(), path_value(path));
        fields.insert("text".to_owned(), Value::Text(text.to_owned()));
        Value::Map(fields)
    };
    let args = set(
        set(
            BoundArgs::new(COMMAND),
            "files",
            Value::List(vec![
                entry("/home/user/app/a.txt", "a\n"),
                entry("/home/user/app/docs/b.txt", "b\n"),
            ]),
        ),
        "mode",
        Value::Enum("create".to_owned()),
    );
    let run = run(&test, &args).unwrap();
    let actions: Vec<ChangeAction> = run.changes.iter().map(|change| change.action).collect();
    assert_eq!(actions, [ChangeAction::Create; 3]);
    let docs = test
        .fs()
        .metadata(&abs("/home/user/app/docs"), Follow::NoFollow)
        .unwrap();
    assert!(docs.kind == nurt_contracts::fs::FileKind::Directory);
    assert_eq!(
        test.fs()
            .read_file(&abs("/home/user/app/docs/b.txt"))
            .unwrap(),
        b"b\n"
    );
    assert_every_change_declared(&test);
}

#[test]
fn a_stale_etag_changes_nothing() {
    let test = app();
    let notes = abs("/home/user/app/notes.txt");
    test.fs().write_file(&notes, b"draft\n").unwrap();
    let tag = test.fs().etag(&notes, Follow::NoFollow).unwrap();
    test.fs().write_file(&notes, b"edited elsewhere\n").unwrap();
    let text = nurt_files::common::etag::etag_text(&tag);
    let args = set(
        write_args("/home/user/app/notes.txt", "final\n", "overwrite"),
        "if_match",
        Value::Text(text),
    );
    let error = run(&test, &args).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Stale);
    assert_eq!(test.fs().read_file(&notes).unwrap(), b"edited elsewhere\n");
    assert!(test.journal().calls().is_empty());
}

/// Document service that knows `.json` files and reports one syntax error
/// when the braces are unbalanced.
struct BraceChecker;

impl DocService for BraceChecker {
    fn formats(&self) -> Vec<DocFormatInfo> {
        Vec::new()
    }

    fn detect_format(&self, path: Option<&Path>, _bytes: &[u8]) -> Option<DocFormatId> {
        path.filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .map(|_| DocFormatId::JSON)
    }

    fn read_keys(
        &self,
        _bytes: &[u8],
        _format: &DocFormatId,
        _key: &KeyPath,
        _depth: Option<u32>,
    ) -> Result<Vec<DocItem>> {
        Ok(Vec::new())
    }

    fn check_syntax(&self, bytes: &[u8], _format: &DocFormatId) -> Result<Vec<SyntaxIssue>> {
        let open = bytes.iter().filter(|byte| **byte == b'{').count();
        let close = bytes.iter().filter(|byte| **byte == b'}').count();
        if open == close {
            return Ok(Vec::new());
        }
        let position = TextPosition { line: 1, column: 1 };
        Ok(vec![SyntaxIssue {
            severity: IssueSeverity::Error,
            message: "unbalanced braces".to_owned(),
            span: TextSpan {
                bytes: ByteRange::new(0, 0),
                start: position,
                end: position,
            },
            kind: Some("unbalanced".to_owned()),
        }])
    }

    fn serialize_records(
        &self,
        _records: &[Record],
        _format: &DocFormatId,
        _options: &SerializeOptions,
    ) -> Result<Vec<u8>> {
        Ok(Vec::new())
    }
}

/// Context with the brace checker and the directory `/home/user/app`.
fn checked_app() -> TestContext {
    let test = TestContext::new().with_services(Services::new().with_doc(Arc::new(BraceChecker)));
    test.fs().create_dir_all(&abs("/home/user/app")).unwrap();
    test
}

/// Plans and applies a call; a failed apply rolls the transaction back.
fn run_checked(test: &TestContext, args: &BoundArgs) -> Result<Vec<Change>> {
    let stage = stage();
    let effect = stage.as_effect().unwrap();
    let ctx = test.ctx();
    let plan = effect.plan(args, StageInput::none(), &ctx)?;
    let mut tx = test.journal().begin(begin_request(COMMAND));
    match effect.apply(args, plan, tx.as_mut(), &ctx) {
        Ok(outcome) => {
            let changes = outcome.changes;
            tx.commit()?;
            Ok(changes)
        }
        Err(error) => {
            tx.rollback(RollbackReason::ValidateFailed)?;
            Err(error)
        }
    }
}

#[test]
fn a_new_syntax_error_rolls_the_write_back() {
    let test = checked_app();
    let config = abs("/home/user/app/config.json");
    test.fs().write_file(&config, b"{\"port\": 80}\n").unwrap();
    let error = run_checked(
        &test,
        &write_args(
            "/home/user/app/config.json",
            "{\"port\": 8080\n",
            "overwrite",
        ),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::RolledBack);
    assert_eq!(error.problem.as_deref(), Some("validate_failed"));
    assert_eq!(
        error.sample,
        [format!("{config}:1:1 unbalanced braces [json]")]
    );
    assert_every_change_declared(&test);
    assert!(test.journal().calls().iter().any(|call| matches!(
        call,
        JournalCall::Rollback {
            reason: Some(RollbackReason::ValidateFailed),
            ..
        }
    )));
}

#[test]
fn an_error_present_before_the_write_does_not_block_it() {
    let test = checked_app();
    let config = abs("/home/user/app/config.json");
    test.fs().write_file(&config, b"{\"port\": 80\n").unwrap();
    let changes = run_checked(
        &test,
        &write_args(
            "/home/user/app/config.json",
            "{\"port\": 8080\n",
            "overwrite",
        ),
    )
    .unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(test.fs().read_file(&config).unwrap(), b"{\"port\": 8080\n");
    assert_every_change_declared(&test);
}
