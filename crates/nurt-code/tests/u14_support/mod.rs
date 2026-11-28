// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-code/tests/u14_support/mod.rs
// Responsibility:   Test doubles for the `code.rename` and `code.languages` tests.
// Dependencies:     nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Test doubles for the `code.rename` and `code.languages` tests: a file
//! system over the real disk, a journal double that keeps the previous
//! content of every target, a small code service for Python and
//! TypeScript-like text and a scripted semantic provider.

// Each test target uses a subset of these helpers.
#![allow(dead_code)]

use std::ffi::OsString;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use nurt_code::commands::rename::CodeRename;
use nurt_code::semantic::{
    CallDirection, CallEdge, CodeActionInfo, DocumentChange, DocumentPosition, HoverInfo, Location,
    SemanticDiagnostic, SemanticError, SemanticErrorKind, SemanticProvider, SemanticResult,
    SemanticScope, SemanticSymbol, ServerStatus, TextChange, WorkspaceChange,
};
use nurt_contracts::config::Config;
use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, ContentHash, DirEntry, ExtendedMeta, FileId, FileKind, FileMeta, Follow, Fs, FsError,
    FsErrorKind, FsResult, Holder, PathKey, ReadHandle, SpaceInfo, VolumeId, VolumeInfo,
    WriteHandle,
};
use nurt_contracts::journal::{
    Applied, BeforeState, ExemptWrite, Expect, ExternalChange, Mutation, MutationTx, OpHandle,
    OpId, OpKind, PreserveMode, Preserved, RollbackReason, RollbackReport, TargetAction,
    TargetIndex, TargetPath, TargetSpec,
};
use nurt_contracts::record::DiagnosticSeverity;
use nurt_contracts::report::{Change, IrreversibleEntry, Notable};
use nurt_contracts::services::{
    ByteRange, ClassifiedSpan, CodeService, FunctionMetrics, IssueSeverity, LanguageId,
    LanguageInfo, ReferenceRole, Services, SpanClass, SymbolInfo, SymbolKind, SymbolReference,
    SyntaxIssue, SyntaxMatch, SyntaxPattern, SyntaxToken, TextPosition, TextSpan,
};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelReason, CancelToken, EffectPlan, ExecCtx, StageImpl, StageInput,
};
use nurt_contracts::value::{Record, Value};

/// Clock fixed at the Unix epoch.
pub(crate) struct TestClock;

impl Clock for TestClock {
    fn now(&self) -> WallTime {
        WallTime::UNIX_EPOCH
    }

    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: WallTime::UNIX_EPOCH,
            boot: BootId("test".to_owned()),
            since_boot_with_suspend: Duration::ZERO,
            since_boot_active: Duration::ZERO,
        }
    }

    fn sleep(
        &self,
        _duration: Duration,
        _cancel: &CancelToken,
    ) -> std::result::Result<(), CancelReason> {
        Ok(())
    }
}

/// Host with fixed facts and no environment.
pub(crate) struct TestHost(HostInfo);

impl TestHost {
    pub(crate) fn new() -> Self {
        Self(HostInfo {
            os: Os::Linux,
            os_version: "6".to_owned(),
            architecture: "x86_64".to_owned(),
            hostname: "test".to_owned(),
            user: "test".to_owned(),
            home: nurt_testkit::paths::abs_path("/home/test"),
            temp: std::env::temp_dir(),
            path_style: PathStyle::Posix,
            timezone: "UTC".to_owned(),
            locale: None,
            cpus: 1,
        })
    }
}

impl Host for TestHost {
    fn info(&self) -> &HostInfo {
        &self.0
    }

    fn env_var(&self, _name: &str) -> Option<OsString> {
        None
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        Vec::new()
    }
}

/// Read-only file system over the real disk.
pub(crate) struct StdFs;

fn io_error(error: &std::io::Error, op: &str, path: &Path) -> FsError {
    let kind = if error.kind() == std::io::ErrorKind::NotFound {
        FsErrorKind::NotFound
    } else {
        FsErrorKind::Io
    };
    FsError::new(kind, op, Some(path))
}

fn unsupported<T>(op: &str) -> FsResult<T> {
    Err(FsError::new(FsErrorKind::Unsupported, op, None))
}

fn meta_of(meta: &std::fs::Metadata) -> FileMeta {
    let kind = if meta.is_dir() {
        FileKind::Directory
    } else if meta.is_file() {
        FileKind::File
    } else if meta.file_type().is_symlink() {
        FileKind::Symlink
    } else {
        FileKind::Special
    };
    let modified_ns = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| {
            i64::try_from(since.as_nanos()).unwrap_or(i64::MAX)
        });
    #[cfg(unix)]
    let low = std::os::unix::fs::MetadataExt::ino(meta);
    #[cfg(not(unix))]
    let low = 0;
    FileMeta {
        kind,
        size: meta.len(),
        mode: 0o644,
        uid: None,
        gid: None,
        links: 1,
        modified_ns,
        accessed_ns: None,
        created_ns: None,
        changed_ns: None,
        file_id: FileId {
            volume: 0,
            low,
            high: 0,
        },
        windows_attributes: 0,
    }
}

struct MemoryRead {
    cursor: Cursor<Vec<u8>>,
    meta: FileMeta,
}

impl Read for MemoryRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.cursor.read(buf)
    }
}

impl Seek for MemoryRead {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.cursor.seek(pos)
    }
}

impl ReadHandle for MemoryRead {
    fn meta(&self) -> FsResult<FileMeta> {
        Ok(self.meta.clone())
    }
}

impl Fs for StdFs {
    fn metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<FileMeta> {
        let meta = match follow {
            Follow::Follow => std::fs::metadata(path.as_path()),
            Follow::NoFollow => std::fs::symlink_metadata(path.as_path()),
        }
        .map_err(|error| io_error(&error, "metadata", path.as_path()))?;
        Ok(meta_of(&meta))
    }

    fn extended_metadata(&self, _path: &AbsPath, _follow: Follow) -> FsResult<ExtendedMeta> {
        unsupported("extended_metadata")
    }

    fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
        let meta = self.metadata(path, Follow::Follow)?;
        let bytes = std::fs::read(path.as_path())
            .map_err(|error| io_error(&error, "open_read", path.as_path()))?;
        Ok(Box::new(MemoryRead {
            cursor: Cursor::new(bytes),
            meta,
        }))
    }

    fn open_stream_read(&self, _path: &AbsPath, _stream: &str) -> FsResult<Box<dyn ReadHandle>> {
        unsupported("open_stream_read")
    }

    fn read_dir<'a>(
        &'a self,
        path: &AbsPath,
    ) -> FsResult<Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>> {
        let mut names: Vec<OsString> = std::fs::read_dir(path.as_path())
            .map_err(|error| io_error(&error, "read_dir", path.as_path()))?
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.file_name())
            .collect();
        names.sort();
        Ok(Box::new(
            names
                .into_iter()
                .map(|name| Ok(DirEntry { name, kind: None })),
        ))
    }

    fn read_link(&self, _path: &AbsPath) -> FsResult<PathBuf> {
        unsupported("read_link")
    }

    fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
        Ok(path.clone())
    }

    fn content_hash(&self, _path: &AbsPath) -> FsResult<ContentHash> {
        unsupported("content_hash")
    }

    fn volume(&self, _path: &AbsPath) -> FsResult<VolumeInfo> {
        unsupported("volume")
    }

    fn space(&self, _volume: &VolumeId) -> FsResult<SpaceInfo> {
        unsupported("space")
    }

    fn path_key(&self, _path: &AbsPath) -> FsResult<PathKey> {
        unsupported("path_key")
    }

    fn holders(&self, _path: &AbsPath) -> FsResult<Vec<Holder>> {
        Ok(Vec::new())
    }

    fn can_write_dir(&self, _dir: &AbsPath) -> FsResult<bool> {
        Ok(true)
    }
}

/// Record of a journal double: the previous state of every target, kept
/// before the first change, and what happened to the transaction.
#[derive(Debug, Default)]
pub(crate) struct JournalLog {
    pub(crate) saved: Vec<(PathBuf, Option<Vec<u8>>)>,
    pub(crate) preserve_calls: usize,
    pub(crate) mutations: Vec<String>,
    pub(crate) committed: bool,
    pub(crate) rolled_back: bool,
}

impl JournalLog {
    /// Restores every target to its previous state, newest first: the
    /// effect of `undo` on a committed entry and of a rollback.
    pub(crate) fn restore(&mut self) {
        for (path, bytes) in self.saved.iter().rev() {
            match bytes {
                Some(bytes) => {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).unwrap();
                    }
                    std::fs::write(path, bytes).unwrap();
                }
                None => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    }
}

/// Mutation transaction applying changes to the disk and keeping the
/// previous content in a shared [`JournalLog`].
pub(crate) struct TestTx {
    log: Arc<Mutex<JournalLog>>,
    targets: Vec<TargetSpec>,
}

impl TestTx {
    pub(crate) fn new(log: Arc<Mutex<JournalLog>>) -> Self {
        Self {
            log,
            targets: Vec::new(),
        }
    }
}

fn fs_path(path: &TargetPath) -> PathBuf {
    match path {
        TargetPath::Fs(path) => path.as_path().to_path_buf(),
        TargetPath::Provider(_) => panic!("provider targets are not used by code.rename"),
    }
}

fn check_expect(path: &Path, expect: &Expect) -> Result<()> {
    let current = std::fs::metadata(path)
        .ok()
        .map(|meta| meta_of(&meta).etag());
    match (expect, current) {
        (Expect::Etag(planned), Some(current)) if !planned.matches(&current) => Err(Error::new(
            ErrorCode::Stale,
            format!("{} changed", path.display()),
        )),
        (Expect::Etag(_), None) => Err(Error::new(ErrorCode::Stale, "target vanished")),
        (Expect::Absent, Some(_)) => Err(Error::new(ErrorCode::Exists, "target exists")),
        _ => Ok(()),
    }
}

impl MutationTx for TestTx {
    fn op(&self) -> Option<OpId> {
        Some(OpId(1))
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        let mut log = self.log.lock().unwrap();
        log.preserve_calls += 1;
        let mut preserved = Vec::new();
        for spec in targets {
            let path = fs_path(&spec.path);
            check_expect(&path, &spec.expect)?;
            log.saved.push((path.clone(), std::fs::read(&path).ok()));
            if let TargetAction::Rename { to, .. } = &spec.action {
                let to = to.as_path().to_path_buf();
                log.saved.push((to.clone(), std::fs::read(&to).ok()));
            }
            let index = TargetIndex(u32::try_from(self.targets.len()).unwrap());
            self.targets.push(spec.clone());
            preserved.push(Preserved {
                index,
                volume: None,
                mode: PreserveMode::Copy,
                bytes_held: 0,
                before: BeforeState::default(),
                irreversible: None,
                notable: Vec::new(),
            });
        }
        Ok(preserved)
    }

    fn stage(&mut self, _target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        Err(Error::new(
            ErrorCode::Unsupported,
            "staging is not used by code.rename",
        ))
    }

    fn apply(&mut self, target: TargetIndex, mutation: Mutation) -> Result<Applied> {
        let spec = self.targets[usize::try_from(target.0).unwrap()].clone();
        let path = fs_path(&spec.path);
        let mut log = self.log.lock().unwrap();
        match (&spec.action, mutation) {
            (TargetAction::Modify | TargetAction::Create, Mutation::WriteBytes(bytes)) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                std::fs::write(&path, bytes).unwrap();
                log.mutations.push(format!("write {}", path.display()));
            }
            (TargetAction::Delete, Mutation::Remove) => {
                std::fs::remove_file(&path).unwrap();
                log.mutations.push(format!("remove {}", path.display()));
            }
            (TargetAction::Rename { to, .. }, Mutation::Rename) => {
                std::fs::rename(&path, to.as_path()).unwrap();
                log.mutations.push(format!("rename {}", path.display()));
            }
            (action, mutation) => panic!("mutation {mutation:?} does not match {action:?}"),
        }
        Ok(Applied {
            index: target,
            after: nurt_contracts::journal::AfterState::default(),
        })
    }

    fn observe_external(&mut self, _changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        Ok(Vec::new())
    }

    fn mark_irreversible(&mut self, _marker: IrreversibleEntry) {}

    fn record_exempt(&mut self, _write: ExemptWrite) {}

    fn commit(self: Box<Self>) -> Result<OpHandle> {
        self.log.lock().unwrap().committed = true;
        Ok(OpHandle {
            op: Some(OpId(1)),
            kind: OpKind::command("code.rename"),
            bulk: self.targets.len() > 1,
            targets: Vec::new(),
            irreversible: Vec::new(),
            notable: Vec::new(),
            exempt: Vec::new(),
        })
    }

    fn rollback(self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
        let mut log = self.log.lock().unwrap();
        log.restore();
        log.rolled_back = true;
        Ok(RollbackReport {
            op: Some(OpId(1)),
            restored: Vec::new(),
            failed: Vec::new(),
        })
    }
}

/// Result of a rename run by [`execute`].
pub(crate) struct Outcome {
    pub(crate) records: Vec<Record>,
    pub(crate) changes: Vec<Change>,
    pub(crate) notable: Vec<Notable>,
}

/// Runs plan and apply as the executor does: commits after success and
/// rolls back after an error of the apply step.
pub(crate) fn execute(
    stage: &CodeRename,
    args: &BoundArgs,
    input: StageInput<'_>,
    ctx: &ExecCtx,
    log: &Arc<Mutex<JournalLog>>,
) -> Result<Outcome> {
    let effect = stage.as_effect().unwrap();
    let plan = effect.plan(args, input, ctx)?;
    let mut tx = Box::new(TestTx::new(Arc::clone(log)));
    match effect.apply(args, plan, tx.as_mut(), ctx) {
        Ok(outcome) => {
            tx.commit()?;
            let records = StageInput::from(outcome.output)
                .into_records()?
                .collect::<Result<Vec<_>>>()?;
            Ok(Outcome {
                records,
                changes: outcome.changes,
                notable: outcome.notable.iter().map(|entry| entry.code).collect(),
            })
        }
        Err(error) => {
            tx.rollback(RollbackReason::ValidateFailed)?;
            Err(error)
        }
    }
}

/// Runs only the plan step (`preview`).
pub(crate) fn preview(stage: &CodeRename, args: &BoundArgs, ctx: &ExecCtx) -> Result<EffectPlan> {
    stage
        .as_effect()
        .unwrap()
        .plan(args, StageInput::none(), ctx)
}

/// Bound arguments of `code.rename`.
pub(crate) fn args(pairs: &[(&str, Value)]) -> BoundArgs {
    let mut args = BoundArgs::new("code.rename");
    for (name, value) in pairs {
        args.insert(*name, ArgValue::Value(value.clone()), false);
    }
    args
}

pub(crate) fn path_value(path: &Path) -> Value {
    Value::Path(path.to_string_lossy().into_owned())
}

/// Execution context with the disk, the toy code service and `config`.
pub(crate) fn context(config: Config) -> ExecCtx {
    ExecCtx::builder(Arc::new(TestClock), Arc::new(TestHost::new()))
        .fs(Arc::new(StdFs))
        .services(Arc::new(Services::new().with_code(Arc::new(ToyCode))))
        .config(Arc::new(config))
        .build()
}

/// Project directory with a `.git` marker. On macOS the temporary directory
/// lies behind the link `/var`, and the servers answer with the real path, so
/// the project is named by its real path there.
pub(crate) fn project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let base = if cfg!(target_os = "macos") {
        dir.path().canonicalize().unwrap()
    } else {
        dir.path().to_path_buf()
    };
    let root = base.join("proj");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    (dir, root)
}

pub(crate) fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

pub(crate) fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// Code service for small Python (`.py`, `#` comments) and TypeScript or
/// JavaScript (`.ts`, `.js`, `//` comments) files: identifiers, comments,
/// string literals, definitions introduced by `def`, `class` or `function`,
/// and unbalanced parentheses as the only syntax error.
pub(crate) struct ToyCode;

const KEYWORDS: &[&str] = &[
    "def", "class", "function", "import", "from", "return", "const", "let", "export",
];

struct Lexed {
    identifiers: Vec<(usize, usize)>,
    spans: Vec<ClassifiedSpan>,
}

fn comment_marker(language: &LanguageId) -> &'static str {
    if language.as_str() == "python" {
        "#"
    } else {
        "//"
    }
}

fn lex(text: &str, comment: &str) -> Lexed {
    let mut lexed = Lexed {
        identifiers: Vec::new(),
        spans: Vec::new(),
    };
    let mut index = 0;
    while let Some(character) = text[index..].chars().next() {
        if text[index..].starts_with(comment) {
            let end = text[index..].find('\n').map_or(text.len(), |n| index + n);
            lexed.spans.push(classified(index, end, SpanClass::Comment));
            index = end;
        } else if character == '"' || character == '\'' {
            let end = text[index + 1..]
                .find(character)
                .map_or(text.len(), |n| index + 1 + n + 1);
            lexed.spans.push(classified(index, end, SpanClass::String));
            index = end;
        } else if character.is_alphabetic() || character == '_' {
            let end = text[index..]
                .char_indices()
                .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
                .map_or(text.len(), |(n, _)| index + n);
            lexed.identifiers.push((index, end));
            index = end;
        } else {
            index += character.len_utf8();
        }
    }
    lexed
}

fn classified(start: usize, end: usize, class: SpanClass) -> ClassifiedSpan {
    ClassifiedSpan {
        bytes: ByteRange::new(start as u64, end as u64),
        class,
    }
}

fn position(text: &str, offset: usize) -> TextPosition {
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |n| n + 1);
    TextPosition {
        line: line as u32,
        column: text[line_start..offset].chars().count() as u32 + 1,
    }
}

fn span(text: &str, start: usize, end: usize) -> TextSpan {
    TextSpan {
        bytes: ByteRange::new(start as u64, end as u64),
        start: position(text, start),
        end: position(text, end),
    }
}

fn line_of(text: &str, offset: usize) -> &str {
    let start = text[..offset].rfind('\n').map_or(0, |n| n + 1);
    let end = text[offset..].find('\n').map_or(text.len(), |n| offset + n);
    &text[start..end]
}

fn whole_word(text: &str, start: usize, name: &str) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[start + name.len()..].chars().next();
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    !before.is_some_and(ident) && !after.is_some_and(ident)
}

impl ToyCode {
    fn definitions(&self, text: &str, language: &LanguageId) -> Vec<SymbolInfo> {
        let lexed = lex(text, comment_marker(language));
        lexed
            .identifiers
            .windows(2)
            .filter_map(|pair| {
                let keyword = &text[pair[0].0..pair[0].1];
                let kind = match keyword {
                    "def" | "function" => SymbolKind::Function,
                    "class" => SymbolKind::Class,
                    _ => return None,
                };
                let (start, end) = pair[1];
                let line_end = text[end..].find('\n').map_or(text.len(), |n| end + n);
                Some(SymbolInfo {
                    name: text[start..end].to_owned(),
                    kind,
                    container: None,
                    depth: 0,
                    span: span(text, pair[0].0, line_end),
                    name_span: span(text, start, end),
                    signature: None,
                    exported: None,
                })
            })
            .collect()
    }
}

fn text_of(bytes: &[u8]) -> Result<&str> {
    std::str::from_utf8(bytes).map_err(|_| Error::new(ErrorCode::Encoding, "not UTF-8"))
}

fn not_supported<T>() -> Result<T> {
    Err(Error::new(
        ErrorCode::Unsupported,
        "not supported by the test code service",
    ))
}

impl CodeService for ToyCode {
    fn languages(&self) -> Vec<LanguageInfo> {
        [
            ("python", "Python", "py"),
            ("typescript", "TypeScript", "ts"),
            ("javascript", "JavaScript", "js"),
        ]
        .iter()
        .map(|(id, name, extension)| LanguageInfo {
            id: LanguageId::new(*id).unwrap(),
            name: (*name).to_owned(),
            extensions: vec![(*extension).to_owned()],
            grammar_version: Some("0.1.0".to_owned()),
        })
        .collect()
    }

    fn language_of(&self, path: Option<&Path>, _bytes: &[u8]) -> Option<LanguageId> {
        let extension = path?.extension()?.to_str()?;
        let id = match extension {
            "py" => "python",
            "ts" => "typescript",
            "js" => "javascript",
            _ => return None,
        };
        LanguageId::new(id).ok()
    }

    fn outline(
        &self,
        bytes: &[u8],
        language: &LanguageId,
        _depth: Option<u32>,
    ) -> Result<Vec<SymbolInfo>> {
        Ok(self.definitions(text_of(bytes)?, language))
    }

    fn find_symbol(
        &self,
        bytes: &[u8],
        language: &LanguageId,
        name: &str,
    ) -> Result<Vec<SymbolInfo>> {
        let last = name.rsplit('.').next().unwrap_or(name);
        Ok(self
            .definitions(text_of(bytes)?, language)
            .into_iter()
            .filter(|info| info.name == last)
            .collect())
    }

    fn enclosing(
        &self,
        _bytes: &[u8],
        _language: &LanguageId,
        _offset: u64,
    ) -> Result<Option<SymbolInfo>> {
        Ok(None)
    }

    fn classify_spans(&self, bytes: &[u8], language: &LanguageId) -> Result<Vec<ClassifiedSpan>> {
        Ok(lex(text_of(bytes)?, comment_marker(language)).spans)
    }

    fn check_syntax(&self, bytes: &[u8], language: &LanguageId) -> Result<Vec<SyntaxIssue>> {
        let text = text_of(bytes)?;
        let lexed = lex(text, comment_marker(language));
        let mut depth = 0_i64;
        for (index, character) in text.char_indices() {
            let inside = lexed
                .spans
                .iter()
                .any(|span| span.bytes.contains(index as u64));
            match character {
                '(' if !inside => depth += 1,
                ')' if !inside => depth -= 1,
                _ => {}
            }
        }
        if depth == 0 {
            return Ok(Vec::new());
        }
        Ok(vec![SyntaxIssue {
            severity: IssueSeverity::Error,
            message: "unbalanced parentheses".to_owned(),
            span: span(text, text.len(), text.len()),
            kind: Some("unexpected".to_owned()),
        }])
    }

    fn compile_pattern(
        &self,
        _pattern: &str,
        _language: &LanguageId,
    ) -> Result<Box<dyn SyntaxPattern>> {
        not_supported()
    }

    fn syntax_search(
        &self,
        _bytes: &[u8],
        _pattern: &dyn SyntaxPattern,
    ) -> Result<Vec<SyntaxMatch>> {
        not_supported()
    }

    fn render_template(&self, _found: &SyntaxMatch, _template: &str) -> Result<String> {
        not_supported()
    }

    fn references(
        &self,
        bytes: &[u8],
        language: &LanguageId,
        name: &str,
    ) -> Result<Vec<SymbolReference>> {
        let text = text_of(bytes)?;
        let lexed = lex(text, comment_marker(language));
        let mut found = Vec::new();
        for (position, (start, end)) in lexed.identifiers.iter().enumerate() {
            if &text[*start..*end] != name {
                continue;
            }
            let previous = position
                .checked_sub(1)
                .map(|index| &text[lexed.identifiers[index].0..lexed.identifiers[index].1]);
            let line = line_of(text, *start);
            let role = if matches!(previous, Some("def" | "class" | "function")) {
                ReferenceRole::Definition
            } else if line.trim_start().starts_with("import")
                || line.trim_start().starts_with("from")
            {
                ReferenceRole::Import
            } else {
                ReferenceRole::Usage
            };
            found.push(reference(text, *start, *end, role, SpanClass::Code));
        }
        for region in &lexed.spans {
            let (from, to) = (region.bytes.start as usize, region.bytes.end as usize);
            for (offset, _) in text[from..to].match_indices(name) {
                let start = from + offset;
                if whole_word(text, start, name) {
                    found.push(reference(
                        text,
                        start,
                        start + name.len(),
                        ReferenceRole::Usage,
                        region.class,
                    ));
                }
            }
        }
        found.sort_by_key(|item| item.span.bytes.start);
        Ok(found)
    }

    fn function_metrics(
        &self,
        _bytes: &[u8],
        _language: &LanguageId,
    ) -> Result<Vec<FunctionMetrics>> {
        Ok(Vec::new())
    }

    fn tokens(&self, bytes: &[u8], language: &LanguageId) -> Result<Vec<SyntaxToken>> {
        let text = text_of(bytes)?;
        Ok(lex(text, comment_marker(language))
            .identifiers
            .into_iter()
            .map(|(start, end)| SyntaxToken {
                bytes: ByteRange::new(start as u64, end as u64),
                kind: if KEYWORDS.contains(&&text[start..end]) {
                    "keyword".to_owned()
                } else {
                    "identifier".to_owned()
                },
                class: SpanClass::Code,
            })
            .collect())
    }
}

fn reference(
    text: &str,
    start: usize,
    end: usize,
    role: ReferenceRole,
    class: SpanClass,
) -> SymbolReference {
    SymbolReference {
        span: span(text, start, end),
        role,
        class,
        container: None,
        line_text: line_of(text, start).to_owned(),
    }
}

/// Semantic provider with a scripted rename answer, errors reported for
/// content that contains a marker, and fixed server states.
#[derive(Default)]
pub(crate) struct ScriptedProvider {
    pub(crate) rename: Mutex<Option<SemanticResult<WorkspaceChange>>>,
    pub(crate) error_marker: Option<(PathBuf, String)>,
    pub(crate) statuses: Vec<ServerStatus>,
    pub(crate) diagnostics_calls: Mutex<usize>,
}

impl ScriptedProvider {
    pub(crate) fn renaming(changes: Vec<DocumentChange>) -> Self {
        Self {
            rename: Mutex::new(Some(Ok(WorkspaceChange { changes }))),
            ..Self::default()
        }
    }

    pub(crate) fn failing(kind: SemanticErrorKind) -> Self {
        Self {
            rename: Mutex::new(Some(Err(SemanticError::new(
                kind,
                "no server for the test",
            )))),
            ..Self::default()
        }
    }
}

/// Text change replacing columns `start..end` of a line.
pub(crate) fn text_change(
    path: &Path,
    line: u32,
    start: u32,
    end: u32,
    new_text: &str,
) -> DocumentChange {
    DocumentChange::Text(TextChange {
        range: Location {
            path: path.to_path_buf(),
            start: TextPosition {
                line,
                column: start,
            },
            end: TextPosition { line, column: end },
        },
        new_text: new_text.to_owned(),
    })
}

fn unscripted<T>() -> SemanticResult<T> {
    Err(SemanticError::new(
        SemanticErrorKind::Unsupported,
        "not scripted",
    ))
}

impl SemanticProvider for ScriptedProvider {
    fn workspace_symbols(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _query: &str,
    ) -> SemanticResult<Vec<SemanticSymbol>> {
        unscripted()
    }

    fn document_symbols(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _path: &Path,
    ) -> SemanticResult<Vec<SemanticSymbol>> {
        unscripted()
    }

    fn definition(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _at: &DocumentPosition,
    ) -> SemanticResult<Vec<Location>> {
        unscripted()
    }

    fn references(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _at: &DocumentPosition,
        _include: bool,
    ) -> SemanticResult<Vec<Location>> {
        unscripted()
    }

    fn hover(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _at: &DocumentPosition,
    ) -> SemanticResult<Option<HoverInfo>> {
        unscripted()
    }

    fn implementations(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _at: &DocumentPosition,
    ) -> SemanticResult<Vec<Location>> {
        unscripted()
    }

    fn type_definition(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _at: &DocumentPosition,
    ) -> SemanticResult<Vec<Location>> {
        unscripted()
    }

    fn call_hierarchy(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _at: &DocumentPosition,
        _direction: CallDirection,
    ) -> SemanticResult<Vec<CallEdge>> {
        unscripted()
    }

    fn code_actions(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _range: &Location,
    ) -> SemanticResult<Vec<CodeActionInfo>> {
        unscripted()
    }

    fn diagnostics(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        path: &Path,
        text: Option<&str>,
    ) -> SemanticResult<Vec<SemanticDiagnostic>> {
        *self.diagnostics_calls.lock().unwrap() += 1;
        let marked = self.error_marker.as_ref().is_some_and(|(marked, marker)| {
            marked == path && text.is_some_and(|text| text.contains(marker.as_str()))
        });
        if !marked {
            return Ok(Vec::new());
        }
        Ok(vec![SemanticDiagnostic {
            range: Location {
                path: path.to_path_buf(),
                start: TextPosition { line: 1, column: 1 },
                end: TextPosition { line: 1, column: 2 },
            },
            severity: DiagnosticSeverity::Error,
            message: "name is not defined".to_owned(),
            code: None,
            source: None,
            related: Vec::new(),
        }])
    }

    fn rename(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _at: &DocumentPosition,
        _new_name: &str,
    ) -> SemanticResult<WorkspaceChange> {
        self.rename.lock().unwrap().take().unwrap_or_else(|| {
            Err(SemanticError::new(
                SemanticErrorKind::Unavailable,
                "no answer scripted",
            ))
        })
    }

    fn server_status(&self, _ctx: &ExecCtx, language: Option<&LanguageId>) -> Vec<ServerStatus> {
        self.statuses
            .iter()
            .filter(|status| language.is_none_or(|language| status.language == *language))
            .cloned()
            .collect()
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
