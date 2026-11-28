// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-text/tests/diff_unified.rs
// Responsibility:   Unified rendering compared with output of GNU `diff -u` 3.12.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Unified rendering compared with output of GNU `diff -u` 3.12.
//!
//! Each expected text was produced by `LC_ALL=C diff -U<context> --label a
//! --label b old new` for the same input.

use nurt_text::diff::{Unified, unified_diff};

fn render(old: &str, new: &str, context: usize, ignore_whitespace: bool) -> String {
    Unified::new(
        "a",
        "b",
        &unified_diff(old, new, context, ignore_whitespace),
    )
    .to_string()
}

const PROGRAM_OLD: &str = "fn main() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n    println(a);\n    println(b);\n    println(c);\n    let d = 4;\n    let e = 5;\n    let f = 6;\n    let g = 7;\n    let h = 8;\n    let i = 9;\n    done();\n}\n";

const PROGRAM_NEW: &str = "fn main() {\n    let a = 1;\n    let b = 20;\n    let c = 3;\n    println(a);\n    println(b);\n    println(c);\n    let d = 4;\n    let e = 5;\n    let f = 6;\n    let g = 7;\n    let h = 8;\n    let i = 9;\n    finish();\n    done();\n}\n";

#[test]
fn matches_gnu_for_two_hunks_of_a_program() {
    let expected = "--- a\n+++ b\n@@ -1,6 +1,6 @@\n fn main() {\n     let a = 1;\n-    let b = 2;\n+    let b = 20;\n     let c = 3;\n     println(a);\n     println(b);\n@@ -11,5 +11,6 @@\n     let g = 7;\n     let h = 8;\n     let i = 9;\n+    finish();\n     done();\n }\n";
    assert_eq!(render(PROGRAM_OLD, PROGRAM_NEW, 3, false), expected);
}

#[test]
fn matches_gnu_for_repeated_braces_and_blank_lines() {
    let old = "}\n\n}\n\n}\nx\n}\n\n";
    let new = "}\n\n}\ny\n}\n\n}\n\n";
    let expected = "--- a\n+++ b\n@@ -1,8 +1,8 @@\n }\n \n }\n-\n+y\n }\n-x\n+\n }\n \n";
    assert_eq!(render(old, new, 3, false), expected);
}

#[test]
fn matches_gnu_without_final_line_feed_in_both_texts() {
    let expected = "--- a\n+++ b\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n\\ No newline at end of file\n";
    assert_eq!(render("a\nb\nc", "a\nB\nc", 3, false), expected);
}

#[test]
fn matches_gnu_when_new_text_loses_final_line_feed() {
    let expected = "--- a\n+++ b\n@@ -2,2 +2,2 @@\n b\n-c\n+c\n\\ No newline at end of file\n";
    assert_eq!(render("a\nb\nc\n", "a\nb\nc", 1, false), expected);
}

#[test]
fn matches_gnu_when_ignoring_whitespace_of_crlf_lines() {
    let expected = "--- a\n+++ b\n@@ -1,2 +1,3 @@\n x y\r\n z\r\n+w\n";
    assert_eq!(render("x y\r\nz\r\n", "x  y\nz\nw\n", 3, true), expected);
}

#[test]
fn matches_gnu_for_removed_repetition() {
    let old = "a\nb\na\nb\na\nb\n";
    let new = "a\nb\na\nb\n";
    assert_eq!(
        render(old, new, 3, false),
        "--- a\n+++ b\n@@ -2,5 +2,3 @@\n b\n a\n b\n-a\n-b\n"
    );
    assert_eq!(
        render(old, new, 0, false),
        "--- a\n+++ b\n@@ -5,2 +4,0 @@\n-a\n-b\n"
    );
}

#[test]
fn matches_gnu_for_created_and_emptied_text() {
    assert_eq!(
        render("", "x\ny\n", 3, false),
        "--- a\n+++ b\n@@ -0,0 +1,2 @@\n+x\n+y\n"
    );
    assert_eq!(
        render("x\ny\n", "", 3, false),
        "--- a\n+++ b\n@@ -1,2 +0,0 @@\n-x\n-y\n"
    );
}

#[test]
fn matches_gnu_for_reversed_lines() {
    let expected = "--- a\n+++ b\n@@ -1,3 +1,3 @@\n-a\n-b\n c\n+b\n+a\n";
    assert_eq!(render("a\nb\nc\n", "c\nb\na\n", 3, false), expected);
}

#[test]
fn matches_gnu_for_swapped_blocks_between_equal_lines() {
    let expected = "--- a\n+++ b\n@@ -1,5 +1,5 @@\n x\n-a\n-x\n b\n x\n+a\n+x\n";
    assert_eq!(
        render("x\na\nx\nb\nx\n", "x\nb\nx\na\nx\n", 3, false),
        expected
    );
}

#[test]
fn matches_gnu_for_three_hunks_with_two_context_lines() {
    let old: String = (1..=20).map(|n| format!("{n}\n")).collect();
    let new = old
        .replace("\n5\n", "\nfive\n")
        .replace("\n12\n", "\n")
        .replace("\n19\n", "\n19\n19b\n");
    let expected = "--- a\n+++ b\n@@ -3,5 +3,5 @@\n 3\n 4\n-5\n+five\n 6\n 7\n@@ -10,5 +10,4 @@\n 10\n 11\n-12\n 13\n 14\n@@ -18,3 +17,4 @@\n 18\n 19\n+19b\n 20\n";
    assert_eq!(render(&old, &new, 2, false), expected);
}

#[test]
fn renders_nothing_for_equal_texts() {
    assert_eq!(render(PROGRAM_OLD, PROGRAM_OLD, 3, false), "");
}

#[test]
fn matches_gnu_when_identical_end_stops_a_slide() {
    let expected = "--- a\n+++ b\n@@ -1 +0,0 @@\n-a\n@@ -2,0 +2 @@\n+ a\n";
    assert_eq!(render("a\n\na\r", "\n a\na\r", 0, true), expected);
    let expected = "--- a\n+++ b\n@@ -1,3 +1,3 @@\n-a\n \n a\r\n\\ No newline at end of file\n+a\r\n\\ No newline at end of file\n";
    assert_eq!(render("a\n\na\r", "\n a\na\r", 3, true), expected);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
