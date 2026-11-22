// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Strict parser of variant C: stages, assignments, values, sub-pipelines,
//! spans and errors.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_contracts::ast::{
    AstDocument, BinOp, Expr, Head, Pipeline, Span, Stage, Statement, UnOp, strip_for_hash,
};
use nurt_contracts::error::ErrorCode;
use nurt_syntax::error::{SYNTAX_HELP, SyntaxError};
use nurt_syntax::parse::{ParseOptions, Variant, parse};

/// Start of the message of the error for a value written without a
/// parameter name.
const POSITIONAL_MESSAGE: &str = "variant C accepts only name=value arguments";

/// Short name of the problem of a value written without a parameter name.
const POSITIONAL_PROBLEM: &str = "positional_argument";

fn parse_c(text: &str) -> Result<AstDocument, SyntaxError> {
    parse(text, Variant::C, &ParseOptions::default())
}

fn document(text: &str) -> AstDocument {
    parse_c(text).unwrap_or_else(|error| panic!("{text:?} does not parse: {error:?}"))
}

fn error(text: &str) -> SyntaxError {
    match parse_c(text) {
        Ok(document) => panic!("{text:?} parses as {document:?}"),
        Err(error) => error,
    }
}

fn pipeline(document: &AstDocument, index: usize) -> &Pipeline {
    match &document.program.statements[index] {
        Statement::Pipeline(pipeline) => pipeline,
        Statement::Let(binding) => &binding.value,
        other => panic!("unexpected statement {other:?}"),
    }
}

fn head(document: &AstDocument) -> Stage {
    match &pipeline(document, 0).head {
        Head::Stage(stage) => stage.clone(),
        other => panic!("head is not a stage: {other:?}"),
    }
}

fn names(stage: &Stage) -> Vec<Option<&str>> {
    stage.args.iter().map(|arg| arg.name.as_deref()).collect()
}

/// Hash form of a document: the JSON value without `meta` and spans.
fn shape(document: &AstDocument) -> serde_json::Value {
    strip_for_hash(&serde_json::to_value(document).expect("tree serializes"))
}

fn same_tree(left: &str, right: &str) {
    assert_eq!(
        shape(&document(left)),
        shape(&document(right)),
        "{left:?} vs {right:?}"
    );
}

fn span(line: u32, column: u32, end_line: u32, end_column: u32) -> Option<Span> {
    Some(Span {
        line,
        column,
        end_line,
        end_column,
    })
}

fn without_span(expr: &Expr) -> serde_json::Value {
    strip_for_hash(&serde_json::to_value(expr).expect("expression serializes"))
}

fn assert_positional(error: &SyntaxError) {
    assert_eq!(
        error.problem.as_deref(),
        Some(POSITIONAL_PROBLEM),
        "{error:?}"
    );
    assert!(error.message.starts_with(POSITIONAL_MESSAGE), "{error:?}");
}

fn assert_syntax_error(error: &SyntaxError, line: u32, column: u32) {
    assert_eq!(error.code, ErrorCode::Syntax, "{error:?}");
    let at = error.at.as_ref().expect("error has a position");
    assert_eq!(
        (at.line, at.column),
        (Some(line), Some(column)),
        "{error:?}"
    );
    assert_eq!(error.help.as_deref(), Some(SYNTAX_HELP), "{error:?}");
}

mod stages {
    use super::*;

    #[test]
    fn stage_without_arguments_is_the_command_name() {
        let stage = head(&document("remove"));
        assert_eq!(stage.name, "remove");
        assert!(stage.args.is_empty());
    }

    #[test]
    fn reference_head_is_followed_by_a_stage_without_arguments() {
        let document = document("@recent |> count");
        let pipeline = pipeline(&document, 0);
        assert!(matches!(&pipeline.head, Head::Ref(reference) if reference.name == "recent"));
        assert_eq!(pipeline.stages.len(), 1);
        assert_eq!(pipeline.stages[0].name, "count");
        assert!(pipeline.stages[0].args.is_empty());
    }

    #[test]
    fn dotted_command_name_is_one_name() {
        let stage = head(&document(r#"code.symbols name="oldName" kind=function"#));
        assert_eq!(stage.name, "code.symbols");
        assert_eq!(names(&stage), [Some("name"), Some("kind")]);
    }

    #[test]
    fn keyword_in_command_position_is_rejected() {
        let error = error(r#"in path="/a""#);
        assert_syntax_error(&error, 1, 1);
    }

    #[test]
    fn stages_continue_on_the_next_line_after_a_pipe() {
        same_tree(
            "find path=\"/a\" glob=\"*.go\"\n|> first count=20",
            "find path=\"/a\" glob=\"*.go\" |> first count=20",
        );
    }
}

mod assignments {
    use super::*;

    #[test]
    fn assignment_is_a_named_argument() {
        let stage = head(&document(r#"find path="/srv/app""#));
        assert_eq!(names(&stage), [Some("path")]);
        assert!(matches!(&stage.args[0].value, Expr::Text { v, .. } if v == "/srv/app"));
    }

    #[test]
    fn keyword_before_equals_is_a_parameter_name() {
        let stage = head(&document(
            r#"replace pattern="a" with="b" in=["/srv/a.txt"]"#,
        ));
        assert_eq!(names(&stage), [Some("pattern"), Some("with"), Some("in")]);
        assert!(matches!(&stage.args[1].value, Expr::Text { v, .. } if v == "b"));
        assert!(matches!(&stage.args[2].value, Expr::List { items, .. } if items.len() == 1));
    }

    #[test]
    fn space_around_equals_is_accepted() {
        same_tree(
            r#"find path = "/a" glob ="*""#,
            r#"find path="/a" glob="*""#,
        );
        same_tree(r#"find path= "/a""#, r#"find path="/a""#);
    }

    #[test]
    fn line_break_after_equals_is_ignored() {
        same_tree(
            "find path=\n    \"/a\" glob=\"*.go\"",
            r#"find path="/a" glob="*.go""#,
        );
    }

    #[test]
    fn assignments_keep_their_written_order() {
        let stage = head(&document(r#"find glob="*.go" path="/a" depth=1"#));
        assert_eq!(names(&stage), [Some("glob"), Some("path"), Some("depth")]);
    }

    #[test]
    fn repeated_parameter_name_is_left_to_binding() {
        let stage = head(&document("first count=1 count=2"));
        assert_eq!(names(&stage), [Some("count"), Some("count")]);
    }

    #[test]
    fn colon_instead_of_equals_is_corrected_to_equals() {
        let error = error(r#"find path: "/a""#);
        assert_syntax_error(&error, 1, 6);
        assert!(
            error.fix.iter().any(|fix| fix.contains(r#"path="/a""#)),
            "{error:?}"
        );
    }

    #[test]
    fn colon_after_a_space_is_corrected_to_equals() {
        let error = error(r#"find path : "/a""#);
        assert_syntax_error(&error, 1, 6);
        assert!(
            error.fix.iter().any(|fix| fix.contains("path=")),
            "{error:?}"
        );
    }

    #[test]
    fn equals_without_value_is_rejected() {
        let error = error("find path=");
        assert_eq!(error.code, ErrorCode::Syntax, "{error:?}");
    }
}

mod values {
    use super::*;

    #[test]
    fn value_extends_over_several_tokens() {
        let stage = head(&document(
            "filter condition=size > 1mb and modified > ago(24h)",
        ));
        assert_eq!(names(&stage), [Some("condition")]);
        assert!(matches!(
            &stage.args[0].value,
            Expr::Bin { op: BinOp::And, .. }
        ));
    }

    #[test]
    fn value_ends_before_the_next_assignment() {
        let stage = head(&document(r#"filter condition=size > 1mb hidden=true"#));
        assert_eq!(names(&stage), [Some("condition"), Some("hidden")]);
        assert!(matches!(
            &stage.args[0].value,
            Expr::Bin { op: BinOp::Gt, .. }
        ));
    }

    #[test]
    fn value_ends_before_a_keyword_assignment() {
        let stage = head(&document(r#"replace pattern="a" with=x in=["/a"]"#));
        assert!(matches!(&stage.args[1].value, Expr::Ident { v, .. } if v == "x"));
    }

    #[test]
    fn operator_word_continues_a_value() {
        let stage = head(&document(r#"filter condition=name in ["a", "b"]"#));
        assert_eq!(names(&stage), [Some("condition")]);
        assert!(matches!(
            &stage.args[0].value,
            Expr::Bin { op: BinOp::In, .. }
        ));
    }

    #[test]
    fn slash_after_a_space_starts_a_regular_expression() {
        let stage = head(&document(r#"search pattern=/oldName\(/i path="/a""#));
        assert!(matches!(
            &stage.args[0].value,
            Expr::Regex { v, flags, .. } if v == r"oldName\(" && flags == "i"
        ));
        assert_eq!(names(&stage), [Some("pattern"), Some("path")]);
    }

    #[test]
    fn slash_between_operands_is_division() {
        for text in [
            "filter condition=size / 2 > 1",
            "filter condition=size/2 > 1",
        ] {
            let stage = head(&document(text));
            assert_eq!(names(&stage), [Some("condition")], "{text}");
            let Expr::Bin {
                op: BinOp::Gt, l, ..
            } = &stage.args[0].value
            else {
                panic!("{text}: {:?}", stage.args[0].value);
            };
            assert!(matches!(**l, Expr::Bin { op: BinOp::Div, .. }), "{text}");
        }
    }

    #[test]
    fn minus_between_spaces_or_without_spaces_is_subtraction() {
        for text in [
            "filter condition=size - 100 > 0",
            "filter condition=size-100 > 0",
        ] {
            let stage = head(&document(text));
            assert_eq!(names(&stage), [Some("condition")], "{text}");
        }
    }

    #[test]
    fn minus_after_a_space_starts_a_positional_argument() {
        let error = error("first count=20 -1");
        assert_syntax_error(&error, 1, 16);
        assert_positional(&error);
    }

    #[test]
    fn negative_value_after_equals_is_negation() {
        let stage = head(&document("first count=-1"));
        assert!(matches!(
            &stage.args[0].value,
            Expr::Un { op: UnOp::Neg, .. }
        ));
    }

    #[test]
    fn call_list_map_and_range_values_come_from_expressions() {
        let stage = head(&document(
            r#"edit path="/a" changes=[{replace: "a", with: "b"}, {delete_lines: 10..12}] at=ago(1h)"#,
        ));
        assert_eq!(names(&stage), [Some("path"), Some("changes"), Some("at")]);
        assert!(matches!(&stage.args[1].value, Expr::List { items, .. } if items.len() == 2));
        assert!(matches!(&stage.args[2].value, Expr::Call { function, .. } if function == "ago"));
    }

    #[test]
    fn reference_value_keeps_its_fields() {
        let stage = head(&document("move from=@old.path into=\"/a\""));
        assert!(matches!(
            &stage.args[0].value,
            Expr::Ref(reference) if reference.name == "old" && reference.fields == ["path"]
        ));
    }
}

mod positional {
    use super::*;

    #[test]
    fn value_without_a_name_is_rejected() {
        let error = error(r#"find "/a""#);
        assert_syntax_error(&error, 1, 6);
        assert_positional(&error);
        assert!(
            error.fix.is_empty(),
            "the parser does not guess a name: {error:?}"
        );
    }

    #[test]
    fn value_without_a_name_after_assignments_is_rejected() {
        let error = error(r#"find path="/a" "*.go""#);
        assert_syntax_error(&error, 1, 16);
        assert_positional(&error);
    }

    #[test]
    fn bare_word_after_a_complete_value_is_rejected() {
        let error = error(r#"sort by=size desc"#);
        assert_syntax_error(&error, 1, 14);
        assert_positional(&error);
    }

    #[test]
    fn variant_a_text_is_rejected() {
        let error = error(r#"find "/srv/app" glob: "**/*.go""#);
        assert_eq!(error.code, ErrorCode::Syntax);
    }

    #[test]
    fn variant_b_text_is_rejected() {
        let error = error(r#"find(path: "/srv/app")"#);
        assert_eq!(error.code, ErrorCode::Syntax);
    }

    #[test]
    fn comma_between_arguments_is_rejected() {
        let error = error(r#"find path="/a", glob="*""#);
        assert_syntax_error(&error, 1, 15);
    }
}

mod statements {
    use super::*;

    #[test]
    fn let_binds_a_pipeline() {
        let document = document(
            r#"let recent = find path="/srv/app/src" glob="**/*.ts" |> filter condition=modified > ago(1h)"#,
        );
        let Statement::Let(binding) = &document.program.statements[0] else {
            panic!("not a let statement");
        };
        assert_eq!(binding.name, "recent");
        assert_eq!(binding.value.stages.len(), 1);
    }

    #[test]
    fn let_at_statement_start_is_not_an_assignment() {
        let document = document("let total = @recent |> count\n@total |> first count=1");
        assert!(matches!(&document.program.statements[0], Statement::Let(_)));
        assert_eq!(document.program.statements.len(), 2);
    }

    #[test]
    fn empty_program_is_rejected() {
        let error = error("# nothing\n");
        assert_eq!(error.code, ErrorCode::Syntax);
    }
}

mod sub_pipelines {
    use super::*;

    fn check_value(text: &str) -> Expr {
        let document = document(text);
        let pipeline = pipeline(&document, 0);
        let stage = pipeline.stages.last().expect("a verify stage");
        assert_eq!(names(stage), [Some("check")]);
        stage.args[0].value.clone()
    }

    #[test]
    fn group_with_a_pipe_is_a_sub_pipeline() {
        let value = check_value(r#"@x |> verify check=(find path="/a" |> count)"#);
        let Expr::Sub { p, .. } = value else {
            panic!("{value:?}");
        };
        assert_eq!(p.stages.len(), 1);
    }

    #[test]
    fn command_followed_by_an_assignment_is_a_sub_pipeline() {
        let value = check_value(r#"@x |> verify check=(run command=["go", "vet"] cwd="/a")"#);
        let Expr::Sub { p, .. } = value else {
            panic!("{value:?}");
        };
        assert!(p.stages.is_empty());
        let Head::Stage(stage) = &p.head else {
            panic!("{p:?}");
        };
        assert_eq!(names(stage), [Some("command"), Some("cwd")]);
    }

    #[test]
    fn command_followed_by_a_spaced_assignment_is_a_sub_pipeline() {
        let value = check_value(r#"@x |> verify check=(run command = ["go"])"#);
        assert!(matches!(value, Expr::Sub { .. }), "{value:?}");
    }

    #[test]
    fn single_word_in_parentheses_is_an_identifier() {
        let value = check_value("@x |> verify check=(remove)");
        assert_eq!(
            without_span(&value),
            serde_json::json!({"t": "ident", "v": "remove"})
        );
    }

    #[test]
    fn expression_in_parentheses_is_grouping() {
        let stage = head(&document("filter condition=(size > 1mb)"));
        assert!(matches!(
            &stage.args[0].value,
            Expr::Bin { op: BinOp::Gt, .. }
        ));
    }

    #[test]
    fn stages_inside_a_sub_pipeline_use_variant_c() {
        let error = error(r#"@x |> verify check=(run ["go"] |> count)"#);
        assert_positional(&error);
    }

    #[test]
    fn sub_pipeline_spans_lines() {
        same_tree(
            "@x |> verify check=(\n  run command=[\"go\"]\n  |> assert exit_code=0\n)",
            r#"@x |> verify check=(run command=["go"] |> assert exit_code=0)"#,
        );
    }
}

mod spans {
    use super::*;

    #[test]
    fn named_argument_covers_name_and_value() {
        let stage = head(&document(r#"find path="/srv/app" depth=1"#));
        assert_eq!(stage.span, span(1, 1, 1, 29));
        assert_eq!(stage.args[0].span, span(1, 6, 1, 21));
        assert_eq!(stage.args[1].span, span(1, 22, 1, 29));
    }

    #[test]
    fn spaced_assignment_covers_name_and_value() {
        let stage = head(&document(r#"find path = "/a""#));
        assert_eq!(stage.args[0].span, span(1, 6, 1, 17));
    }

    #[test]
    fn multi_token_value_span_ends_at_its_last_token() {
        let stage = head(&document("filter condition=size > 1mb hidden=true"));
        assert_eq!(stage.args[0].span, span(1, 8, 1, 28));
    }

    #[test]
    fn value_after_a_line_break_spans_two_lines() {
        let stage = head(&document("find path=\n  \"/a\""));
        assert_eq!(stage.args[0].span, span(1, 6, 2, 7));
    }

    #[test]
    fn stage_without_arguments_covers_its_name() {
        let document = document("@recent |> count");
        assert_eq!(pipeline(&document, 0).stages[0].span, span(1, 12, 1, 17));
    }
}
