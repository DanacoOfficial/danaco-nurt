// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-text/tests/patch_git.rs
// Responsibility:   Application of patches in git and unified format to a set of files.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Application of patches in git and unified format to a set of files.

use std::collections::BTreeMap;

use nurt_contracts::error::ErrorCode;
use nurt_text::patch::{ApplyOptions, FileChange, FileOperation, Patch, PatchError, plan};

/// Output of `git diff --cached -M` for a rename with a content change, a
/// modification, a deletion, a mode change and a new file.
const GIT_PATCH: &str = "\
diff --git a/old_name.txt b/new_name.txt
similarity index 77%
rename from old_name.txt
rename to new_name.txt
index 9864d22..0debe8e 100644
--- a/old_name.txt
+++ b/new_name.txt
@@ -1,5 +1,5 @@
 line one
 line two
-line three
+line 3
 line four
 line five
diff --git a/notes.txt b/notes.txt
index 7a28df3..24941eb 100644
--- a/notes.txt
+++ b/notes.txt
@@ -1,4 +1,5 @@
 alpha
-beta
+BETA
 gamma
 delta
+epsilon
diff --git a/obsolete.txt b/obsolete.txt
deleted file mode 100644
index 6e263ab..0000000
--- a/obsolete.txt
+++ /dev/null
@@ -1 +0,0 @@
-obsolete
diff --git a/run.sh b/run.sh
old mode 100644
new mode 100755
diff --git a/src/lib.rs b/src/lib.rs
new file mode 100644
index 0000000..3151666
--- /dev/null
+++ b/src/lib.rs
@@ -0,0 +1 @@
+created
";

/// Files of the tree before [`GIT_PATCH`].
fn tree_before() -> BTreeMap<String, String> {
    [
        ("notes.txt", "alpha\nbeta\ngamma\ndelta\n"),
        (
            "old_name.txt",
            "line one\nline two\nline three\nline four\nline five\n",
        ),
        ("run.sh", "#!/bin/sh\necho run\n"),
        ("obsolete.txt", "obsolete\n"),
        ("src/main.rs", "fn main() {}\n"),
    ]
    .into_iter()
    .map(|(path, content)| (path.to_owned(), content.to_owned()))
    .collect()
}

/// Files of the tree after [`GIT_PATCH`].
fn tree_after() -> BTreeMap<String, String> {
    [
        ("notes.txt", "alpha\nBETA\ngamma\ndelta\nepsilon\n"),
        (
            "new_name.txt",
            "line one\nline two\nline 3\nline four\nline five\n",
        ),
        ("run.sh", "#!/bin/sh\necho run\n"),
        ("src/lib.rs", "created\n"),
        ("src/main.rs", "fn main() {}\n"),
    ]
    .into_iter()
    .map(|(path, content)| (path.to_owned(), content.to_owned()))
    .collect()
}

/// Performs planned changes on a map of files, in order.
fn perform(tree: &mut BTreeMap<String, String>, changes: &[FileChange]) {
    for change in changes {
        match change {
            FileChange::Create { path, content, .. } | FileChange::Modify { path, content, .. } => {
                tree.insert(path.clone(), content.clone());
            }
            FileChange::Delete { path } => {
                tree.remove(path);
            }
            FileChange::Rename {
                from, to, content, ..
            } => {
                tree.remove(from);
                tree.insert(to.clone(), content.clone());
            }
        }
    }
}

fn strip_one() -> ApplyOptions {
    ApplyOptions {
        strip: 1,
        ..ApplyOptions::default()
    }
}

#[test]
fn parses_git_extended_headers() {
    let patch = Patch::parse(GIT_PATCH).unwrap();
    let operations: Vec<FileOperation> = patch.files.iter().map(|file| file.operation).collect();
    assert_eq!(
        operations,
        [
            FileOperation::Rename,
            FileOperation::Modify,
            FileOperation::Delete,
            FileOperation::Modify,
            FileOperation::Create,
        ]
    );
    let mode_change = &patch.files[3];
    assert_eq!(
        (mode_change.old_mode, mode_change.new_mode),
        (Some(0o100_644), Some(0o100_755))
    );
    assert!(mode_change.hunks.is_empty());
    assert_eq!(patch.files[0].source.as_deref(), Some("old_name.txt"));
    assert_eq!(patch.files[4].old_path, None);
}

#[test]
fn applies_git_patch_to_tree() {
    let patch = Patch::parse(GIT_PATCH).unwrap();
    let before = tree_before();
    let changes = plan(&patch, &strip_one(), |path| before.get(path).cloned()).unwrap();
    assert_eq!(
        changes[3].change,
        FileChange::Modify {
            path: "run.sh".to_owned(),
            content: "#!/bin/sh\necho run\n".to_owned(),
            mode: Some(0o100_755),
        }
    );
    let mut tree = before.clone();
    perform(
        &mut tree,
        &changes
            .into_iter()
            .map(|planned| planned.change)
            .collect::<Vec<_>>(),
    );
    assert_eq!(tree, tree_after());
}

#[test]
fn reverts_git_patch_in_reverse() {
    let patch = Patch::parse(GIT_PATCH).unwrap();
    let after = tree_after();
    let options = ApplyOptions {
        reverse: true,
        ..strip_one()
    };
    let changes = plan(&patch, &options, |path| after.get(path).cloned()).unwrap();
    let mut tree = after.clone();
    perform(
        &mut tree,
        &changes
            .into_iter()
            .map(|planned| planned.change)
            .collect::<Vec<_>>(),
    );
    assert_eq!(tree, tree_before());
}

#[test]
fn lists_paths_after_strip() {
    let patch = Patch::parse(GIT_PATCH).unwrap();
    let paths = patch.paths(1).unwrap();
    assert_eq!(
        paths,
        [
            "new_name.txt",
            "notes.txt",
            "obsolete.txt",
            "old_name.txt",
            "run.sh",
            "src/lib.rs"
        ]
    );
}

#[test]
fn keeps_git_prefixes_without_strip() {
    let patch = Patch::parse("--- a/notes.txt\n+++ b/notes.txt\n@@ -1 +1 @@\n-1\n+2\n").unwrap();
    let error = plan(&patch, &ApplyOptions::default(), |path| {
        (path == "notes.txt").then(|| "1\n".to_owned())
    })
    .unwrap_err();
    assert_eq!(
        error,
        PatchError::NotFound {
            path: "a/notes.txt".to_owned()
        }
    );
}

#[test]
fn strips_two_components() {
    let patch =
        Patch::parse("--- x/a/notes.txt\n+++ y/b/notes.txt\n@@ -1 +1 @@\n-1\n+2\n").unwrap();
    let options = ApplyOptions {
        strip: 2,
        ..ApplyOptions::default()
    };
    let changes = plan(&patch, &options, |path| {
        (path == "notes.txt").then(|| "1\n".to_owned())
    })
    .unwrap();
    assert!(
        matches!(&changes[0].change, FileChange::Modify { path, content, .. } if path == "notes.txt" && content == "2\n")
    );
}

#[test]
fn rejects_strip_beyond_path_depth() {
    let patch = Patch::parse("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-1\n+2\n").unwrap();
    let options = ApplyOptions {
        strip: 3,
        ..ApplyOptions::default()
    };
    let error = plan(&patch, &options, |_| None).unwrap_err();
    assert_eq!(
        error,
        PatchError::Strip {
            path: "a/x".to_owned(),
            strip: 3
        }
    );
    assert_eq!(error.code(), ErrorCode::Format);
}

#[test]
fn rejects_paths_leaving_root() {
    let patch = Patch::parse("--- a/../x\n+++ b/../x\n@@ -1 +1 @@\n-1\n+2\n").unwrap();
    let error = plan(&patch, &strip_one(), |_| Some("1\n".to_owned())).unwrap_err();
    assert_eq!(
        error,
        PatchError::UnsafePath {
            path: "../x".to_owned()
        }
    );
}

#[test]
fn applies_pure_rename_and_empty_new_file() {
    let patch = Patch::parse(
        "diff --git a/a.txt b/b.txt\nsimilarity index 100%\nrename from a.txt\nrename to b.txt\n\
         diff --git a/empty b/empty\nnew file mode 100644\nindex 0000000..e69de29\n",
    )
    .unwrap();
    let changes = plan(&patch, &strip_one(), |path| {
        (path == "a.txt").then(|| "x\n".to_owned())
    })
    .unwrap();
    let changes: Vec<FileChange> = changes.into_iter().map(|planned| planned.change).collect();
    assert_eq!(
        changes,
        [
            FileChange::Rename {
                from: "a.txt".to_owned(),
                to: "b.txt".to_owned(),
                content: "x\n".to_owned(),
                mode: None,
            },
            FileChange::Create {
                path: "empty".to_owned(),
                content: String::new(),
                mode: Some(0o100_644)
            },
        ]
    );
}

#[test]
fn rejects_rename_onto_existing_file() {
    let patch =
        Patch::parse("diff --git a/a b/b\nsimilarity index 100%\nrename from a\nrename to b\n")
            .unwrap();
    let error = plan(&patch, &strip_one(), |_| Some("x\n".to_owned())).unwrap_err();
    assert_eq!(
        error,
        PatchError::Exists {
            path: "b".to_owned()
        }
    );
}

#[test]
fn applies_copy_and_reverts_it() {
    let patch = Patch::parse(
        "diff --git a/a b/c\nsimilarity index 50%\ncopy from a\ncopy to c\nindex 1..2 100644\n\
         --- a/a\n+++ b/c\n@@ -1,2 +1,2 @@\n x\n-y\n+z\n",
    )
    .unwrap();
    let source = |path: &str| (path == "a").then(|| "x\ny\n".to_owned());
    let changes = plan(&patch, &strip_one(), source).unwrap();
    assert_eq!(
        changes[0].change,
        FileChange::Create {
            path: "c".to_owned(),
            content: "x\nz\n".to_owned(),
            mode: None
        }
    );

    let both = |path: &str| match path {
        "a" => Some("x\ny\n".to_owned()),
        "c" => Some("x\nz\n".to_owned()),
        _ => None,
    };
    let options = ApplyOptions {
        reverse: true,
        ..strip_one()
    };
    let changes = plan(&patch, &options, both).unwrap();
    assert_eq!(
        changes[0].change,
        FileChange::Delete {
            path: "c".to_owned()
        }
    );
}

#[test]
fn rejects_binary_git_patch_with_path() {
    let error = Patch::parse(
        "diff --git a/logo.png b/logo.png\nindex 1..2 100644\nBinary files a/logo.png and b/logo.png differ\n",
    )
    .unwrap_err();
    assert_eq!(
        error,
        PatchError::Binary {
            path: "b/logo.png".to_owned()
        }
    );
    assert!(
        error
            .to_string()
            .starts_with("binary patch for b/logo.png is not supported")
    );
}

#[test]
fn rejects_mismatched_hunk_without_partial_plan() {
    let patch = Patch::parse(GIT_PATCH).unwrap();
    let mut before = tree_before();
    before.insert(
        "notes.txt".to_owned(),
        "alpha\nchanged\ngamma\ndelta\n".to_owned(),
    );
    let error = plan(&patch, &strip_one(), |path| before.get(path).cloned()).unwrap_err();
    let PatchError::Rejected { path, rejection } = &error else {
        panic!("expected a rejected hunk, got {error:?}");
    };
    assert_eq!(path, "notes.txt");
    assert_eq!(rejection.hunk, 1);
    assert_eq!(rejection.line, 1);
    assert_eq!(rejection.expected, ["alpha", "beta", "gamma", "delta"]);
    assert_eq!(rejection.actual, ["alpha", "changed", "gamma", "delta"]);
    assert_eq!(error.code(), ErrorCode::PatchRejected);
}

#[test]
fn applies_with_fuzz_after_context_changed() {
    let patch = Patch::parse("--- a\n+++ a\n@@ -2,5 +2,5 @@\n b\n c\n-d\n+D\n e\n f\n").unwrap();
    let file = "a\nB\nc\nd\ne\nF\ng\n";
    let read = |_: &str| Some(file.to_owned());
    assert_eq!(
        plan(&patch, &ApplyOptions::default(), read)
            .unwrap_err()
            .code(),
        ErrorCode::PatchRejected
    );
    let options = ApplyOptions {
        fuzz: 1,
        ..ApplyOptions::default()
    };
    let changes = plan(&patch, &options, read).unwrap();
    assert!(
        matches!(&changes[0].change, FileChange::Modify { content, .. } if content == "a\nB\nc\nD\ne\nF\ng\n")
    );
    assert_eq!(changes[0].hunks[0].fuzz, 1);
}

#[test]
fn applies_lf_patch_to_crlf_file() {
    let patch =
        Patch::parse("--- a\n+++ a\n@@ -1,3 +1,4 @@\n one\n-two\n+2\n+2.5\n three\n").unwrap();
    let changes = plan(&patch, &ApplyOptions::default(), |_| {
        Some("one\r\ntwo\r\nthree\r\n".to_owned())
    })
    .unwrap();
    assert!(
        matches!(&changes[0].change, FileChange::Modify { content, .. } if content == "one\r\n2\r\n2.5\r\nthree\r\n")
    );
}

#[test]
fn applies_crlf_patch_to_lf_file() {
    let patch =
        Patch::parse("--- a\r\n+++ a\r\n@@ -1,2 +1,2 @@\r\n one\r\n-two\r\n+2\r\n").unwrap();
    let changes = plan(&patch, &ApplyOptions::default(), |_| {
        Some("one\ntwo\n".to_owned())
    })
    .unwrap();
    assert!(
        matches!(&changes[0].change, FileChange::Modify { content, .. } if content == "one\n2\n")
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
