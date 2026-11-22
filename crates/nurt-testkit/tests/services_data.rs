// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Data service doubles: registered answers, unsupported inputs and scripted
//! process details.

#![allow(
    clippy::cognitive_complexity,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::env::Clock;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::{
    ByteRange, ClassifiedSpan, CodeService, DocFormatId, DocFormatInfo, DocService,
    FileHistoryStat, FileHolder, FileHolderService, FunctionMetrics, GitChange, GitPathStatus,
    GitQueryService, IssueSeverity, LanguageId, LanguageInfo, LineEnding, ProcessHost,
    ReferenceRole, ReviewRule, ReviewRuleProvider, RuleSeverity, SerializeOptions,
    ServiceSpawnSpec, SpanClass, SymbolInfo, SymbolKind, SymbolReference, SyntaxIssue, SyntaxMatch,
    SyntaxToken, TextCodecService, TextEncoding, TextPosition, TextSpan,
};
use nurt_contracts::stage::CancelToken;
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::services::{
    ProgramScript, ScriptedProcessHost, StaticCodeService, StaticDocService, StaticFileHolders,
    StaticGitQuery, StaticReviewRules, UnavailableProcessHost, UnsupportedCodeService,
    UnsupportedDocService, Utf8TextCodec,
};

fn span(start: u64, end: u64) -> TextSpan {
    TextSpan {
        bytes: ByteRange::new(start, end),
        start: TextPosition { line: 1, column: 1 },
        end: TextPosition { line: 1, column: 2 },
    }
}

fn spawn_spec(program: &str) -> ServiceSpawnSpec {
    ServiceSpawnSpec {
        program: program.to_owned(),
        args: Vec::new(),
        cwd: "/".into(),
        env: Vec::new(),
        inherit_env: true,
        temp_dir: None,
    }
}

fn symbol(name: &str, container: Option<&str>, depth: u32, range: (u64, u64)) -> SymbolInfo {
    SymbolInfo {
        name: name.to_owned(),
        kind: SymbolKind::Function,
        container: container.map(str::to_owned),
        depth,
        span: span(range.0, range.1),
        name_span: span(range.0, range.0 + 1),
        signature: None,
        exported: None,
    }
}

#[test]
fn unsupported_document_service_fails_every_request_with_the_format() {
    let key = KeyPath::parse("a").unwrap();
    let format = DocFormatId::JSON;
    let doc = UnsupportedDocService;
    let errors = [
        doc.read_keys(b"{}", &format, &key, None).unwrap_err(),
        doc.check_syntax(b"{}", &format).unwrap_err(),
        doc.serialize_records(&[], &format, &SerializeOptions::default())
            .unwrap_err(),
    ];
    for error in errors {
        assert_eq!(error.code(), ErrorCode::Unsupported);
        assert!(error.to_string().contains("json"), "{error}");
    }
}

#[test]
fn unsupported_code_service_fails_every_request_with_the_language() {
    let language = LanguageId::new("rust").unwrap();
    let code = UnsupportedCodeService;
    let pattern_error = code.compile_pattern("f()", &language).err().unwrap();
    let errors = [
        code.outline(b"", &language, None).unwrap_err(),
        code.find_symbol(b"", &language, "f").unwrap_err(),
        code.enclosing(b"", &language, 0).unwrap_err(),
        code.classify_spans(b"", &language).unwrap_err(),
        code.check_syntax(b"", &language).unwrap_err(),
        pattern_error,
        code.references(b"", &language, "f").unwrap_err(),
        code.function_metrics(b"", &language).unwrap_err(),
        code.tokens(b"", &language).unwrap_err(),
    ];
    for error in errors {
        assert_eq!(error.code(), ErrorCode::UnsupportedLanguage);
    }
    let found = SyntaxMatch {
        span: span(0, 1),
        node_kind: "call".to_owned(),
        text: "f()".to_owned(),
        captures: Vec::new(),
    };
    assert_eq!(
        code.render_template(&found, "g()").unwrap_err().code(),
        ErrorCode::UnsupportedLanguage
    );
    let rust = StaticCodeService::new().with_language(info(&language, "rs"));
    let pattern = rust.compile_pattern("f($X)", &language).unwrap();
    assert_eq!(
        code.syntax_search(b"", pattern.as_ref())
            .unwrap_err()
            .code(),
        ErrorCode::UnsupportedLanguage
    );
}

fn info(id: &LanguageId, extension: &str) -> LanguageInfo {
    LanguageInfo {
        id: id.clone(),
        name: id.as_str().to_owned(),
        extensions: vec![extension.to_owned()],
        grammar_version: None,
    }
}

#[test]
fn unavailable_process_host_fails_runs_and_spawns() {
    let host = UnavailableProcessHost;
    let spec = nurt_contracts::services::ProgramSpec::new("git", Vec::new(), "/");
    assert_eq!(
        host.run_program(&spec, &CancelToken::new())
            .unwrap_err()
            .code(),
        ErrorCode::ExternalMissing
    );
    assert_eq!(
        host.spawn_service(&spawn_spec("lsp")).err().unwrap().code(),
        ErrorCode::ExternalMissing
    );
}

#[test]
fn static_documents_list_formats_items_and_registered_issues() {
    let key = KeyPath::parse("a").unwrap();
    let format = DocFormatInfo {
        id: DocFormatId::YAML,
        extensions: vec!["yaml".to_owned()],
        editable: false,
    };
    let issue = SyntaxIssue {
        severity: IssueSeverity::Warning,
        message: "tab".to_owned(),
        span: span(0, 1),
        kind: None,
    };
    let doc = StaticDocService::new()
        .with_format(format.clone())
        .with_items(DocFormatId::YAML, &key, Vec::new())
        .with_issues(DocFormatId::YAML, "a:\tb", vec![issue.clone()]);
    assert_eq!(doc.formats(), [format]);
    assert!(
        doc.read_keys(b"", &DocFormatId::YAML, &key, None)
            .unwrap()
            .is_empty()
    );
    let other = KeyPath::parse("b").unwrap();
    assert!(
        doc.read_keys(b"", &DocFormatId::YAML, &other, None)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        doc.check_syntax(b"a:\tb", &DocFormatId::YAML).unwrap(),
        [issue]
    );
    assert!(
        doc.check_syntax(b"a: b", &DocFormatId::YAML)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        doc.serialize_records(&[], &DocFormatId::YAML, &SerializeOptions::default())
            .unwrap_err()
            .code(),
        ErrorCode::Unsupported
    );
}

#[test]
fn static_documents_serialize_pretty_json() {
    let doc = StaticDocService::json();
    let options = SerializeOptions {
        pretty: true,
        ..SerializeOptions::default()
    };
    let bytes = doc
        .serialize_records(&[], &DocFormatId::JSON, &options)
        .unwrap();
    assert_eq!(bytes, b"[]");
}

#[test]
fn static_code_answers_registered_data_per_language() {
    let rust = LanguageId::new("rust").unwrap();
    let outer = symbol("Outer", None, 0, (0, 100));
    let inner = symbol("run", Some("Outer"), 1, (10, 20));
    let reference = SymbolReference {
        span: span(3, 4),
        role: ReferenceRole::Usage,
        class: SpanClass::Code,
        container: None,
        line_text: "run()".to_owned(),
    };
    let metrics = FunctionMetrics {
        name: "run".to_owned(),
        container: Some("Outer".to_owned()),
        span: span(10, 20),
        lines: 3,
        parameters: 0,
        cyclomatic: 1,
        max_nesting: 0,
    };
    let token = SyntaxToken {
        bytes: ByteRange::new(0, 3),
        kind: "ident".to_owned(),
        class: SpanClass::Code,
    };
    let class = ClassifiedSpan {
        bytes: ByteRange::new(0, 3),
        class: SpanClass::Comment,
    };
    let issue = SyntaxIssue {
        severity: IssueSeverity::Error,
        message: "unexpected".to_owned(),
        span: span(0, 1),
        kind: None,
    };
    let code = StaticCodeService::new()
        .with_language(info(&rust, "rs"))
        .with_symbols(&rust, vec![outer.clone(), inner.clone()])
        .with_spans(&rust, vec![class])
        .with_issues(&rust, vec![issue.clone()])
        .with_references(&rust, "run", vec![reference.clone()])
        .with_metrics(&rust, vec![metrics.clone()])
        .with_tokens(&rust, vec![token.clone()]);

    assert_eq!(code.languages().len(), 1);
    assert_eq!(
        code.language_of(Some(Path::new("lib.RS")), b""),
        Some(rust.clone())
    );
    assert_eq!(code.language_of(Some(Path::new("lib.py")), b""), None);
    assert_eq!(code.language_of(None, b""), None);

    assert_eq!(
        code.outline(b"", &rust, None).unwrap(),
        [outer.clone(), inner.clone()]
    );
    assert_eq!(
        code.outline(b"", &rust, Some(0)).unwrap(),
        std::slice::from_ref(&outer)
    );
    assert_eq!(
        code.find_symbol(b"", &rust, "run").unwrap(),
        std::slice::from_ref(&inner)
    );
    assert_eq!(
        code.find_symbol(b"", &rust, "Outer.run").unwrap(),
        std::slice::from_ref(&inner)
    );
    assert_eq!(
        code.find_symbol(b"", &rust, "Outer").unwrap(),
        std::slice::from_ref(&outer)
    );
    assert_eq!(code.enclosing(b"", &rust, 15).unwrap(), Some(inner));
    assert_eq!(code.enclosing(b"", &rust, 50).unwrap(), Some(outer));
    assert_eq!(code.enclosing(b"", &rust, 500).unwrap(), None);
    assert_eq!(code.classify_spans(b"", &rust).unwrap(), [class]);
    assert_eq!(code.check_syntax(b"", &rust).unwrap(), [issue]);
    assert_eq!(code.references(b"", &rust, "run").unwrap(), [reference]);
    assert!(code.references(b"", &rust, "other").unwrap().is_empty());
    assert_eq!(code.function_metrics(b"", &rust).unwrap(), [metrics]);
    assert_eq!(code.tokens(b"", &rust).unwrap(), [token]);
}

#[test]
fn static_patterns_list_distinct_metavariables() {
    let rust = LanguageId::new("rust").unwrap();
    let code = StaticCodeService::new().with_language(info(&rust, "rs"));
    let pattern = code.compile_pattern("f($A, $$$B, $A, $)", &rust).unwrap();
    assert_eq!(pattern.metavariables(), ["A", "B"]);
    assert_eq!(pattern.language(), &rust);
    assert_eq!(pattern.source(), "f($A, $$$B, $A, $)");
    assert!(
        pattern
            .as_any()
            .is::<nurt_testkit::services::StaticPattern>()
    );
    let found = SyntaxMatch {
        span: span(0, 1),
        node_kind: "call".to_owned(),
        text: String::new(),
        captures: Vec::new(),
    };
    assert_eq!(code.syntax_search(b"", pattern.as_ref()).unwrap(), []);
    assert_eq!(code.render_template(&found, "x").unwrap(), "x");
}

#[test]
fn static_git_answers_registered_lists() {
    let root = Path::new("/repo");
    let history = |path: &str, commits| FileHistoryStat {
        path: path.to_owned(),
        commits,
        authors: 1,
        last_change: None,
    };
    let git = StaticGitQuery::new("/repo")
        .with_changed(["a.rs", "b.rs"])
        .with_tracked(["a.rs"])
        .with_ignored(["target/", "notes.txt"])
        .with_history(vec![
            history("a.rs", 1),
            history("b.rs", 5),
            history("c.rs", 3),
        ])
        .with_status(vec![GitPathStatus {
            path: "a.rs".to_owned(),
            index: GitChange::Added,
            worktree: GitChange::Unmodified,
            original_path: None,
        }]);
    assert_eq!(
        git.repo_root(Path::new("/repo/src")).unwrap(),
        Some(root.to_path_buf())
    );
    assert_eq!(git.repo_root(Path::new("/other")).unwrap(), None);
    assert_eq!(git.changed_files(root, None).unwrap(), ["a.rs", "b.rs"]);
    assert_eq!(git.tracked_files(root).unwrap(), ["a.rs"]);
    assert!(git.is_ignored(root, "target/debug/x").unwrap());
    assert!(git.is_ignored(root, "notes.txt").unwrap());
    assert!(!git.is_ignored(root, "src/lib.rs").unwrap());
    let top = git.file_history(root, 30, 2).unwrap();
    assert_eq!(
        top.iter()
            .map(|stat| stat.path.as_str())
            .collect::<Vec<_>>(),
        ["b.rs", "c.rs"]
    );
    assert_eq!(git.status(root, &[]).unwrap().len(), 1);
}

#[test]
fn text_codec_detects_endings_byte_order_marks_and_binary() {
    let codec = Utf8TextCodec;
    assert_eq!(codec.detect(b"abc").line_ending, LineEnding::None);
    assert_eq!(codec.detect(b"a\nb\n").line_ending, LineEnding::Lf);
    assert_eq!(codec.detect(b"a\r\nb\r\n").line_ending, LineEnding::CrLf);
    assert_eq!(codec.detect(b"a\rb\r").line_ending, LineEnding::Cr);
    assert!(codec.detect(b"a\0b").binary);
    assert!(codec.detect(&[0xFF, 0xFE]).binary);
    let with_mark = codec.detect(&[0xEF, 0xBB, 0xBF, b'a']);
    assert!(with_mark.encoding.unwrap().bom);

    let utf8 = TextEncoding::utf8();
    let marked = TextEncoding {
        bom: true,
        ..utf8.clone()
    };
    assert_eq!(codec.encode("é", &utf8).unwrap(), "é".as_bytes());
    assert_eq!(
        codec.encode("a", &marked).unwrap(),
        [0xEF, 0xBB, 0xBF, b'a']
    );
    assert_eq!(codec.decode(&[0xEF, 0xBB, 0xBF, b'a'], &utf8).unwrap(), "a");
    let latin = TextEncoding {
        label: "windows-1252".to_owned(),
        bom: false,
    };
    assert_eq!(
        codec.encode("a", &latin).unwrap_err().code(),
        ErrorCode::Encoding
    );
}

#[test]
fn registered_holders_and_rules_are_returned() {
    let holder = FileHolder {
        pid: 7,
        name: "editor".to_owned(),
        kind: None,
    };
    let holders = StaticFileHolders::new().with_holders("/a.txt", vec![holder.clone()]);
    assert_eq!(holders.holders(Path::new("/a.txt")).unwrap(), [holder]);
    assert!(holders.holders(Path::new("/b.txt")).unwrap().is_empty());
    let rule = ReviewRule {
        id: "r1".to_owned(),
        pattern: "todo".to_owned(),
        severity: RuleSeverity::Warning,
        summary: "unfinished".to_owned(),
    };
    assert_eq!(StaticReviewRules(vec![rule.clone()]).rules(), [rule]);
}

#[test]
fn scripted_programs_report_names_signals_and_kills() {
    let clock = Arc::new(VirtualClock::new());
    let host = ScriptedProcessHost::new(clock.clone());
    let script = ProgramScript::new("crash").killed_by(11);
    assert_eq!(script.program(), "crash");
    host.script(script);
    host.script(
        ProgramScript::new("daemon")
            .stderr(Duration::from_secs(1), "log\n")
            .runs_for(Duration::from_secs(10))
            .exits_with(0),
    );
    let spec = nurt_contracts::services::ProgramSpec::new("crash", Vec::new(), "/");
    let outcome = host.run_program(&spec, &CancelToken::new()).unwrap();
    assert_eq!(outcome.exit_code, None);
    assert_eq!(outcome.signal, Some(11));

    let mut child = host.spawn_service(&spawn_spec("daemon")).unwrap();
    let started = child.started();
    assert_eq!(started, clock.now());
    let mut stdin = child.take_stdin().unwrap();
    stdin.write_all(b"x").unwrap();
    stdin.flush().unwrap();
    let mut stderr = child.take_stderr().unwrap();
    let mut line = String::new();
    stderr.read_to_string(&mut line).unwrap();
    assert_eq!(line, "log\n");
    assert!(child.take_stderr().is_none());
    clock.advance(Duration::from_secs(20));
    assert_eq!(child.try_wait().unwrap().unwrap().exit_code, Some(0));
    child.kill_tree().unwrap();
    let after_kill = child.wait_timeout(Duration::from_secs(1)).unwrap().unwrap();
    assert!(after_kill.signal.is_some());
}

#[test]
fn scripted_runs_stop_when_cancelled_while_waiting() {
    let host = ScriptedProcessHost::new(Arc::new(VirtualClock::new()));
    host.script(ProgramScript::new("slow").runs_for(Duration::from_secs(60)));
    let cancel = CancelToken::new();
    cancel.cancel(nurt_contracts::stage::CancelReason::Requested);
    let spec = nurt_contracts::services::ProgramSpec::new("slow", Vec::new(), "/");
    assert!(host.run_program(&spec, &cancel).unwrap().cancelled);
}
