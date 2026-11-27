// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-contracts/tests/services_contract.rs
// Responsibility:   Contract tests of the shared services: object safety, the registry, identifiers and the serialized form of the data types.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Contract tests of the shared services: object safety, the registry,
//! identifiers and the serialized form of the data types.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::any::Any;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::env::WallTime;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::{
    ByteRange, ChildExit, ClassifiedSpan, CodeService, DocFormatId, DocFormatInfo, DocItem,
    DocService, ExcludePreset, FileHistoryStat, FileHolder, FileHolderService, FileListService,
    FunctionMetrics, GitChange, GitPathStatus, GitQueryService, IssueSeverity, LanguageId,
    LanguageInfo, LineEnding, ProcessHost, ProgramOutcome, ProgramSpec, ReferenceRole, ReviewRule,
    ReviewRuleProvider, RuleSeverity, SerializeOptions, ServiceChild, ServiceKind,
    ServiceSpawnSpec, Services, SpanClass, SymbolInfo, SymbolKind, SymbolReference, SyntaxCapture,
    SyntaxIssue, SyntaxMatch, SyntaxPattern, SyntaxToken, TextCodecService, TextDetection,
    TextEncoding, TextPosition, TextSpan, WalkEntry, WalkIter, WalkKind, WalkSpec,
};
use nurt_contracts::stage::CancelToken;
use nurt_contracts::value::{Record, Value};

fn assert_service<T: ?Sized + Send + Sync>() {}

#[test]
fn service_traits_are_object_safe_and_thread_safe() {
    assert_service::<dyn DocService>();
    assert_service::<dyn CodeService>();
    assert_service::<dyn ProcessHost>();
    assert_service::<dyn FileListService>();
    assert_service::<dyn GitQueryService>();
    assert_service::<dyn TextCodecService>();
    assert_service::<dyn FileHolderService>();
    assert_service::<dyn ReviewRuleProvider>();
    assert_service::<dyn SyntaxPattern>();
    assert_service::<Services>();
    fn assert_send<T: ?Sized + Send>() {}
    assert_send::<dyn ServiceChild>();
}

fn span(start: u64, end: u64) -> TextSpan {
    TextSpan {
        bytes: ByteRange::new(start, end),
        start: TextPosition {
            line: 1,
            column: u32::try_from(start).unwrap() + 1,
        },
        end: TextPosition {
            line: 1,
            column: u32::try_from(end).unwrap() + 1,
        },
    }
}

fn unsupported(what: &str) -> Error {
    Error::new(
        ErrorCode::UnsupportedLanguage,
        format!("no grammar for {what}"),
    )
}

struct MockDocs;

impl DocService for MockDocs {
    fn formats(&self) -> Vec<DocFormatInfo> {
        vec![DocFormatInfo {
            id: DocFormatId::JSON,
            extensions: vec!["json".to_owned()],
            editable: true,
        }]
    }

    fn detect_format(&self, path: Option<&Path>, bytes: &[u8]) -> Option<DocFormatId> {
        let by_name = path.and_then(Path::extension).is_some_and(|e| e == "json");
        (by_name || bytes.starts_with(b"{")).then_some(DocFormatId::JSON)
    }

    fn read_keys(
        &self,
        _bytes: &[u8],
        _format: &DocFormatId,
        key: &KeyPath,
        _depth: Option<u32>,
    ) -> Result<Vec<DocItem>> {
        Ok(vec![DocItem {
            key: key.clone(),
            value: Value::Int(1),
            span: Some(span(7, 8)),
            comment: None,
            document: None,
            anchor: None,
            alias_of: None,
            tag: None,
        }])
    }

    fn check_syntax(&self, bytes: &[u8], _format: &DocFormatId) -> Result<Vec<SyntaxIssue>> {
        Ok(if bytes.ends_with(b"}") {
            Vec::new()
        } else {
            vec![SyntaxIssue {
                severity: IssueSeverity::Error,
                message: "expected `}`".to_owned(),
                span: span(0, 0),
                kind: Some("missing".to_owned()),
            }]
        })
    }

    fn serialize_records(
        &self,
        records: &[Record],
        _format: &DocFormatId,
        _options: &SerializeOptions,
    ) -> Result<Vec<u8>> {
        Ok(format!("[{}]", records.len()).into_bytes())
    }
}

#[derive(Debug)]
struct MockPattern {
    language: LanguageId,
    source: String,
}

impl SyntaxPattern for MockPattern {
    fn language(&self) -> &LanguageId {
        &self.language
    }

    fn source(&self) -> &str {
        &self.source
    }

    fn metavariables(&self) -> Vec<String> {
        vec!["X".to_owned()]
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct MockCode;

impl MockCode {
    fn check(language: &LanguageId) -> Result<()> {
        if language.as_str() == "rust" {
            Ok(())
        } else {
            Err(unsupported(language.as_str()))
        }
    }

    fn symbol() -> SymbolInfo {
        SymbolInfo {
            name: "main".to_owned(),
            kind: SymbolKind::Function,
            container: None,
            depth: 0,
            span: span(0, 12),
            name_span: span(3, 7),
            signature: Some("fn main()".to_owned()),
            exported: Some(false),
        }
    }
}

impl CodeService for MockCode {
    fn languages(&self) -> Vec<LanguageInfo> {
        vec![LanguageInfo {
            id: LanguageId::new("rust").unwrap(),
            name: "Rust".to_owned(),
            extensions: vec!["rs".to_owned()],
            grammar_version: Some("0.24".to_owned()),
        }]
    }

    fn language_of(&self, path: Option<&Path>, _bytes: &[u8]) -> Option<LanguageId> {
        path.and_then(Path::extension)
            .is_some_and(|e| e == "rs")
            .then(|| LanguageId::new("rust").unwrap())
    }

    fn outline(&self, _: &[u8], language: &LanguageId, _: Option<u32>) -> Result<Vec<SymbolInfo>> {
        Self::check(language)?;
        Ok(vec![Self::symbol()])
    }

    fn find_symbol(&self, _: &[u8], language: &LanguageId, name: &str) -> Result<Vec<SymbolInfo>> {
        Self::check(language)?;
        Ok(if name == "main" {
            vec![Self::symbol()]
        } else {
            Vec::new()
        })
    }

    fn enclosing(
        &self,
        _: &[u8],
        language: &LanguageId,
        offset: u64,
    ) -> Result<Option<SymbolInfo>> {
        Self::check(language)?;
        Ok((offset < 12).then(Self::symbol))
    }

    fn classify_spans(&self, _: &[u8], language: &LanguageId) -> Result<Vec<ClassifiedSpan>> {
        Self::check(language)?;
        Ok(vec![ClassifiedSpan {
            bytes: ByteRange::new(13, 20),
            class: SpanClass::Comment,
        }])
    }

    fn check_syntax(&self, _: &[u8], language: &LanguageId) -> Result<Vec<SyntaxIssue>> {
        Self::check(language)?;
        Ok(Vec::new())
    }

    fn compile_pattern(
        &self,
        pattern: &str,
        language: &LanguageId,
    ) -> Result<Box<dyn SyntaxPattern>> {
        Self::check(language)?;
        Ok(Box::new(MockPattern {
            language: language.clone(),
            source: pattern.to_owned(),
        }))
    }

    fn syntax_search(&self, bytes: &[u8], pattern: &dyn SyntaxPattern) -> Result<Vec<SyntaxMatch>> {
        let own = pattern.as_any().downcast_ref::<MockPattern>().unwrap();
        let text = String::from_utf8_lossy(bytes).into_owned();
        Ok(vec![SyntaxMatch {
            span: span(0, u64::try_from(bytes.len()).unwrap()),
            node_kind: "call_expression".to_owned(),
            text,
            captures: vec![SyntaxCapture {
                name: own.metavariables()[0].clone(),
                spans: vec![span(4, 5)],
                text: "1".to_owned(),
            }],
        }])
    }

    fn render_template(&self, found: &SyntaxMatch, template: &str) -> Result<String> {
        let mut out = template.to_owned();
        for capture in &found.captures {
            out = out.replace(&format!("${}", capture.name), &capture.text);
        }
        Ok(out)
    }

    fn references(&self, _: &[u8], language: &LanguageId, _: &str) -> Result<Vec<SymbolReference>> {
        Self::check(language)?;
        Ok(vec![SymbolReference {
            span: span(3, 7),
            role: ReferenceRole::Definition,
            class: SpanClass::Code,
            container: None,
            line_text: "fn main() {}".to_owned(),
        }])
    }

    fn function_metrics(&self, _: &[u8], language: &LanguageId) -> Result<Vec<FunctionMetrics>> {
        Self::check(language)?;
        Ok(vec![FunctionMetrics {
            name: "main".to_owned(),
            container: None,
            span: span(0, 12),
            lines: 1,
            parameters: 0,
            cyclomatic: 1,
            max_nesting: 0,
        }])
    }

    fn tokens(&self, _: &[u8], language: &LanguageId) -> Result<Vec<SyntaxToken>> {
        Self::check(language)?;
        Ok(vec![SyntaxToken {
            bytes: ByteRange::new(0, 2),
            kind: "fn".to_owned(),
            class: SpanClass::Code,
        }])
    }
}

struct MockChild {
    stdout: Option<Cursor<Vec<u8>>>,
    killed: bool,
}

impl ServiceChild for MockChild {
    fn pid(&self) -> u32 {
        4242
    }

    fn started(&self) -> WallTime {
        WallTime::from_unix_nanos(1_000)
    }

    fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        Some(Box::new(Vec::new()))
    }

    fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        self.stdout
            .take()
            .map(|c| Box::new(c) as Box<dyn Read + Send>)
    }

    fn take_stderr(&mut self) -> Option<Box<dyn Read + Send>> {
        None
    }

    fn try_wait(&mut self) -> Result<Option<ChildExit>> {
        Ok(self.killed.then_some(ChildExit {
            exit_code: None,
            signal: Some(9),
        }))
    }

    fn wait_timeout(&mut self, _timeout: Duration) -> Result<Option<ChildExit>> {
        self.try_wait()
    }

    fn kill_tree(&mut self) -> Result<()> {
        self.killed = true;
        Ok(())
    }
}

struct MockProcesses;

impl ProcessHost for MockProcesses {
    fn resolve_program(&self, program: &str, _cwd: &Path) -> Result<PathBuf> {
        Ok(PathBuf::from("/usr/bin").join(program))
    }

    fn run_program(&self, spec: &ProgramSpec, cancel: &CancelToken) -> Result<ProgramOutcome> {
        Ok(ProgramOutcome {
            program_path: self.resolve_program(&spec.program, &spec.cwd)?,
            exit_code: Some(0),
            cancelled: cancel.is_cancelled(),
            stdout: spec.args.join(" ").into_bytes(),
            ..ProgramOutcome::default()
        })
    }

    fn spawn_service(&self, _spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
        Ok(Box::new(MockChild {
            stdout: Some(Cursor::new(b"Content-Length: 2\r\n\r\n{}".to_vec())),
            killed: false,
        }))
    }
}

struct MockFiles;

impl FileListService for MockFiles {
    fn walk<'a>(&'a self, spec: &WalkSpec) -> Result<WalkIter<'a>> {
        let root = spec.root.clone();
        let skip_git = spec.exclude_presets.contains(&ExcludePreset::Vcs);
        let names = ["src/main.rs", ".git/config", "README.md"];
        Ok(Box::new(
            names
                .into_iter()
                .filter(move |n| !(skip_git && n.starts_with(".git/")))
                .map(move |n| -> Result<WalkEntry> {
                    Ok(WalkEntry {
                        path: root.join(n),
                        depth: 2,
                        is_dir: false,
                    })
                }),
        ))
    }
}

struct MockGit;

impl GitQueryService for MockGit {
    fn repo_root(&self, path: &Path) -> Result<Option<PathBuf>> {
        Ok(path.ancestors().last().map(Path::to_path_buf))
    }

    fn status(&self, _root: &Path, _paths: &[String]) -> Result<Vec<GitPathStatus>> {
        Ok(vec![GitPathStatus {
            path: "src/lib.rs".to_owned(),
            index: GitChange::Unmodified,
            worktree: GitChange::Modified,
            original_path: None,
        }])
    }

    fn changed_files(&self, _root: &Path, _against: Option<&str>) -> Result<Vec<String>> {
        Ok(vec!["src/lib.rs".to_owned()])
    }

    fn tracked_files(&self, _root: &Path) -> Result<Vec<String>> {
        Ok(vec!["src/lib.rs".to_owned(), "Cargo.toml".to_owned()])
    }

    fn is_ignored(&self, _root: &Path, path: &str) -> Result<bool> {
        Ok(path.starts_with("target/"))
    }

    fn file_history(&self, _root: &Path, _days: u32, limit: u32) -> Result<Vec<FileHistoryStat>> {
        let stat = FileHistoryStat {
            path: "src/lib.rs".to_owned(),
            commits: 12,
            authors: 3,
            last_change: Some(WallTime::from_unix_nanos(5)),
        };
        Ok(std::iter::repeat_n(stat, usize::try_from(limit.min(1)).unwrap()).collect())
    }
}

struct MockCodec;

impl TextCodecService for MockCodec {
    fn detect(&self, bytes: &[u8]) -> TextDetection {
        let binary = bytes.contains(&0);
        TextDetection {
            binary,
            encoding: (!binary).then(TextEncoding::utf8),
            line_ending: if bytes.contains(&b'\n') {
                LineEnding::Lf
            } else {
                LineEnding::None
            },
        }
    }

    fn decode(&self, bytes: &[u8], _encoding: &TextEncoding) -> Result<String> {
        String::from_utf8(bytes.to_vec())
            .map_err(|e| Error::new(ErrorCode::Encoding, e.to_string()))
    }

    fn encode(&self, text: &str, _encoding: &TextEncoding) -> Result<Vec<u8>> {
        Ok(text.as_bytes().to_vec())
    }
}

struct MockHolders;

impl FileHolderService for MockHolders {
    fn holders(&self, _path: &Path) -> Result<Vec<FileHolder>> {
        Ok(vec![FileHolder {
            pid: 7,
            name: "editor".to_owned(),
            kind: None,
        }])
    }
}

struct MockRules;

impl ReviewRuleProvider for MockRules {
    fn rules(&self) -> Vec<ReviewRule> {
        vec![ReviewRule {
            id: "private_key".to_owned(),
            pattern: "-----BEGIN [A-Z ]*PRIVATE KEY-----".to_owned(),
            severity: RuleSeverity::Error,
            summary: "Private key material.".to_owned(),
        }]
    }
}

fn full_registry() -> Services {
    Services::new()
        .with_doc(Arc::new(MockDocs))
        .with_code(Arc::new(MockCode))
        .with_process_host(Arc::new(MockProcesses))
        .with_file_list(Arc::new(MockFiles))
        .with_git_query(Arc::new(MockGit))
        .with_text_codec(Arc::new(MockCodec))
        .with_file_holders(Arc::new(MockHolders))
        .with_review_rules(Arc::new(MockRules))
}

#[test]
fn empty_registry_has_no_services() {
    let services = Services::default();
    assert!(services.available().is_empty());
    for kind in ServiceKind::ALL {
        assert!(!services.has(kind), "{kind}");
    }
    assert!(services.doc().is_none());
    assert!(services.code().is_none());
    assert!(services.process_host().is_none());
    assert!(services.file_list().is_none());
    assert!(services.git_query().is_none());
    assert!(services.text_codec().is_none());
    assert!(services.file_holders().is_none());
    assert!(services.review_rules().is_none());
    assert_eq!(format!("{services:?}"), "Services { available: [] }");
}

#[test]
fn registry_exposes_every_service() {
    let services = full_registry();
    assert_eq!(services.available(), ServiceKind::ALL.to_vec());
    let partial = Services::new().with_code(Arc::new(MockCode));
    assert_eq!(partial.available(), vec![ServiceKind::Code]);
    let cloned = services.clone();
    assert!(cloned.has(ServiceKind::ReviewRules));
}

#[test]
fn document_service_through_the_registry() {
    let services = full_registry();
    let docs = services.doc().unwrap();
    let format = docs
        .detect_format(Some(Path::new("package.json")), b"{}")
        .unwrap();
    assert_eq!(format, DocFormatId::JSON);
    let key = KeyPath::parse("scripts.test").unwrap();
    let items = docs
        .read_keys(b"{\"scripts\":{\"test\":1}}", &format, &key, None)
        .unwrap();
    assert_eq!(items[0].key, key);
    assert_eq!(items[0].value, Value::Int(1));
    assert!(docs.check_syntax(b"{}", &format).unwrap().is_empty());
    assert_eq!(
        docs.check_syntax(b"{", &format).unwrap()[0].severity,
        IssueSeverity::Error
    );
    let bytes = docs
        .serialize_records(&[], &format, &SerializeOptions::default())
        .unwrap();
    assert_eq!(bytes, b"[0]");
}

#[test]
fn code_service_through_the_registry() {
    let services = full_registry();
    let code = services.code().unwrap();
    let rust = code.language_of(Some(Path::new("main.rs")), b"").unwrap();
    assert_eq!(code.languages()[0].id, rust);
    assert_eq!(
        code.outline(b"fn main() {}", &rust, None).unwrap()[0].kind,
        SymbolKind::Function
    );
    assert_eq!(code.find_symbol(b"", &rust, "main").unwrap().len(), 1);
    assert!(code.find_symbol(b"", &rust, "other").unwrap().is_empty());
    assert!(code.enclosing(b"", &rust, 3).unwrap().is_some());
    assert_eq!(
        code.classify_spans(b"", &rust).unwrap()[0].class,
        SpanClass::Comment
    );
    assert!(code.check_syntax(b"", &rust).unwrap().is_empty());
    let pattern = code.compile_pattern("foo($X)", &rust).unwrap();
    assert_eq!(pattern.source(), "foo($X)");
    assert_eq!(pattern.language(), &rust);
    let found = code.syntax_search(b"foo(1)", pattern.as_ref()).unwrap();
    assert_eq!(
        code.render_template(&found[0], "bar($X)").unwrap(),
        "bar(1)"
    );
    assert_eq!(
        code.references(b"", &rust, "main").unwrap()[0].role,
        ReferenceRole::Definition
    );
    assert_eq!(code.function_metrics(b"", &rust).unwrap()[0].cyclomatic, 1);
    assert_eq!(code.tokens(b"", &rust).unwrap()[0].kind, "fn");

    let python = LanguageId::new("python").unwrap();
    let error = code.outline(b"", &python, None).unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnsupportedLanguage);
}

#[test]
fn process_host_through_the_registry() {
    let services = full_registry();
    let host = services.process_host().unwrap();
    let spec = ProgramSpec::new("git", vec!["status".to_owned()], "/home/user/project");
    assert!(spec.inherit_env);
    let cancel = CancelToken::new();
    let outcome = host.run_program(&spec, &cancel).unwrap();
    assert_eq!(outcome.program_path, PathBuf::from("/usr/bin/git"));
    assert_eq!(outcome.stdout, b"status");
    assert!(!outcome.cancelled);

    let spawn = ServiceSpawnSpec {
        program: "rust-analyzer".to_owned(),
        args: Vec::new(),
        cwd: PathBuf::from("/home/user/project"),
        env: Vec::new(),
        inherit_env: true,
        temp_dir: None,
    };
    let mut child = host.spawn_service(&spawn).unwrap();
    assert_eq!(child.pid(), 4242);
    assert_eq!(child.started(), WallTime::from_unix_nanos(1_000));
    let mut header = String::new();
    child
        .take_stdout()
        .unwrap()
        .read_to_string(&mut header)
        .unwrap();
    assert!(header.starts_with("Content-Length"));
    assert!(child.take_stdout().is_none());
    assert!(child.take_stderr().is_none());
    child.take_stdin().unwrap().write_all(b"{}").unwrap();
    assert_eq!(child.try_wait().unwrap(), None);
    child.kill_tree().unwrap();
    assert_eq!(
        child
            .wait_timeout(Duration::from_millis(10))
            .unwrap()
            .unwrap()
            .signal,
        Some(9)
    );
}

#[test]
fn other_services_through_the_registry() {
    let services = full_registry();
    let files = services.file_list().unwrap();
    let listed: Vec<_> = files
        .walk(&WalkSpec::source_tree("/home/user/project"))
        .unwrap()
        .map(|e| e.unwrap().path)
        .collect();
    assert_eq!(listed.len(), 2);

    let git = services.git_query().unwrap();
    assert_eq!(
        git.status(Path::new("/r"), &[]).unwrap()[0].worktree,
        GitChange::Modified
    );
    assert_eq!(git.changed_files(Path::new("/r"), None).unwrap().len(), 1);
    assert_eq!(git.tracked_files(Path::new("/r")).unwrap().len(), 2);
    assert!(git.is_ignored(Path::new("/r"), "target/debug").unwrap());
    assert_eq!(
        git.file_history(Path::new("/r"), 90, 5).unwrap()[0].commits,
        12
    );
    assert!(git.repo_root(Path::new("/r/src")).unwrap().is_some());

    let codec = services.text_codec().unwrap();
    let detected = codec.detect(b"a\nb");
    assert_eq!(detected.encoding, Some(TextEncoding::utf8()));
    assert_eq!(detected.line_ending, LineEnding::Lf);
    assert!(codec.detect(b"\0").binary);
    let text = codec.decode(b"abc", &TextEncoding::utf8()).unwrap();
    assert_eq!(codec.encode(&text, &TextEncoding::utf8()).unwrap(), b"abc");
    assert_eq!(
        codec
            .decode(&[0xff], &TextEncoding::utf8())
            .unwrap_err()
            .code(),
        ErrorCode::Encoding
    );

    assert_eq!(
        services
            .file_holders()
            .unwrap()
            .holders(Path::new("/f"))
            .unwrap()[0]
            .pid,
        7
    );
    assert_eq!(
        services.review_rules().unwrap().rules()[0].severity,
        RuleSeverity::Error
    );
}

#[test]
fn identifiers_are_validated() {
    for valid in ["json", "c", "cpp", "objective-c", "x_1", "c++", "a2"] {
        assert_eq!(LanguageId::new(valid).unwrap().as_str(), valid);
    }
    let long = "a".repeat(65);
    for invalid in [
        "",
        "Json",
        "1c",
        "-a",
        "a b",
        "a.b",
        "\u{e9}",
        long.as_str(),
    ] {
        assert!(
            LanguageId::new(invalid).is_err(),
            "`{invalid}` must be rejected"
        );
        assert!(
            DocFormatId::new(invalid).is_err(),
            "`{invalid}` must be rejected"
        );
    }
    for format in DocFormatId::BUILT_IN {
        assert_eq!(DocFormatId::new(format.as_str()).unwrap(), format);
    }
    let error = "Bad".parse::<DocFormatId>().unwrap_err();
    assert!(error.to_string().contains("`Bad`"));
    assert_eq!(DocFormatId::TOML.to_string(), "toml");
}

#[test]
fn identifiers_serialize_as_text() {
    let json = serde_json::to_value(DocFormatId::YAML).unwrap();
    assert_eq!(json, serde_json::json!("yaml"));
    let back: DocFormatId = serde_json::from_value(json).unwrap();
    assert_eq!(back, DocFormatId::YAML);
    assert!(serde_json::from_value::<LanguageId>(serde_json::json!("Rust")).is_err());
    let schema = serde_json::to_value(schemars::schema_for!(LanguageId)).unwrap();
    assert_eq!(schema["type"], "string");
}

#[test]
fn service_kinds_have_unique_names() {
    let names: Vec<&str> = ServiceKind::ALL.iter().map(|k| k.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len());
    for kind in ServiceKind::ALL {
        assert_eq!(
            serde_json::to_value(kind).unwrap(),
            serde_json::json!(kind.as_str())
        );
    }
}

#[test]
fn walk_spec_defaults_match_find() {
    let spec = WalkSpec::new("/home/user/project");
    assert_eq!(spec.globs, vec!["**".to_owned()]);
    assert_eq!(spec.kind, WalkKind::File);
    assert!(spec.hidden);
    assert!(!spec.follow_links);
    assert!(!spec.gitignore);
    assert!(spec.exclude.is_empty());
    assert!(spec.exclude_presets.is_empty());
    assert!(spec.case_sensitive);
    let tree = WalkSpec::source_tree("/home/user/project");
    assert!(tree.gitignore);
    assert_eq!(
        tree.exclude_presets,
        vec![
            ExcludePreset::Vcs,
            ExcludePreset::Deps,
            ExcludePreset::Cache
        ]
    );
}

#[test]
fn data_types_round_trip_through_json() {
    let issue = SyntaxIssue {
        severity: IssueSeverity::Warning,
        message: "duplicate key".to_owned(),
        span: span(2, 5),
        kind: Some("duplicate_key".to_owned()),
    };
    let json = serde_json::to_value(&issue).unwrap();
    assert_eq!(json["severity"], "warning");
    assert_eq!(
        json["span"]["bytes"],
        serde_json::json!({"start": 2, "end": 5})
    );
    assert_eq!(serde_json::from_value::<SyntaxIssue>(json).unwrap(), issue);

    let symbol = MockCode::symbol();
    let json = serde_json::to_value(&symbol).unwrap();
    assert_eq!(json["kind"], "function");
    assert_eq!(serde_json::from_value::<SymbolInfo>(json).unwrap(), symbol);

    let spec = ProgramSpec::new("npm", vec!["test".to_owned()], "/home/user/app");
    let json = serde_json::to_value(&spec).unwrap();
    assert_eq!(json["stdin"], "null");
    assert_eq!(serde_json::from_value::<ProgramSpec>(json).unwrap(), spec);

    let item = DocItem {
        key: KeyPath::parse("a[0]").unwrap(),
        value: Value::Text("x".to_owned()),
        span: None,
        comment: Some("note".to_owned()),
        document: Some(1),
        anchor: None,
        alias_of: None,
        tag: None,
    };
    let json = serde_json::to_value(&item).unwrap();
    assert_eq!(json["key"], "a[0]");
    assert_eq!(serde_json::from_value::<DocItem>(json).unwrap(), item);

    let range = ByteRange::new(3, 7);
    assert_eq!(range.len(), 4);
    assert!(range.contains(3) && !range.contains(7));
    assert!(ByteRange::new(5, 5).is_empty());
    assert!(ByteRange::new(6, 5).is_empty());
}

#[test]
fn schemas_generate_for_data_types() {
    for schema in [
        schemars::schema_for!(SyntaxIssue),
        schemars::schema_for!(DocItem),
        schemars::schema_for!(SymbolInfo),
        schemars::schema_for!(SyntaxMatch),
        schemars::schema_for!(ProgramSpec),
        schemars::schema_for!(ProgramOutcome),
        schemars::schema_for!(WalkSpec),
        schemars::schema_for!(GitPathStatus),
        schemars::schema_for!(TextDetection),
        schemars::schema_for!(ServiceKind),
    ] {
        assert!(serde_json::to_value(&schema).unwrap().is_object());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
