// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-correct::bash::tree
// Responsibility:   Construction of surface tree documents and their canonical text.
// Dependencies:     nurt_contracts, nurt_syntax
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Construction of surface tree documents and their canonical text.
//!
//! Translations report ready program text in corrections, fixes, next steps
//! and candidates. The text is the canonical form of variant A printed by
//! [`nurt_syntax::print`], without the final line feed.

use nurt_contracts::ast::{AstDocument, Expr, Head, Pipeline, Program, Stage, Statement};
use nurt_syntax::parse::Variant;
use nurt_syntax::print::{print, print_expr};

/// Prints a whole document in canonical text, statements separated by line
/// feeds, without a final line feed.
pub(crate) fn print_document(document: &AstDocument) -> String {
    let mut text = print(document, Variant::A);
    if text.ends_with('\n') {
        text.pop();
    }
    text
}

/// Prints a sequence of statements as one document.
pub(crate) fn print_program(statements: Vec<Statement>) -> String {
    print_document(&document(statements))
}

/// Prints one pipeline as a statement.
pub(crate) fn print_pipeline(pipeline: &Pipeline) -> String {
    print_program(vec![Statement::Pipeline(pipeline.clone())])
}

/// Prints one stage as a pipeline of that stage alone.
pub(crate) fn print_stage(stage: &Stage) -> String {
    print_pipeline(&Pipeline {
        head: Head::Stage(stage.clone()),
        stages: Vec::new(),
        span: None,
    })
}

/// Prints text as a canonical text literal.
pub(crate) fn print_text(text: &str) -> String {
    print_expr(
        &Expr::Text {
            v: text.to_owned(),
            span: None,
        },
        Variant::A,
    )
}

/// Whether the text is an identifier of the language.
pub(crate) fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Builds a document of the statements.
pub(crate) fn document(statements: Vec<Statement>) -> AstDocument {
    AstDocument::new(Program { statements })
}

#[cfg(test)]
mod tests {
    use nurt_contracts::ast::{Arg, Let, Ref};

    use super::*;

    fn text(v: &str) -> Expr {
        Expr::Text {
            v: v.to_owned(),
            span: None,
        }
    }

    fn stage(name: &str, args: Vec<Arg>) -> Stage {
        Stage {
            name: name.to_owned(),
            args,
            span: None,
        }
    }

    fn positional(value: Expr) -> Arg {
        Arg {
            name: None,
            value,
            span: None,
        }
    }

    #[test]
    fn texts_use_escapes_or_raw_form() {
        assert_eq!(print_text("x\n"), "\"x\\n\"");
        assert_eq!(print_text("a\"b"), "r#\"a\"b\"#");
        assert_eq!(print_text("a\\d"), "r\"a\\d\"");
    }

    #[test]
    fn stages_and_pipelines_print_without_final_line_feed() {
        let find = stage("find", vec![positional(text("/a"))]);
        assert_eq!(print_stage(&find), "find \"/a\"");
        let pipeline = Pipeline {
            head: Head::Stage(find),
            stages: vec![stage("remove", Vec::new())],
            span: None,
        };
        assert_eq!(print_pipeline(&pipeline), "find \"/a\"\n|> remove");
        let bare = Pipeline {
            head: Head::Stage(stage("processes", Vec::new())),
            stages: vec![stage("count", Vec::new())],
            span: None,
        };
        assert_eq!(print_pipeline(&bare), "processes |> count");
    }

    #[test]
    fn documents_separate_statements_by_line_feeds() {
        let find = Pipeline {
            head: Head::Stage(stage("find", vec![positional(text("/a"))])),
            stages: Vec::new(),
            span: None,
        };
        let use_ref = Pipeline {
            head: Head::Ref(Ref {
                name: "x".to_owned(),
                fields: Vec::new(),
                span: None,
            }),
            stages: vec![stage("count", Vec::new())],
            span: None,
        };
        let statements = vec![
            Statement::Let(Let {
                name: "x".to_owned(),
                value: find,
                span: None,
            }),
            Statement::Pipeline(use_ref),
        ];
        assert_eq!(
            print_program(statements),
            "let x = find \"/a\"\n@x |> count"
        );
    }

    #[test]
    fn identifiers_are_lower_case_words() {
        assert!(is_identifier("x_1"));
        assert!(!is_identifier("X"));
        assert!(!is_identifier("1x"));
        assert!(!is_identifier(""));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
