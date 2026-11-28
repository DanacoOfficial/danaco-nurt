// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::coverage::tests
// Responsibility:   Tests of the coverage report readers.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests of the coverage report readers.

use std::path::Path;

use nurt_contracts::record::CoverageFormat;

use super::{Tally, detect_format, parse, percent};
use crate::stage::test::support::{corpus, write};

#[test]
fn lcov_counts_lines_and_functions_per_file() {
    let coverage = parse(
        &corpus("coverage/sample.lcov"),
        CoverageFormat::Lcov,
        Path::new("/p"),
    )
    .unwrap();
    assert!((coverage.lines_pct - 50.0).abs() < f64::EPSILON);
    assert_eq!(coverage.functions_pct, Some(50.0));
    let files = coverage.files.unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path.as_str(), "sum.ts");
    let uncovered = files[0].uncovered.as_ref().unwrap();
    assert_eq!(uncovered.len(), 1);
}

#[test]
fn lcov_falls_back_to_totals_without_line_records() {
    let text = "SF:/p/a.ts\nLF:10\nLH:7\nFNF:2\nFNH:1\nBRF:4\nBRH:3\nend_of_record\n";
    let coverage = parse(text, CoverageFormat::Lcov, Path::new("/p")).unwrap();
    assert!((coverage.lines_pct - 70.0).abs() < f64::EPSILON);
    assert_eq!(coverage.functions_pct, Some(50.0));
    assert_eq!(coverage.branches_pct, Some(75.0));
    assert_eq!(coverage.files.unwrap()[0].path.as_str(), "a.ts");
}

#[test]
fn lcov_rejects_text_without_records() {
    assert!(parse("nothing here\n", CoverageFormat::Lcov, Path::new("/p")).is_err());
}

#[test]
fn lcov_merges_records_of_the_same_file_taking_the_larger_count() {
    let text = "SF:a.ts\nDA:1,0\nend_of_record\nSF:a.ts\nDA:1,3\nDA:2,0\nend_of_record\n";
    let coverage = parse(text, CoverageFormat::Lcov, Path::new("/p")).unwrap();
    assert!((coverage.lines_pct - 50.0).abs() < f64::EPSILON);
    assert_eq!(coverage.files.unwrap().len(), 1);
}

#[test]
fn cobertura_reads_classes_relative_to_the_source_directory() {
    let coverage = parse(
        &corpus("coverage/sample.cobertura.xml"),
        CoverageFormat::Cobertura,
        Path::new("/project"),
    )
    .unwrap();
    assert!((coverage.lines_pct - 66.67).abs() < f64::EPSILON);
    let files = coverage.files.unwrap();
    assert_eq!(files[0].path.as_str(), "module.py");
    assert!((files[0].lines_pct - 50.0).abs() < f64::EPSILON);
    assert_eq!(files[1].uncovered, None);
}

#[test]
fn cobertura_reads_branch_conditions_and_methods() {
    let text = r#"<coverage><sources><source>/p/src</source></sources><packages><package><classes>
<class filename="a.py"><methods><method name="f"><lines><line number="1" hits="1"/></lines></method><method name="g"><lines><line number="3" hits="0"/></lines></method></methods>
<lines><line number="1" hits="1" branch="true" condition-coverage="50% (1/2)"/><line number="3" hits="0"/></lines></class>
</classes></package></packages></coverage>"#;
    let coverage = parse(text, CoverageFormat::Cobertura, Path::new("/p")).unwrap();
    assert_eq!(coverage.branches_pct, Some(50.0));
    assert_eq!(coverage.functions_pct, Some(50.0));
    assert_eq!(coverage.files.unwrap()[0].path.as_str(), "src/a.py");
}

#[test]
fn cobertura_rejects_broken_xml() {
    assert!(
        parse(
            "<coverage><packages>",
            CoverageFormat::Cobertura,
            Path::new("/p")
        )
        .is_err()
    );
}

#[test]
fn go_profile_counts_statements_once_per_block() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "go.mod", "module example.com/covgo\n");
    let text = format!(
        "{}example.com/covgo/sum.go:3.24,5.2 1 3\n",
        corpus("coverage/sample.go.txt")
    );
    let coverage = parse(&text, CoverageFormat::Go, dir.path()).unwrap();
    assert!((coverage.lines_pct - 25.0).abs() < f64::EPSILON);
    let files = coverage.files.unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path.as_str(), "sum.go");
}

#[test]
fn go_profile_without_mode_line_is_rejected() {
    assert!(parse("a.go:1.1,2.2 1 1\n", CoverageFormat::Go, Path::new("/p")).is_err());
}

#[test]
fn istanbul_detail_report_derives_statements_functions_and_branches() {
    let coverage = parse(
        &corpus("coverage/sample.istanbul.json"),
        CoverageFormat::Istanbul,
        Path::new("/work"),
    )
    .unwrap();
    assert_eq!(coverage.statements_pct, Some(66.67));
    assert_eq!(coverage.functions_pct, Some(100.0));
    assert_eq!(coverage.branches_pct, Some(50.0));
    assert_eq!(coverage.files.unwrap()[0].path.as_str(), "src/a.js");
}

#[test]
fn istanbul_summary_report_uses_the_stated_totals() {
    let coverage = parse(
        &corpus("coverage/sample.istanbul-summary.json"),
        CoverageFormat::Istanbul,
        Path::new("/work"),
    )
    .unwrap();
    assert!((coverage.lines_pct - 80.0).abs() < f64::EPSILON);
    assert_eq!(coverage.functions_pct, Some(50.0));
}

#[test]
fn istanbul_rejects_text_that_is_not_an_object() {
    assert!(parse("[1]", CoverageFormat::Istanbul, Path::new("/p")).is_err());
    assert!(parse("{", CoverageFormat::Istanbul, Path::new("/p")).is_err());
}

#[test]
fn llvm_export_reads_summary_and_uncovered_segments() {
    let coverage = parse(
        &corpus("coverage/sample.llvm.json"),
        CoverageFormat::Llvm,
        Path::new("/work"),
    )
    .unwrap();
    assert!((coverage.lines_pct - 90.0).abs() < f64::EPSILON);
    assert_eq!(coverage.functions_pct, Some(100.0));
    assert_eq!(coverage.files.unwrap()[0].path.as_str(), "src/lib.rs");
}

#[test]
fn go_profile_block_with_an_unbounded_end_line_is_cut() {
    let text = "mode: set\na.go:1.1,18446744073709551615.1 1 1\n";
    let coverage = parse(text, CoverageFormat::Go, Path::new("/p")).unwrap();
    assert!((coverage.lines_pct - 100.0).abs() < f64::EPSILON);
    assert_eq!(coverage.files.unwrap().len(), 1);
}

#[test]
fn llvm_segment_followed_by_an_unbounded_line_is_cut() {
    let text = r#"{"data":[{"files":[{"filename":"/work/src/lib.rs",
"segments":[[1,1,5,true,true,false],[18446744073709551615,2,0,true,true,false]],
"summary":{"lines":{"count":2,"covered":1},"functions":{"count":1,"covered":1}}}]}]}"#;
    let coverage = parse(text, CoverageFormat::Llvm, Path::new("/work")).unwrap();
    assert!((coverage.lines_pct - 50.0).abs() < f64::EPSILON);
    assert_eq!(coverage.files.unwrap()[0].path.as_str(), "src/lib.rs");
}

#[test]
fn totals_that_overflow_are_saturated_instead_of_panicking() {
    let text = "SF:a.ts\nLF:18446744073709551615\nLH:18446744073709551615\nend_of_record\n\
                SF:b.ts\nLF:5\nLH:1\nend_of_record\n";
    let coverage = parse(text, CoverageFormat::Lcov, Path::new("/p")).unwrap();
    assert!((coverage.lines_pct - 100.0).abs() < f64::EPSILON);
    assert_eq!(coverage.files.unwrap().len(), 2);
}

#[test]
fn llvm_rejects_other_json() {
    assert!(parse("{\"a\":1}", CoverageFormat::Llvm, Path::new("/p")).is_err());
}

#[test]
fn format_is_detected_from_content() {
    assert_eq!(detect_format("mode: set\n"), CoverageFormat::Go);
    assert_eq!(
        detect_format("<?xml version=\"1.0\"?>"),
        CoverageFormat::Cobertura
    );
    assert_eq!(
        detect_format("{\"type\":\"llvm.coverage.json.export\"}"),
        CoverageFormat::Llvm
    );
    assert_eq!(detect_format("{\"/a.js\":{}}"), CoverageFormat::Istanbul);
    assert_eq!(detect_format("SF:a\n"), CoverageFormat::Lcov);
}

#[test]
fn percent_of_nothing_is_complete() {
    assert!((percent(Tally { found: 0, hit: 0 }) - 100.0).abs() < f64::EPSILON);
    assert!((percent(Tally { found: 3, hit: 1 }) - 33.33).abs() < f64::EPSILON);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
