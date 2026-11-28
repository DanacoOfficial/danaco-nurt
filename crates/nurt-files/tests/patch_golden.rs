// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/patch_golden.rs
// Responsibility:   Examples of `docs/commands/patch.md`, run on the virtual file system with the recording journal, and the validation of patched text.
// Dependencies:     nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Examples of `docs/commands/patch.md`, run on the virtual file system
//! with the recording journal, and the validation of patched text.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

mod support_report;

use std::path::Path;
use std::sync::Arc;

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{JournalStore, RollbackReason, TargetPath};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::record::{ChangeAction, Effect};
use nurt_contracts::services::{
    ByteRange, DocFormatId, DocFormatInfo, DocItem, DocService, IssueSeverity, SerializeOptions,
    Services, SyntaxIssue, TextPosition, TextSpan,
};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, Event, LogEvent, LogLevel, StageData, StageImpl, StageInput,
};
use nurt_contracts::value::{Record, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::fixtures::begin_request;
use nurt_testkit::journal::JournalCall;
use nurt_testkit::vfs::test_path;

/// Diff written by `git diff` for the first example.
const FIX_TIMEOUT: &str = "\
diff --git a/src/config.rs b/src/config.rs
index 3b18e51..a2c4f0d 100644
--- a/src/config.rs
+++ b/src/config.rs
@@ -1,4 +1,4 @@
 pub struct Config {
-    pub timeout: u32,
+    pub timeout: u64,
     pub port: u16,
 }
";

/// Diff written by `git diff -M` for the second example: a new file, a
/// deleted file and a renamed file.
const LAYOUT: &str = "\
diff --git a/docs/setup.md b/docs/setup.md
new file mode 100644
index 0000000..e69de29
--- /dev/null
+++ b/docs/setup.md
@@ -0,0 +1,2 @@
+# Setup
+Run the installer.
diff --git a/INSTALL.txt b/INSTALL.txt
deleted file mode 100644
index 9daeafb..0000000
--- a/INSTALL.txt
+++ /dev/null
@@ -1 +0,0 @@
-Run the installer.
diff --git a/src/util.rs b/src/support/util.rs
similarity index 100%
rename from src/util.rs
rename to src/support/util.rs
";

const CONFIG_BEFORE: &[u8] =
    b"pub struct Config {\r\n    pub timeout: u32,\r\n    pub port: u16,\r\n}\r\n";
const CONFIG_AFTER: &[u8] =
    b"pub struct Config {\r\n    pub timeout: u64,\r\n    pub port: u16,\r\n}\r\n";

fn patch_stage() -> Arc<dyn StageImpl> {
    nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == "patch")
        .expect("patch is registered")
}

fn abs(text: &str) -> AbsPath {
    test_path(text).unwrap()
}

fn language(text: &str) -> String {
    abs(text).to_language_string()
}

fn with_files(files: &[(&str, &[u8])]) -> TestContext {
    let test = TestContext::new();
    for (path, content) in files {
        test.fs().write_file(&abs(path), content).unwrap();
    }
    test
}

fn args(from: &str, params: &[(&str, Value)]) -> BoundArgs {
    let mut args = BoundArgs::new("patch");
    args.insert("from", ArgValue::Value(Value::Path(language(from))), false);
    args.insert(
        "root",
        ArgValue::Value(Value::Path(language("/srv/app"))),
        false,
    );
    args.insert("strip", ArgValue::Value(Value::Int(1)), false);
    for (name, value) in params {
        args.insert(*name, ArgValue::Value(value.clone()), false);
    }
    args
}

/// Plan and apply in one transaction, as the executor does.
fn run(test: &TestContext, args: &BoundArgs) -> Result<Effect> {
    let stage = patch_stage();
    let effect_stage = stage.as_effect().expect("patch is an effect stage");
    let ctx = test.ctx();
    let plan = effect_stage.plan(args, StageInput::none(), &ctx)?;
    support_report::assert_plan_valid(args.command(), &plan.changes);
    let mut tx = test.journal().begin(begin_request("patch"));
    let outcome = effect_stage.apply(args, plan, tx.as_mut(), &ctx)?;
    support_report::assert_changes_valid(args.command(), &outcome.changes);
    let StageData::Records(stream) = outcome.output.data else {
        panic!("expected the effect record");
    };
    let records: Vec<Record> = stream.collect::<Result<_>>()?;
    tx.commit()?;
    Ok(Effect::try_from(records[0].clone()).unwrap())
}

fn read(test: &TestContext, path: &str) -> Vec<u8> {
    test.fs().read_file(&abs(path)).unwrap()
}

fn declared(test: &TestContext) -> Vec<TargetPath> {
    test.journal()
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            JournalCall::Preserve { targets, .. } => Some(targets),
            _ => None,
        })
        .flatten()
        .collect()
}

fn applied_ok(test: &TestContext) -> usize {
    test.journal()
        .calls()
        .into_iter()
        .filter(|call| matches!(call, JournalCall::Apply { ok: true, .. }))
        .count()
}

/// Example `patch from: "/srv/patches/fix-timeout.diff" root: "/srv/app"
/// strip: 1` applies a git diff with `lf` line endings to a file with `crlf`
/// line endings, which keeps them.
#[test]
fn applies_a_git_diff_to_a_crlf_file() {
    let test = with_files(&[
        ("/srv/app/src/config.rs", CONFIG_BEFORE),
        ("/srv/patches/fix-timeout.diff", FIX_TIMEOUT.as_bytes()),
    ]);
    let effect = run(&test, &args("/srv/patches/fix-timeout.diff", &[])).unwrap();
    assert_eq!(read(&test, "/srv/app/src/config.rs"), CONFIG_AFTER);
    let changes = effect.changes.unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].action, ChangeAction::Modify);
    assert_eq!(changes[0].path.as_str(), language("/srv/app/src/config.rs"));
    assert_eq!((changes[0].added, changes[0].removed), (Some(1), Some(1)));
    assert_eq!(effect.kind, "patch");
    assert_eq!(declared(&test).len(), applied_ok(&test));
}

/// Example `patch from: "/srv/patches/layout.diff" root: "/srv/app" strip: 1`
/// creates, deletes and renames files in one transaction.
#[test]
fn creates_deletes_and_renames_files() {
    let test = with_files(&[
        ("/srv/app/INSTALL.txt", b"Run the installer.\n"),
        ("/srv/app/src/util.rs", b"pub fn run() {}\n"),
        ("/srv/patches/layout.diff", LAYOUT.as_bytes()),
    ]);
    let effect = run(&test, &args("/srv/patches/layout.diff", &[])).unwrap();
    assert_eq!(
        read(&test, "/srv/app/docs/setup.md"),
        b"# Setup\nRun the installer.\n"
    );
    assert!(!test.fs().exists(&abs("/srv/app/INSTALL.txt")));
    assert!(!test.fs().exists(&abs("/srv/app/src/util.rs")));
    assert_eq!(
        read(&test, "/srv/app/src/support/util.rs"),
        b"pub fn run() {}\n"
    );
    let summary: Vec<(String, ChangeAction)> = effect
        .changes
        .unwrap()
        .iter()
        .map(|change| (change.path.as_str().to_owned(), change.action))
        .collect();
    assert_eq!(
        summary,
        [
            (language("/srv/app/docs"), ChangeAction::Create),
            (language("/srv/app/src/support"), ChangeAction::Create),
            (
                language("/srv/app/src/support/util.rs"),
                ChangeAction::Rename
            ),
            (language("/srv/app/INSTALL.txt"), ChangeAction::Delete),
            (language("/srv/app/docs/setup.md"), ChangeAction::Create),
        ]
    );
    assert_eq!(declared(&test).len(), applied_ok(&test));
    assert_eq!(effect.targets, Some(5));
}

/// Example `patch from: "/srv/patches/fix-timeout.diff" root: "/srv/app"
/// strip: 1 reverse: true` undoes the first example.
#[test]
fn reverse_undoes_a_diff() {
    let test = with_files(&[
        ("/srv/app/src/config.rs", CONFIG_AFTER),
        ("/srv/patches/fix-timeout.diff", FIX_TIMEOUT.as_bytes()),
    ]);
    run(
        &test,
        &args(
            "/srv/patches/fix-timeout.diff",
            &[("reverse", Value::Bool(true))],
        ),
    )
    .unwrap();
    assert_eq!(read(&test, "/srv/app/src/config.rs"), CONFIG_BEFORE);
}

/// Example of a diff that no longer matches: `E_PATCH_REJECTED` names the
/// file and the line, and nothing changes.
#[test]
fn rejects_a_diff_that_does_not_match() {
    let changed = b"pub struct Config {\n    pub timeout: Duration,\n    pub port: u16,\n}\n";
    let test = with_files(&[
        ("/srv/app/src/config.rs", changed),
        ("/srv/patches/fix-timeout.diff", FIX_TIMEOUT.as_bytes()),
    ]);
    let error = run(&test, &args("/srv/patches/fix-timeout.diff", &[])).unwrap_err();
    assert_eq!(error.code(), ErrorCode::PatchRejected);
    let location = error.location.clone().unwrap();
    assert_eq!(location.path, language("/srv/app/src/config.rs"));
    assert_eq!(location.line, Some(1));
    assert_eq!(read(&test, "/srv/app/src/config.rs"), changed);
    assert!(declared(&test).is_empty());
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

/// Manifest of the validation examples, with balanced braces.
const MANIFEST: &[u8] = b"{\n  \"name\": \"app\",\n  \"version\": \"1.0.0\"\n}\n";

/// Manifest without its closing brace.
const OPEN_MANIFEST: &[u8] = b"{\n  \"name\": \"app\",\n  \"version\": \"1.0.0\"\n";

/// Diff that changes `README.md` and drops the closing brace of
/// `package.json`.
const BUMP: &str = "\
--- a/README.md
+++ b/README.md
@@ -1 +1 @@
-Version 1.0.0.
+Version 1.1.0.
--- a/package.json
+++ b/package.json
@@ -1,4 +1,3 @@
 {
   \"name\": \"app\",
-  \"version\": \"1.0.0\"
-}
+  \"version\": \"1.1.0\",
";

/// Diff that changes the version of a manifest without a closing brace.
const BUMP_OPEN: &str = "\
--- a/package.json
+++ b/package.json
@@ -1,3 +1,3 @@
 {
   \"name\": \"app\",
-  \"version\": \"1.0.0\"
+  \"version\": \"1.1.0\"
";

/// Context with the brace checker, the project and the diffs.
fn checked_app(manifest: &[u8]) -> TestContext {
    let test = TestContext::new().with_services(Services::new().with_doc(Arc::new(BraceChecker)));
    for (path, content) in [
        ("/srv/app/package.json", manifest),
        ("/srv/app/README.md", b"Version 1.0.0.\n".as_slice()),
        ("/srv/patches/bump.diff", BUMP.as_bytes()),
        ("/srv/patches/bump-open.diff", BUMP_OPEN.as_bytes()),
    ] {
        test.fs().write_file(&abs(path), content).unwrap();
    }
    test
}

/// Plans and applies a call as the executor does: a failed apply rolls the
/// transaction back.
fn run_checked(test: &TestContext, args: &BoundArgs) -> Result<Effect> {
    let stage = patch_stage();
    let effect_stage = stage.as_effect().expect("patch is an effect stage");
    let ctx = test.ctx();
    let plan = effect_stage.plan(args, StageInput::none(), &ctx)?;
    support_report::assert_plan_valid(args.command(), &plan.changes);
    let mut tx = test.journal().begin(begin_request("patch"));
    match effect_stage.apply(args, plan, tx.as_mut(), &ctx) {
        Ok(outcome) => {
            support_report::assert_changes_valid(args.command(), &outcome.changes);
            let StageData::Records(stream) = outcome.output.data else {
                panic!("expected the effect record");
            };
            let records: Vec<Record> = stream.collect::<Result<_>>()?;
            tx.commit()?;
            Ok(Effect::try_from(records[0].clone()).unwrap())
        }
        Err(error) => {
            tx.rollback(RollbackReason::ValidateFailed)?;
            Err(error)
        }
    }
}

fn validate(mode: &str) -> (&'static str, Value) {
    ("validate", Value::Enum(mode.to_owned()))
}

/// Example `patch from: "/srv/patches/bump.diff" root: "/srv/app" strip: 1`:
/// a diff that adds a syntax error to one file rolls the whole patch back.
#[test]
fn a_new_syntax_error_rolls_the_whole_patch_back() {
    let test = checked_app(MANIFEST);
    let error = run_checked(&test, &args("/srv/patches/bump.diff", &[])).unwrap_err();
    assert_eq!(error.code(), ErrorCode::RolledBack);
    assert_eq!(error.problem.as_deref(), Some("validate_failed"));
    assert_eq!(
        error.sample,
        [format!(
            "{}:1:1 unbalanced braces [json]",
            language("/srv/app/package.json")
        )]
    );
    assert_eq!(
        declared(&test),
        [
            TargetPath::Fs(abs("/srv/app/README.md")),
            TargetPath::Fs(abs("/srv/app/package.json")),
        ]
    );
    assert_eq!(applied_ok(&test), 2);
    let calls = test.journal().calls();
    assert!(calls.iter().any(|call| matches!(
        call,
        JournalCall::Rollback {
            reason: Some(RollbackReason::ValidateFailed),
            ..
        }
    )));
    assert!(
        !calls
            .iter()
            .any(|call| matches!(call, JournalCall::Commit { .. }))
    );
}

/// A syntax error that the file already had does not block the patch.
#[test]
fn an_error_present_before_the_patch_does_not_block_it() {
    let test = checked_app(OPEN_MANIFEST);
    let effect = run_checked(&test, &args("/srv/patches/bump-open.diff", &[])).unwrap();
    assert_eq!(effect.targets, Some(1));
    assert_eq!(
        read(&test, "/srv/app/package.json"),
        b"{\n  \"name\": \"app\",\n  \"version\": \"1.1.0\"\n"
    );
    assert_eq!(declared(&test).len(), applied_ok(&test));
}

/// `validate: warn` reports the new error and keeps the patch; `off` does
/// not check it.
#[test]
fn warn_reports_a_new_error_and_off_skips_the_check() {
    let test = checked_app(MANIFEST);
    run_checked(&test, &args("/srv/patches/bump.diff", &[validate("warn")])).unwrap();
    assert_eq!(read(&test, "/srv/app/README.md"), b"Version 1.1.0.\n");
    assert_eq!(
        read(&test, "/srv/app/package.json"),
        b"{\n  \"name\": \"app\",\n  \"version\": \"1.1.0\",\n"
    );
    let events = test.events().take();
    let warnings: Vec<&LogEvent> = events
        .iter()
        .filter_map(|event| match event {
            Event::Log(log) if log.level == LogLevel::Warn => Some(log),
            _ => None,
        })
        .collect();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].source.as_deref(), Some("patch"));
    assert!(warnings[0].message.contains("1 syntax error"));

    let test = checked_app(MANIFEST);
    run_checked(&test, &args("/srv/patches/bump.diff", &[validate("off")])).unwrap();
    assert!(test.events().take().is_empty());
    assert_eq!(read(&test, "/srv/app/README.md"), b"Version 1.1.0.\n");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
