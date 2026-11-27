// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::diff::funcname
// Responsibility:   Function lines: the text after `@@` in a hunk header and the boundaries.
// Dependencies:     regex
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Function lines: the text after `@@` in a hunk header and the boundaries
//! used by the function context.
//!
//! Without a configured pattern, a line that starts with an ASCII letter,
//! `_` or `$` is a function line. A pattern list (`diff.<driver>.xfuncname`,
//! one extended regular expression per line, `!` negates) selects function
//! lines by the first pattern that matches; the first capture group, when
//! present, is the reported text. The reported text has no trailing white
//! space and at most [`FUNCNAME_MAX`] bytes.

use regex::bytes::{Regex, RegexBuilder};

use super::lines::{is_space, line_body};

/// Longest function text reported in a hunk header, in bytes.
pub const FUNCNAME_MAX: usize = 80;

/// One pattern of a list.
#[derive(Debug, Clone)]
struct Pattern {
    negate: bool,
    regex: Regex,
}

/// Recognizer of function lines.
#[derive(Debug, Clone, Default)]
pub struct FuncMatcher {
    patterns: Option<Vec<Pattern>>,
}

/// Error of a pattern list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternError {
    /// The pattern that failed to compile.
    pub pattern: String,
    /// Description of the problem.
    pub reason: String,
}

impl FuncMatcher {
    /// The rule used without a configured pattern.
    #[must_use]
    pub fn default_rule() -> Self {
        Self { patterns: None }
    }

    /// Compiles a pattern list, one pattern per line.
    ///
    /// # Errors
    ///
    /// Returns the first pattern that is not a valid regular expression.
    pub fn from_patterns(list: &str, ignore_case: bool) -> Result<Self, PatternError> {
        let mut patterns = Vec::new();
        for line in list.split('\n').filter(|line| !line.is_empty()) {
            let (negate, source) = match line.strip_prefix('!') {
                Some(rest) => (true, rest),
                None => (false, line),
            };
            let regex = RegexBuilder::new(source)
                .case_insensitive(ignore_case)
                .unicode(false)
                .build()
                .map_err(|error| PatternError {
                    pattern: line.to_owned(),
                    reason: error.to_string(),
                })?;
            patterns.push(Pattern { negate, regex });
        }
        Ok(Self {
            patterns: Some(patterns),
        })
    }

    /// The text reported for `line` when it is a function line.
    #[must_use]
    pub fn function_text(&self, line: &[u8]) -> Option<Vec<u8>> {
        let body = line_body(line);
        match &self.patterns {
            None => default_rule(body),
            Some(patterns) => pattern_rule(patterns, body),
        }
    }

    /// The line is a function line.
    #[must_use]
    pub fn is_function(&self, line: &[u8]) -> bool {
        self.function_text(line).is_some()
    }

    /// Index of the nearest function line at or before `start`, searching
    /// down to index 0.
    pub(crate) fn find_backward(&self, lines: &[&[u8]], start: usize) -> Option<usize> {
        let first = start.min(lines.len().checked_sub(1)?);
        (0..=first)
            .rev()
            .find(|&index| self.is_function(lines[index]))
    }
}

fn default_rule(body: &[u8]) -> Option<Vec<u8>> {
    let first = *body.first()?;
    if !(first.is_ascii_alphabetic() || first == b'_' || first == b'$') {
        return None;
    }
    let cut = &body[..body.len().min(FUNCNAME_MAX)];
    Some(trim_end(cut).to_vec())
}

fn pattern_rule(patterns: &[Pattern], body: &[u8]) -> Option<Vec<u8>> {
    let body = body.strip_suffix(b"\r").unwrap_or(body);
    for pattern in patterns {
        let Some(captures) = pattern.regex.captures(body) else {
            continue;
        };
        if pattern.negate {
            return None;
        }
        let found = captures.get(1).or_else(|| captures.get(0))?;
        let text = found.as_bytes();
        let cut = &text[..text.len().min(FUNCNAME_MAX)];
        return Some(trim_end(cut).to_vec());
    }
    None
}

fn trim_end(text: &[u8]) -> &[u8] {
    let end = text
        .iter()
        .rposition(|&byte| !is_space(byte))
        .map_or(0, |index| index + 1);
    &text[..end]
}

/// Function line patterns of the built-in drivers selected by the attribute
/// `diff=<name>`; `None` for an unknown driver.
#[must_use]
pub fn builtin_patterns(driver: &str) -> Option<&'static str> {
    Some(match driver {
        "rust" => concat!(
            "^[\t ]*((pub(\\([^)]+\\))?[\t ]+)?((async|const|unsafe|extern([\t ]+\"[^\"]+\"))[\t ]+)*",
            "(struct|enum|union|mod|trait|fn|impl|macro_rules!)[< \t]+[^;]*)$"
        ),
        "python" => "^[\t ]*((class|(async[\t ]+)?def)[\t ].*)$",
        "golang" => "^(func[\t ].*)$\n^(type[\t ].*(struct|interface)[\t ]*\\{.*)$",
        "java" => concat!(
            "!^[\t ]*(catch|do|for|if|instanceof|new|return|switch|throw|while)\n",
            "^[\t ]*(([A-Za-z_][A-Za-z_0-9]*[\t ]+)+[A-Za-z_][A-Za-z_0-9]*[\t ]*\\([^;]*)$"
        ),
        "cpp" => concat!(
            "!^[\t ]*[A-Za-z_][A-Za-z_0-9]*:[[:space:]]*($|/[/*])\n",
            "^((::[[:space:]]*)?[A-Za-z_].*)$"
        ),
        "markdown" => "^ {0,3}#{1,6}[ \t].*",
        "bash" => "^[\t ]*((function[\t ]+)?[A-Za-z_][A-Za-z_0-9]*[\t ]*\\(\\)[\t ]*\\{?.*)$",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_rule_accepts_letters_underscore_and_dollar() {
        let rule = FuncMatcher::default_rule();
        assert_eq!(
            rule.function_text(b"int main(void)   \n"),
            Some(b"int main(void)".to_vec())
        );
        assert!(rule.is_function(b"_start\n"));
        assert!(rule.is_function(b"$x\n"));
        assert!(!rule.is_function(b"  indented\n"));
        assert!(!rule.is_function(b"{\n"));
        assert!(!rule.is_function(b"\n"));
    }

    #[test]
    fn default_rule_cuts_text_at_eighty_bytes() {
        let long = format!("{}  tail\n", "a".repeat(79));
        let text = FuncMatcher::default_rule().function_text(long.as_bytes());
        assert_eq!(text, Some("a".repeat(79).into_bytes()));
    }

    #[test]
    fn patterns_report_first_group_and_honour_negation() {
        let rule = FuncMatcher::from_patterns("!^skip\n^def (\\w+)", false).unwrap();
        assert_eq!(
            rule.function_text(b"def name(x):\r\n"),
            Some(b"name".to_vec())
        );
        assert!(!rule.is_function(b"skip def x\n"));
        assert!(!rule.is_function(b"other\n"));
    }

    #[test]
    fn pattern_text_is_cut_before_trailing_white_space_is_removed() {
        let rule = FuncMatcher::from_patterns("^(.*)$", false).unwrap();
        let line = format!("{} tail\n", "a".repeat(79));
        assert_eq!(
            rule.function_text(line.as_bytes()),
            Some("a".repeat(79).into_bytes())
        );
    }

    #[test]
    fn rejects_invalid_pattern() {
        let error = FuncMatcher::from_patterns("^(open", false).unwrap_err();
        assert_eq!(error.pattern, "^(open");
    }

    #[test]
    fn searches_backward_from_start() {
        let lines: Vec<&[u8]> = vec![b"fn a\n", b"  x\n", b"fn b\n", b"  y\n"];
        let rule = FuncMatcher::default_rule();
        assert_eq!(rule.find_backward(&lines, 3), Some(2));
        assert_eq!(rule.find_backward(&lines, 1), Some(0));
        assert_eq!(rule.find_backward(&lines, 10), Some(2));
        assert_eq!(rule.find_backward(&[], 0), None);
    }

    #[test]
    fn builtin_drivers_compile_and_match() {
        for (driver, line) in [
            ("rust", "pub fn run(x: u8) {"),
            ("python", "    def method(self):"),
            ("golang", "func main() {"),
            ("java", "  public static void main(String[] args) {"),
            ("cpp", "int main(int argc)"),
            ("markdown", "## Section"),
            ("bash", "deploy() {"),
        ] {
            let patterns = builtin_patterns(driver).unwrap();
            let rule = FuncMatcher::from_patterns(patterns, false).unwrap();
            assert!(rule.is_function(line.as_bytes()), "{driver}: {line}");
        }
        assert!(builtin_patterns("unknown").is_none());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
