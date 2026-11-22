// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Syntax validation of changed text: the `validate` parameter of `write`,
//! `replace`, `insert`, `edit` and `patch`.
//!
//! A changed file of a known format is checked after the change and before
//! the commit. The checker is chosen by the file name and the content: the
//! structured document service for document formats (JSON, YAML, TOML and the
//! other formats of the documents group), otherwise the source code service
//! for programming languages. Only syntax errors added by the change count:
//! an error of the new content that has the same kind and message as an
//! error of the previous content was already there and does not reject the
//! change. Warnings of the checkers are not counted.
//!
//! | Value | New syntax error |
//! |---|---|
//! | `off` | not checked |
//! | `warn` | a warning event; the change is committed |
//! | `reject` | `E_ROLLED_BACK` with the new errors; the executor rolls the change back |
//!
//! A file of no known format, and a context without the document and code
//! services, is not validated.

use nurt_contracts::error::{Error, ErrorCode, Location, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::services::{IssueSeverity, SyntaxIssue};
use nurt_contracts::stage::{BoundArgs, Event, ExecCtx, LogEvent, LogLevel};
use nurt_contracts::value::Value;
use serde_json::Value as Json;

/// Name of the `validate` parameter.
pub(crate) const VALIDATE: &str = "validate";

const VALUES: [&str; 3] = ["off", "warn", "reject"];
/// Number of new syntax errors listed in a rejection.
const ISSUES_SHOWN: usize = 8;

/// Value of the `validate` parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Validate {
    /// `off`: the result is not checked.
    Off,
    /// `warn`: new syntax errors are reported and the change is kept.
    Warn,
    /// `reject`: new syntax errors roll the change back.
    #[default]
    Reject,
}

/// Reads `validate`; [`Validate::Reject`] without a value.
///
/// # Errors
///
/// Fails with `E_TYPE` for a value that is not an enumeration value and with
/// `E_UNKNOWN_VALUE` for a value other than `off`, `warn` and `reject`.
pub(crate) fn validate_param(args: &BoundArgs) -> Result<Validate> {
    match args.value(VALIDATE) {
        None | Some(Value::Null) => Ok(Validate::default()),
        Some(Value::Enum(text) | Value::Text(text)) => match text.as_str() {
            "off" => Ok(Validate::Off),
            "warn" => Ok(Validate::Warn),
            "reject" => Ok(Validate::Reject),
            other => Err(Error::new(
                ErrorCode::UnknownValue,
                format!(
                    "`{other}` is not a value of `{VALIDATE}`: use one of {}",
                    VALUES.join(", ")
                ),
            )
            .with_found(other)
            .with_expected(serde_json::json!(VALUES))),
        },
        Some(_) => Err(Error::new(
            ErrorCode::Type,
            format!(
                "`{}` expects a value of the enumeration for `{VALIDATE}`",
                args.command()
            ),
        )),
    }
}

/// Syntax errors added by a change, with the name of the format or language
/// that was checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AddedIssues {
    /// Format or language, for example `json` or `rust`.
    pub(crate) checker: String,
    /// Errors of the new content that the previous content did not have.
    pub(crate) issues: Vec<SyntaxIssue>,
}

/// Syntax errors that `after` adds to `before`, the previous and the new
/// content of `path`; `None` when no checker knows the file.
#[must_use]
pub(crate) fn added_issues(
    ctx: &ExecCtx,
    path: &AbsPath,
    before: &[u8],
    after: &[u8],
) -> Option<AddedIssues> {
    let name = Some(path.as_path());
    if let Some(doc) = ctx.doc()
        && let Some(format) = doc
            .detect_format(name, before)
            .or_else(|| doc.detect_format(name, after))
    {
        let new = doc.check_syntax(after, &format).ok()?;
        let old = doc.check_syntax(before, &format).unwrap_or_default();
        return Some(AddedIssues {
            checker: format.to_string(),
            issues: added(&old, new),
        });
    }
    let code = ctx.code()?;
    let language = code
        .language_of(name, before)
        .or_else(|| code.language_of(name, after))?;
    let new = code.check_syntax(after, &language).ok()?;
    let old = code.check_syntax(before, &language).unwrap_or_default();
    Some(AddedIssues {
        checker: language.to_string(),
        issues: added(&old, new),
    })
}

fn is_error(issue: &SyntaxIssue) -> bool {
    matches!(issue.severity, IssueSeverity::Error)
}

/// Errors of `after` without a counterpart of the same kind and message in
/// `before`; every error of `before` is the counterpart of one error only.
fn added(before: &[SyntaxIssue], after: Vec<SyntaxIssue>) -> Vec<SyntaxIssue> {
    let mut known: Vec<&SyntaxIssue> = before.iter().filter(|issue| is_error(issue)).collect();
    let mut new = Vec::new();
    for issue in after.into_iter().filter(is_error) {
        let counterpart = known
            .iter()
            .position(|old| old.kind == issue.kind && old.message == issue.message);
        match counterpart {
            Some(index) => {
                known.swap_remove(index);
            }
            None => new.push(issue),
        }
    }
    new
}

/// Checks the new content of a changed file as `mode` requires.
///
/// `before` and `after` are the previous and the new content, `command` the
/// command that changed the file; it names the source of a warning.
///
/// # Errors
///
/// Fails with `E_ROLLED_BACK` ([`rejection`]) when `mode` is
/// [`Validate::Reject`] and the new content adds a syntax error.
pub(crate) fn check_change(
    ctx: &ExecCtx,
    command: &str,
    path: &AbsPath,
    before: &[u8],
    after: &[u8],
    mode: Validate,
) -> Result<()> {
    if mode == Validate::Off {
        return Ok(());
    }
    let Some(found) = added_issues(ctx, path, before, after) else {
        return Ok(());
    };
    if found.issues.is_empty() {
        return Ok(());
    }
    if mode == Validate::Reject {
        return Err(rejection(path, &found));
    }
    let lines: Vec<String> = found
        .issues
        .iter()
        .take(ISSUES_SHOWN)
        .map(|issue| issue_line(path, &found.checker, issue))
        .collect();
    ctx.emit(Event::Log(LogEvent {
        level: LogLevel::Warn,
        message: format!(
            "the change adds {} to `{path}`: {}",
            error_count(found.issues.len()),
            lines.join("; ")
        ),
        source: Some(command.to_owned()),
    }));
    Ok(())
}

/// `E_ROLLED_BACK` for a change that adds syntax errors to `path`.
///
/// Every new error is a `sample` line `path:line:column message [checker]`;
/// the cause is `E_DOC_PARSE` with the position of the first new error.
#[must_use]
pub(crate) fn rejection(path: &AbsPath, found: &AddedIssues) -> Error {
    let mut error = Error::new(
        ErrorCode::RolledBack,
        format!(
            "the change adds {} to `{path}` and was rolled back: correct the new text or pass `validate: warn`",
            error_count(found.issues.len())
        ),
    )
    .with_problem("validate_failed")
    .with_detail("path", Json::from(path.to_language_string()))
    .with_detail("checker", Json::from(found.checker.clone()))
    .with_actual(serde_json::json!(found.issues.len()));
    for issue in found.issues.iter().take(ISSUES_SHOWN) {
        error = error.with_sample(issue_line(path, &found.checker, issue));
    }
    if let Some(first) = found.issues.first() {
        error = error.with_cause(
            Error::new(ErrorCode::DocParse, first.message.clone())
                .with_location(issue_location(path, first)),
        );
    }
    error
}

fn issue_line(path: &AbsPath, checker: &str, issue: &SyntaxIssue) -> String {
    format!(
        "{}:{}:{} {} [{checker}]",
        path.to_language_string(),
        issue.span.start.line,
        issue.span.start.column,
        issue.message
    )
}

fn issue_location(path: &AbsPath, issue: &SyntaxIssue) -> Location {
    Location {
        path: path.to_language_string(),
        line: Some(u64::from(issue.span.start.line)),
        column: Some(u64::from(issue.span.start.column)),
        end_line: Some(u64::from(issue.span.end.line)),
        end_column: Some(u64::from(issue.span.end.column)),
        byte: Some(issue.span.bytes.start),
    }
}

fn error_count(count: usize) -> String {
    if count == 1 {
        "1 syntax error".to_owned()
    } else {
        format!("{count} syntax errors")
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use nurt_contracts::keypath::KeyPath;
    use nurt_contracts::services::{
        ByteRange, DocFormatId, DocFormatInfo, DocItem, DocService, SerializeOptions, Services,
        TextPosition, TextSpan,
    };
    use nurt_contracts::stage::ArgValue;
    use nurt_contracts::value::Record;
    use nurt_testkit::ctx::TestContext;

    use super::*;
    use crate::commands::edit::fixture::abs;

    /// Document service that knows `.json` files and reports one error for
    /// unbalanced braces and one for every `!`.
    struct BraceChecker;

    fn issue(kind: &str, message: &str, offset: u64) -> SyntaxIssue {
        let position = TextPosition { line: 1, column: 1 };
        SyntaxIssue {
            severity: IssueSeverity::Error,
            message: message.to_owned(),
            span: TextSpan {
                bytes: ByteRange::new(offset, offset),
                start: position,
                end: position,
            },
            kind: Some(kind.to_owned()),
        }
    }

    impl DocService for BraceChecker {
        fn formats(&self) -> Vec<DocFormatInfo> {
            Vec::new()
        }

        fn detect_format(&self, path: Option<&Path>, _bytes: &[u8]) -> Option<DocFormatId> {
            path.filter(|path| path.extension().is_some_and(|ext| ext == "json"))
                .map(|_| DocFormatId::JSON)
        }

        fn read_keys(
            &self,
            _bytes: &[u8],
            _format: &DocFormatId,
            _key: &KeyPath,
            _depth: Option<u32>,
        ) -> Result<Vec<DocItem>> {
            Ok(Vec::new())
        }

        fn check_syntax(&self, bytes: &[u8], _format: &DocFormatId) -> Result<Vec<SyntaxIssue>> {
            let open = bytes.iter().filter(|byte| **byte == b'{').count();
            let close = bytes.iter().filter(|byte| **byte == b'}').count();
            let mut issues = Vec::new();
            if open != close {
                issues.push(issue("unbalanced", "unbalanced braces", 0));
            }
            for (offset, _) in bytes.iter().enumerate().filter(|(_, byte)| **byte == b'!') {
                issues.push(issue("unexpected", "unexpected `!`", offset as u64));
            }
            let mut warning = issue("style", "trailing space", 0);
            warning.severity = IssueSeverity::Warning;
            if bytes.ends_with(b" ") {
                issues.push(warning);
            }
            Ok(issues)
        }

        fn serialize_records(
            &self,
            _records: &[Record],
            _format: &DocFormatId,
            _options: &SerializeOptions,
        ) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
    }

    fn checked() -> TestContext {
        TestContext::new().with_services(Services::new().with_doc(Arc::new(BraceChecker)))
    }

    fn arg(value: Value) -> BoundArgs {
        BoundArgs::new("edit").with(VALIDATE, ArgValue::Value(value), false)
    }

    #[test]
    fn validate_defaults_to_reject_and_reads_every_value() {
        assert_eq!(
            validate_param(&BoundArgs::new("edit")).unwrap(),
            Validate::Reject
        );
        for (name, expected) in [
            ("off", Validate::Off),
            ("warn", Validate::Warn),
            ("reject", Validate::Reject),
        ] {
            assert_eq!(
                validate_param(&arg(Value::Enum(name.to_owned()))).unwrap(),
                expected
            );
        }
        let unknown = validate_param(&arg(Value::Text("strict".to_owned()))).unwrap_err();
        assert_eq!(unknown.code(), ErrorCode::UnknownValue);
        let wrong = validate_param(&arg(Value::Bool(true))).unwrap_err();
        assert_eq!(wrong.code(), ErrorCode::Type);
    }

    #[test]
    fn a_new_syntax_error_rejects_the_change() {
        let test = checked();
        let path = abs("/data/config.json");
        let error = check_change(
            &test.ctx(),
            "edit",
            &path,
            b"{\"a\": 1}",
            b"{\"a\": 1",
            Validate::Reject,
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::RolledBack);
        assert_eq!(error.problem.as_deref(), Some("validate_failed"));
        assert_eq!(error.sample.len(), 1);
        assert!(error.sample[0].ends_with("unbalanced braces [json]"));
        let cause = error.cause.as_ref().unwrap();
        assert_eq!(cause.code(), ErrorCode::DocParse);
        assert_eq!(
            cause.location.as_ref().map(|location| location.line),
            Some(Some(1))
        );
    }

    #[test]
    fn errors_present_before_the_change_do_not_reject_it() {
        let test = checked();
        let path = abs("/data/config.json");
        check_change(
            &test.ctx(),
            "edit",
            &path,
            b"{\"a\": 1",
            b"{\"a\": 2",
            Validate::Reject,
        )
        .unwrap();
        let found = added_issues(&test.ctx(), &path, b"{!}", b"{!!} ").unwrap();
        assert_eq!(found.checker, "json");
        assert_eq!(found.issues.len(), 1);
        assert_eq!(found.issues[0].kind.as_deref(), Some("unexpected"));
    }

    #[test]
    fn warn_reports_new_errors_and_keeps_the_change() {
        let test = checked();
        let path = abs("/data/config.json");
        check_change(&test.ctx(), "replace", &path, b"{}", b"{!}", Validate::Warn).unwrap();
        let events = test.events().take();
        assert_eq!(events.len(), 1);
        let Event::Log(log) = &events[0] else {
            panic!("expected a log event, got {:?}", events[0]);
        };
        assert_eq!(log.level, LogLevel::Warn);
        assert_eq!(log.source.as_deref(), Some("replace"));
        assert!(log.message.contains("1 syntax error"));
    }

    #[test]
    fn off_and_unknown_formats_are_not_validated() {
        let test = checked();
        let json = abs("/data/config.json");
        check_change(&test.ctx(), "edit", &json, b"{}", b"{", Validate::Off).unwrap();
        let text = abs("/data/notes.txt");
        check_change(&test.ctx(), "edit", &text, b"{}", b"{", Validate::Reject).unwrap();
        assert_eq!(added_issues(&test.ctx(), &text, b"{}", b"{"), None);
    }

    #[test]
    fn contexts_without_checkers_are_not_validated() {
        let test = TestContext::new();
        let path = abs("/data/config.json");
        assert_eq!(added_issues(&test.ctx(), &path, b"{}", b"{"), None);
        check_change(&test.ctx(), "edit", &path, b"{}", b"{", Validate::Reject).unwrap();
        let bare = TestContext::new().with_services(Services::new());
        assert_eq!(added_issues(&bare.ctx(), &path, b"{}", b"{"), None);
    }

    #[test]
    fn rejection_lists_at_most_the_shown_errors() {
        let path = abs("/data/a.json");
        let issues = (0..20)
            .map(|n| issue("unexpected", "unexpected `!`", n))
            .collect();
        let error = rejection(
            &path,
            &AddedIssues {
                checker: "json".to_owned(),
                issues,
            },
        );
        assert_eq!(error.sample.len(), ISSUES_SHOWN);
        assert!(error.message.contains("20 syntax errors"));
        assert_eq!(error.actual, Some(serde_json::json!(20)));
    }
}
