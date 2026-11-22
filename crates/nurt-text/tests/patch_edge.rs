// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Edge cases of patch parsing, path resolution and planning: renames,
//! copies, deletions, mode changes, reverse application and error reporting.

use nurt_contracts::error::ErrorCode;
use nurt_text::diff::unified_diff;
use nurt_text::patch::{
    ApplyOptions, FileChange, FileOperation, Patch, PatchError, Rejection, apply_hunks, plan,
};

const RENAME_PATCH: &str = "diff --git a/old.txt b/new.txt\n\
    similarity index 90%\n\
    rename from old.txt\n\
    rename to new.txt\n\
    --- a/old.txt\n\
    +++ b/new.txt\n\
    @@ -1 +1 @@\n\
    -a\n\
    +b\n";

const MODE_BITS: u32 = 0o100_755;

fn strip_one() -> ApplyOptions {
    ApplyOptions {
        strip: 1,
        ..ApplyOptions::default()
    }
}

fn malformed(text: &str) -> PatchError {
    Patch::parse(text).err().unwrap_or(PatchError::NoChanges)
}

#[test]
fn plans_rename_with_content_change() {
    let patch = Patch::parse(RENAME_PATCH).unwrap();
    let changes = plan(&patch, &strip_one(), |path| {
        (path == "old.txt").then(|| "a\n".to_owned())
    })
    .unwrap();
    assert_eq!(
        changes[0].change,
        FileChange::Rename {
            from: "old.txt".to_owned(),
            to: "new.txt".to_owned(),
            content: "b\n".to_owned(),
            mode: None,
        }
    );
}

#[test]
fn plans_rename_in_reverse() {
    let patch = Patch::parse(RENAME_PATCH).unwrap();
    let options = ApplyOptions {
        reverse: true,
        ..strip_one()
    };
    let changes = plan(&patch, &options, |path| {
        (path == "new.txt").then(|| "b\n".to_owned())
    })
    .unwrap();
    assert_eq!(
        changes[0].change,
        FileChange::Rename {
            from: "new.txt".to_owned(),
            to: "old.txt".to_owned(),
            content: "a\n".to_owned(),
            mode: None,
        }
    );
}

#[test]
fn rejects_rename_of_missing_source() {
    let patch = Patch::parse(RENAME_PATCH).unwrap();
    let error = plan(&patch, &strip_one(), |_| None).unwrap_err();
    assert!(matches!(error, PatchError::NotFound { ref path } if path == "old.txt"));
}

#[test]
fn accepts_legacy_rename_header_names() {
    let patch = Patch::parse("diff --git a/x b/y\nrename old x\nrename new y\n").unwrap();
    assert_eq!(patch.files[0].operation, FileOperation::Rename);
    assert_eq!(patch.files[0].source.as_deref(), Some("x"));
    assert_eq!(patch.files[0].target.as_deref(), Some("y"));
}

const COPY_PATCH: &str = "diff --git a/o.txt b/c.txt\ncopy from o.txt\ncopy to c.txt\n";

#[test]
fn plans_copy_as_creation() {
    let patch = Patch::parse(COPY_PATCH).unwrap();
    assert_eq!(patch.files[0].operation, FileOperation::Copy);
    let changes = plan(&patch, &strip_one(), |path| {
        (path == "o.txt").then(|| "same\n".to_owned())
    })
    .unwrap();
    assert_eq!(
        changes[0].change,
        FileChange::Create {
            path: "c.txt".to_owned(),
            content: "same\n".to_owned(),
            mode: None,
        }
    );
}

#[test]
fn rejects_copy_onto_existing_file() {
    let patch = Patch::parse(COPY_PATCH).unwrap();
    let error = plan(&patch, &strip_one(), |_| Some("x\n".to_owned())).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Exists);
}

#[test]
fn reverses_copy_into_deletion_of_the_copy() {
    let patch = Patch::parse(COPY_PATCH).unwrap();
    let options = ApplyOptions {
        reverse: true,
        ..strip_one()
    };
    let changes = plan(&patch, &options, |_| Some("same\n".to_owned())).unwrap();
    assert_eq!(
        changes[0].change,
        FileChange::Delete {
            path: "c.txt".to_owned()
        }
    );
}

#[test]
fn rejects_reverse_copy_when_copy_differs_from_source() {
    let patch = Patch::parse(COPY_PATCH).unwrap();
    let options = ApplyOptions {
        reverse: true,
        ..strip_one()
    };
    let error = plan(&patch, &options, |path| {
        Some(
            if path == "c.txt" {
                "edited\n"
            } else {
                "same\n"
            }
            .to_owned(),
        )
    })
    .unwrap_err();
    assert!(matches!(error, PatchError::Leftover { ref path } if path == "c.txt"));
    assert_eq!(error.code(), ErrorCode::PatchRejected);
}

const DELETE_PATCH: &str = "diff --git a/x b/x\n\
    deleted file mode 100644\n\
    --- a/x\n\
    +++ /dev/null\n\
    @@ -1 +0,0 @@\n\
    -x\n";

#[test]
fn plans_deletion_of_matching_file() {
    let patch = Patch::parse(DELETE_PATCH).unwrap();
    assert_eq!(patch.files[0].operation, FileOperation::Delete);
    let changes = plan(&patch, &strip_one(), |_| Some("x\n".to_owned())).unwrap();
    assert_eq!(
        changes[0].change,
        FileChange::Delete {
            path: "x".to_owned()
        }
    );
}

#[test]
fn rejects_deletion_that_leaves_content() {
    let patch = Patch::parse(DELETE_PATCH).unwrap();
    let error = plan(&patch, &strip_one(), |_| Some("x\nz\n".to_owned())).unwrap_err();
    assert!(matches!(error, PatchError::Leftover { ref path } if path == "x"));
}

#[test]
fn reverses_deletion_into_creation() {
    let patch = Patch::parse(DELETE_PATCH).unwrap();
    let options = ApplyOptions {
        reverse: true,
        ..strip_one()
    };
    let changes = plan(&patch, &options, |_| None).unwrap();
    assert_eq!(
        changes[0].change,
        FileChange::Create {
            path: "x".to_owned(),
            content: "x\n".to_owned(),
            mode: Some(0o100_644),
        }
    );
}

#[test]
fn plans_mode_change_without_hunks() {
    let patch = Patch::parse("diff --git a/s b/s\nold mode 100644\nnew mode 100755\n").unwrap();
    let changes = plan(&patch, &strip_one(), |_| Some("body\n".to_owned())).unwrap();
    assert_eq!(
        changes[0].change,
        FileChange::Modify {
            path: "s".to_owned(),
            content: "body\n".to_owned(),
            mode: Some(MODE_BITS),
        }
    );
}

#[test]
fn skips_section_that_changes_nothing() {
    let patch = Patch::parse("diff --git a/s b/s\nindex 1..2 100644\n").unwrap();
    let changes = plan(&patch, &strip_one(), |_| Some("body\n".to_owned())).unwrap();
    assert!(changes.is_empty());
}

#[test]
fn rejects_git_section_without_file_names() {
    let patch = Patch::parse("diff --git x y\nnew file mode 100644\n").unwrap();
    let error = plan(&patch, &ApplyOptions::default(), |_| None).unwrap_err();
    assert_eq!(error, PatchError::MissingPath { line: 1 });
    assert_eq!(error.code(), ErrorCode::Format);
}

#[test]
fn reports_missing_file_for_unified_modification() {
    let patch = Patch::parse("--- a\n+++ b\n@@ -1 +1 @@\n-1\n+2\n").unwrap();
    let error = plan(&patch, &ApplyOptions::default(), |_| None).unwrap_err();
    assert!(matches!(error, PatchError::NotFound { ref path } if path == "a"));
}

#[test]
fn modifies_new_path_when_only_it_exists() {
    let patch = Patch::parse("--- a\n+++ b\n@@ -1 +1 @@\n-1\n+2\n").unwrap();
    let changes = plan(&patch, &ApplyOptions::default(), |path| {
        (path == "b").then(|| "1\n".to_owned())
    })
    .unwrap();
    assert!(matches!(&changes[0].change, FileChange::Modify { path, .. } if path == "b"));
}

#[test]
fn lists_sorted_paths_after_strip() {
    let patch = Patch::parse(RENAME_PATCH).unwrap();
    assert_eq!(patch.paths(1).unwrap(), ["new.txt", "old.txt"]);
}

#[test]
fn rejects_strip_beyond_rename_source_depth() {
    let patch = Patch::parse(RENAME_PATCH).unwrap();
    let error = patch.paths(2).unwrap_err();
    assert!(matches!(error, PatchError::Strip { strip: 2, .. }));
    assert_eq!(error.code(), ErrorCode::Format);
}

#[test]
fn reports_both_null_file_names() {
    let error = malformed("--- /dev/null\n+++ /dev/null\n@@ -0,0 +1 @@\n+a\n");
    assert!(matches!(error, PatchError::Malformed { .. }));
}

#[test]
fn reports_unterminated_quoted_name() {
    let error = malformed("--- \"abc\n+++ b\n@@ -1 +1 @@\n-a\n+b\n");
    assert_eq!(
        error,
        PatchError::Malformed {
            line: 1,
            reason: "invalid quoted file name"
        }
    );
}

#[test]
fn reports_invalid_file_mode() {
    let error = malformed("diff --git a/x b/x\nnew mode zz\n");
    assert!(matches!(
        error,
        PatchError::Malformed {
            reason: "invalid file mode",
            ..
        }
    ));
}

#[test]
fn reports_invalid_hunk_header() {
    let error = malformed("--- a\n+++ a\n@@ -x +1 @@\n-a\n");
    assert!(matches!(
        error,
        PatchError::Malformed {
            line: 3,
            reason: "invalid hunk header"
        }
    ));
}

#[test]
fn reports_hunk_longer_than_its_header() {
    let old = malformed("--- a\n+++ a\n@@ -1 +1 @@\n-a\n-b\n+c\n");
    assert!(matches!(
        old,
        PatchError::Malformed {
            reason: "hunk has more old lines than its header states",
            ..
        }
    ));
    let new = malformed("--- a\n+++ a\n@@ -1 +1 @@\n+a\n+b\n-c\n");
    assert!(matches!(
        new,
        PatchError::Malformed {
            reason: "hunk has more new lines than its header states",
            ..
        }
    ));
}

#[test]
fn reports_marker_without_preceding_line() {
    let error = malformed("--- a\n+++ a\n@@ -1 +1 @@\n\\ No newline at end of file\n-a\n+b\n");
    assert!(matches!(
        error,
        PatchError::Malformed {
            reason: "marker line without a preceding line",
            ..
        }
    ));
}

#[test]
fn reports_binary_marker_inside_git_section() {
    let error = malformed("diff --git a/x b/x\nBinary files a/x and b/x differ\n");
    assert_eq!(
        error,
        PatchError::Binary {
            path: "b/x".to_owned()
        }
    );
}

#[test]
fn parses_crlf_patch_headers() {
    let patch = Patch::parse("--- a\r\n+++ a\r\n@@ -1 +1 @@\r\n-x\r\n+y\r\n").unwrap();
    assert_eq!(patch.files[0].old_path.as_deref(), Some("a"));
    assert_eq!(patch.files[0].hunks.len(), 1);
}

#[test]
fn applies_hunk_anchored_at_end_of_file() {
    let hunks = unified_diff("a\nb\nc\nd\n", "a\nb\nc\nD\n", 3, false);
    assert_eq!(
        apply_hunks("z\na\nb\nc\nd\n", &hunks, 0).unwrap().text,
        "z\na\nb\nc\nD\n"
    );
    assert!(apply_hunks("a\nb\nc\nd\ne\n", &hunks, 0).is_err());
}

#[test]
fn applies_hunk_marked_without_final_line_feed() {
    let patch =
        Patch::parse("--- a\n+++ a\n@@ -1 +1 @@\n-x\n\\ No newline at end of file\n+y\n\\ No newline at end of file\n")
            .unwrap();
    let applied = apply_hunks("x", &patch.files[0].hunks, 0).unwrap();
    assert_eq!(applied.text, "y");
}

#[test]
fn ignores_final_carriage_returns_when_no_exact_match_exists() {
    let hunks = unified_diff("a\nb\n", "a\nB\n", 3, false);
    let applied = apply_hunks("a\r\nb\r\n", &hunks, 0).unwrap();
    assert_eq!(applied.text, "a\r\nB\r\n");
}

#[test]
fn describes_rejection_and_every_error_kind() {
    let rejection = Rejection {
        hunk: 2,
        line: 7,
        expected: vec!["a".to_owned()],
        actual: vec!["b".to_owned()],
    };
    assert_eq!(rejection.to_string(), "hunk 2 does not apply at line 7");
    let path = || "p".to_owned();
    let errors = [
        (PatchError::NoChanges, ErrorCode::Format),
        (PatchError::Binary { path: path() }, ErrorCode::Format),
        (PatchError::MissingPath { line: 3 }, ErrorCode::Format),
        (
            PatchError::Strip {
                path: path(),
                strip: 2,
            },
            ErrorCode::Format,
        ),
        (PatchError::UnsafePath { path: path() }, ErrorCode::Format),
        (PatchError::NotFound { path: path() }, ErrorCode::NotFound),
        (PatchError::Exists { path: path() }, ErrorCode::Exists),
        (
            PatchError::Rejected {
                path: path(),
                rejection,
            },
            ErrorCode::PatchRejected,
        ),
        (
            PatchError::Leftover { path: path() },
            ErrorCode::PatchRejected,
        ),
        (
            PatchError::Malformed {
                line: 1,
                reason: "bad",
            },
            ErrorCode::Format,
        ),
    ];
    for (error, code) in errors {
        let text = error.to_string();
        assert_eq!(error.code(), code, "{text}");
        assert!(text.starts_with(|c: char| c.is_lowercase()), "{text}");
        assert!(!text.ends_with('.'), "{text}");
    }
}
