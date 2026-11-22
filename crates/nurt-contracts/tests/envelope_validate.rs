// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Rules of the request envelope and the response checked by the Rust types.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_contracts::envelope::{
    DEFAULT_DIFF_CONTEXT, DEFAULT_LIMIT, DiffMode, ENVELOPE_VERSION, EnvelopeError, Language,
    OutputFormat, Rendered, Request, RequestInput, RequestOptions, Response, Verbosity,
    is_absolute_path, is_duration,
};
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::report::{Report, ReportStatus, ReportViolation};

#[test]
fn constructors_produce_valid_requests() {
    for request in [
        Request::program("find \"/srv/app\""),
        Request::cursor("c_91fe"),
        Request::accept("c1"),
    ] {
        assert_eq!(request.nurt, ENVELOPE_VERSION);
        assert!(request.validate().is_ok(), "{request:?}");
    }
    let request = Request::program("find \"/srv/app\"");
    assert_eq!(
        request.input().unwrap(),
        RequestInput::Program {
            text: "find \"/srv/app\"",
            variant: None
        }
    );
    assert_eq!(
        Request::cursor("c_1").input().unwrap(),
        RequestInput::Cursor("c_1")
    );
    assert_eq!(
        Request::accept("c2").input().unwrap(),
        RequestInput::Accept("c2")
    );
}

#[test]
fn exactly_one_input_is_required() {
    let mut request = Request::program("find \"/srv/app\"");
    request.program = None;
    assert_eq!(request.input(), Err(EnvelopeError::MissingInput));

    let mut request = Request::program("find \"/srv/app\"");
    request.accept = Some("c1".to_owned());
    request.cursor = Some("c_1".to_owned());
    assert_eq!(
        request.input(),
        Err(EnvelopeError::ConflictingInputs {
            fields: vec!["program", "cursor", "accept"]
        })
    );

    let mut request = Request::cursor("c_1");
    request.variant = Some(SyntaxVariant::B);
    assert_eq!(request.input(), Err(EnvelopeError::VariantWithoutProgram));

    let mut request = Request::program("find \"/srv/app\"");
    request.variant = Some(SyntaxVariant::B);
    assert_eq!(
        request.input().unwrap(),
        RequestInput::Program {
            text: "find \"/srv/app\"",
            variant: Some(SyntaxVariant::B)
        }
    );

    assert_eq!(
        Request::cursor("").input(),
        Err(EnvelopeError::EmptyField { field: "cursor" })
    );
    assert_eq!(
        Request::accept("").input(),
        Err(EnvelopeError::EmptyField { field: "accept" })
    );
}

#[test]
fn version_is_checked() {
    let mut request = Request::program("x");
    request.nurt = "2.0".to_owned();
    assert!(matches!(
        request.validate(),
        Err(EnvelopeError::UnsupportedVersion { .. })
    ));
}

#[test]
fn unknown_fields_are_rejected() {
    let typo = serde_json::json!({ "nurt": "1.0", "program": "x", "options": { "preveiw": true } });
    assert!(serde_json::from_value::<Request>(typo).is_err());
    let extra = serde_json::json!({ "nurt": "1.0", "program": "x", "dry_run": true });
    assert!(serde_json::from_value::<Request>(extra).is_err());
}

#[test]
fn absent_options_take_the_built_in_defaults() {
    let options = RequestOptions::default();
    assert!(options.is_empty());
    assert!(!options.preview());
    assert_eq!(options.limit(), DEFAULT_LIMIT);
    assert_eq!(options.format(), OutputFormat::Text);
    assert_eq!(options.verbosity(), Verbosity::Normal);
    assert_eq!(options.diff(), DiffMode::Unified);
    assert_eq!(options.diff_context(), DEFAULT_DIFF_CONTEXT);
    assert_eq!(options.max_chars(), 0);
    assert!(!options.script());
    assert_eq!(Language::default(), Language::En);
    let value = serde_json::to_value(Request::program("x")).unwrap();
    assert!(value.get("options").is_none());
}

#[test]
fn option_rules() {
    let check = |options: RequestOptions| Request::program("x").with_options(options).validate();

    assert!(matches!(
        check(RequestOptions {
            limit: Some(0),
            ..RequestOptions::default()
        }),
        Err(EnvelopeError::LimitOutOfRange { limit: 0 })
    ));
    assert!(matches!(
        check(RequestOptions {
            limit: Some(5001),
            ..RequestOptions::default()
        }),
        Err(EnvelopeError::LimitOutOfRange { limit: 5001 })
    ));
    assert!(
        check(RequestOptions {
            limit: Some(5000),
            ..RequestOptions::default()
        })
        .is_ok()
    );
    assert!(matches!(
        check(RequestOptions {
            base: Some("srv/app".to_owned()),
            ..RequestOptions::default()
        }),
        Err(EnvelopeError::RelativePath { option: "base", .. })
    ));
    assert!(matches!(
        check(RequestOptions {
            save_to: Some("out.txt".to_owned()),
            ..RequestOptions::default()
        }),
        Err(EnvelopeError::RelativePath {
            option: "save_to",
            ..
        })
    ));
    assert!(matches!(
        check(RequestOptions {
            timeout: Some("2 minutes".to_owned()),
            ..RequestOptions::default()
        }),
        Err(EnvelopeError::InvalidTimeout { .. })
    ));
    assert_eq!(
        check(RequestOptions {
            session: Some(String::new()),
            ..RequestOptions::default()
        }),
        Err(EnvelopeError::EmptyField { field: "session" })
    );
    assert_eq!(
        check(RequestOptions {
            fields: Some(vec![String::new()]),
            ..RequestOptions::default()
        }),
        Err(EnvelopeError::EmptyField { field: "fields" })
    );
}

#[test]
fn absolute_paths_and_durations() {
    for path in [
        "/srv/app",
        "C:/Users/user",
        "c:\\proj",
        "x:/",
        "//server/share",
        "\\\\server\\share",
        "registry:/HKCU/Software",
        "env:/PATH",
    ] {
        assert!(is_absolute_path(path), "{path}");
    }
    for path in ["", "srv/app", "./app", "C:proj", "~/app", "1:/x"] {
        assert!(!is_absolute_path(path), "{path}");
    }
    for text in ["250ms", "30s", "5m", "1h30m", "7d", "120s"] {
        assert!(is_duration(text), "{text}");
    }
    for text in ["", "120", "s", "1.5s", "10 s", "1w", "m5"] {
        assert!(!is_duration(text), "{text}");
    }
}

#[test]
fn response_rules() {
    let mut response = Response::new(Report::new(ReportStatus::Ok, "find"));
    assert!(response.validate().is_ok());
    assert!(!response.has_more());

    response.report.next_cursor = Some("c_1".to_owned());
    assert!(response.has_more());

    response.rendered = Some(Rendered {
        format: OutputFormat::Json,
        text: "{}".to_owned(),
    });
    assert_eq!(response.validate(), Err(EnvelopeError::RenderedJson));

    response.rendered = Some(Rendered {
        format: OutputFormat::Text,
        text: "ok".to_owned(),
    });
    assert!(response.validate().is_ok());

    response.report.status = ReportStatus::Failed;
    assert_eq!(
        response.validate(),
        Err(EnvelopeError::Report(ReportViolation::MissingError {
            status: ReportStatus::Failed
        }))
    );
    let error = response.validate().unwrap_err();
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn error_messages_name_the_rule() {
    let error = EnvelopeError::ConflictingInputs {
        fields: vec!["program", "ast"],
    };
    assert!(error.to_string().contains("program, ast"));
    assert!(
        EnvelopeError::LimitOutOfRange { limit: 9000 }
            .to_string()
            .contains("9000")
    );
}
