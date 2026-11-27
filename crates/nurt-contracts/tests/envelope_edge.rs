// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-contracts/tests/envelope_edge.rs
// Responsibility:   Boundaries of the request envelope and the response: path and duration.
// Dependencies:     serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Boundaries of the request envelope and the response: path and duration
//! notation, option rules, error messages and the form of the JSON documents.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::error::Error as _;

use nurt_contracts::ast::{AstDocument, Head, Pipeline, Program, Stage, Statement};
use nurt_contracts::envelope::{
    DiffMode, EnvelopeError, Language, MAX_LIMIT, OutputFormat, Rendered, Request, RequestInput,
    RequestOptions, Response, Verbosity, is_absolute_path, is_duration,
};
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::report::{Report, ReportStatus, ReportViolation};
use serde_json::json;

fn tree() -> AstDocument {
    AstDocument::new(Program {
        statements: vec![Statement::Pipeline(Pipeline {
            head: Head::Stage(Stage {
                name: "count".to_owned(),
                args: Vec::new(),
                span: None,
            }),
            stages: Vec::new(),
            span: None,
        })],
    })
}

fn options_with(change: impl FnOnce(&mut RequestOptions)) -> RequestOptions {
    let mut options = RequestOptions::default();
    change(&mut options);
    options
}

fn validate(options: RequestOptions) -> Result<(), EnvelopeError> {
    Request::program("count").with_options(options).validate()
}

#[test]
fn syntax_tree_requests_carry_the_tree_and_round_trip() {
    let request = Request::ast(tree());
    assert!(request.validate().is_ok());
    assert!(matches!(request.input(), Ok(RequestInput::Ast(_))));
    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(
        value["ast"]["program"]["statements"][0]["head"]["name"],
        "count"
    );
    assert!(value.get("program").is_none());
    assert!(value.get("options").is_none());
    assert_eq!(serde_json::from_value::<Request>(value).unwrap(), request);
}

#[test]
fn a_tree_and_program_text_together_are_conflicting_inputs() {
    let mut request = Request::ast(tree());
    request.program = Some("count".to_owned());
    assert_eq!(
        request.input(),
        Err(EnvelopeError::ConflictingInputs {
            fields: vec!["program", "ast"]
        })
    );
    assert!(request.validate().is_err());
}

#[test]
fn a_tree_with_an_unknown_member_is_not_a_request() {
    let value = json!({
        "nurt": "1.0",
        "ast": {"nurt": "1.0", "program": {"statements": []}, "extra": 1}
    });
    assert!(serde_json::from_value::<Request>(value).is_err());
}

#[test]
fn the_variant_is_written_with_the_program() {
    let mut request = Request::program("count");
    request.variant = Some(SyntaxVariant::C);
    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(value["variant"], "c");
    assert_eq!(serde_json::from_value::<Request>(value).unwrap(), request);
}

#[test]
fn only_the_given_options_are_serialized() {
    let options = RequestOptions {
        preview: Some(false),
        limit: Some(20),
        format: Some(OutputFormat::Markdown),
        ..RequestOptions::default()
    };
    assert!(!options.is_empty());
    assert_eq!(
        serde_json::to_value(&options).unwrap(),
        json!({"preview": false, "limit": 20, "format": "markdown"})
    );
    assert_eq!(options.format(), OutputFormat::Markdown);
    assert!(!options.preview());
    assert_eq!(options.limit(), 20);

    let empty: Request = serde_json::from_value(json!({
        "nurt": "1.0",
        "program": "count",
        "options": {}
    }))
    .unwrap();
    assert!(empty.options.is_empty());
    assert!(
        serde_json::to_value(&empty)
            .unwrap()
            .get("options")
            .is_none()
    );
}

#[test]
fn every_option_reads_back_through_its_accessor() {
    let options = RequestOptions {
        preview: Some(true),
        verbosity: Some(Verbosity::Detailed),
        diff: Some(DiffMode::Stat),
        diff_context: Some(0),
        max_chars: Some(80),
        script: Some(true),
        language: Some(Language::Pl),
        ..RequestOptions::default()
    };
    assert!(options.preview());
    assert_eq!(options.verbosity(), Verbosity::Detailed);
    assert_eq!(options.diff(), DiffMode::Stat);
    assert_eq!(options.diff_context(), 0);
    assert_eq!(options.max_chars(), 80);
    assert!(options.script());
    let value = serde_json::to_value(&options).unwrap();
    assert_eq!(value["language"], "pl");
    assert_eq!(value["diff"], "stat");
    assert_eq!(value["verbosity"], "detailed");
}

#[test]
fn the_limit_accepts_exactly_one_to_the_maximum() {
    assert_eq!(MAX_LIMIT, 5000);
    for limit in [1, 2, 4999, 5000] {
        assert!(
            validate(options_with(|options| options.limit = Some(limit))).is_ok(),
            "{limit}"
        );
    }
    for limit in [0, 5001, u32::MAX] {
        assert_eq!(
            validate(options_with(|options| options.limit = Some(limit))),
            Err(EnvelopeError::LimitOutOfRange { limit })
        );
    }
}

#[test]
fn the_first_broken_option_is_reported() {
    let broken = options_with(|options| {
        options.limit = Some(0);
        options.base = Some("relative".to_owned());
        options.save_to = Some("relative".to_owned());
        options.timeout = Some("soon".to_owned());
        options.session = Some(String::new());
        options.fields = Some(vec![String::new()]);
    });
    assert!(matches!(
        validate(broken.clone()),
        Err(EnvelopeError::LimitOutOfRange { limit: 0 })
    ));
    let mut broken = broken;
    broken.limit = None;
    assert!(matches!(
        validate(broken.clone()),
        Err(EnvelopeError::RelativePath { option: "base", .. })
    ));
    broken.base = None;
    assert!(matches!(
        validate(broken.clone()),
        Err(EnvelopeError::RelativePath {
            option: "save_to",
            ..
        })
    ));
    broken.save_to = None;
    assert!(matches!(
        validate(broken.clone()),
        Err(EnvelopeError::InvalidTimeout { .. })
    ));
    broken.timeout = None;
    assert_eq!(
        validate(broken.clone()),
        Err(EnvelopeError::EmptyField { field: "session" })
    );
    broken.session = None;
    assert_eq!(
        validate(broken.clone()),
        Err(EnvelopeError::EmptyField { field: "fields" })
    );
    broken.fields = Some(vec!["path".to_owned()]);
    assert!(validate(broken).is_ok());
}

#[test]
fn versions_of_the_first_major_are_accepted_for_every_minor() {
    for version in ["1.0", "1.1", "1.9", "1.10"] {
        let mut request = Request::program("count");
        request.nurt = version.to_owned();
        assert!(request.validate().is_ok(), "{version}");
    }
    for version in ["", "1", "1.", "2.0", "0.9", "1.x", "v1.0"] {
        let mut request = Request::program("count");
        request.nurt = version.to_owned();
        assert_eq!(
            request.validate(),
            Err(EnvelopeError::UnsupportedVersion {
                found: version.to_owned()
            }),
            "{version:?}"
        );
    }
}

#[test]
fn absolute_path_notation_has_four_forms() {
    for path in [
        "/",
        "//",
        "/srv/app",
        "C:/",
        "z:\\",
        "x:\\y",
        "a:/",
        "\\\\server\\share",
        "\\\\x",
        "registry:/",
        "env:/PATH",
        "my_provider:/x",
        "a1:/x",
        "archive:/home/user/a.zip!/dir",
    ] {
        assert!(is_absolute_path(path), "{path}");
    }
    for path in [
        "",
        " ",
        " /a",
        "C:",
        "C:x",
        "C:..\\x",
        "1:/x",
        "ab:",
        ":/x",
        "-:/x",
        "a-b:/x",
        "Registry:/x",
        "\u{e9}:/x",
        "\\x",
        "~/a",
        "./a",
        "a/b",
    ] {
        assert!(!is_absolute_path(path), "{path:?}");
    }
}

#[test]
fn durations_are_pairs_of_a_number_and_a_unit() {
    for text in [
        "0s",
        "007m",
        "1ms",
        "1h1m1s1ms",
        "1s2s",
        "99999999999999999999999999d",
        "2d12h",
    ] {
        assert!(is_duration(text), "{text}");
    }
    for text in [
        "",
        "ms",
        "s",
        "1",
        "1 s",
        "1S",
        "1.0s",
        "-1s",
        "+1s",
        "1ms5",
        "1mss",
        "1w",
        "1y",
        "\u{663}s",
        "\u{ff11}s",
        "1\u{e9}",
    ] {
        assert!(!is_duration(text), "{text:?}");
    }
}

#[test]
fn every_envelope_error_names_its_rule() {
    let cases = [
        (
            EnvelopeError::UnsupportedVersion {
                found: "2.0".to_owned(),
            },
            "unsupported envelope version `2.0`",
        ),
        (
            EnvelopeError::MissingInput,
            "give one of `program`, `ast`, `cursor` or `accept`",
        ),
        (
            EnvelopeError::ConflictingInputs {
                fields: vec!["program", "ast", "cursor"],
            },
            "given: program, ast, cursor",
        ),
        (
            EnvelopeError::VariantWithoutProgram,
            "`variant` applies only to `program`",
        ),
        (
            EnvelopeError::EmptyField { field: "cursor" },
            "`cursor` must not be empty",
        ),
        (
            EnvelopeError::LimitOutOfRange { limit: 0 },
            "`limit` is 0; expected 1 to 5000",
        ),
        (
            EnvelopeError::RelativePath {
                option: "save_to",
                path: "out.txt".to_owned(),
            },
            "`save_to` must be an absolute path; given `out.txt`",
        ),
        (
            EnvelopeError::InvalidTimeout {
                timeout: "soon".to_owned(),
            },
            "given `soon`",
        ),
        (
            EnvelopeError::RenderedJson,
            "the format `json` is not rendered as text",
        ),
        (
            EnvelopeError::Report(ReportViolation::ZeroOp),
            "invalid report: operation numbers start at 1",
        ),
    ];
    for (error, fragment) in cases {
        assert!(error.to_string().contains(fragment), "{error:?}: {error}");
    }
    assert!(EnvelopeError::MissingInput.source().is_none());
    let nested = EnvelopeError::Report(ReportViolation::EmptyKind);
    assert_eq!(
        nested.source().unwrap().to_string(),
        ReportViolation::EmptyKind.to_string()
    );
}

#[test]
fn responses_check_the_version_the_rendering_and_the_report() {
    let mut response = Response::new(Report::new(ReportStatus::Ok, "find"));
    assert_eq!(response.nurt, "1.0");
    response.nurt = "2.0".to_owned();
    assert!(matches!(
        response.validate(),
        Err(EnvelopeError::UnsupportedVersion { .. })
    ));
    response.nurt = "1.3".to_owned();
    for format in [
        OutputFormat::Text,
        OutputFormat::Markdown,
        OutputFormat::Compact,
    ] {
        response.rendered = Some(Rendered {
            format,
            text: "ok\n".to_owned(),
        });
        assert!(response.validate().is_ok(), "{format:?}");
    }
    response.report.op = Some(0);
    assert_eq!(
        response.validate(),
        Err(EnvelopeError::Report(ReportViolation::ZeroOp))
    );
}

#[test]
fn a_response_without_rendered_text_omits_the_member() {
    let response = Response::new(Report::new(ReportStatus::Ok, "find"));
    let value = serde_json::to_value(&response).unwrap();
    assert_eq!(
        value,
        json!({
            "nurt": "1.0",
            "report": {"nurt": "1.0", "status": "ok", "kind": "find"}
        })
    );
    assert_eq!(serde_json::from_value::<Response>(value).unwrap(), response);
    assert!(!response.has_more());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
