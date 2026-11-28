// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-text::template
// Responsibility:   Replacement templates for regular expression substitutions.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Replacement templates for regular expression substitutions.
//!
//! A template is the replacement text of `replace … with:` when the pattern is a
//! regular expression. It references capture groups of the pattern:
//!
//! | Form | Meaning |
//! |---|---|
//! | `$1`, `$12` | group by number; all following digits belong to the number |
//! | `${1}` | group by number, delimited from following digits |
//! | `${name}` | group by name |
//! | `$0`, `${0}` | the whole match |
//! | `$$` | a literal `$` |
//!
//! Any other `$` is an error, so a misspelt reference never passes silently.
//! A template is checked against the groups of its pattern once, before any
//! match is replaced; a reference to a group that does not exist is an error
//! with its position. [`Template::literal`] disables the syntax.

use std::error::Error;
use std::fmt;

use nurt_contracts::error::ErrorCode;

/// Capture groups of a pattern that a template may reference.
///
/// The default value describes a pattern without groups, where only the whole
/// match (`$0`) exists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Groups {
    count: usize,
    names: Vec<(String, usize)>,
}

impl Groups {
    /// Groups described by the capture names of a compiled pattern.
    ///
    /// The iterator yields one item per group, starting with group 0 (the
    /// whole match); unnamed groups yield `None`. This is the form returned by
    /// `capture_names` of `regex::Regex` and `fancy_regex::Regex`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_text::template::Groups;
    ///
    /// let regex = regex::Regex::new(r"(?<key>\w+)=(\d+)").unwrap();
    /// let groups = Groups::from_names(regex.capture_names());
    /// assert_eq!(groups.count(), 2);
    /// assert_eq!(groups.index_of("key"), Some(1));
    /// ```
    pub fn from_names<'a, I>(names: I) -> Self
    where
        I: IntoIterator<Item = Option<&'a str>>,
    {
        let mut groups = Self::default();
        for (index, name) in names.into_iter().enumerate().skip(1) {
            groups.count = index;
            if let Some(name) = name {
                groups.names.push((name.to_owned(), index));
            }
        }
        groups
    }

    /// Number of groups, without group 0.
    #[must_use]
    pub fn count(&self) -> usize {
        self.count
    }

    /// Number of the group with the name.
    #[must_use]
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.names
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, index)| *index)
    }

    fn names(&self) -> Vec<String> {
        self.names.iter().map(|(name, _)| name.clone()).collect()
    }
}

/// Text of the capture groups of one match.
pub trait CaptureSource {
    /// Text of the group with the number, `None` when the group did not
    /// participate in the match.
    fn group(&self, index: usize) -> Option<&str>;
}

impl CaptureSource for regex::Captures<'_> {
    fn group(&self, index: usize) -> Option<&str> {
        self.get(index).map(|found| found.as_str())
    }
}

impl CaptureSource for fancy_regex::Captures<'_, str> {
    fn group(&self, index: usize) -> Option<&str> {
        self.get(index).map(|found| found.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    Group(usize),
}

/// Parsed replacement template.
///
/// # Examples
///
/// ```
/// use nurt_text::template::{Groups, Template};
///
/// let regex = regex::Regex::new(r"(\w+)@(?<host>\w+)").unwrap();
/// let template = Template::parse("${host}: $1 ($$)", &Groups::from_names(regex.capture_names()))?;
/// let captures = regex.captures("ann@example").unwrap();
/// let mut out = String::new();
/// template.expand_into(&captures, &mut out);
/// assert_eq!(out, "example: ann ($)");
/// # Ok::<(), nurt_text::template::TemplateError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    parts: Vec<Part>,
}

impl Template {
    /// Parses a template and checks every reference against the groups.
    ///
    /// # Errors
    ///
    /// Returns a [`TemplateError`] with the position of the first `$` that
    /// starts no valid reference or references a group that does not exist.
    pub fn parse(source: &str, groups: &Groups) -> Result<Self, TemplateError> {
        let mut parser = Parser {
            source,
            groups,
            parts: Vec::new(),
            text: String::new(),
        };
        parser.run()?;
        Ok(parser.finish())
    }

    /// Template that inserts the text as it is, without references.
    #[must_use]
    pub fn literal(text: &str) -> Self {
        let parts = if text.is_empty() {
            Vec::new()
        } else {
            vec![Part::Text(text.to_owned())]
        };
        Self { parts }
    }

    /// Whether the template contains no group references.
    #[must_use]
    pub fn is_literal(&self) -> bool {
        self.parts.iter().all(|part| matches!(part, Part::Text(_)))
    }

    /// Appends the expansion of the template for one match.
    ///
    /// A group that did not participate in the match expands to nothing.
    pub fn expand_into<C>(&self, captures: &C, out: &mut String)
    where
        C: CaptureSource + ?Sized,
    {
        for part in &self.parts {
            match part {
                Part::Text(text) => out.push_str(text),
                Part::Group(index) => out.push_str(captures.group(*index).unwrap_or_default()),
            }
        }
    }
}

struct Parser<'a> {
    source: &'a str,
    groups: &'a Groups,
    parts: Vec<Part>,
    text: String,
}

impl Parser<'_> {
    fn run(&mut self) -> Result<(), TemplateError> {
        let mut rest = self.source;
        while let Some(dollar) = rest.find('$') {
            self.text.push_str(&rest[..dollar]);
            let offset = self.source.len() - rest.len() + dollar;
            let after = &rest[dollar + 1..];
            let consumed = self.reference(offset, after)?;
            rest = &after[consumed..];
        }
        self.text.push_str(rest);
        Ok(())
    }

    /// Reads the reference that follows a `$` and returns the number of bytes
    /// it occupies after the `$`.
    fn reference(&mut self, offset: usize, after: &str) -> Result<usize, TemplateError> {
        if after.starts_with('$') {
            self.text.push('$');
            return Ok(1);
        }
        if let Some(inner) = after.strip_prefix('{') {
            let close = inner
                .find('}')
                .ok_or_else(|| self.error(TemplateErrorKind::Unclosed, offset))?;
            self.group(&inner[..close], offset)?;
            return Ok(close + 2);
        }
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return Err(self.error(TemplateErrorKind::LoneDollar, offset));
        }
        self.group(&after[..digits], offset)?;
        Ok(digits)
    }

    fn group(&mut self, reference: &str, offset: usize) -> Result<(), TemplateError> {
        if reference.is_empty() {
            return Err(self.error(TemplateErrorKind::EmptyReference, offset));
        }
        let index = if reference.bytes().all(|byte| byte.is_ascii_digit()) {
            reference
                .parse::<usize>()
                .ok()
                .filter(|index| *index <= self.groups.count())
                .ok_or_else(|| {
                    let kind = TemplateErrorKind::MissingGroup {
                        reference: reference.to_owned(),
                        count: self.groups.count(),
                    };
                    self.error(kind, offset)
                })?
        } else {
            self.groups.index_of(reference).ok_or_else(|| {
                let kind = TemplateErrorKind::UnknownName {
                    name: reference.to_owned(),
                    names: self.groups.names(),
                };
                self.error(kind, offset)
            })?
        };
        if !self.text.is_empty() {
            self.parts.push(Part::Text(std::mem::take(&mut self.text)));
        }
        self.parts.push(Part::Group(index));
        Ok(())
    }

    fn error(&self, kind: TemplateErrorKind, offset: usize) -> TemplateError {
        let column = self.source[..offset].chars().count() + 1;
        TemplateError {
            kind,
            offset,
            column,
        }
    }

    fn finish(mut self) -> Template {
        if !self.text.is_empty() {
            self.parts.push(Part::Text(self.text));
        }
        Template { parts: self.parts }
    }
}

/// Problem found in a replacement template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateErrorKind {
    /// A `$` that starts no reference.
    LoneDollar,
    /// A `${` without the closing `}`.
    Unclosed,
    /// The reference `${}`.
    EmptyReference,
    /// A group number above the number of groups.
    MissingGroup {
        /// Number as written in the template.
        reference: String,
        /// Number of groups of the pattern.
        count: usize,
    },
    /// A group name that the pattern does not define.
    UnknownName {
        /// Name as written in the template.
        name: String,
        /// Names the pattern defines.
        names: Vec<String>,
    },
}

/// Invalid replacement template, with the position of the offending `$`.
///
/// Maps to the error code `E_REGEX`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateError {
    kind: TemplateErrorKind,
    offset: usize,
    column: usize,
}

impl TemplateError {
    /// The problem.
    #[must_use]
    pub fn kind(&self) -> &TemplateErrorKind {
        &self.kind
    }

    /// Byte offset of the `$` in the template.
    #[must_use]
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Column of the `$` in the template, from 1, in Unicode scalar values.
    #[must_use]
    pub fn column(&self) -> usize {
        self.column
    }

    /// Error code: `E_REGEX`.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        ErrorCode::Regex
    }
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let column = self.column;
        match &self.kind {
            TemplateErrorKind::LoneDollar => write!(
                f,
                "`$` at column {column} starts no group reference; write `$$` for a literal dollar sign"
            ),
            TemplateErrorKind::Unclosed => {
                write!(f, "`${{` at column {column} is not closed; add `}}`")
            }
            TemplateErrorKind::EmptyReference => write!(
                f,
                "`${{}}` at column {column} names no group; give a group number or name"
            ),
            TemplateErrorKind::MissingGroup { reference, count } => {
                write!(f, "group {reference} at column {column} does not exist; ")?;
                match count {
                    0 => f.write_str("the pattern has no groups"),
                    1 => f.write_str("the pattern has 1 group"),
                    _ => write!(f, "the pattern has groups 1 to {count}"),
                }
            }
            TemplateErrorKind::UnknownName { name, names } => {
                write!(f, "group `{name}` at column {column} does not exist; ")?;
                if names.is_empty() {
                    f.write_str("the pattern has no named groups")
                } else {
                    write!(f, "known names: {}", names.join(", "))
                }
            }
        }
    }
}

impl Error for TemplateError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups(pattern: &str) -> Groups {
        Groups::from_names(regex::Regex::new(pattern).unwrap().capture_names())
    }

    fn expand(template: &str, pattern: &str, haystack: &str) -> String {
        let regex = regex::Regex::new(pattern).unwrap();
        let template = Template::parse(template, &groups(pattern)).unwrap();
        let mut out = String::new();
        template.expand_into(&regex.captures(haystack).unwrap(), &mut out);
        out
    }

    fn kind(template: &str, pattern: &str) -> TemplateErrorKind {
        Template::parse(template, &groups(pattern))
            .unwrap_err()
            .kind()
            .clone()
    }

    #[test]
    fn expands_numbered_groups() {
        assert_eq!(expand("$2-$1", r"(\w+) (\w+)", "ab cd"), "cd-ab");
    }

    #[test]
    fn expands_braced_number_before_digits() {
        assert_eq!(expand("${1}0", r"(\d)", "7"), "70");
    }

    #[test]
    fn takes_all_digits_after_dollar_as_number() {
        let pattern = r"(a)(b)(c)(d)(e)(f)(g)(h)(i)(j)(k)(l)";
        assert_eq!(expand("$12", pattern, "abcdefghijkl"), "l");
    }

    #[test]
    fn expands_named_groups() {
        assert_eq!(expand("${year}", r"(?<year>\d{4})", "2026"), "2026");
    }

    #[test]
    fn expands_python_style_named_groups() {
        assert_eq!(expand("<${y}>", r"(?P<y>\d+)", "12"), "<12>");
    }

    #[test]
    fn expands_whole_match_as_group_zero() {
        assert_eq!(expand("[$0]", r"\d+", "x42"), "[42]");
        assert_eq!(expand("[${0}]", r"\d+", "x42"), "[42]");
    }

    #[test]
    fn expands_double_dollar_to_literal_dollar() {
        assert_eq!(expand("$$1 $$$1", r"(\d)", "5"), "$1 $5");
    }

    #[test]
    fn expands_non_participating_group_to_nothing() {
        assert_eq!(expand("<$2>", r"(a)|(b)", "a"), "<>");
    }

    #[test]
    fn expands_fancy_regex_captures() {
        let regex = fancy_regex::Regex::new(r"(\w)\1").unwrap();
        let groups = Groups::from_names(regex.capture_names());
        assert_eq!(groups.count(), 1);
        let template = Template::parse("<$1>", &groups).unwrap();
        let captures = regex.captures("xaay").unwrap().unwrap();
        let mut out = String::new();
        template.expand_into(&captures, &mut out);
        assert_eq!(out, "<a>");
    }

    #[test]
    fn keeps_non_ascii_text_around_references() {
        assert_eq!(expand("café $1 ü", r"(\d)", "3"), "café 3 ü");
    }

    #[test]
    fn rejects_lone_dollar_with_column() {
        let error = Template::parse("ab $x", &groups(r"(a)")).unwrap_err();
        assert_eq!(error.kind(), &TemplateErrorKind::LoneDollar);
        assert_eq!(error.column(), 4);
        assert_eq!(error.offset(), 3);
        assert_eq!(error.code(), ErrorCode::Regex);
    }

    #[test]
    fn rejects_trailing_dollar() {
        assert_eq!(kind("cost $", r"(a)"), TemplateErrorKind::LoneDollar);
    }

    #[test]
    fn counts_column_in_characters() {
        let error = Template::parse("éü$x", &groups(r"(a)")).unwrap_err();
        assert_eq!(error.column(), 3);
        assert_eq!(error.offset(), 4);
    }

    #[test]
    fn rejects_unclosed_brace() {
        assert_eq!(kind("${1", r"(a)"), TemplateErrorKind::Unclosed);
    }

    #[test]
    fn rejects_empty_braces() {
        assert_eq!(kind("${}", r"(a)"), TemplateErrorKind::EmptyReference);
    }

    #[test]
    fn rejects_group_number_above_count() {
        assert_eq!(
            kind("$3", r"(a)(b)"),
            TemplateErrorKind::MissingGroup {
                reference: "3".to_owned(),
                count: 2
            }
        );
    }

    #[test]
    fn rejects_number_too_large_for_usize() {
        let reference = "9".repeat(40);
        assert_eq!(
            kind(&format!("${reference}"), r"(a)"),
            TemplateErrorKind::MissingGroup {
                reference,
                count: 1
            }
        );
    }

    #[test]
    fn rejects_unknown_name_listing_known_names() {
        assert_eq!(
            kind("${b}", r"(?<a>x)(?<c>y)"),
            TemplateErrorKind::UnknownName {
                name: "b".to_owned(),
                names: vec!["a".to_owned(), "c".to_owned()]
            }
        );
    }

    #[test]
    fn rejects_group_reference_for_pattern_without_groups() {
        let error = Template::parse("$1", &Groups::default()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "group 1 at column 1 does not exist; the pattern has no groups"
        );
    }

    #[test]
    fn describes_every_error_kind() {
        let messages = [
            Template::parse("$", &Groups::default()).unwrap_err(),
            Template::parse("${", &Groups::default()).unwrap_err(),
            Template::parse("${}", &Groups::default()).unwrap_err(),
            Template::parse("$2", &groups("(a)")).unwrap_err(),
            Template::parse("$3", &groups("(a)(b)")).unwrap_err(),
            Template::parse("${x}", &Groups::default()).unwrap_err(),
            Template::parse("${x}", &groups("(?<y>a)")).unwrap_err(),
        ]
        .map(|error| error.to_string());
        assert_eq!(
            messages,
            [
                "`$` at column 1 starts no group reference; write `$$` for a literal dollar sign",
                "`${` at column 1 is not closed; add `}`",
                "`${}` at column 1 names no group; give a group number or name",
                "group 2 at column 1 does not exist; the pattern has 1 group",
                "group 3 at column 1 does not exist; the pattern has groups 1 to 2",
                "group `x` at column 1 does not exist; the pattern has no named groups",
                "group `x` at column 1 does not exist; known names: y",
            ]
        );
    }

    #[test]
    fn literal_template_keeps_dollar_signs() {
        let template = Template::literal("$1 ${x} $$");
        assert!(template.is_literal());
        let mut out = String::new();
        template.expand_into(
            &regex::Regex::new("a").unwrap().captures("a").unwrap(),
            &mut out,
        );
        assert_eq!(out, "$1 ${x} $$");
    }

    #[test]
    fn empty_literal_template_expands_to_nothing() {
        let template = Template::literal("");
        assert!(template.is_literal());
        let mut out = String::new();
        template.expand_into(
            &regex::Regex::new("a").unwrap().captures("a").unwrap(),
            &mut out,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn template_without_references_is_literal() {
        assert!(
            Template::parse("plain $$", &Groups::default())
                .unwrap()
                .is_literal()
        );
        assert!(
            !Template::parse("$0", &Groups::default())
                .unwrap()
                .is_literal()
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
