// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-plan::text
// Responsibility:   Program text of bound pipelines, used for the calls proposed in `next_steps`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Program text of bound pipelines, used for the calls proposed in
//! `next_steps`.
//!
//! The text is written in the word notation: the command name followed by
//! the arguments the program gave, each as `name: value`. Arguments filled
//! in from catalog defaults are left out, so the text binds to the same
//! pipeline.

use nurt_contracts::bound::{BoundExpr, BoundMapEntry, BoundPipeline, BoundRef, BoundStage};

const PIPE: &str = " |> ";

/// Text of a whole pipeline; `None` when a value has no text form.
pub(crate) fn pipeline(pipeline: &BoundPipeline) -> Option<String> {
    let (head, first) = match &pipeline.head {
        nurt_contracts::bound::BoundHead::Stage(stage) => (None, Some(stage)),
        nurt_contracts::bound::BoundHead::Ref(reference) => (Some(reference), None),
        _ => return None,
    };
    let stages: Vec<&BoundStage> = first.into_iter().chain(&pipeline.stages).collect();
    stages_text(head, &stages)
}

/// Text of a pipeline made of an optional named result and stages.
pub(crate) fn stages_text(head: Option<&BoundRef>, stages: &[&BoundStage]) -> Option<String> {
    let mut parts = Vec::with_capacity(stages.len() + 1);
    if let Some(reference) = head {
        parts.push(reference_text(reference));
    }
    for stage in stages {
        parts.push(stage_text(stage)?);
    }
    Some(parts.join(PIPE))
}

fn stage_text(stage: &BoundStage) -> Option<String> {
    let mut text = stage.name.clone();
    for arg in &stage.args {
        if arg.default || arg.value == BoundExpr::Null {
            continue;
        }
        text.push(' ');
        text.push_str(&arg.name);
        text.push_str(": ");
        text.push_str(&expr(&arg.value)?);
    }
    Some(text)
}

fn reference_text(reference: &BoundRef) -> String {
    let mut text = format!("@{}", reference.name);
    for field in &reference.fields {
        text.push('.');
        text.push_str(field);
    }
    text
}

fn quoted(text: &str) -> Option<String> {
    serde_json::to_string(text).ok()
}

fn joined(items: &[BoundExpr]) -> Option<String> {
    let parts: Option<Vec<String>> = items.iter().map(expr).collect();
    Some(parts?.join(", "))
}

fn map_text(entries: &[BoundMapEntry]) -> Option<String> {
    let parts: Option<Vec<String>> = entries
        .iter()
        .map(|entry| Some(format!("{}: {}", entry.k, expr(&entry.v)?)))
        .collect();
    Some(format!("{{{}}}", parts?.join(", ")))
}

fn range_text(lo: Option<i64>, hi: Option<i64>) -> String {
    match (lo, hi) {
        (Some(lo), Some(hi)) if lo == hi => lo.to_string(),
        (Some(lo), Some(hi)) => format!("{lo}..{hi}"),
        (Some(lo), None) => format!("{lo}.."),
        (None, Some(hi)) => format!("..{hi}"),
        (None, None) => "all".to_owned(),
    }
}

/// Operand of an operator: binary operations are parenthesized.
fn operand(value: &BoundExpr) -> Option<String> {
    match value {
        BoundExpr::Bin { .. } => Some(format!("({})", expr(value)?)),
        other => expr(other),
    }
}

fn expr(value: &BoundExpr) -> Option<String> {
    match value {
        BoundExpr::Null => Some("null".to_owned()),
        BoundExpr::Bool { v } => Some(v.to_string()),
        BoundExpr::Int { v } => Some(v.to_string()),
        BoundExpr::Num { v } => Some(v.to_string()),
        BoundExpr::Text { v }
        | BoundExpr::Path { v }
        | BoundExpr::Glob { v }
        | BoundExpr::Key { v }
        | BoundExpr::Instant { v } => quoted(v),
        BoundExpr::Enum { v } => Some(v.clone()),
        BoundExpr::Size { v } => Some(format!("{v}b")),
        BoundExpr::Dur { v } => Some(format!("{v}ms")),
        BoundExpr::Regex { v, flags } => Some(format!("/{}/{flags}", v.replace('/', "\\/"))),
        BoundExpr::List { items } => Some(format!("[{}]", joined(items)?)),
        BoundExpr::Map { entries } => map_text(entries),
        BoundExpr::Range { lo, hi } => Some(range_text(*lo, *hi)),
        BoundExpr::Ref(reference) => Some(reference_text(reference)),
        BoundExpr::Sub { p } => Some(format!("({})", pipeline(p)?)),
        BoundExpr::Field { name, of } => match of {
            Some(base) => Some(format!("{}.{name}", operand(base)?)),
            None => Some(name.clone()),
        },
        BoundExpr::Call { function, args } => Some(format!("{function}({})", joined(args)?)),
        BoundExpr::Un { op, x } => {
            let symbol = op.as_str();
            let separator = if symbol.chars().all(char::is_alphabetic) {
                " "
            } else {
                ""
            };
            Some(format!("{symbol}{separator}{}", operand(x)?))
        }
        BoundExpr::Bin { op, l, r } => {
            Some(format!("{} {} {}", operand(l)?, op.as_str(), operand(r)?))
        }
        BoundExpr::Spread { x } => Some(format!("...{}", operand(x)?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "tests fail by panicking"
    )]

    use nurt_contracts::ast::{BinOp, UnOp};
    use nurt_contracts::bound::{BoundArg, BoundHead, Phase};
    use nurt_contracts::kinds::EffectKind;

    use super::*;

    fn stage(name: &str, args: Vec<BoundArg>) -> BoundStage {
        BoundStage {
            name: name.to_owned(),
            effect: EffectKind::Read,
            phase: Phase::Transform,
            waits: false,
            args,
            span: None,
        }
    }

    fn field(name: &str) -> BoundExpr {
        BoundExpr::Field {
            name: name.to_owned(),
            of: None,
        }
    }

    #[test]
    fn writes_given_arguments_and_skips_defaults() {
        let find = stage(
            "find",
            vec![
                BoundArg::given(
                    "path",
                    BoundExpr::Path {
                        v: "/srv/my app".to_owned(),
                    },
                ),
                BoundArg::from_default("hidden", BoundExpr::Bool { v: true }),
                BoundArg::given("depth", BoundExpr::Null),
            ],
        );
        let first = stage(
            "first",
            vec![BoundArg::given("count", BoundExpr::Int { v: 10 })],
        );
        let pipeline = BoundPipeline {
            head: BoundHead::Stage(find),
            stages: vec![first],
            span: None,
        };
        assert_eq!(
            super::pipeline(&pipeline).unwrap(),
            "find path: \"/srv/my app\" |> first count: 10"
        );
    }

    #[test]
    fn writes_expressions_with_parentheses() {
        let condition = BoundExpr::Bin {
            op: BinOp::And,
            l: Box::new(BoundExpr::Bin {
                op: BinOp::Gt,
                l: Box::new(field("size")),
                r: Box::new(BoundExpr::Size { v: 1_000 }),
            }),
            r: Box::new(BoundExpr::Un {
                op: UnOp::Not,
                x: Box::new(field("hidden")),
            }),
        };
        assert_eq!(expr(&condition).unwrap(), "(size > 1000b) and not hidden");
        let negative = BoundExpr::Un {
            op: UnOp::Neg,
            x: Box::new(BoundExpr::Int { v: 3 }),
        };
        assert_eq!(expr(&negative).unwrap(), "-3");
    }

    #[test]
    fn writes_literals_of_every_kind() {
        let cases = [
            (BoundExpr::Null, "null"),
            (BoundExpr::Num { v: 2.5 }, "2.5"),
            (BoundExpr::Dur { v: 3_000 }, "3000ms"),
            (
                BoundExpr::Enum {
                    v: "desc".to_owned(),
                },
                "desc",
            ),
            (
                BoundExpr::Regex {
                    v: "a/b".to_owned(),
                    flags: "i".to_owned(),
                },
                "/a\\/b/i",
            ),
            (
                BoundExpr::Range {
                    lo: Some(0),
                    hi: Some(0),
                },
                "0",
            ),
            (
                BoundExpr::Range {
                    lo: Some(1),
                    hi: Some(5),
                },
                "1..5",
            ),
            (
                BoundExpr::Range {
                    lo: Some(40),
                    hi: None,
                },
                "40..",
            ),
            (
                BoundExpr::Range {
                    lo: None,
                    hi: Some(9),
                },
                "..9",
            ),
            (BoundExpr::Range { lo: None, hi: None }, "all"),
            (
                BoundExpr::List {
                    items: vec![
                        BoundExpr::Text { v: "go".to_owned() },
                        BoundExpr::Text {
                            v: "test".to_owned(),
                        },
                    ],
                },
                "[\"go\", \"test\"]",
            ),
            (
                BoundExpr::Map {
                    entries: vec![BoundMapEntry {
                        k: "lang".to_owned(),
                        v: BoundExpr::Text { v: "C".to_owned() },
                    }],
                },
                "{lang: \"C\"}",
            ),
            (
                BoundExpr::Ref(BoundRef {
                    name: "recent".to_owned(),
                    fields: vec!["path".to_owned()],
                }),
                "@recent.path",
            ),
            (
                BoundExpr::Call {
                    function: "ago".to_owned(),
                    args: vec![BoundExpr::Dur { v: 60_000 }],
                },
                "ago(60000ms)",
            ),
            (
                BoundExpr::Field {
                    name: "name".to_owned(),
                    of: Some(Box::new(field("parent"))),
                },
                "parent.name",
            ),
            (
                BoundExpr::Spread {
                    x: Box::new(field("paths")),
                },
                "...paths",
            ),
        ];
        for (value, text) in cases {
            assert_eq!(expr(&value).unwrap(), text);
        }
    }

    #[test]
    fn writes_sub_pipelines_and_named_heads() {
        let count = stage("count", Vec::new());
        let pipeline = BoundPipeline {
            head: BoundHead::Ref(BoundRef {
                name: "recent".to_owned(),
                fields: Vec::new(),
            }),
            stages: vec![count],
            span: None,
        };
        let sub = BoundExpr::Sub { p: pipeline };
        assert_eq!(expr(&sub).unwrap(), "(@recent |> count)");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
