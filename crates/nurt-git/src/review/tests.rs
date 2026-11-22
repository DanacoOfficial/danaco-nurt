// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use std::sync::Arc;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::{FindingKind, FindingSeverity, FindingTarget};
use nurt_contracts::report::Notable;
use nurt_contracts::services::{ReviewRule, ReviewRuleProvider, RuleSeverity, Services};
use nurt_contracts::stage::{ArgValue, StageFrame};
use nurt_contracts::value::Value;

use super::*;
use crate::diff::fixture::Fixture;
use crate::diff::{Side, hunks_for};
use crate::repo::GitRepo;
use crate::testutil::{FixedClock, RepoBuilder, TestHost};

struct Rules(Vec<ReviewRule>);

impl ReviewRuleProvider for Rules {
    fn rules(&self) -> Vec<ReviewRule> {
        self.0.clone()
    }
}

fn token_rule(pattern: &str) -> ReviewRule {
    ReviewRule {
        id: "test_token".to_owned(),
        pattern: pattern.to_owned(),
        severity: RuleSeverity::Error,
        summary: "access token in an added line".to_owned(),
    }
}

fn context(provider: Option<Arc<dyn ReviewRuleProvider>>) -> ExecCtx {
    let mut builder = ExecCtx::builder(
        Arc::new(FixedClock::at_unix_seconds(RepoBuilder::EPOCH)),
        Arc::new(TestHost::new("/nonexistent-home")),
    )
    .frame(StageFrame {
        command: COMMAND.to_owned(),
        ..StageFrame::default()
    });
    if let Some(provider) = provider {
        builder = builder.services(Arc::new(Services::new().with_review_rules(provider)));
    }
    builder.build()
}

fn request(fixture: &Fixture, scope: Scope, checks: &[Check]) -> ReviewRequest {
    let mut request = ReviewRequest::new(fixture.root());
    request.scope = scope;
    request.checks = checks.iter().copied().collect();
    request
}

fn run(request: &ReviewRequest) -> ReviewOutput {
    run_review(request, &context(None)).unwrap()
}

/// Path, rule and line of every finding, in report order.
fn summary(output: &ReviewOutput) -> Vec<(String, String, Option<u64>)> {
    output
        .findings
        .iter()
        .map(|finding| {
            (
                finding.path.to_string(),
                finding.rule.clone().unwrap_or_default(),
                finding.line,
            )
        })
        .collect()
}

fn has_note(output: &ReviewOutput, code: Notable) -> bool {
    output.notes.iter().any(|note| note.code == code)
}

fn entry(path: &str, rule: &str, line: u64) -> (String, String, Option<u64>) {
    (path.to_owned(), rule.to_owned(), Some(line))
}

#[test]
fn staged_conflict_markers_are_errors_with_canonical_hunk_ids() {
    let fixture = Fixture::new();
    fixture.commit(&[("a.txt", "one\n")]);
    let staged = "one\n<<<<<<< HEAD\n=======\n>>>>>>> topic\n";
    fixture.stage(&[("a.txt", staged)]);
    fixture.write("a.txt", staged);
    let output = run(&request(&fixture, Scope::Staged, &[Check::ConflictMarkers]));
    assert_eq!(
        summary(&output),
        [
            entry("a.txt", "conflict_markers", 2),
            entry("a.txt", "conflict_markers", 4)
        ]
    );
    let hunks = hunks_for(&fixture.repo(), Side::Staged, "a.txt".into()).unwrap();
    let expected = format!("a.txt#1:{}", hunks[0].fingerprint());
    for finding in &output.findings {
        assert_eq!(finding.severity, FindingSeverity::Error);
        assert_eq!(finding.kind, FindingKind::ConflictMarkers);
        assert_eq!(finding.target, Some(FindingTarget::File));
        assert_eq!(finding.hunk.as_deref(), Some(expected.as_str()));
    }
}

#[test]
fn working_scope_reviews_unstaged_and_untracked_changes_only() {
    let fixture = Fixture::new();
    fixture.commit(&[("a.txt", "one\n")]);
    fixture.stage(&[("a.txt", "one \n")]);
    fixture.write("a.txt", "one \n");
    fixture.write("b.txt", "two \n");
    let output = run(&request(&fixture, Scope::Working, &[Check::Whitespace]));
    assert_eq!(summary(&output), [entry("b.txt", "whitespace", 1)]);
    assert_eq!(output.findings[0].hunk, None);
    let staged = run(&request(&fixture, Scope::Staged, &[Check::Whitespace]));
    assert_eq!(summary(&staged), [entry("a.txt", "whitespace", 1)]);
}

#[test]
fn all_scope_gives_hunk_ids_only_where_one_side_is_unchanged() {
    let fixture = Fixture::new();
    fixture.commit(&[("a.txt", "a\n"), ("b.txt", "b\n"), ("c.txt", "c\n")]);
    fixture.stage(&[("a.txt", "a \n"), ("b.txt", "b1\n")]);
    fixture.write("a.txt", "a \n");
    fixture.write("b.txt", "b2 \n");
    fixture.write("c.txt", "c \n");
    let output = run(&request(&fixture, Scope::All, &[Check::Whitespace]));
    assert_eq!(
        summary(&output),
        [
            entry("a.txt", "whitespace", 1),
            entry("b.txt", "whitespace", 1),
            entry("c.txt", "whitespace", 1)
        ]
    );
    let hunks: Vec<bool> = output
        .findings
        .iter()
        .map(|finding| finding.hunk.is_some())
        .collect();
    assert_eq!(hunks, [true, false, true]);
}

#[test]
fn trailing_whitespace_is_one_finding_per_file_with_a_count() {
    let fixture = Fixture::new();
    fixture.commit(&[("a.txt", "1\n2\n3\n4\n")]);
    fixture.write("a.txt", "1\n2 \n3\t\n4  \n");
    let output = run(&request(&fixture, Scope::Working, &[Check::Whitespace]));
    assert_eq!(summary(&output), [entry("a.txt", "whitespace", 2)]);
    assert_eq!(output.findings[0].count, Some(3));
    assert!(output.findings[0].message.starts_with("3 added lines end"));
}

#[test]
fn markdown_files_keep_trailing_spaces_but_not_a_space_before_a_tab() {
    let fixture = Fixture::new();
    fixture.write("README.md", "line  \n \tindented\n");
    let output = run(&request(&fixture, Scope::All, &[Check::Whitespace]));
    assert_eq!(
        summary(&output),
        [entry("README.md", "space_before_tab", 2)]
    );
}

#[test]
fn blank_lines_at_the_end_are_reported_at_the_first_of_them() {
    let fixture = Fixture::new();
    fixture.write("a.txt", "a\n\n \n");
    let output = run(&request(&fixture, Scope::All, &[Check::Whitespace]));
    assert_eq!(
        summary(&output),
        [
            entry("a.txt", "whitespace", 3),
            entry("a.txt", "blank_at_eof", 2)
        ]
    );
}

#[test]
fn a_missing_final_line_feed_is_reported_when_the_change_removes_it() {
    let fixture = Fixture::new();
    fixture.commit(&[("kept.txt", "a"), ("lost.txt", "a\n")]);
    fixture.write("kept.txt", "a\nb");
    fixture.write("lost.txt", "a\nb");
    fixture.write("new.txt", "x");
    let output = run(&request(&fixture, Scope::Working, &[Check::FinalNewline]));
    assert_eq!(
        summary(&output),
        [
            entry("lost.txt", "final_newline", 2),
            entry("new.txt", "final_newline", 1)
        ]
    );
}

#[test]
fn debug_statements_are_reported_in_source_files_only() {
    let fixture = Fixture::new();
    fixture.write("app.js", "const x = 1;\nconsole.log(x);\n");
    fixture.write("notes.txt", "console.log(x);\n");
    let output = run(&request(&fixture, Scope::All, &[Check::DebugStatements]));
    assert_eq!(summary(&output), [entry("app.js", "debug_statements", 2)]);
    assert!(output.findings[0].message.contains("`console.log`"));
    assert_eq!(output.findings[0].severity, FindingSeverity::Warning);
}

#[test]
fn large_untracked_files_are_binary_and_not_read() {
    let fixture = Fixture::new();
    let mut content = vec![b'a'; usize::try_from(LARGE_FILE_BYTES).unwrap() + 1];
    content.extend_from_slice(b" \n");
    fixture.write_bytes("big.txt", &content);
    let checks = [Check::LargeFiles, Check::Binary, Check::Whitespace];
    let output = run(&request(&fixture, Scope::All, &checks));
    let rules: Vec<String> = summary(&output).into_iter().map(|item| item.1).collect();
    assert_eq!(rules, ["large_files", "binary"]);
    let large = &output.findings[0];
    assert_eq!(large.value, Some((LARGE_FILE_BYTES + 3) as f64));
    assert_eq!(large.threshold, Some(LARGE_FILE_BYTES as f64));
    assert_eq!(large.line, None);
}

#[test]
fn binary_content_skips_the_line_checks() {
    let fixture = Fixture::new();
    fixture.write_bytes("data.bin", b"\0\x01<<<<<<< x \n");
    let output = run(&request(&fixture, Scope::All, &Check::ALL));
    assert_eq!(summary(&output).len(), 1);
    assert_eq!(output.findings[0].kind, FindingKind::Binary);
}

#[test]
fn secrets_without_a_rule_service_are_noted_and_not_checked() {
    let fixture = Fixture::new();
    fixture.write("cfg.txt", "key = tok_0123abcd\n");
    let output = run(&request(&fixture, Scope::All, &[Check::Secrets]));
    assert!(output.findings.is_empty());
    assert!(has_note(&output, Notable::SourceUnavailable));
}

#[test]
fn secret_findings_name_the_rule_and_never_the_value() {
    let fixture = Fixture::new();
    fixture.write("cfg.txt", "user = me\nkey = tok_0123abcd\n");
    let provider: Arc<dyn ReviewRuleProvider> =
        Arc::new(Rules(vec![token_rule("tok_[0-9a-f]{8}")]));
    let review = request(&fixture, Scope::All, &[Check::Secrets]);
    let output = run_review(&review, &context(Some(provider))).unwrap();
    assert_eq!(summary(&output), [entry("cfg.txt", "test_token", 2)]);
    let finding = &output.findings[0];
    assert_eq!(finding.severity, FindingSeverity::Error);
    assert_eq!(finding.kind, FindingKind::Secrets);
    assert!(!finding.message.contains("tok_0123abcd"));
    assert_eq!(finding.sample, None);
    assert!(!has_note(&output, Notable::SourceUnavailable));
}

#[test]
fn invalid_secret_rules_are_noted_and_skipped() {
    let fixture = Fixture::new();
    fixture.write("cfg.txt", "key = tok_0123abcd\n");
    let provider: Arc<dyn ReviewRuleProvider> = Arc::new(Rules(vec![token_rule("(")]));
    let review = request(&fixture, Scope::All, &[Check::Secrets]);
    let output = run_review(&review, &context(Some(provider))).unwrap();
    assert!(output.findings.is_empty());
    let note = output
        .notes
        .iter()
        .find(|note| note.code == Notable::SourceUnavailable)
        .unwrap();
    assert!(note.detail.as_deref().unwrap().contains("`test_token`"));
}

#[test]
fn tests_check_names_missing_and_unchanged_tests() {
    let fixture = Fixture::new();
    fixture.commit(&[("src/lexer.rs", "a\n"), ("tests/lexer_tests.rs", "t\n")]);
    fixture.write("src/lexer.rs", "b\n");
    fixture.write("src/parser.rs", "p\n");
    let output = run(&request(&fixture, Scope::All, &[Check::Tests]));
    let rules: Vec<(String, String)> = summary(&output)
        .into_iter()
        .map(|item| (item.0, item.1))
        .collect();
    assert_eq!(
        rules,
        [
            ("src/lexer.rs".to_owned(), "tests_unchanged".to_owned()),
            ("src/parser.rs".to_owned(), "tests_missing".to_owned())
        ]
    );
    assert!(
        output.findings[0]
            .message
            .contains("`tests/lexer_tests.rs`")
    );
}

#[test]
fn a_changed_test_satisfies_the_tests_check() {
    let fixture = Fixture::new();
    fixture.write("src/parser/mod.rs", "p\n");
    fixture.write("tests/parser_test.rs", "t\n");
    let output = run(&request(&fixture, Scope::All, &[Check::Tests]));
    assert!(output.findings.is_empty());
}

#[test]
fn untracked_files_beyond_the_limit_are_counted_as_skipped() {
    let fixture = Fixture::new();
    for number in 0..UNTRACKED_MAX + 2 {
        fixture.write(&format!("f{number:03}.txt"), "x\n");
    }
    let output = run(&request(&fixture, Scope::All, &[Check::Whitespace]));
    let note = output
        .notes
        .iter()
        .find(|note| note.code == Notable::ReviewSkipped)
        .unwrap();
    assert_eq!(note.count, Some(2));
}

#[test]
fn paths_select_and_exclude_leaves_out_files() {
    let fixture = Fixture::new();
    for path in ["a.txt", "b.txt", "sub/c.txt"] {
        fixture.write(path, "x \n");
    }
    let mut selected = request(&fixture, Scope::All, &[Check::Whitespace]);
    selected.paths = vec!["sub".to_owned()];
    assert_eq!(
        summary(&run(&selected)),
        [entry("sub/c.txt", "whitespace", 1)]
    );
    let mut excluding = request(&fixture, Scope::All, &[Check::Whitespace]);
    excluding.exclude = vec!["b.txt".to_owned()];
    let output = run(&excluding);
    assert_eq!(
        summary(&output),
        [
            entry("a.txt", "whitespace", 1),
            entry("sub/c.txt", "whitespace", 1)
        ]
    );
    let note = output
        .notes
        .iter()
        .find(|note| note.code == Notable::Excluded)
        .unwrap();
    assert_eq!(note.count, Some(1));
}

#[test]
fn a_range_compares_two_revisions_without_hunk_ids() {
    let fixture = Fixture::new();
    let first = fixture.commit(&[("a.txt", "one\n")]);
    let second = fixture.commit(&[("a.txt", "one\n>>>>>>> topic\n")]);
    fixture.write("a.txt", "one\n");
    let mut range = request(&fixture, Scope::Working, &[Check::ConflictMarkers]);
    range.base = Some(first.to_string());
    range.target = Some(second.to_string());
    let output = run(&range);
    assert_eq!(summary(&output), [entry("a.txt", "conflict_markers", 2)]);
    assert_eq!(output.findings[0].hunk, None);
    range.target = None;
    assert_eq!(summary(&run(&range)), summary(&output));
}

#[test]
fn target_without_base_is_rejected() {
    let fixture = Fixture::new();
    let mut review = request(&fixture, Scope::All, &Check::ALL);
    review.target = Some("HEAD".to_owned());
    let error = run_review(&review, &context(None)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::MissingParam);
}

#[test]
fn subject_max_below_the_minimum_is_rejected() {
    let fixture = Fixture::new();
    let mut review = request(&fixture, Scope::All, &Check::ALL);
    review.subject_max = SUBJECT_MAX_MIN - 1;
    let error = run_review(&review, &context(None)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn errors_come_before_warnings_and_message_findings_name_the_root() {
    let fixture = Fixture::new();
    fixture.write("a.txt", "x \n<<<<<<< HEAD\n");
    let mut review = request(&fixture, Scope::All, &Check::ALL);
    review.message = Some("Add the parser".to_owned());
    review.conventional = true;
    let output = run(&review);
    let order: Vec<(String, FindingSeverity)> = output
        .findings
        .iter()
        .map(|finding| (finding.rule.clone().unwrap(), finding.severity))
        .collect();
    assert_eq!(
        order,
        [
            ("conflict_markers".to_owned(), FindingSeverity::Error),
            ("conventional_header".to_owned(), FindingSeverity::Error),
            ("whitespace".to_owned(), FindingSeverity::Warning),
        ]
    );
    let message = &output.findings[1];
    assert_eq!(message.target, Some(FindingTarget::Message));
    assert_eq!(
        message.path.to_string(),
        GitRepo::discover(fixture.root(), &context(None))
            .unwrap()
            .root()
            .display()
            .to_string()
    );
}

#[test]
fn a_bare_repository_has_no_working_tree_to_review() {
    let dir = tempfile::tempdir().unwrap();
    gix::init_bare(dir.path()).unwrap();
    let review = ReviewRequest::new(dir.path());
    let error = run_review(&review, &context(None)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitUnsupported);
}

#[test]
fn check_names_expand_aliases_and_reject_unknown_names() {
    let names =
        |list: &[&str]| -> Vec<String> { list.iter().map(|name| (*name).to_owned()).collect() };
    let checks = parse_checks(&names(&["style", "message"])).unwrap();
    assert_eq!(checks.len(), 5);
    assert!(checks.contains(&Check::Subject));
    assert_eq!(parse_checks(&[]).unwrap().len(), Check::ALL.len());
    let error = parse_checks(&names(&["lint"])).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn request_from_args_reads_the_parameters_of_the_catalog() {
    let text = |value: &str| ArgValue::Value(Value::Text(value.to_owned()));
    let args = BoundArgs::new(COMMAND)
        .with(
            "repo",
            ArgValue::Value(Value::Path("/work/repo".to_owned())),
            false,
        )
        .with("scope", text("staged"), false)
        .with(
            "checks",
            ArgValue::Value(Value::List(vec![
                Value::Text("binary".to_owned()),
                Value::Text("subject".to_owned()),
            ])),
            false,
        )
        .with(
            "paths",
            ArgValue::Value(Value::Path("src".to_owned())),
            false,
        )
        .with("subject_max", ArgValue::Value(Value::Int(50)), false)
        .with("conventional", ArgValue::Value(Value::Bool(true)), false)
        .with("message", text("feat(core): add numbers"), false);
    let request = ReviewRequest::from_args(&args).unwrap();
    assert_eq!(request.scope, Scope::Staged);
    assert_eq!(request.paths, ["src"]);
    assert_eq!(
        request.checks,
        [Check::Binary, Check::Subject].into_iter().collect()
    );
    assert_eq!(request.subject_max, 50);
    assert!(request.conventional);
    let unknown = args.clone().with("scope", text("cached"), false);
    assert_eq!(
        ReviewRequest::from_args(&unknown).unwrap_err().code(),
        ErrorCode::InvalidValue
    );
    let negative = args.with("subject_max", ArgValue::Value(Value::Int(-1)), false);
    assert_eq!(
        ReviewRequest::from_args(&negative).unwrap_err().code(),
        ErrorCode::InvalidValue
    );
}
