// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Static check of example programs against the command catalog.
//!
//! The check stands in for `nurt check` until the executor is available to the
//! documentation build. It splits a program into pipelines and stages, binds
//! the arguments of every stage to the parameters of the catalog and verifies
//! command names, parameter names, enumeration values, literal kinds, required
//! parameters and the compatibility of the record kinds between stages. The
//! check does not evaluate expressions and does not know record fields.

use std::collections::HashSet;

use nurt_contracts::bound::STAGE_CALL_PARAMETERS;
use nurt_contracts::spec::{ParamSpec, ParamType, ScalarType, StageSpec};

use crate::catalog::Catalog;
use crate::lexer::{Kind, Token, tokenize};

/// Problem found in a program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckIssue {
    /// Stable code of the problem, in the style of the language error codes.
    pub code: &'static str,
    /// Description of the problem.
    pub message: String,
}

impl CheckIssue {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CheckIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// Checker of programs: the interface used by the documentation checks.
pub trait ProgramChecker {
    /// Checks a program and returns the problems found; an empty list means
    /// the program is valid.
    fn check(&self, program: &str) -> Vec<CheckIssue>;
}

/// Checker that binds programs against a catalog without running them.
#[derive(Debug, Clone, Copy)]
pub struct CatalogChecker<'a> {
    catalog: &'a Catalog,
}

impl<'a> CatalogChecker<'a> {
    /// Creates a checker for a catalog.
    #[must_use]
    pub fn new(catalog: &'a Catalog) -> Self {
        Self { catalog }
    }
}

/// Class of an argument value, as far as it is known without evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Class {
    Text,
    Regex,
    Int,
    Number,
    Bool,
    Size,
    Duration,
    Range,
    Word(String),
    Null,
    List,
    Map,
    Group,
    Other,
}

struct Arg {
    name: Option<String>,
    tokens: Vec<Token>,
}

struct Stage {
    head: Option<Token>,
    args: Vec<Arg>,
}

fn is_open(kind: &Kind) -> bool {
    matches!(kind, Kind::Punct('(' | '[' | '{'))
}

fn is_close(kind: &Kind) -> bool {
    matches!(kind, Kind::Punct(')' | ']' | '}'))
}

fn matching(open: char) -> char {
    match open {
        '(' => ')',
        '[' => ']',
        _ => '}',
    }
}

/// Removes line breaks that continue a pipeline or sit inside brackets and
/// verifies that brackets are balanced.
fn normalize(tokens: Vec<Token>, issues: &mut Vec<CheckIssue>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    let mut stack: Vec<char> = Vec::new();
    let mut iter = tokens.into_iter().peekable();
    while let Some(token) = iter.next() {
        match &token.kind {
            Kind::Punct(c @ ('(' | '[' | '{')) => stack.push(matching(*c)),
            Kind::Punct(c @ (')' | ']' | '}')) => {
                if stack.pop() != Some(*c) {
                    issues.push(CheckIssue::new("E_SYNTAX", format!("unbalanced {c}")));
                }
            }
            Kind::Newline => {
                let next_is_pipe = iter.peek().is_some_and(|t| t.kind == Kind::Pipe);
                let prev_is_pipe = out.last().is_some_and(|t| t.kind == Kind::Pipe);
                let blank = out.last().is_none_or(|t| t.kind == Kind::Newline);
                if !stack.is_empty() || next_is_pipe || prev_is_pipe || blank {
                    continue;
                }
            }
            _ => {}
        }
        out.push(token);
    }
    if !stack.is_empty() {
        issues.push(CheckIssue::new("E_SYNTAX", "unclosed bracket"));
    }
    out
}

fn split_top_level(tokens: &[Token], separator: &Kind) -> Vec<Vec<Token>> {
    let mut parts: Vec<Vec<Token>> = vec![Vec::new()];
    let mut depth = 0usize;
    for token in tokens {
        if is_open(&token.kind) {
            depth += 1;
        } else if is_close(&token.kind) {
            depth = depth.saturating_sub(1);
        }
        if depth == 0 && token.kind == *separator {
            parts.push(Vec::new());
        } else if let Some(last) = parts.last_mut() {
            last.push(token.clone());
        }
    }
    parts
}

/// Returns the index after the bracket group that starts at `start`.
fn group_end(tokens: &[Token], start: usize) -> usize {
    let mut depth = 0usize;
    for (offset, token) in tokens[start..].iter().enumerate() {
        if is_open(&token.kind) {
            depth += 1;
        } else if is_close(&token.kind) {
            depth -= 1;
            if depth == 0 {
                return start + offset + 1;
            }
        }
    }
    tokens.len()
}

fn is_binary_word(word: &str) -> bool {
    matches!(word, "and" | "or" | "in")
}

/// Returns the index after the atom (with its prefix operators) at `start`.
fn atom_end(tokens: &[Token], start: usize) -> usize {
    let mut i = start;
    while i < tokens.len() {
        match &tokens[i].kind {
            Kind::Operator(op) if op == "-" => i += 1,
            Kind::Word(word) if word == "not" => i += 1,
            _ => break,
        }
    }
    let Some(token) = tokens.get(i) else {
        return tokens.len();
    };
    if is_open(&token.kind) {
        return group_end(tokens, i);
    }
    if matches!(token.kind, Kind::Word(_))
        && tokens
            .get(i + 1)
            .is_some_and(|next| next.kind == Kind::Punct('(') && !next.space_before)
    {
        return group_end(tokens, i + 1);
    }
    i + 1
}

/// Returns the index after the expression that starts at `start`.
fn expression_end(tokens: &[Token], start: usize) -> usize {
    let mut i = atom_end(tokens, start);
    while let Some(token) = tokens.get(i) {
        let names_argument = tokens
            .get(i + 1)
            .is_some_and(|next| next.kind == Kind::Punct(':') && !next.space_before);
        let binary = match &token.kind {
            Kind::Operator(_) => true,
            Kind::Word(word) => is_binary_word(word) && !names_argument,
            _ => false,
        };
        if !binary {
            break;
        }
        i = atom_end(tokens, i + 1);
    }
    i.min(tokens.len())
}

fn split_args(tokens: &[Token]) -> Vec<Arg> {
    let mut args = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let named = matches!(tokens[i].kind, Kind::Word(_))
            && tokens
                .get(i + 1)
                .is_some_and(|t| t.kind == Kind::Punct(':') && !t.space_before);
        let (name, start) = match (&tokens[i].kind, named) {
            (Kind::Word(word), true) => (Some(word.clone()), i + 2),
            _ => (None, i),
        };
        let end = expression_end(tokens, start)
            .max(start + 1)
            .min(tokens.len());
        args.push(Arg {
            name,
            tokens: tokens[start.min(tokens.len())..end].to_vec(),
        });
        i = end;
    }
    args
}

fn parse_stage(tokens: &[Token]) -> Stage {
    let (head, rest) = match tokens.split_first() {
        Some((head, rest)) => (Some(head.clone()), rest),
        None => (None, &[][..]),
    };
    Stage {
        head,
        args: split_args(rest),
    }
}

fn classify(tokens: &[Token]) -> Class {
    let [token] = tokens else {
        return match tokens.first().map(|t| &t.kind) {
            Some(Kind::Punct('[')) if group_end(tokens, 0) == tokens.len() => Class::List,
            Some(Kind::Punct('{')) if group_end(tokens, 0) == tokens.len() => Class::Map,
            Some(Kind::Punct('(')) if group_end(tokens, 0) == tokens.len() => Class::Group,
            _ => Class::Other,
        };
    };
    match &token.kind {
        Kind::Text(_) => Class::Text,
        Kind::Regex(_) => Class::Regex,
        Kind::Int(_) => Class::Int,
        Kind::Number(_) => Class::Number,
        Kind::Size(_) => Class::Size,
        Kind::Duration(_) => Class::Duration,
        Kind::Range(_) => Class::Range,
        Kind::Word(word) => match word.as_str() {
            "true" | "false" => Class::Bool,
            "null" => Class::Null,
            _ => Class::Word(word.clone()),
        },
        _ => Class::Other,
    }
}

fn tag(class: &Class) -> &'static str {
    match class {
        Class::Text => "text",
        Class::Regex => "regex",
        Class::Int => "int",
        Class::Number => "number",
        Class::Bool => "bool",
        Class::Size => "size",
        Class::Duration => "duration",
        Class::Range => "range",
        Class::Word(word) if word == "all" => "all",
        Class::Word(_) => "word",
        Class::Null => "null",
        Class::List => "list",
        Class::Map => "map",
        Class::Group => "group",
        Class::Other => "other",
    }
}

fn scalar_accepts(scalar: ScalarType, class: &Class) -> bool {
    use ScalarType as S;
    let accepted: &[&str] = match scalar {
        S::Text | S::Path | S::Glob | S::Key | S::Percent | S::Instant | S::Bytes => &["text"],
        S::Regex => &["regex"],
        S::Int => &["int"],
        S::Number => &["int", "number"],
        S::Bool => &["bool"],
        S::Size => &["size", "text"],
        S::Duration => &["duration", "text"],
        S::Range => &["range", "int", "all"],
        S::Pipeline => &["group"],
        S::Field => &["word", "all"],
        _ => return true,
    };
    accepted.contains(&tag(class))
}

fn accepts(ty: &ParamType, class: &Class) -> bool {
    if matches!(class, Class::Other) {
        return true;
    }
    ty.alternatives()
        .iter()
        .any(|alternative| match alternative {
            ParamType::Scalar(scalar) => scalar_accepts(*scalar, class),
            ParamType::Enum(values) => {
                matches!(class, Class::Word(word) if values.iter().any(|v| v == word))
            }
            ParamType::List(inner) => matches!(class, Class::List) || accepts(inner, class),
            ParamType::Map(_) => matches!(class, Class::Map),
            ParamType::Stream(_) => matches!(class, Class::Other),
            _ => true,
        })
}

fn describe(class: &Class) -> String {
    match class {
        Class::Word(word) => format!("the word {word}"),
        other => format!("{other:?}").to_lowercase(),
    }
}

/// Assigns positional arguments to positional parameters.
///
/// A repeated parameter takes the arguments that the parameters before and
/// after it do not take; with fewer arguments than parameters, the repeated
/// parameter takes the first one.
fn assign_positional<'s>(positional: &[&'s ParamSpec], count: usize) -> Vec<Option<&'s ParamSpec>> {
    let Some(repeated) = positional.iter().position(|param| param.repeated) else {
        return (0..count).map(|i| positional.get(i).copied()).collect();
    };
    let after = positional.len() - repeated - 1;
    let trailing = count > repeated + after;
    (0..count)
        .map(|i| {
            if i < repeated {
                positional.get(i).copied()
            } else if trailing && i >= count - after {
                positional.get(repeated + 1 + i - (count - after)).copied()
            } else {
                positional.get(repeated).copied()
            }
        })
        .collect()
}

struct Binder<'a> {
    checker: &'a CatalogChecker<'a>,
    defined: HashSet<String>,
    issues: Vec<CheckIssue>,
}

fn output_kinds(
    spec: &StageSpec,
    form_output: Option<&[String]>,
    previous: &[String],
) -> Vec<String> {
    let declared: Vec<String> = match form_output {
        Some(output) if !output.is_empty() => output.to_vec(),
        _ => spec.output.iter().map(ToString::to_string).collect(),
    };
    let mut out = Vec::new();
    for kind in declared {
        if kind == "same" {
            out.extend(previous.iter().cloned());
        } else {
            out.push(kind);
        }
    }
    out
}

fn input_compatible(accepted: &[String], previous: &[String]) -> bool {
    accepted.iter().any(|kind| kind == "any")
        || previous.iter().any(|kind| kind == "any")
        || accepted.iter().any(|kind| previous.contains(kind))
}

impl Binder<'_> {
    fn issue(&mut self, code: &'static str, message: impl Into<String>) {
        self.issues.push(CheckIssue::new(code, message));
    }

    fn statement(&mut self, tokens: &[Token]) {
        let mut tokens = tokens;
        let mut binding: Option<String> = None;
        if let [first, name, eq, ..] = tokens
            && first.kind == Kind::Word("let".to_string())
            && let Kind::Word(name) = &name.kind
            && eq.kind == Kind::Operator("=".to_string())
        {
            binding = Some(name.clone());
            tokens = &tokens[3..];
        }
        self.pipeline(tokens, false);
        if let Some(name) = binding {
            self.defined.insert(name);
        }
    }

    fn pipeline(&mut self, tokens: &[Token], nested: bool) {
        let stages: Vec<Stage> = split_top_level(tokens, &Kind::Pipe)
            .iter()
            .map(|part| parse_stage(part))
            .collect();
        let mut previous: Vec<String> = Vec::new();
        for (position, stage) in stages.iter().enumerate() {
            let head_position = position == 0 && !nested;
            previous = self.stage(stage, position == 0, head_position, &previous);
        }
    }

    fn stage(
        &mut self,
        stage: &Stage,
        first: bool,
        source_position: bool,
        previous: &[String],
    ) -> Vec<String> {
        let Some(head) = &stage.head else {
            self.issue("E_SYNTAX", "empty pipeline stage");
            return Vec::new();
        };
        match &head.kind {
            Kind::Reference(name) if first => {
                let base = name.split('.').next().unwrap_or(name);
                if !self.defined.contains(base) {
                    self.issue("E_UNKNOWN_NAME", format!("unknown name @{base}"));
                }
                vec!["any".to_string()]
            }
            Kind::Word(name) => self.command_stage(name, stage, source_position, previous),
            _ => {
                self.issue("E_SYNTAX", "a stage must start with a command name");
                Vec::new()
            }
        }
    }

    fn command_stage(
        &mut self,
        name: &str,
        stage: &Stage,
        source_position: bool,
        previous: &[String],
    ) -> Vec<String> {
        let Some(spec) = self.checker.catalog.command(name) else {
            self.issue("E_UNKNOWN_COMMAND", format!("unknown command {name}"));
            return vec!["any".to_string()];
        };
        let input: Vec<String> = spec.input.iter().map(ToString::to_string).collect();
        if source_position {
            if !input.iter().any(|kind| kind == "none") {
                self.issue(
                    "E_STAGE_INPUT",
                    format!("{name} needs input and cannot start a program"),
                );
            }
        } else if !previous.is_empty() && !input_compatible(&input, previous) {
            self.issue(
                "E_STAGE_INPUT",
                format!(
                    "{name} does not accept the records {} of the previous stage",
                    previous.join("|")
                ),
            );
        }
        let form = self.select_form(spec, source_position, previous);
        let (positional, form_output) = match form {
            Some(form) => {
                let names: Vec<&ParamSpec> = form
                    .positional
                    .iter()
                    .filter_map(|n| spec.param(n))
                    .collect();
                let output: Vec<String> = form.output.iter().map(ToString::to_string).collect();
                (names, Some(output))
            }
            None => (spec.positional(), None),
        };
        let required: Vec<String> = match form {
            Some(form) if source_position => form.required.clone(),
            Some(_) => Vec::new(),
            None if source_position => spec
                .params
                .iter()
                .filter(|p| p.required)
                .map(|p| p.name.clone())
                .chain(spec.required_groups.iter().cloned())
                .collect(),
            None => Vec::new(),
        };
        self.bind_args(spec, stage, &positional, &required);
        output_kinds(spec, form_output.as_deref(), previous)
    }

    fn select_form<'s>(
        &self,
        spec: &'s StageSpec,
        source_position: bool,
        previous: &[String],
    ) -> Option<&'s nurt_contracts::spec::FormSpec> {
        if spec.forms.is_empty() {
            return None;
        }
        if source_position {
            return spec.forms.first();
        }
        spec.forms.iter().find(|form| {
            let accepted: Vec<String> = form.input.iter().map(ToString::to_string).collect();
            input_compatible(&accepted, previous)
        })
    }

    fn bind_args(
        &mut self,
        spec: &StageSpec,
        stage: &Stage,
        positional: &[&ParamSpec],
        required: &[String],
    ) {
        let mut given: Vec<String> = Vec::new();
        let unnamed = stage.args.iter().filter(|arg| arg.name.is_none()).count();
        let mut assigned = assign_positional(positional, unnamed).into_iter();
        for (index, arg) in stage.args.iter().enumerate() {
            let class = classify(&arg.tokens);
            let param = match &arg.name {
                Some(name) => match spec.param(name) {
                    Some(param) => Some(param),
                    None if STAGE_CALL_PARAMETERS.contains(&name.as_str()) => continue,
                    None => {
                        self.unknown_named(spec, name);
                        continue;
                    }
                },
                None => assigned.next().flatten(),
            };
            let Some(param) = param else {
                self.issue(
                    "E_ARGUMENT_COUNT",
                    format!("{} has no parameter for argument {}", spec.name, index + 1),
                );
                continue;
            };
            if given.contains(&param.name) && !param.repeated {
                self.issue(
                    "E_DUPLICATE_PARAM",
                    format!("{}: parameter {} is given twice", spec.name, param.name),
                );
            }
            given.push(param.name.clone());
            self.check_value(spec, param, &class, &arg.tokens);
        }
        self.check_required(spec, &given, required);
        self.check_exclusive(spec, &given);
    }

    fn unknown_named(&mut self, spec: &StageSpec, name: &str) {
        let expression_param = spec
            .params
            .iter()
            .any(|p| p.ty.contains_scalar(ScalarType::NamedExpression));
        if !expression_param {
            self.issue(
                "E_UNKNOWN_PARAM",
                format!("{} has no parameter {name}", spec.name),
            );
        }
    }

    fn check_value(
        &mut self,
        spec: &StageSpec,
        param: &ParamSpec,
        class: &Class,
        tokens: &[Token],
    ) {
        if !accepts(&param.ty, class) {
            let allowed = param.ty.enum_values();
            let hint = if allowed.is_empty() {
                String::new()
            } else {
                format!(" (values: {})", allowed.join(", "))
            };
            self.issue(
                "E_TYPE",
                format!(
                    "{}: parameter {} of type {} does not accept {}{hint}",
                    spec.name,
                    param.name,
                    param.ty,
                    describe(class)
                ),
            );
        }
        if matches!(class, Class::Group) && param.ty.contains_scalar(ScalarType::Pipeline) {
            let inner = &tokens[1..tokens.len() - 1];
            let mut parts: Vec<Token> = inner.to_vec();
            parts.retain(|t| t.kind != Kind::Newline);
            self.pipeline(&parts, true);
        }
    }

    fn check_required(&mut self, spec: &StageSpec, given: &[String], required: &[String]) {
        for name in required {
            let satisfied = if let Some(param) = spec.param(name) {
                given.contains(name) || param.default_from.is_some()
            } else {
                spec.params
                    .iter()
                    .filter(|p| p.exclusive_group.as_deref() == Some(name))
                    .any(|p| given.contains(&p.name))
            };
            if !satisfied {
                self.issue(
                    "E_MISSING_PARAM",
                    format!("{}: missing required parameter {name}", spec.name),
                );
            }
        }
    }

    fn check_exclusive(&mut self, spec: &StageSpec, given: &[String]) {
        for (group, members) in spec.exclusive_groups() {
            let used: Vec<&&str> = members
                .iter()
                .filter(|m| given.iter().any(|g| g.as_str() == **m))
                .collect();
            if used.len() > 1 {
                self.issue(
                    "E_EXCLUSIVE_PARAMS",
                    format!(
                        "{}: parameters of group {group} exclude each other",
                        spec.name
                    ),
                );
            }
        }
    }
}

impl ProgramChecker for CatalogChecker<'_> {
    fn check(&self, program: &str) -> Vec<CheckIssue> {
        let (tokens, problems) = tokenize(program);
        let mut issues: Vec<CheckIssue> = problems
            .into_iter()
            .map(|message| CheckIssue::new("E_LEXICAL", message))
            .collect();
        if !issues.is_empty() {
            return issues;
        }
        let tokens = normalize(tokens, &mut issues);
        if !issues.is_empty() {
            return issues;
        }
        let mut binder = Binder {
            checker: self,
            defined: HashSet::new(),
            issues,
        };
        for statement in split_top_level(&tokens, &Kind::Newline) {
            if !statement.is_empty() {
                binder.statement(&statement);
            }
        }
        if tokens.iter().all(|t| t.kind == Kind::Newline) {
            binder.issue("E_SYNTAX", "the program is empty");
        }
        binder.issues
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::fixtures::catalog;

    fn codes(program: &str) -> Vec<&'static str> {
        let catalog = catalog();
        CatalogChecker::new(&catalog)
            .check(program)
            .into_iter()
            .map(|issue| issue.code)
            .collect()
    }

    #[test]
    fn accepts_pipeline_with_named_arguments() {
        assert!(codes("find \"/home/user\" glob: \"**/*.go\" depth: 2 |> filter size > 1mb and name ~ /x/ |> count").is_empty());
    }

    #[test]
    fn accepts_multiline_pipeline_with_leading_pipes() {
        assert!(codes("find \"/home/user\"\n|> filter size > 1mb\n|> count\n").is_empty());
    }

    #[test]
    fn accepts_let_binding_and_reference() {
        assert!(codes("let a = find \"/home/user\"\n@a |> count").is_empty());
    }

    #[test]
    fn reports_unknown_command() {
        assert_eq!(codes("fnid \"/a\""), vec!["E_UNKNOWN_COMMAND"]);
    }

    #[test]
    fn reports_unknown_parameter() {
        assert_eq!(codes("find \"/a\" depht: 2"), vec!["E_UNKNOWN_PARAM"]);
    }

    #[test]
    fn reports_wrong_enumeration_value() {
        assert_eq!(codes("find \"/a\" type: dir"), vec!["E_TYPE"]);
    }

    #[test]
    fn reports_wrong_literal_kind() {
        assert_eq!(codes("find \"/a\" depth: \"2\""), vec!["E_TYPE"]);
    }

    #[test]
    fn reports_missing_required_parameter_of_source() {
        assert_eq!(codes("find"), vec!["E_MISSING_PARAM"]);
    }

    #[test]
    fn reports_too_many_positional_arguments() {
        assert_eq!(
            codes("find \"/a\" \"*.go\" \"x\""),
            vec!["E_ARGUMENT_COUNT"]
        );
    }

    #[test]
    fn accepts_repeated_positional_arguments() {
        assert!(codes("remove \"/a\" \"/b\" \"/c\"").is_empty());
    }

    fn param(name: &str, repeated: bool) -> ParamSpec {
        serde_json::from_value(serde_json::json!({
            "name": name, "type": "path", "position": 1, "repeated": repeated, "summary": "Text."
        }))
        .expect("parameter fixture parses")
    }

    #[test]
    fn repeated_parameter_leaves_the_last_arguments_to_later_positions() {
        let from = param("from", true);
        let to = param("to", false);
        let names = |count: usize| -> Vec<&str> {
            assign_positional(&[&from, &to], count)
                .into_iter()
                .map(|p| p.map_or("-", |p| p.name.as_str()))
                .collect()
        };
        assert_eq!(names(1), vec!["from"]);
        assert_eq!(names(2), vec!["from", "to"]);
        assert_eq!(names(4), vec!["from", "from", "from", "to"]);
    }

    #[test]
    fn reports_stage_command_at_program_start() {
        assert_eq!(codes("filter size > 1mb"), vec!["E_STAGE_INPUT"]);
    }

    #[test]
    fn reports_unknown_reference() {
        assert_eq!(codes("@x |> count"), vec!["E_UNKNOWN_NAME"]);
    }

    #[test]
    fn reports_lexical_and_bracket_problems() {
        assert_eq!(codes("find \"/a"), vec!["E_LEXICAL"]);
        assert_eq!(codes("find [\"/a\""), vec!["E_SYNTAX"]);
    }

    #[test]
    fn reports_empty_program() {
        assert_eq!(codes("# only a comment\n"), vec!["E_SYNTAX"]);
    }

    #[test]
    fn remove_accepts_entries_from_find() {
        assert!(codes("find \"/a\" |> remove").is_empty());
    }

    #[test]
    fn remove_rejects_records_of_other_kinds() {
        assert_eq!(
            codes("git.status \"/a\" |> remove on_missing: error"),
            vec!["E_STAGE_INPUT"]
        );
    }
}
