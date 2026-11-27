// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::complexity::tests
// Responsibility:   Complexity findings on function metrics whose values were counted with the.
// Dependencies:     nurt_contracts, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Complexity findings on function metrics whose values were counted with the
//! reference tool for each language.

use nurt_contracts::services::{ByteRange, FunctionMetrics, TextPosition, TextSpan};
use serde::Deserialize;

use super::findings_for;

#[derive(Deserialize)]
struct Sample {
    path: String,
    threshold: u32,
    highest: u32,
    functions: Vec<Function>,
    findings: Vec<(String, u32)>,
}

#[derive(Deserialize)]
struct Function {
    name: String,
    container: Option<String>,
    start: u32,
    end: u32,
    cyclomatic: u32,
}

fn metrics(sample: &Sample) -> Vec<FunctionMetrics> {
    sample
        .functions
        .iter()
        .map(|function| FunctionMetrics {
            name: function.name.clone(),
            container: function.container.clone(),
            span: TextSpan {
                bytes: ByteRange::new(0, 0),
                start: TextPosition {
                    line: function.start,
                    column: 1,
                },
                end: TextPosition {
                    line: function.end,
                    column: 1,
                },
            },
            lines: function.end - function.start + 1,
            parameters: 0,
            cyclomatic: function.cyclomatic,
            max_nesting: 0,
        })
        .collect()
}

fn check(json: &str) {
    let sample: Sample = serde_json::from_str(json).unwrap();
    let result = findings_for(&sample.path, &metrics(&sample), sample.threshold);
    assert_eq!(result.functions, sample.functions.len());
    assert_eq!(result.highest, sample.highest);
    let found: Vec<(String, u32)> = result
        .findings
        .iter()
        .map(|item| {
            let message = item.message.split(':').next().unwrap_or("").to_owned();
            (message, item.value.map_or(0, |value| value as u32))
        })
        .collect();
    assert_eq!(found, sample.findings);
    for item in &result.findings {
        assert_eq!(item.path.as_str(), sample.path);
        assert_eq!(item.threshold, Some(f64::from(sample.threshold)));
    }
}

macro_rules! language_test {
    ($name:ident, $file:literal) => {
        #[test]
        fn $name() {
            check(include_str!(concat!(
                "../../../tests/data/analyze/complexity/",
                $file,
                ".json"
            )));
        }
    };
}

language_test!(reports_functions_above_threshold_go, "go");
language_test!(reports_functions_above_threshold_rust, "rust");
language_test!(reports_functions_above_threshold_python, "python");
language_test!(reports_functions_above_threshold_typescript, "typescript");
language_test!(reports_functions_above_threshold_java, "java");
language_test!(reports_functions_above_threshold_c, "c");
language_test!(reports_functions_above_threshold_cpp, "cpp");
language_test!(reports_functions_above_threshold_csharp, "csharp");
language_test!(reports_functions_above_threshold_ruby, "ruby");
language_test!(reports_functions_above_threshold_php, "php");

#[test]
fn finding_carries_span_value_and_threshold() {
    let sample = Sample {
        path: "a.go".to_owned(),
        threshold: 15,
        highest: 17,
        functions: vec![Function {
            name: "hard".to_owned(),
            container: Some("K".to_owned()),
            start: 3,
            end: 30,
            cyclomatic: 17,
        }],
        findings: Vec::new(),
    };
    let result = findings_for("a.go", &metrics(&sample), 15);
    let item = &result.findings[0];
    assert_eq!(item.line, Some(3));
    assert_eq!(item.end_line, Some(30));
    assert_eq!(item.value, Some(17.0));
    assert_eq!(item.message, "K.hard: cyclomatic complexity 17 exceeds 15");
}

#[test]
fn accepts_function_equal_to_threshold() {
    let sample = Sample {
        path: "a.go".to_owned(),
        threshold: 15,
        highest: 15,
        functions: vec![Function {
            name: "edge".to_owned(),
            container: None,
            start: 1,
            end: 2,
            cyclomatic: 15,
        }],
        findings: Vec::new(),
    };
    assert!(
        findings_for("a.go", &metrics(&sample), 15)
            .findings
            .is_empty()
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
