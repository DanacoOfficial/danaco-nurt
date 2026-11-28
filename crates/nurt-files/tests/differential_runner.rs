// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/differential_runner.rs
// Responsibility:   Tests of the differential test runner.
// Dependencies:     support_diff
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests of the differential test runner.

// Test code unwraps results so that a failure aborts the test with the original error.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support_diff;

use std::path::Path;

use support_diff::{
    DiffError, SetDiff, assert_same, compare, normalize_nul_paths, normalize_path, normalize_paths,
    normalize_text,
};

#[test]
fn normalizes_separators_and_prefixes_of_paths() {
    let root = Path::new("/tmp/corpus");
    assert_eq!(normalize_path("/tmp/corpus/a/b.txt", root), "a/b.txt");
    assert_eq!(normalize_path("./a/b.txt", root), "a/b.txt");
    assert_eq!(
        normalize_path("a\\b.txt", root),
        if cfg!(windows) { "a/b.txt" } else { "a\\b.txt" },
        "a backslash in a Unix name is part of the name"
    );
    assert_eq!(
        normalize_path("C:\\w\\corpus\\a.txt", Path::new("C:\\w\\corpus")),
        "a.txt"
    );
}

#[test]
fn composes_decomposed_names() {
    let decomposed = "z\u{030C}.txt";
    assert_eq!(normalize_path(decomposed, Path::new("/r")), "\u{017E}.txt");
}

#[test]
fn sorts_and_drops_empty_lines() {
    let root = Path::new("/r");
    let paths = normalize_paths(b"./b\n\n/r/a\r\nc\n", root).unwrap();
    assert_eq!(paths, ["a", "b", "c"]);
}

#[test]
fn splits_nul_terminated_output() {
    let paths = normalize_nul_paths(b"./b\0./a\0", Path::new("/r")).unwrap();
    assert_eq!(paths, ["a", "b"]);
}

#[test]
fn rejects_output_that_is_not_utf8() {
    assert!(matches!(
        normalize_text(&[0x66, 0xFF]),
        Err(DiffError::NotUtf8)
    ));
}

#[test]
fn keeps_line_ends_of_the_output() {
    assert_eq!(normalize_text(b"a\r\nb\n").unwrap(), "a\r\nb\n");
}

#[test]
fn compare_reports_both_directions_and_duplicates() {
    let nurt = ["a".to_string(), "a".to_string(), "b".to_string()];
    let reference = ["a".to_string(), "c".to_string()];
    assert_eq!(
        compare(&nurt, &reference),
        SetDiff {
            only_nurt: vec!["a".into(), "b".into()],
            only_reference: vec!["c".into()],
        }
    );
    assert!(compare(&nurt, &nurt).is_empty());
}

#[test]
#[should_panic(expected = "results differ")]
fn assert_same_fails_on_difference() {
    assert_same("pair", &["a".to_string()], &["b".to_string()]);
}

#[cfg(all(feature = "differential", unix))]
mod programs {
    use std::path::Path;
    use std::time::Duration;

    use super::{DiffError, assert_same, normalize_paths};
    use crate::support_diff::{locate, require, require_with, run};

    const LIMIT: Duration = Duration::from_secs(10);

    #[test]
    fn locates_a_program_present_on_every_system() {
        assert!(locate("sh").is_some());
        assert!(locate("nurt-no-such-program").is_none());
    }

    #[test]
    fn skips_when_the_program_is_missing() {
        assert!(require_with("nurt-no-such-program", false).is_none());
    }

    #[test]
    #[should_panic(expected = "not found in PATH")]
    fn fails_when_a_missing_program_is_required() {
        let _ = require_with("nurt-no-such-program", true);
    }

    #[test]
    fn a_present_program_is_found_in_either_mode() {
        assert!(require_with("sh", true).is_some());
        assert!(require_with("sh", false).is_some());
    }

    #[test]
    fn captures_output_and_exit_code() {
        let Some(sh) = require("sh") else { return };
        let script = "printf 'b\\na\\n'; printf err >&2; exit 3";
        let out = run(&sh, &["-c", script], Path::new("."), LIMIT).unwrap();
        assert_eq!(out.code, Some(3));
        assert_eq!(out.stdout, b"b\na\n");
        assert_eq!(out.stderr, b"err");
    }

    #[test]
    fn kills_a_program_that_exceeds_the_limit() {
        let Some(sh) = require("sh") else { return };
        let limit = Duration::from_millis(200);
        let result = run(&sh, &["-c", "sleep 30"], Path::new("."), limit);
        assert!(matches!(result, Err(DiffError::Timeout(_))));
    }

    #[test]
    fn normalizes_the_output_of_sort_for_comparison() {
        let Some(sort) = require("sort") else { return };
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("in.txt"), "./b\n./a\n./c\n").unwrap();
        let out = run(&sort, &["in.txt"], dir.path(), LIMIT).unwrap();
        let paths = normalize_paths(&out.stdout, dir.path()).unwrap();
        assert_same("sort", &["c".into(), "a".into(), "b".into()], &paths);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
