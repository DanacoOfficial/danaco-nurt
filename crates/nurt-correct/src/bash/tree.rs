// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Construction of surface tree nodes and their canonical text.
//!
//! Translations report ready program text in corrections, fixes, next steps
//! and candidates. The text follows the canonical form of variant A: one
//! stage per line when the head has arguments and further stages follow,
//! literals in their canonical spelling.

use nurt_contracts::ast::{
    AstDocument, BinOp, Expr, Head, KeySeg, Let, MapEntry, Pipeline, Program, Ref, Stage,
    Statement, UnOp, Wildcard,
};

/// Column limit of a one-line pipeline or list.
const LINE_WIDTH: usize = 100;

/// Indentation of one nesting level of a multi-line list.
const INDENT: &str = "  ";

/// Prints a whole document in canonical text, statements separated by line
/// feeds, without a final line feed.
pub(crate) fn print_document(document: &AstDocument) -> String {
    print_program(&document.program)
}

pub(crate) fn print_program(program: &Program) -> String {
    program
        .statements
        .iter()
        .map(print_statement)
        .collect::<Vec<_>>()
        .join("\n")
}

fn print_statement(statement: &Statement) -> String {
    match statement {
        Statement::Let(binding) => print_let(binding),
        Statement::Pipeline(pipeline) => print_pipeline(pipeline, ""),
        _ => String::new(),
    }
}

fn print_let(binding: &Let) -> String {
    let prefix = format!("let {} = ", binding.name);
    print_pipeline(&binding.value, &prefix)
}

pub(crate) fn print_pipeline(pipeline: &Pipeline, prefix: &str) -> String {
    let head = print_head(&pipeline.head);
    let stages: Vec<String> = pipeline.stages.iter().map(print_stage).collect();
    let head_has_args = matches!(&pipeline.head, Head::Stage(stage) if !stage.args.is_empty());
    let one_line = std::iter::once(head.clone())
        .chain(stages.iter().cloned())
        .collect::<Vec<_>>()
        .join(" |> ");
    let multi_line = !stages.is_empty()
        && (head_has_args || prefix.chars().count() + one_line.chars().count() > LINE_WIDTH);
    if multi_line {
        let mut text = format!("{prefix}{head}");
        for stage in stages {
            text.push_str("\n|> ");
            text.push_str(&stage);
        }
        text
    } else {
        format!("{prefix}{one_line}")
    }
}

fn print_head(head: &Head) -> String {
    match head {
        Head::Stage(stage) => print_stage(stage),
        Head::Ref(reference) => print_ref(reference),
        _ => String::new(),
    }
}

pub(crate) fn print_stage(stage: &Stage) -> String {
    let mut text = stage.name.clone();
    for arg in &stage.args {
        text.push(' ');
        if let Some(name) = &arg.name {
            text.push_str(name);
            text.push_str(": ");
        }
        text.push_str(&print_expr(&arg.value, 0));
    }
    text
}

fn print_ref(reference: &Ref) -> String {
    let mut text = format!("@{}", reference.name);
    for field in &reference.fields {
        text.push('.');
        text.push_str(field);
    }
    text
}

/// Binding strength of comparison operators.
const COMPARISON_LEVEL: u8 = 4;

/// Binding strength of an expression, from loosest.
fn level(expr: &Expr) -> u8 {
    match expr {
        Expr::Bin { op, .. } => binary_level(*op),
        Expr::Un { op: UnOp::Not, .. } => 3,
        Expr::Un { .. } => 7,
        _ => 8,
    }
}

fn binary_level(op: BinOp) -> u8 {
    match op {
        BinOp::Or => 1,
        BinOp::And => 2,
        BinOp::Add | BinOp::Sub => 5,
        BinOp::Mul | BinOp::Div => 6,
        _ => COMPARISON_LEVEL,
    }
}

fn print_expr(expr: &Expr, indent: usize) -> String {
    match expr {
        Expr::Ref(reference) => print_ref(reference),
        Expr::Text { v, .. } => print_text(v),
        Expr::Ident { v, .. } => v.clone(),
        Expr::Regex { v, flags, .. } => format!("/{}/{flags}", v.replace('/', "\\/")),
        Expr::Int { v, .. } => v.to_string(),
        Expr::Num { v, .. } => print_number(*v),
        Expr::Size { v, .. } => print_size(*v),
        Expr::Dur { v, .. } => print_duration(*v),
        Expr::Bool { v, .. } => v.to_string(),
        Expr::Null { .. } => "null".to_owned(),
        Expr::List { items, .. } => print_list(items, indent),
        Expr::Map { entries, .. } => print_map(entries, indent),
        Expr::Spread { x, .. } => format!("...{}", print_expr(x, indent)),
        Expr::Field { name, of, .. } => match of {
            Some(of) => format!("{}.{name}", print_expr(of, indent)),
            None => name.clone(),
        },
        Expr::Call { function, args, .. } => {
            let args: Vec<String> = args.iter().map(|arg| print_expr(arg, indent)).collect();
            format!("{function}({})", args.join(", "))
        }
        Expr::Un { op, x, .. } => print_unary(*op, x, indent),
        Expr::Bin { op, l, r, .. } => print_binary(*op, l, r, indent),
        Expr::Range { lo, hi, .. } => format!(
            "{}..{}",
            lo.map(|v| v.to_string()).unwrap_or_default(),
            hi.map(|v| v.to_string()).unwrap_or_default()
        ),
        Expr::Sub { p, .. } => format!("({})", print_sub(p)),
        Expr::Key { segs, .. } => print_key(segs),
        _ => String::new(),
    }
}

fn print_sub(pipeline: &Pipeline) -> String {
    std::iter::once(print_head(&pipeline.head))
        .chain(pipeline.stages.iter().map(print_stage))
        .collect::<Vec<_>>()
        .join(" |> ")
}

fn print_unary(op: UnOp, operand: &Expr, indent: usize) -> String {
    let inner = print_expr(operand, indent);
    match op {
        UnOp::Not if level(operand) <= 3 => format!("not ({inner})"),
        UnOp::Not => format!("not {inner}"),
        _ if level(operand) == 7 => format!("-({inner})"),
        _ => format!("-{inner}"),
    }
}

fn print_binary(op: BinOp, left: &Expr, right: &Expr, indent: usize) -> String {
    let own = binary_level(op);
    let comparison = own == COMPARISON_LEVEL;
    let wrap_left = level(left) < own || (comparison && level(left) == 4);
    let wrap_right = level(right) <= own;
    let left = wrap(print_expr(left, indent), wrap_left);
    let right = wrap(print_expr(right, indent), wrap_right);
    format!("{left} {} {right}", op.as_str())
}

fn wrap(text: String, parens: bool) -> String {
    if parens { format!("({text})") } else { text }
}

fn print_list(items: &[Expr], indent: usize) -> String {
    let printed: Vec<String> = items
        .iter()
        .map(|item| print_expr(item, indent + 1))
        .collect();
    let one_line = format!("[{}]", printed.join(", "));
    let nested = items.len() >= 2
        && items
            .iter()
            .any(|item| matches!(item, Expr::List { .. } | Expr::Map { .. }));
    if !nested && one_line.chars().count() <= LINE_WIDTH {
        return one_line;
    }
    let inner = INDENT.repeat(indent + 1);
    let outer = INDENT.repeat(indent);
    let body: Vec<String> = printed
        .iter()
        .map(|item| format!("{inner}{item}"))
        .collect();
    format!("[\n{}\n{outer}]", body.join("\n"))
}

fn print_map(entries: &[MapEntry], indent: usize) -> String {
    let printed: Vec<String> = entries
        .iter()
        .map(|entry| {
            format!(
                "{}: {}",
                print_key_name(&entry.k),
                print_expr(&entry.v, indent + 1)
            )
        })
        .collect();
    format!("{{{}}}", printed.join(", "))
}

fn print_key_name(key: &str) -> String {
    if is_identifier(key) {
        key.to_owned()
    } else {
        print_text(key)
    }
}

fn print_key(segments: &[KeySeg]) -> String {
    let mut text = String::new();
    for segment in segments {
        match segment {
            KeySeg::Name(name) => {
                if !text.is_empty() {
                    text.push('.');
                }
                text.push_str(name);
            }
            KeySeg::Index(index) => text.push_str(&format!("[{index}]")),
            KeySeg::Wild { wild } => text.push_str(match wild {
                Wildcard::Level => "[*]",
                Wildcard::Append => "[+]",
                _ => ".**",
            }),
            _ => {}
        }
    }
    print_text(&text)
}

/// Whether the text is an identifier of the language.
pub(crate) fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Prints text in canonical form: a raw literal when the text contains `"`
/// or `\` and no control character other than tab and line feed, an
/// escaped literal otherwise.
pub(crate) fn print_text(text: &str) -> String {
    let needs_raw = text.contains(['"', '\\']);
    let raw_allowed = !text
        .chars()
        .any(|c| c.is_control() && c != '\t' && c != '\n');
    if needs_raw && raw_allowed {
        let mut hashes = 0;
        while text.contains(&format!("\"{}", "#".repeat(hashes))) {
            hashes += 1;
        }
        let fence = "#".repeat(hashes);
        return format!("r{fence}\"{text}\"{fence}");
    }
    let mut escaped = String::with_capacity(text.len() + 2);
    escaped.push('"');
    for c in text.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\0' => escaped.push_str("\\0"),
            c if c.is_control() => escaped.push_str(&format!("\\u{{{:x}}}", u32::from(c))),
            c => escaped.push(c),
        }
    }
    escaped.push('"');
    escaped
}

fn print_number(value: f64) -> String {
    let text = value.to_string();
    if text.contains('.') {
        text
    } else {
        format!("{text}.0")
    }
}

/// Size units in the order used to break ties: decimal before binary,
/// larger before smaller.
const SIZE_UNITS: &[(&str, u64)] = &[
    ("tb", 1_000_000_000_000),
    ("gb", 1_000_000_000),
    ("mb", 1_000_000),
    ("kb", 1_000),
    ("b", 1),
    ("tib", 1 << 40),
    ("gib", 1 << 30),
    ("mib", 1 << 20),
    ("kib", 1 << 10),
];

/// Fraction digits allowed in a size literal.
const SIZE_FRACTION: u128 = 1_000;

pub(crate) fn print_size(bytes: u64) -> String {
    if bytes == 0 {
        return "0b".to_owned();
    }
    let mut best: Option<String> = None;
    for &(unit, factor) in SIZE_UNITS {
        let Some(candidate) = size_form(bytes, unit, factor) else {
            continue;
        };
        if best.as_ref().is_none_or(|b| candidate.len() < b.len()) {
            best = Some(candidate);
        }
    }
    best.unwrap_or_else(|| format!("{bytes}b"))
}

fn size_form(bytes: u64, unit: &str, factor: u64) -> Option<String> {
    if bytes.is_multiple_of(factor) {
        return Some(format!("{}{unit}", bytes / factor));
    }
    let scaled = u128::from(bytes) * SIZE_FRACTION;
    let factor = u128::from(factor);
    if !scaled.is_multiple_of(factor) {
        return None;
    }
    let thousandths = scaled / factor;
    let whole = thousandths / SIZE_FRACTION;
    let fraction = format!("{:03}", thousandths % SIZE_FRACTION);
    Some(format!("{whole}.{}{unit}", fraction.trim_end_matches('0')))
}

const DURATION_UNITS: &[(&str, u64)] = &[
    ("d", 86_400_000),
    ("h", 3_600_000),
    ("m", 60_000),
    ("s", 1_000),
    ("ms", 1),
];

pub(crate) fn print_duration(millis: u64) -> String {
    if millis == 0 {
        return "0s".to_owned();
    }
    let mut rest = millis;
    let mut text = String::new();
    for &(unit, factor) in DURATION_UNITS {
        if rest >= factor {
            text.push_str(&format!("{}{unit}", rest / factor));
            rest %= factor;
        }
    }
    text
}

pub(crate) fn document(statements: Vec<Statement>) -> AstDocument {
    AstDocument::new(Program { statements })
}

#[cfg(test)]
mod tests {
    use nurt_contracts::ast::Arg;

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

    fn named(name: &str, value: Expr) -> Arg {
        Arg {
            name: Some(name.to_owned()),
            value,
            span: None,
        }
    }

    #[test]
    fn texts_use_escapes_or_raw_form() {
        assert_eq!(print_text("x\n"), "\"x\\n\"");
        assert_eq!(print_text("a\"b"), "r#\"a\"b\"#");
        assert_eq!(print_text("a\\d"), "r\"a\\d\"");
        assert_eq!(print_text("a\"#b"), "r##\"a\"#b\"##");
        assert_eq!(print_text("\u{1}\\"), "\"\\u{1}\\\\\"");
    }

    #[test]
    fn sizes_take_the_shortest_exact_form() {
        assert_eq!(print_size(0), "0b");
        assert_eq!(print_size(1_000_000), "1mb");
        assert_eq!(print_size(1_048_576), "1mib");
        assert_eq!(print_size(10 * 1_048_576), "10mib");
        assert_eq!(print_size(1_500_000_000), "1.5gb");
        assert_eq!(print_size(512), "512b");
        assert_eq!(print_size(1_024_000), "1024kb");
        assert_eq!(print_size(1_572_864), "1.5mib");
    }

    #[test]
    fn durations_are_greedy() {
        assert_eq!(print_duration(0), "0s");
        assert_eq!(print_duration(86_400_000), "1d");
        assert_eq!(print_duration(90_000), "1m30s");
        assert_eq!(print_duration(500), "500ms");
    }

    #[test]
    fn pipelines_with_arguments_break_lines() {
        let pipeline = Pipeline {
            head: Head::Stage(stage("find", vec![positional(text("/a"))])),
            stages: vec![stage("remove", vec![])],
            span: None,
        };
        assert_eq!(print_pipeline(&pipeline, ""), "find \"/a\"\n|> remove");
        let bare = Pipeline {
            head: Head::Stage(stage("processes", vec![])),
            stages: vec![stage("count", vec![])],
            span: None,
        };
        assert_eq!(print_pipeline(&bare, ""), "processes |> count");
    }

    #[test]
    fn expressions_get_parentheses_only_when_needed() {
        let size = Expr::Ident {
            v: "size".to_owned(),
            span: None,
        };
        let gt = Expr::Bin {
            op: BinOp::Gt,
            l: Box::new(size.clone()),
            r: Box::new(Expr::Size {
                v: 1000,
                span: None,
            }),
            span: None,
        };
        let both = Expr::Bin {
            op: BinOp::And,
            l: Box::new(gt.clone()),
            r: Box::new(gt.clone()),
            span: None,
        };
        assert_eq!(print_expr(&both, 0), "size > 1kb and size > 1kb");
        let negated = Expr::Un {
            op: UnOp::Not,
            x: Box::new(both),
            span: None,
        };
        assert_eq!(print_expr(&negated, 0), "not (size > 1kb and size > 1kb)");
        let simple_not = Expr::Un {
            op: UnOp::Not,
            x: Box::new(size),
            span: None,
        };
        assert_eq!(print_expr(&simple_not, 0), "not size");
    }

    #[test]
    fn lists_maps_and_references() {
        let list = Expr::List {
            items: vec![text("go"), text("test")],
            span: None,
        };
        assert_eq!(print_expr(&list, 0), "[\"go\", \"test\"]");
        let map = Expr::Map {
            entries: vec![
                MapEntry {
                    k: "VAR".to_owned(),
                    v: text("x"),
                    span: None,
                },
                MapEntry {
                    k: "mode".to_owned(),
                    v: text("y"),
                    span: None,
                },
            ],
            span: None,
        };
        assert_eq!(print_expr(&map, 0), "{\"VAR\": \"x\", mode: \"y\"}");
        let reference = Ref {
            name: "x".to_owned(),
            fields: vec!["path".to_owned()],
            span: None,
        };
        assert_eq!(print_ref(&reference), "@x.path");
        let spread = Expr::Spread {
            x: Box::new(Expr::Ref(reference)),
            span: None,
        };
        assert_eq!(print_expr(&spread, 0), "...@x.path");
    }

    #[test]
    fn nested_lists_print_one_item_per_line() {
        let inner = Expr::List {
            items: vec![text("a")],
            span: None,
        };
        let outer = Expr::List {
            items: vec![inner.clone(), inner],
            span: None,
        };
        assert_eq!(print_expr(&outer, 0), "[\n  [\"a\"]\n  [\"a\"]\n]");
    }

    #[test]
    fn statements_and_lets() {
        let find = Pipeline {
            head: Head::Stage(stage(
                "find",
                vec![positional(text("/a")), named("glob", text("*.go"))],
            )),
            stages: Vec::new(),
            span: None,
        };
        let run = Pipeline {
            head: Head::Stage(stage(
                "run",
                vec![positional(Expr::List {
                    items: vec![
                        text("wc"),
                        Expr::Spread {
                            x: Box::new(Expr::Ref(Ref {
                                name: "x".to_owned(),
                                fields: vec!["path".to_owned()],
                                span: None,
                            })),
                            span: None,
                        },
                    ],
                    span: None,
                })],
            )),
            stages: Vec::new(),
            span: None,
        };
        let doc = document(vec![
            Statement::Let(Let {
                name: "x".to_owned(),
                value: find,
                span: None,
            }),
            Statement::Pipeline(run),
        ]);
        assert_eq!(
            print_document(&doc),
            "let x = find \"/a\" glob: \"*.go\"\nrun [\"wc\", ...@x.path]"
        );
    }

    #[test]
    fn other_literals() {
        assert_eq!(
            print_expr(
                &Expr::Regex {
                    v: "a/b".to_owned(),
                    flags: "i".to_owned(),
                    span: None
                },
                0
            ),
            "/a\\/b/i"
        );
        assert_eq!(
            print_expr(
                &Expr::Range {
                    lo: None,
                    hi: Some(10),
                    span: None
                },
                0
            ),
            "..10"
        );
        assert_eq!(print_expr(&Expr::Num { v: 3.0, span: None }, 0), "3.0");
        assert_eq!(print_expr(&Expr::Null { span: None }, 0), "null");
        let call = Expr::Call {
            function: "ago".to_owned(),
            args: vec![Expr::Dur {
                v: 86_400_000,
                span: None,
            }],
            span: None,
        };
        assert_eq!(print_expr(&call, 0), "ago(1d)");
        let key = Expr::Key {
            segs: vec![KeySeg::Name("a".to_owned()), KeySeg::Index(0)],
            span: None,
        };
        assert_eq!(print_expr(&key, 0), "\"a[0]\"");
    }
}
