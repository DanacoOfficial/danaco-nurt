// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-code::syntax_check
// Responsibility:   Syntax errors of source code.
// Dependencies:     nurt_contracts, tree_sitter
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Syntax errors of source code.
//!
//! The parser recovers from errors by inserting `ERROR` nodes around text it
//! cannot place and zero-width `MISSING` nodes where a token is required.
//! Every outermost `ERROR` node is reported as `unexpected` and every
//! `MISSING` node as `missing`; nodes inside an `ERROR` node are part of
//! the same problem and are not reported again.

use std::path::Path;

use nurt_contracts::error::Result;
use nurt_contracts::record::{Diagnostic, DiagnosticSeverity};
use nurt_contracts::services::{IssueSeverity, LanguageId, SyntaxIssue};
use tree_sitter::Node;

use crate::local::advance;
use crate::registry::{ParsedSource, Registry};
use crate::symbols::records::path_text;

/// Largest number of issues reported for one file; a file with more
/// problems is not source code of the language.
pub const MAX_ISSUES: usize = 100;

/// Longest text quoted in the message of an `unexpected` issue, in
/// characters.
const QUOTE_MAX: usize = 40;

/// Value of the `source` field of diagnostics from syntax analysis.
pub const DIAGNOSTIC_SOURCE: &str = "syntax";

/// Syntax errors of source text in source order; an empty list means that
/// the text parses.
///
/// # Errors
///
/// Fails with `E_UNSUPPORTED_LANGUAGE` when the language has no grammar.
///
/// # Examples
///
/// ```
/// use nurt_code::registry::Registry;
/// use nurt_code::syntax_check::check_syntax;
/// use nurt_contracts::services::LanguageId;
///
/// let json = LanguageId::new("json")?;
/// assert!(check_syntax(Registry::builtin(), b"{\"a\": 1}", &json)?.is_empty());
/// let issues = check_syntax(Registry::builtin(), b"{\"a\": }", &json)?;
/// assert_eq!(issues[0].span.start.line, 1);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn check_syntax(
    registry: &Registry,
    source: &[u8],
    language: &LanguageId,
) -> Result<Vec<SyntaxIssue>> {
    let parsed = registry.parse(language, source)?;
    Ok(issues(&parsed))
}

/// Syntax errors of a parsed source in source order.
#[must_use]
pub fn issues(parsed: &ParsedSource<'_>) -> Vec<SyntaxIssue> {
    let root = parsed.tree.root_node();
    let mut found = Vec::new();
    if !root.has_error() {
        return found;
    }
    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        let issue = if node.is_error() {
            Some(unexpected(parsed, node))
        } else if node.is_missing() {
            Some(missing(parsed, node))
        } else {
            None
        };
        let inside = issue.is_none() && node.has_error();
        if let Some(issue) = issue {
            found.push(issue);
            if found.len() >= MAX_ISSUES {
                break;
            }
        }
        if inside && cursor.goto_first_child() {
            continue;
        }
        if !advance(&mut cursor) {
            break;
        }
    }
    found
}

fn unexpected(parsed: &ParsedSource<'_>, node: Node<'_>) -> SyntaxIssue {
    let text = parsed.text(node);
    let first_line = text.lines().next().unwrap_or_default().trim();
    let message = if first_line.is_empty() {
        "unexpected end of input".to_owned()
    } else {
        format!("unexpected `{}`", quote(first_line))
    };
    SyntaxIssue {
        severity: IssueSeverity::Error,
        message,
        span: parsed.span(node),
        kind: Some("unexpected".to_owned()),
    }
}

fn missing(parsed: &ParsedSource<'_>, node: Node<'_>) -> SyntaxIssue {
    let what = if node.is_named() {
        node.kind().replace('_', " ")
    } else {
        format!("`{}`", node.kind())
    };
    SyntaxIssue {
        severity: IssueSeverity::Error,
        message: format!("missing {what}"),
        span: parsed.span(node),
        kind: Some("missing".to_owned()),
    }
}

fn quote(text: &str) -> String {
    if text.chars().count() <= QUOTE_MAX {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(QUOTE_MAX).collect();
    short.push_str("...");
    short
}

/// `diagnostic` record of a syntax issue of the file `path`.
///
/// ```
/// use std::path::Path;
/// use nurt_code::registry::Registry;
/// use nurt_code::syntax_check::{check_syntax, diagnostic_record};
/// use nurt_contracts::services::LanguageId;
///
/// let go = LanguageId::new("go")?;
/// let issues = check_syntax(Registry::builtin(), b"package p\nfunc f( {}\n", &go)?;
/// let record = diagnostic_record(&issues[0], Path::new("/src/p.go"), &go);
/// assert_eq!(record.source, "syntax");
/// assert_eq!(record.language.as_deref(), Some("go"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
pub fn diagnostic_record(issue: &SyntaxIssue, path: &Path, language: &LanguageId) -> Diagnostic {
    let severity = match issue.severity {
        IssueSeverity::Warning => DiagnosticSeverity::Warning,
        _ => DiagnosticSeverity::Error,
    };
    Diagnostic {
        path: path_text(path),
        line: u64::from(issue.span.start.line),
        column: u64::from(issue.span.start.column),
        severity,
        message: issue.message.clone(),
        source: DIAGNOSTIC_SOURCE.to_owned(),
        code: issue.kind.clone(),
        end_line: Some(u64::from(issue.span.end.line)),
        end_column: Some(u64::from(issue.span.end.column)),
        language: Some(language.to_string()),
        related: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(language: &str, source: &str) -> Vec<SyntaxIssue> {
        check_syntax(
            Registry::builtin(),
            source.as_bytes(),
            &LanguageId::new(language).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn valid_sources_have_no_issues() {
        assert!(check("python", "def f(x):\n    return x\n").is_empty());
        assert!(check("rust", "fn main() { let x = 1; }\n").is_empty());
        assert!(check("typescript", "const a: number = 1;\n").is_empty());
    }

    #[test]
    fn missing_token_is_reported_at_its_position() {
        let issues = check("c", "int main() { return 0 }\n");
        assert!(!issues.is_empty());
        let issue = issues
            .iter()
            .find(|issue| issue.kind.as_deref() == Some("missing"))
            .unwrap();
        assert!(issue.message.contains("`;`"));
        assert_eq!(issue.span.start.line, 1);
        assert_eq!(issue.span.bytes.start, issue.span.bytes.end);
    }

    #[test]
    fn unexpected_text_is_reported_once_per_error_node() {
        let issues = check("json", "{\"a\": 1} }\n");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind.as_deref(), Some("unexpected"));
        assert_eq!(issues[0].message, "unexpected `}`");
        let issues = check("python", "def f(:\n    pass\n");
        assert!(!issues.is_empty());
        let starts: Vec<u64> = issues.iter().map(|issue| issue.span.bytes.start).collect();
        let mut sorted = starts.clone();
        sorted.sort_unstable();
        assert_eq!(starts, sorted);
    }

    #[test]
    fn long_unexpected_text_is_shortened() {
        let text = "x".repeat(QUOTE_MAX * 2);
        assert!(quote(&text).chars().count() <= QUOTE_MAX + 3);
        assert_eq!(quote("ab"), "ab");
    }

    #[test]
    fn issue_count_is_bounded() {
        let source = ")\n".repeat(MAX_ISSUES * 3);
        assert!(check("javascript", &source).len() <= MAX_ISSUES);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
