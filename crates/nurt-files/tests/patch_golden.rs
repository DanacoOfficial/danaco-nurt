// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Examples of `docs/commands/patch.md`, run on the virtual file system
//! with the recording journal.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::sync::Arc;

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{JournalStore, TargetPath};
use nurt_contracts::record::{ChangeAction, Effect};
use nurt_contracts::stage::{ArgValue, BoundArgs, StageData, StageImpl, StageInput};
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
    let mut tx = test.journal().begin(begin_request("patch"));
    let outcome = effect_stage.apply(args, plan, tx.as_mut(), &ctx)?;
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
            (language("/srv/app/src/util.rs"), ChangeAction::Rename),
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
