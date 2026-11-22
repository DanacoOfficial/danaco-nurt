// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Programs that the correction produces for typical mistakes: the executed
//! forms, the ranked candidates and the ready corrections all parse
//! strictly.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

mod parse_a_support;

use parse_a_support::{check_invariants, json_a};

/// Programs produced by the correction, in the order of the cases.
const CORRECTED: [&str; 19] = [
    r#"remove "/srv/app/build/old.log""#,
    r#"find "/srv/app" glob: "*.go" case_sensitive: false |> count"#,
    r#"find "/srv/app" glob: "*.go" |> filter size > 10mb"#,
    r#"remove "/srv/app/build""#,
    r#"run ["go", "test", "./..."] cwd: "/srv/app""#,
    r#"search "oldName(" "/srv/app/src""#,
    r#"replace "old(" "new(" in: ["/srv/app/a.go"]"#,
    r#"replace /old\(/ "new(" in: ["/srv/app/a.go"]"#,
    r#"find "/srv/app" |> filter size > 10mib |> remove"#,
    r#"find "/srv/app" |> filter size > 10mb |> remove"#,
    r#"create "/srv/out" type: directory"#,
    r#"copy "/srv/app/dist" into: "/srv/out""#,
    r#"remove "/srv/build""#,
    r#"find "C:/proj" glob: "**/*.go" |> filter size > 1mb |> sort size order: desc |> first 5"#,
    r#"list "C:/proj" |> rename /$/ ".bak""#,
    r#"find "/srv/app" type: directory |> filter modified > ago(1h) |> sort size order: asc"#,
    r#"search /x/ "/a" |> replace "y" |> verify (run ["go","build"] cwd: "/a")"#,
    r#"replace "x" "newName(" in: ["/a/b.go"]"#,
    r#"replace "x" "newName( in: [\"/a/b.go\"]""#,
];

#[test]
fn corrected_programs_parse_strictly() {
    for text in CORRECTED {
        let json = json_a(text);
        check_invariants(&json).unwrap_or_else(|problem| panic!("{text}: {problem}"));
    }
}

#[test]
fn closed_verify_holds_a_sub_pipeline() {
    let json = json_a(CORRECTED[16]);
    let verify = &json["program"]["statements"][0]["stages"][1];
    assert_eq!(verify["name"], "verify");
    assert_eq!(verify["args"][0]["value"]["t"], "sub");
    assert_eq!(verify["args"][0]["value"]["p"]["head"]["name"], "run");
}

#[test]
fn quote_closed_at_the_end_of_the_line_keeps_one_text() {
    let json = json_a(CORRECTED[18]);
    let args = &json["program"]["statements"][0]["head"]["args"];
    assert_eq!(args.as_array().map(Vec::len), Some(2));
    assert_eq!(args[1]["value"]["v"], "newName( in: [\"/a/b.go\"]");
}
