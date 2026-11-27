// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Mapping of the positional parameters of a shell script or function
//! (`$1`, `$2`, `$@`, `shift`) to named parameters of a program.
//!
//! The text is rewritten so that `$1` becomes `$name`, where the name is
//! taken from the use: the variable of an assignment such as
//! `dir="$1"` or `dir=${1:-/srv}`, otherwise `arg1`, `arg2`. Assignments that
//! only name a parameter are removed, `shift` moves the numbering and `$@`
//! becomes the list parameter `args`. Function definitions are left
//! untouched: their parameters are mapped when the function is translated.

use regex::Regex;

/// Name of the list parameter that replaces `$@` and `$*`.
pub(crate) const REST_NAME: &str = "args";

/// Type of a declared parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// Text.
    Text,
    /// A path, found as the operand of a command that takes paths.
    Path,
    /// An integer, found in a numeric comparison.
    Int,
    /// A flag.
    Bool,
    /// A list of text, the replacement of `$@`.
    List,
}

impl ParamKind {
    /// Name of the type in the program text.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Path => "path",
            Self::Int => "int",
            Self::Bool => "bool",
            Self::List => "list<text>",
        }
    }

    const fn rank(self) -> u8 {
        match self {
            Self::Int => 3,
            Self::Path => 2,
            Self::Bool | Self::List => 1,
            Self::Text => 0,
        }
    }
}

/// A declared parameter of a script or function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptParam {
    /// Name of the parameter.
    pub name: String,
    /// Type of the parameter.
    pub kind: ParamKind,
    /// Default value as program text.
    pub default: Option<String>,
    /// Remark for the report: the name was not found in the script.
    pub note: Option<String>,
}

impl ScriptParam {
    /// A parameter whose declaration is not known, only its place.
    pub(crate) fn placeholder() -> Self {
        Self {
            name: String::new(),
            kind: ParamKind::Text,
            default: None,
            note: None,
        }
    }

    /// The declaration as written in `params` or `define`.
    #[must_use]
    pub fn declaration(&self) -> String {
        match &self.default {
            Some(default) => format!("{}: {} = {default}", self.name, self.kind.name()),
            None => format!("{}: {}", self.name, self.kind.name()),
        }
    }
}

/// Text with positional parameters replaced by named ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewritten {
    /// The rewritten text.
    pub text: String,
    /// Parameters in the order of their position.
    pub params: Vec<ScriptParam>,
}

/// Commands whose operands are paths.
const PATH_COMMANDS: &[&str] = &[
    "rm", "mkdir", "rmdir", "cp", "mv", "cat", "ls", "touch", "chmod", "chown", "find", "tar",
    "du", "stat", "tee", "head", "tail", "ln", "cd", "gzip", "gunzip", "unzip", "zip", "wc",
];

const ASSIGNMENT: &str = r#"^(\s*)(?:(?:local|declare|readonly)\s+)?([A-Za-z_][A-Za-z0-9_]*)=["']?\$(?:\{([1-9])(?::-([^}"']*))?\}|([1-9]))["']?\s*(?:#.*)?$"#;

/// Replaces the positional parameters of a script.
///
/// # Examples
///
/// ```
/// use nurt_correct::bash::rewrite_positionals;
///
/// let rewritten = rewrite_positionals("dir=\"$1\"\nrm -rf \"$dir\"\ncp \"$2\" /srv/out");
/// assert_eq!(rewritten.params[0].declaration(), "dir: path");
/// assert_eq!(rewritten.params[1].declaration(), "arg2: path");
/// assert!(rewritten.text.contains("cp \"$arg2\" /srv/out"));
/// ```
#[must_use]
pub fn rewrite_positionals(text: &str) -> Rewritten {
    let mut collector = Collector::default();
    let assignment = Regex::new(ASSIGNMENT).ok();
    let mut lines = Vec::new();
    let mut skip_depth = 0_i32;
    for line in text.split('\n') {
        let trimmed = line.trim();
        if skip_depth > 0 || starts_function(trimmed) {
            skip_depth += brace_balance(line);
            lines.push(line.to_owned());
            continue;
        }
        if let Some(rest) = shift_count(trimmed) {
            collector.offset += rest;
            lines.push(String::new());
            continue;
        }
        if let Some(captures) = assignment.as_ref().and_then(|re| re.captures(line)) {
            let index = captures
                .get(3)
                .or_else(|| captures.get(5))
                .and_then(|m| m.as_str().parse::<usize>().ok());
            if let (Some(index), Some(name)) = (index, captures.get(2)) {
                let default = captures.get(4).map(|m| m.as_str().to_owned());
                collector.assigned(index + collector.offset, name.as_str(), default);
                lines.push(String::new());
                continue;
            }
        }
        lines.push(collector.line(line));
    }
    collector.refine(&lines);
    collector.finish(lines.join("\n"))
}

fn starts_function(line: &str) -> bool {
    let rest = line.strip_prefix("function ").unwrap_or(line).trim_start();
    let name_end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(rest.len());
    if name_end == 0 {
        return false;
    }
    let after = rest[name_end..].trim_start();
    let keyword = line.starts_with("function ");
    (after.starts_with("()") || keyword)
        && after.trim_start_matches("()").trim_start().starts_with('{')
}

fn brace_balance(line: &str) -> i32 {
    let mut balance = 0;
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match (quote, c) {
            (None, '\'' | '"') => quote = Some(c),
            (Some(open), _) if open == c => quote = None,
            (None, '{') => balance += 1,
            (None, '}') => balance -= 1,
            _ => {}
        }
    }
    balance
}

fn shift_count(line: &str) -> Option<usize> {
    let rest = line.strip_prefix("shift")?;
    let rest = rest.trim();
    if rest.is_empty() {
        Some(1)
    } else {
        rest.parse().ok()
    }
}

#[derive(Debug, Default)]
struct Collector {
    offset: usize,
    found: Vec<Found>,
    rest: bool,
}

#[derive(Debug)]
struct Found {
    index: usize,
    name: Option<String>,
    kind: ParamKind,
    default: Option<String>,
}

impl Collector {
    fn entry(&mut self, index: usize) -> &mut Found {
        if let Some(position) = self.found.iter().position(|f| f.index == index) {
            return &mut self.found[position];
        }
        self.found.push(Found {
            index,
            name: None,
            kind: ParamKind::Text,
            default: None,
        });
        let last = self.found.len() - 1;
        &mut self.found[last]
    }

    fn assigned(&mut self, index: usize, name: &str, default: Option<String>) {
        let entry = self.entry(index);
        entry.name = Some(name.to_ascii_lowercase());
        if default.is_some() {
            entry.default = default;
        }
    }

    fn name_of(&mut self, index: usize) -> String {
        let entry = self.entry(index);
        entry.name.clone().unwrap_or_else(|| format!("arg{index}"))
    }

    fn raise(&mut self, index: usize, kind: ParamKind) {
        let entry = self.entry(index);
        if kind.rank() > entry.kind.rank() {
            entry.kind = kind;
        }
    }

    /// Rewrites one line and records the uses of parameters in it.
    fn line(&mut self, line: &str) -> String {
        let chars: Vec<char> = line.chars().collect();
        let mut out = String::new();
        let mut quote_single = false;
        let mut at = 0;
        while at < chars.len() {
            let c = chars[at];
            if c == '\'' {
                quote_single = !quote_single;
            }
            if c == '#' && !quote_single && (at == 0 || chars[at - 1].is_whitespace()) {
                out.extend(&chars[at..]);
                break;
            }
            if c != '$' || quote_single {
                out.push(c);
                at += 1;
                continue;
            }
            let kind = segment_kind(&chars, at);
            match self.expansion(&chars, at, kind) {
                Some((text, next)) => {
                    out.push_str(&text);
                    at = next;
                }
                None => {
                    out.push(c);
                    at += 1;
                }
            }
        }
        out
    }

    fn expansion(&mut self, chars: &[char], at: usize, kind: ParamKind) -> Option<(String, usize)> {
        let next = *chars.get(at + 1)?;
        if next == '@' || next == '*' {
            self.rest = true;
            return Some((format!("${REST_NAME}"), at + 2));
        }
        if next.is_ascii_digit() && next != '0' {
            let index = next.to_digit(10)? as usize + self.offset;
            self.raise(index, kind);
            return Some((format!("${}", self.name_of(index)), at + 2));
        }
        if next == '{' {
            let close = chars[at + 2..].iter().position(|c| *c == '}')? + at + 2;
            let inner: String = chars[at + 2..close].iter().collect();
            let (digit, default) = match inner.split_once(":-") {
                Some((d, def)) => (d, Some(def.to_owned())),
                None => (inner.as_str(), None),
            };
            if digit == "@" || digit == "*" {
                self.rest = true;
                return Some((format!("${REST_NAME}"), close + 1));
            }
            let index = digit
                .parse::<usize>()
                .ok()
                .filter(|n| (1..=9).contains(n))?
                + self.offset;
            self.raise(index, kind);
            if default.is_some() {
                self.entry(index).default = default;
            }
            return Some((format!("${}", self.name_of(index)), close + 1));
        }
        None
    }

    /// Raises the kind of a named parameter from the uses of its name in the
    /// rewritten lines (`rm -rf "$dir"` makes `dir` a path).
    fn refine(&mut self, lines: &[String]) {
        let named: Vec<(usize, String)> = self
            .found
            .iter()
            .filter_map(|found| found.name.clone().map(|name| (found.index, name)))
            .collect();
        for (index, name) in named {
            for line in lines {
                let chars: Vec<char> = line.chars().collect();
                let lower: Vec<char> = line.to_ascii_lowercase().chars().collect();
                for needle in [format!("${name}"), format!("${{{name}}}")] {
                    let pattern: Vec<char> = needle.chars().collect();
                    let mut at = 0;
                    while at + pattern.len() <= lower.len() {
                        if lower[at..at + pattern.len()] == pattern[..] {
                            let after = lower.get(at + pattern.len());
                            let ends = needle.ends_with('}')
                                || after.is_none_or(|c| !(c.is_ascii_alphanumeric() || *c == '_'));
                            if ends {
                                self.raise(index, segment_kind(&chars, at));
                            }
                        }
                        at += 1;
                    }
                }
            }
        }
    }

    fn finish(mut self, text: String) -> Rewritten {
        self.found.sort_by_key(|found| found.index);
        let mut params: Vec<ScriptParam> = self
            .found
            .iter()
            .map(|found| {
                let named = found.name.is_some();
                ScriptParam {
                    name: found
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("arg{}", found.index)),
                    kind: found.kind,
                    default: found
                        .default
                        .as_ref()
                        .map(|value| default_text(found.kind, value)),
                    note: (!named).then(|| {
                        format!(
                            "no name for ${} in the script; the parameter is called arg{}",
                            found.index, found.index
                        )
                    }),
                }
            })
            .collect();
        if self.rest {
            params.push(ScriptParam {
                name: REST_NAME.to_owned(),
                kind: ParamKind::List,
                default: Some("[]".to_owned()),
                note: Some("`$@` is the list parameter args".to_owned()),
            });
        }
        Rewritten { text, params }
    }
}

fn default_text(kind: ParamKind, value: &str) -> String {
    if kind == ParamKind::Int && value.chars().all(|c| c.is_ascii_digit()) && !value.is_empty() {
        value.to_owned()
    } else {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// Kind of the parameter used at `at`, read from the command that holds it:
/// the text between the nearest separators around the position.
fn segment_kind(chars: &[char], at: usize) -> ParamKind {
    let separator = |c: char| matches!(c, ';' | '&' | '|' | '(' | ')');
    let start = chars[..at]
        .iter()
        .rposition(|c| separator(*c))
        .map_or(0, |i| i + 1);
    let end = chars[at..]
        .iter()
        .position(|c| separator(*c))
        .map_or(chars.len(), |i| i + at);
    let segment: String = chars[start..end].iter().collect();
    let before: String = chars[start..at].iter().collect();
    // The value of a flag that takes one: `-mtime +$days` is a number of days,
    // `-name $pattern` is text and `-C $dir` is a directory.
    let flag = before
        .split_whitespace()
        .rev()
        .find(|word| !word.chars().all(|c| matches!(c, '+' | '-' | '"' | '\'')));
    if let Some(previous) = flag {
        if VALUE_FLAGS_INT.contains(&previous) {
            return ParamKind::Int;
        }
        if VALUE_FLAGS_TEXT.contains(&previous) {
            return ParamKind::Text;
        }
        if VALUE_FLAGS_PATH.contains(&previous) {
            return ParamKind::Path;
        }
    }
    line_kind(&segment)
}

/// Flags whose value is a number.
const VALUE_FLAGS_INT: &[&str] = &[
    "-mtime",
    "-mmin",
    "-atime",
    "-ctime",
    "-maxdepth",
    "-mindepth",
    "-n",
    "-c",
];

/// Flags whose value is text.
const VALUE_FLAGS_TEXT: &[&str] = &[
    "-name", "-iname", "-type", "-user", "-group", "-perm", "-m", "-p", "-j", "-u", "-g", "-t",
];

/// Flags whose value is a path.
const VALUE_FLAGS_PATH: &[&str] = &["-C", "--directory", "--file", "-f"];

fn line_kind(line: &str) -> ParamKind {
    let words: Vec<&str> = line
        .split_whitespace()
        .skip_while(|word| {
            matches!(
                *word,
                "then" | "do" | "else" | "elif" | "if" | "while" | "until"
            )
        })
        .collect();
    let numeric = ["-eq", "-ne", "-gt", "-ge", "-lt", "-le"];
    if words.iter().any(|word| numeric.contains(word)) {
        return ParamKind::Int;
    }
    let first = words
        .iter()
        .find(|word| !word.contains('=') && **word != "sudo")
        .copied()
        .unwrap_or_default();
    let tests = ["-d", "-f", "-e", "-r", "-w", "-x", "-s", "-L"];
    let guarded = matches!(words.first().copied(), Some("[" | "[[" | "test"));
    if PATH_COMMANDS.contains(&first) || (guarded && words.iter().any(|word| tests.contains(word)))
    {
        return ParamKind::Path;
    }
    ParamKind::Text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_by_assignment_and_default() {
        let out = rewrite_positionals("keep=${2:-7}\ndir=$1\nfind \"$dir\" -mtime +\"$keep\"");
        assert_eq!(out.params.len(), 2);
        assert_eq!(out.params[0].name, "dir");
        assert_eq!(out.params[1].declaration(), "keep: int = 7");
        assert_eq!(out.text.lines().filter(|l| !l.is_empty()).count(), 1);
    }

    #[test]
    fn a_data_flag_of_another_program_is_not_a_file_test() {
        let out = rewrite_positionals("curl -s -X POST -d \"$1\" https://hooks.example.com/x");
        assert_eq!(out.params[0].kind, ParamKind::Text);
        let out = rewrite_positionals("if [ -d \"$1\" ]; then echo yes; fi");
        assert_eq!(out.params[0].kind, ParamKind::Path);
    }

    #[test]
    fn shift_moves_the_numbering() {
        let out = rewrite_positionals("mode=$1\nshift\nrm \"$1\"");
        assert_eq!(out.params[0].name, "mode");
        assert_eq!(out.params[1].name, "arg2");
        assert!(out.text.contains("rm \"$arg2\""));
    }

    #[test]
    fn rest_becomes_a_list_parameter() {
        let out = rewrite_positionals("for f in \"$@\"; do rm \"$f\"; done");
        assert!(out.text.contains("\"$args\""));
        assert_eq!(out.params[0].declaration(), "args: list<text> = []");
    }

    #[test]
    fn numeric_comparison_makes_an_integer() {
        let out = rewrite_positionals("if [ \"$1\" -gt 3 ]; then echo big; fi");
        assert_eq!(out.params[0].kind, ParamKind::Int);
    }

    #[test]
    fn function_bodies_are_left_alone() {
        let out = rewrite_positionals("clean() {\n  rm \"$1\"\n}\nclean \"$2\"");
        assert!(out.text.contains("rm \"$1\""));
        assert_eq!(out.params[0].name, "arg2");
    }

    #[test]
    fn single_quotes_and_comments_are_not_rewritten() {
        let out = rewrite_positionals("echo '$1' # $2");
        assert!(out.params.is_empty());
    }
}
