// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Service doubles.
//!
//! The first group answers queries with empty results and fails every
//! request that needs a real implementation with the error code the service
//! contract prescribes for an unsupported input. [`test_services`] registers
//! them for contexts in which the providing command group is not under test:
//!
//! | Double | Empty answers | Error |
//! |---|---|---|
//! | [`UnsupportedDocService`] | no formats, no detected format | `E_UNSUPPORTED` |
//! | [`UnsupportedCodeService`] | no languages, no detected language | `E_UNSUPPORTED_LANGUAGE` |
//! | [`UnavailableProcessHost`] | none | `E_EXTERNAL_MISSING` |
//! | [`EmptyFileList`] | every listing is empty | none |
//!
//! The second group answers with data given by the test:
//!
//! | Double | Service | Data |
//! |---|---|---|
//! | [`ScriptedProcessHost`] | [`ProcessHost`] | programs with output over time, exit status and file changes ([`ProgramScript`]) |
//! | [`StaticDocService`] | [`DocService`] | formats, values per key, syntax issues; JSON built in |
//! | [`StaticCodeService`] | [`CodeService`] | languages, symbols, spans, matches, references, metrics |
//! | [`StaticGitQuery`] | [`GitQueryService`] | repository root, status, changed, tracked and ignored paths, history |
//! | [`Utf8TextCodec`] | [`TextCodecService`] | UTF-8 with and without byte order mark |
//! | [`StaticFileHolders`] | [`FileHolderService`] | holders per path |
//! | [`StaticReviewRules`] | [`ReviewRuleProvider`] | rules |
//!
//! Data doubles return what was registered regardless of the content passed
//! by the caller, except where a row above names a built-in behaviour.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nurt_contracts::env::{Clock, WallTime};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::{
    ByteRange, ChildExit, ClassifiedSpan, CodeService, DocFormatId, DocFormatInfo, DocItem,
    DocService, FileHistoryStat, FileHolder, FileHolderService, FileListService, FunctionMetrics,
    GitChange, GitPathStatus, GitQueryService, IssueSeverity, LanguageId, LanguageInfo, LineEnding,
    ProcessHost, ProgramOutcome, ProgramSpec, ReviewRule, ReviewRuleProvider, SerializeOptions,
    ServiceChild, ServiceSpawnSpec, Services, SymbolInfo, SymbolReference, SyntaxIssue,
    SyntaxMatch, SyntaxPattern, SyntaxToken, TextCodecService, TextDetection, TextEncoding,
    TextPosition, TextSpan, WalkIter, WalkSpec,
};
use nurt_contracts::stage::{CancelToken, OutputStream};
use nurt_contracts::value::Record;

use crate::clock::VirtualClock;
use crate::vfs::VirtualFs;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Document service that knows no format.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnsupportedDocService;

fn unsupported_format(format: &DocFormatId) -> Error {
    Error::new(
        ErrorCode::Unsupported,
        format!("document format `{format}` is not available in this context"),
    )
}

impl DocService for UnsupportedDocService {
    fn formats(&self) -> Vec<DocFormatInfo> {
        Vec::new()
    }

    fn detect_format(&self, _path: Option<&Path>, _bytes: &[u8]) -> Option<DocFormatId> {
        None
    }

    fn read_keys(
        &self,
        _bytes: &[u8],
        format: &DocFormatId,
        _key: &KeyPath,
        _depth: Option<u32>,
    ) -> Result<Vec<DocItem>> {
        Err(unsupported_format(format))
    }

    fn check_syntax(&self, _bytes: &[u8], format: &DocFormatId) -> Result<Vec<SyntaxIssue>> {
        Err(unsupported_format(format))
    }

    fn serialize_records(
        &self,
        _records: &[Record],
        format: &DocFormatId,
        _options: &SerializeOptions,
    ) -> Result<Vec<u8>> {
        Err(unsupported_format(format))
    }
}

/// Code service that has no grammar.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnsupportedCodeService;

fn unsupported_language(language: &str) -> Error {
    Error::new(
        ErrorCode::UnsupportedLanguage,
        format!("no grammar for `{language}` is available in this context"),
    )
}

impl CodeService for UnsupportedCodeService {
    fn languages(&self) -> Vec<LanguageInfo> {
        Vec::new()
    }

    fn language_of(&self, _path: Option<&Path>, _bytes: &[u8]) -> Option<LanguageId> {
        None
    }

    fn outline(
        &self,
        _bytes: &[u8],
        language: &LanguageId,
        _depth: Option<u32>,
    ) -> Result<Vec<SymbolInfo>> {
        Err(unsupported_language(language.as_str()))
    }

    fn find_symbol(
        &self,
        _bytes: &[u8],
        language: &LanguageId,
        _name: &str,
    ) -> Result<Vec<SymbolInfo>> {
        Err(unsupported_language(language.as_str()))
    }

    fn enclosing(
        &self,
        _bytes: &[u8],
        language: &LanguageId,
        _offset: u64,
    ) -> Result<Option<SymbolInfo>> {
        Err(unsupported_language(language.as_str()))
    }

    fn classify_spans(&self, _bytes: &[u8], language: &LanguageId) -> Result<Vec<ClassifiedSpan>> {
        Err(unsupported_language(language.as_str()))
    }

    fn check_syntax(&self, _bytes: &[u8], language: &LanguageId) -> Result<Vec<SyntaxIssue>> {
        Err(unsupported_language(language.as_str()))
    }

    fn compile_pattern(
        &self,
        _pattern: &str,
        language: &LanguageId,
    ) -> Result<Box<dyn SyntaxPattern>> {
        Err(unsupported_language(language.as_str()))
    }

    fn syntax_search(
        &self,
        _bytes: &[u8],
        pattern: &dyn SyntaxPattern,
    ) -> Result<Vec<SyntaxMatch>> {
        Err(unsupported_language(pattern.language().as_str()))
    }

    fn render_template(&self, _found: &SyntaxMatch, _template: &str) -> Result<String> {
        Err(Error::new(
            ErrorCode::UnsupportedLanguage,
            "structural templates are not available in this context",
        ))
    }

    fn references(
        &self,
        _bytes: &[u8],
        language: &LanguageId,
        _name: &str,
    ) -> Result<Vec<SymbolReference>> {
        Err(unsupported_language(language.as_str()))
    }

    fn function_metrics(
        &self,
        _bytes: &[u8],
        language: &LanguageId,
    ) -> Result<Vec<FunctionMetrics>> {
        Err(unsupported_language(language.as_str()))
    }

    fn tokens(&self, _bytes: &[u8], language: &LanguageId) -> Result<Vec<SyntaxToken>> {
        Err(unsupported_language(language.as_str()))
    }
}

/// Program host that finds no program.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableProcessHost;

fn missing_program(program: &str) -> Error {
    Error::new(
        ErrorCode::ExternalMissing,
        format!("program `{program}` cannot be started in this context"),
    )
}

impl ProcessHost for UnavailableProcessHost {
    fn resolve_program(&self, program: &str, _cwd: &Path) -> Result<PathBuf> {
        Err(missing_program(program))
    }

    fn run_program(&self, spec: &ProgramSpec, _cancel: &CancelToken) -> Result<ProgramOutcome> {
        Err(missing_program(&spec.program))
    }

    fn spawn_service(&self, spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
        Err(missing_program(&spec.program))
    }
}

/// File listing that lists nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyFileList;

impl FileListService for EmptyFileList {
    fn walk<'a>(&'a self, _spec: &WalkSpec) -> Result<WalkIter<'a>> {
        Ok(Box::new(std::iter::empty()))
    }
}

/// Registry with the four doubles of the first group of this module.
///
/// ```
/// use nurt_testkit::services::test_services;
///
/// let services = test_services();
/// assert!(services.doc().is_some_and(|doc| doc.formats().is_empty()));
/// assert!(services.file_list().is_some());
/// ```
#[must_use]
pub fn test_services() -> Services {
    Services::new()
        .with_doc(Arc::new(UnsupportedDocService))
        .with_code(Arc::new(UnsupportedCodeService))
        .with_process_host(Arc::new(UnavailableProcessHost))
        .with_file_list(Arc::new(EmptyFileList))
}

/// Signal number reported for a scripted child stopped by
/// [`ServiceChild::kill_tree`] (`SIGKILL`).
pub const KILL_SIGNAL: i32 = 9;

/// Directory of the program file reported for scripts without a path.
pub const DEFAULT_PROGRAM_DIR: &str = "/usr/bin";

/// Output written by a scripted program at a point of its run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptedOutput {
    /// Time since the start of the program.
    pub at: Duration,
    /// Stream.
    pub stream: OutputStream,
    /// Bytes written.
    pub bytes: Vec<u8>,
}

/// Change of the virtual file system made by a scripted program when it
/// exits by itself.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptedChange {
    /// The file is written with this content (parents are created).
    Write {
        /// File.
        path: AbsPath,
        /// New content.
        content: Vec<u8>,
    },
    /// The entry is removed.
    Remove {
        /// Entry.
        path: AbsPath,
    },
}

impl ScriptedChange {
    fn path(&self) -> &AbsPath {
        match self {
            Self::Write { path, .. } | Self::Remove { path } => path,
        }
    }
}

/// Behaviour of a scripted program.
///
/// A script matches a run when the program name equals the script program
/// and, when the script lists arguments, the arguments are equal. By default
/// a program writes nothing, runs for no time and exits with code 0.
///
/// ```
/// use std::time::Duration;
///
/// use nurt_testkit::services::ProgramScript;
///
/// let script = ProgramScript::new("cargo")
///     .with_args(["build"])
///     .stdout(Duration::from_millis(10), "Compiling demo\n")
///     .runs_for(Duration::from_secs(2))
///     .exits_with(101);
/// assert_eq!(script.program(), "cargo");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramScript {
    program: String,
    args: Option<Vec<String>>,
    path: Option<PathBuf>,
    output: Vec<ScriptedOutput>,
    duration: Duration,
    exit_code: Option<i32>,
    signal: Option<i32>,
    changes: Vec<ScriptedChange>,
    once: bool,
}

impl ProgramScript {
    /// Script of `program` that exits with code 0 at once.
    #[must_use]
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: None,
            path: None,
            output: Vec::new(),
            duration: Duration::ZERO,
            exit_code: Some(0),
            signal: None,
            changes: Vec::new(),
            once: false,
        }
    }

    /// Program name matched by the script.
    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }

    /// Matches only runs with exactly these arguments.
    #[must_use]
    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = Some(args.into_iter().map(Into::into).collect());
        self
    }

    /// File reported by [`ProcessHost::resolve_program`]; the default is
    /// the program name under [`DEFAULT_PROGRAM_DIR`].
    #[must_use]
    pub fn at_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Writes `bytes` to standard output `at` after the start.
    #[must_use]
    pub fn stdout(self, at: Duration, bytes: impl Into<Vec<u8>>) -> Self {
        self.output(at, OutputStream::Stdout, bytes)
    }

    /// Writes `bytes` to standard error `at` after the start.
    #[must_use]
    pub fn stderr(self, at: Duration, bytes: impl Into<Vec<u8>>) -> Self {
        self.output(at, OutputStream::Stderr, bytes)
    }

    fn output(mut self, at: Duration, stream: OutputStream, bytes: impl Into<Vec<u8>>) -> Self {
        self.output.push(ScriptedOutput {
            at,
            stream,
            bytes: bytes.into(),
        });
        self.output.sort_by_key(|output| output.at);
        self
    }

    /// The program exits `duration` after the start.
    #[must_use]
    pub fn runs_for(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// The program exits with `code`.
    #[must_use]
    pub fn exits_with(mut self, code: i32) -> Self {
        self.exit_code = Some(code);
        self.signal = None;
        self
    }

    /// The program is ended by `signal` at the end of its run.
    #[must_use]
    pub fn killed_by(mut self, signal: i32) -> Self {
        self.exit_code = None;
        self.signal = Some(signal);
        self
    }

    /// The program writes `content` to `path` when it exits.
    #[must_use]
    pub fn writes(mut self, path: AbsPath, content: impl Into<Vec<u8>>) -> Self {
        self.changes.push(ScriptedChange::Write {
            path,
            content: content.into(),
        });
        self
    }

    /// The program removes `path` when it exits.
    #[must_use]
    pub fn removes(mut self, path: AbsPath) -> Self {
        self.changes.push(ScriptedChange::Remove { path });
        self
    }

    /// The script serves one run only and is then forgotten.
    #[must_use]
    pub fn once(mut self) -> Self {
        self.once = true;
        self
    }

    fn resolved_path(&self) -> PathBuf {
        self.path
            .clone()
            .unwrap_or_else(|| Path::new(DEFAULT_PROGRAM_DIR).join(&self.program))
    }

    fn matches(&self, program: &str, args: Option<&[String]>) -> bool {
        self.program == program
            && match (&self.args, args) {
                (Some(expected), Some(actual)) => expected.as_slice() == actual,
                _ => true,
            }
    }

    fn exit(&self) -> ChildExit {
        ChildExit {
            exit_code: self.exit_code,
            signal: self.signal,
        }
    }
}

fn apply_changes(fs: Option<&VirtualFs>, changes: &[ScriptedChange]) -> Result<()> {
    let Some(fs) = fs else {
        return Ok(());
    };
    for change in changes {
        let result = match change {
            ScriptedChange::Write { path, content } => fs.write_file(path, content),
            ScriptedChange::Remove { path } => fs.remove_entry(path),
        };
        result.map_err(|error| crate::journal::fs_error_to_error(&error))?;
    }
    Ok(())
}

#[derive(Debug)]
struct HostState {
    scripts: Vec<ProgramScript>,
    runs: Vec<ProgramSpec>,
    spawned: Vec<ServiceSpawnSpec>,
    stdin: BTreeMap<u32, Arc<Mutex<Vec<u8>>>>,
}

/// Program host that runs scripted programs on a virtual clock.
///
/// [`ProcessHost::run_program`] advances the clock by the run time of the
/// script, cut by the time limit of the request, and applies the file
/// changes of the script to the virtual file system when the program exits
/// by itself. [`ProcessHost::spawn_service`] returns a [`ServiceChild`]
/// whose output readers advance the clock to the time of the next output and
/// which exits when the clock reaches the end of its run. Programs without a
/// script fail with `E_EXTERNAL_MISSING`.
///
/// ```
/// use std::path::Path;
/// use std::sync::Arc;
/// use std::time::Duration;
///
/// use nurt_contracts::services::{ProcessHost, ProgramSpec};
/// use nurt_contracts::stage::CancelToken;
/// use nurt_testkit::clock::VirtualClock;
/// use nurt_testkit::services::{ProgramScript, ScriptedProcessHost};
///
/// let clock = Arc::new(VirtualClock::new());
/// let host = ScriptedProcessHost::new(clock.clone());
/// host.script(
///     ProgramScript::new("make")
///         .stdout(Duration::from_secs(1), "built\n")
///         .runs_for(Duration::from_secs(3)),
/// );
/// let spec = ProgramSpec::new("make", Vec::new(), "/project");
/// let outcome = host.run_program(&spec, &CancelToken::new()).unwrap();
/// assert_eq!(outcome.exit_code, Some(0));
/// assert_eq!(outcome.stdout, b"built\n");
/// assert_eq!(outcome.duration_ms, 3_000);
/// assert_eq!(host.runs().len(), 1);
/// ```
#[derive(Debug)]
pub struct ScriptedProcessHost {
    clock: Arc<VirtualClock>,
    fs: Option<Arc<VirtualFs>>,
    state: Mutex<HostState>,
    next_pid: AtomicU32,
}

/// Process identifier of the first child started by a
/// [`ScriptedProcessHost`].
pub const FIRST_PID: u32 = 1000;

impl ScriptedProcessHost {
    /// Host on `clock` without a file system; scripted file changes are
    /// reported but not applied.
    #[must_use]
    pub fn new(clock: Arc<VirtualClock>) -> Self {
        Self {
            clock,
            fs: None,
            state: Mutex::new(HostState {
                scripts: Vec::new(),
                runs: Vec::new(),
                spawned: Vec::new(),
                stdin: BTreeMap::new(),
            }),
            next_pid: AtomicU32::new(FIRST_PID),
        }
    }

    /// Applies scripted file changes to `fs`.
    #[must_use]
    pub fn with_fs(mut self, fs: Arc<VirtualFs>) -> Self {
        self.fs = Some(fs);
        self
    }

    /// Adds a script; earlier scripts take precedence over later ones.
    pub fn script(&self, script: ProgramScript) {
        lock(&self.state).scripts.push(script);
    }

    /// Specifications of the runs of [`ProcessHost::run_program`], in call
    /// order.
    #[must_use]
    pub fn runs(&self) -> Vec<ProgramSpec> {
        lock(&self.state).runs.clone()
    }

    /// Specifications of the processes started by
    /// [`ProcessHost::spawn_service`], in call order.
    #[must_use]
    pub fn spawned(&self) -> Vec<ServiceSpawnSpec> {
        lock(&self.state).spawned.clone()
    }

    /// Bytes written so far to standard input of the child `pid`.
    #[must_use]
    pub fn stdin_of(&self, pid: u32) -> Vec<u8> {
        lock(&self.state)
            .stdin
            .get(&pid)
            .map(|buffer| lock(buffer).clone())
            .unwrap_or_default()
    }

    fn take_script(&self, program: &str, args: Option<&[String]>) -> Result<ProgramScript> {
        let mut state = lock(&self.state);
        let index = state
            .scripts
            .iter()
            .position(|script| script.matches(program, args))
            .ok_or_else(|| missing_program(program))?;
        if state.scripts[index].once {
            Ok(state.scripts.remove(index))
        } else {
            Ok(state.scripts[index].clone())
        }
    }
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn bounded(mut bytes: Vec<u8>, limit: Option<u64>) -> (Vec<u8>, bool) {
    match limit.and_then(|limit| usize::try_from(limit).ok()) {
        Some(limit) if bytes.len() > limit => {
            bytes.truncate(limit);
            (bytes, true)
        }
        _ => (bytes, false),
    }
}

impl ProcessHost for ScriptedProcessHost {
    fn resolve_program(&self, program: &str, _cwd: &Path) -> Result<PathBuf> {
        let state = lock(&self.state);
        state
            .scripts
            .iter()
            .find(|script| script.program == program)
            .map(ProgramScript::resolved_path)
            .ok_or_else(|| missing_program(program))
    }

    fn run_program(&self, spec: &ProgramSpec, cancel: &CancelToken) -> Result<ProgramOutcome> {
        let script = self.take_script(&spec.program, Some(&spec.args))?;
        lock(&self.state).runs.push(spec.clone());
        let mut outcome = ProgramOutcome {
            program_path: script.resolved_path(),
            ..ProgramOutcome::default()
        };
        if cancel.is_cancelled() {
            outcome.cancelled = true;
            return Ok(outcome);
        }
        let limit = spec.timeout_ms.map(Duration::from_millis);
        let (run_time, timed_out) = match limit {
            Some(limit) if limit < script.duration => (limit, true),
            _ => (script.duration, false),
        };
        if self.clock.sleep(run_time, cancel).is_err() {
            outcome.cancelled = true;
            return Ok(outcome);
        }
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        for output in script.output.iter().filter(|output| output.at <= run_time) {
            match output.stream {
                OutputStream::Stdout => stdout.extend_from_slice(&output.bytes),
                OutputStream::Stderr => stderr.extend_from_slice(&output.bytes),
            }
        }
        (outcome.stdout, outcome.stdout_truncated) = bounded(stdout, spec.output_limit);
        (outcome.stderr, outcome.stderr_truncated) = bounded(stderr, spec.output_limit);
        outcome.duration_ms = duration_ms(run_time);
        if timed_out {
            outcome.timed_out = true;
            return Ok(outcome);
        }
        apply_changes(self.fs.as_deref(), &script.changes)?;
        outcome.exit_code = script.exit_code;
        outcome.signal = script.signal;
        if spec.detect_changes {
            outcome.changed_paths = script
                .changes
                .iter()
                .map(|change| change.path().as_path().to_path_buf())
                .collect();
        }
        Ok(outcome)
    }

    fn spawn_service(&self, spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
        let script = self.take_script(&spec.program, Some(&spec.args))?;
        let pid = self.next_pid.fetch_add(1, Ordering::Relaxed);
        let stdin = Arc::new(Mutex::new(Vec::new()));
        {
            let mut state = lock(&self.state);
            state.spawned.push(spec.clone());
            state.stdin.insert(pid, Arc::clone(&stdin));
        }
        let start = self.clock.monotonic();
        let killed = Arc::new(AtomicBool::new(false));
        let reader = |stream: OutputStream| -> Box<dyn Read + Send> {
            Box::new(TimedReader {
                chunks: script
                    .output
                    .iter()
                    .filter(|output| output.stream == stream && !output.bytes.is_empty())
                    .map(|output| (output.at, output.bytes.clone()))
                    .collect(),
                position: 0,
                clock: Arc::clone(&self.clock),
                start,
                end: script.duration,
                killed: Arc::clone(&killed),
            })
        };
        Ok(Box::new(ScriptedChild {
            pid,
            started: self.clock.now(),
            start,
            stdin: Some(Box::new(SharedWriter(stdin))),
            stdout: Some(reader(OutputStream::Stdout)),
            stderr: Some(reader(OutputStream::Stderr)),
            clock: Arc::clone(&self.clock),
            fs: self.fs.clone(),
            killed,
            changes_applied: false,
            script,
        }))
    }
}

struct SharedWriter(Arc<Mutex<Vec<u8>>>);

impl Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        lock(&self.0).extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Reader of one output stream of a scripted child: a read returns the
/// next chunk, first advancing the clock to its time when it lies in the
/// future, and end of input once all chunks are read or the child is
/// stopped.
struct TimedReader {
    chunks: Vec<(Duration, Vec<u8>)>,
    position: usize,
    clock: Arc<VirtualClock>,
    start: Duration,
    end: Duration,
    killed: Arc<AtomicBool>,
}

impl Read for TimedReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.killed.load(Ordering::Acquire) || buf.is_empty() {
            return Ok(0);
        }
        let Some((at, bytes)) = self.chunks.first_mut() else {
            return Ok(0);
        };
        let elapsed = self.clock.monotonic().saturating_sub(self.start);
        let due = (*at).min(self.end);
        if due > elapsed {
            self.clock.advance(due - elapsed);
        }
        let rest = &bytes[self.position..];
        let count = rest.len().min(buf.len());
        buf[..count].copy_from_slice(&rest[..count]);
        self.position += count;
        if self.position >= bytes.len() {
            self.chunks.remove(0);
            self.position = 0;
        }
        Ok(count)
    }
}

/// Child started by [`ScriptedProcessHost::spawn_service`].
struct ScriptedChild {
    pid: u32,
    started: WallTime,
    start: Duration,
    stdin: Option<Box<dyn Write + Send>>,
    stdout: Option<Box<dyn Read + Send>>,
    stderr: Option<Box<dyn Read + Send>>,
    clock: Arc<VirtualClock>,
    fs: Option<Arc<VirtualFs>>,
    killed: Arc<AtomicBool>,
    changes_applied: bool,
    script: ProgramScript,
}

impl ScriptedChild {
    fn elapsed(&self) -> Duration {
        self.clock.monotonic().saturating_sub(self.start)
    }

    fn exited(&mut self) -> Result<ChildExit> {
        if !self.changes_applied {
            self.changes_applied = true;
            apply_changes(self.fs.as_deref(), &self.script.changes)?;
        }
        Ok(self.script.exit())
    }
}

fn killed_exit() -> ChildExit {
    ChildExit {
        exit_code: None,
        signal: Some(KILL_SIGNAL),
    }
}

impl ServiceChild for ScriptedChild {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn started(&self) -> WallTime {
        self.started
    }

    fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        self.stdin.take()
    }

    fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        self.stdout.take()
    }

    fn take_stderr(&mut self) -> Option<Box<dyn Read + Send>> {
        self.stderr.take()
    }

    fn try_wait(&mut self) -> Result<Option<ChildExit>> {
        if self.killed.load(Ordering::Acquire) {
            return Ok(Some(killed_exit()));
        }
        if self.elapsed() >= self.script.duration {
            return self.exited().map(Some);
        }
        Ok(None)
    }

    fn wait_timeout(&mut self, timeout: Duration) -> Result<Option<ChildExit>> {
        if self.killed.load(Ordering::Acquire) {
            return Ok(Some(killed_exit()));
        }
        let remaining = self.script.duration.saturating_sub(self.elapsed());
        if remaining <= timeout {
            self.clock.advance(remaining);
            return self.exited().map(Some);
        }
        self.clock.advance(timeout);
        Ok(None)
    }

    fn kill_tree(&mut self) -> Result<()> {
        self.killed.store(true, Ordering::Release);
        Ok(())
    }
}

/// Document service answering with registered data.
///
/// The JSON format is built in when the service is created with
/// [`StaticDocService::json`]: format detection by the `json` extension or by
/// content that parses as JSON, syntax checks by the JSON parser and
/// serialization of records as a JSON array. Values read by key are always
/// the registered ones ([`StaticDocService::with_items`]); an unregistered
/// key yields an empty list. Formats that are not registered fail with
/// `E_UNSUPPORTED`.
///
/// ```
/// use nurt_contracts::keypath::KeyPath;
/// use nurt_contracts::services::{DocFormatId, DocService};
/// use nurt_testkit::services::StaticDocService;
///
/// let doc = StaticDocService::json();
/// assert_eq!(doc.detect_format(None, b"{\"a\": 1}"), Some(DocFormatId::JSON));
/// assert_eq!(doc.check_syntax(b"{\"a\":", &DocFormatId::JSON).unwrap().len(), 1);
/// let key = KeyPath::parse("a").unwrap();
/// assert!(doc.read_keys(b"{}", &DocFormatId::JSON, &key, None).unwrap().is_empty());
/// ```
#[derive(Debug, Clone, Default)]
pub struct StaticDocService {
    formats: Vec<DocFormatInfo>,
    items: Vec<(DocFormatId, String, Vec<DocItem>)>,
    issues: Vec<(DocFormatId, Vec<u8>, Vec<SyntaxIssue>)>,
}

impl StaticDocService {
    /// Service without formats.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Service with the built-in JSON format.
    #[must_use]
    pub fn json() -> Self {
        Self::new().with_format(DocFormatInfo {
            id: DocFormatId::JSON,
            extensions: vec!["json".to_owned()],
            editable: true,
        })
    }

    /// Registers a format.
    #[must_use]
    pub fn with_format(mut self, format: DocFormatInfo) -> Self {
        self.formats.push(format);
        self
    }

    /// Registers the values returned for `key` in documents of `format`.
    #[must_use]
    pub fn with_items(mut self, format: DocFormatId, key: &KeyPath, items: Vec<DocItem>) -> Self {
        self.items.push((format, key.to_string(), items));
        self
    }

    /// Registers the issues reported for exactly `bytes` in `format`.
    #[must_use]
    pub fn with_issues(
        mut self,
        format: DocFormatId,
        bytes: impl Into<Vec<u8>>,
        issues: Vec<SyntaxIssue>,
    ) -> Self {
        self.issues.push((format, bytes.into(), issues));
        self
    }

    fn require(&self, format: &DocFormatId) -> Result<()> {
        if self.formats.iter().any(|known| &known.id == format) {
            Ok(())
        } else {
            Err(unsupported_format(format))
        }
    }
}

/// Position after `text`, one-based, columns in Unicode scalar values.
fn position_after(text: &str) -> TextPosition {
    let line = text.matches('\n').count() + 1;
    let column = text
        .rsplit('\n')
        .next()
        .map_or(0, |last| last.chars().count())
        + 1;
    TextPosition {
        line: u32::try_from(line).unwrap_or(u32::MAX),
        column: u32::try_from(column).unwrap_or(u32::MAX),
    }
}

fn json_issue(bytes: &[u8], error: &serde_json::Error) -> SyntaxIssue {
    let text = String::from_utf8_lossy(bytes);
    let mut offset = 0_usize;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        if index + 1 >= error.line() {
            offset += error.column().saturating_sub(1).min(line.len());
            break;
        }
        offset += line.len();
    }
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let start = position_after(&text[..offset]);
    let at = u64::try_from(offset).unwrap_or(u64::MAX);
    SyntaxIssue {
        severity: IssueSeverity::Error,
        message: error.to_string(),
        span: TextSpan {
            bytes: ByteRange::new(at, at),
            start,
            end: start,
        },
        kind: Some("syntax".to_owned()),
    }
}

impl DocService for StaticDocService {
    fn formats(&self) -> Vec<DocFormatInfo> {
        self.formats.clone()
    }

    fn detect_format(&self, path: Option<&Path>, bytes: &[u8]) -> Option<DocFormatId> {
        let extension = path
            .and_then(Path::extension)
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        if let Some(extension) = extension
            && let Some(format) = self
                .formats
                .iter()
                .find(|format| format.extensions.contains(&extension))
        {
            return Some(format.id.clone());
        }
        let json = self.require(&DocFormatId::JSON).is_ok()
            && serde_json::from_slice::<serde_json::Value>(bytes).is_ok();
        json.then_some(DocFormatId::JSON)
    }

    fn read_keys(
        &self,
        _bytes: &[u8],
        format: &DocFormatId,
        key: &KeyPath,
        _depth: Option<u32>,
    ) -> Result<Vec<DocItem>> {
        self.require(format)?;
        let key = key.to_string();
        Ok(self
            .items
            .iter()
            .find(|(known, text, _)| known == format && *text == key)
            .map(|(_, _, items)| items.clone())
            .unwrap_or_default())
    }

    fn check_syntax(&self, bytes: &[u8], format: &DocFormatId) -> Result<Vec<SyntaxIssue>> {
        self.require(format)?;
        if let Some((_, _, issues)) = self
            .issues
            .iter()
            .find(|(known, content, _)| known == format && content == bytes)
        {
            return Ok(issues.clone());
        }
        if *format == DocFormatId::JSON
            && let Err(error) = serde_json::from_slice::<serde_json::Value>(bytes)
        {
            return Ok(vec![json_issue(bytes, &error)]);
        }
        Ok(Vec::new())
    }

    fn serialize_records(
        &self,
        records: &[Record],
        format: &DocFormatId,
        options: &SerializeOptions,
    ) -> Result<Vec<u8>> {
        self.require(format)?;
        if *format != DocFormatId::JSON {
            return Err(unsupported_format(format));
        }
        let encoded = if options.pretty {
            serde_json::to_vec_pretty(records)
        } else {
            serde_json::to_vec(records)
        };
        encoded.map_err(|error| {
            Error::new(
                ErrorCode::Encoding,
                format!("records cannot be written as JSON: {error}"),
            )
        })
    }
}

/// Compiled pattern of a [`StaticCodeService`]: the pattern text and its
/// metavariables (`$X`, `$$$X`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticPattern {
    language: LanguageId,
    source: String,
}

impl SyntaxPattern for StaticPattern {
    fn language(&self) -> &LanguageId {
        &self.language
    }

    fn source(&self) -> &str {
        &self.source
    }

    fn metavariables(&self) -> Vec<String> {
        let mut names = Vec::new();
        let mut rest = self.source.as_str();
        while let Some(index) = rest.find('$') {
            rest = rest[index..].trim_start_matches('$');
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                .collect();
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
        names
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[derive(Debug, Clone, Default)]
struct LanguageData {
    symbols: Vec<SymbolInfo>,
    spans: Vec<ClassifiedSpan>,
    issues: Vec<SyntaxIssue>,
    matches: BTreeMap<String, Vec<SyntaxMatch>>,
    references: BTreeMap<String, Vec<SymbolReference>>,
    metrics: Vec<FunctionMetrics>,
    tokens: Vec<SyntaxToken>,
}

/// Code service answering with registered data per language.
///
/// The language of a file is detected by its extension. A pattern compiles
/// when its language is registered; its matches are the ones registered for
/// the pattern text. [`CodeService::render_template`] replaces `$$$X` and
/// `$X` by the text captured for `X`. Languages that are not registered fail
/// with `E_UNSUPPORTED_LANGUAGE`.
///
/// ```
/// use std::path::Path;
///
/// use nurt_contracts::services::{CodeService, LanguageId, LanguageInfo};
/// use nurt_testkit::services::StaticCodeService;
///
/// let rust = LanguageId::new("rust").unwrap();
/// let code = StaticCodeService::new().with_language(LanguageInfo {
///     id: rust.clone(),
///     name: "Rust".to_owned(),
///     extensions: vec!["rs".to_owned()],
///     grammar_version: None,
/// });
/// assert_eq!(code.language_of(Some(Path::new("main.rs")), b""), Some(rust.clone()));
/// let pattern = code.compile_pattern("foo($X, $$$REST)", &rust).unwrap();
/// assert_eq!(pattern.metavariables(), ["X", "REST"]);
/// ```
#[derive(Debug, Clone, Default)]
pub struct StaticCodeService {
    languages: Vec<LanguageInfo>,
    data: BTreeMap<LanguageId, LanguageData>,
}

impl StaticCodeService {
    /// Service without languages.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a language without data.
    #[must_use]
    pub fn with_language(mut self, language: LanguageInfo) -> Self {
        self.data.entry(language.id.clone()).or_default();
        self.languages.push(language);
        self
    }

    fn entry(&mut self, language: &LanguageId) -> &mut LanguageData {
        self.data.entry(language.clone()).or_default()
    }

    /// Registers the symbols of the outline, in source order.
    #[must_use]
    pub fn with_symbols(mut self, language: &LanguageId, symbols: Vec<SymbolInfo>) -> Self {
        self.entry(language).symbols = symbols;
        self
    }

    /// Registers the comment and string spans.
    #[must_use]
    pub fn with_spans(mut self, language: &LanguageId, spans: Vec<ClassifiedSpan>) -> Self {
        self.entry(language).spans = spans;
        self
    }

    /// Registers the syntax issues.
    #[must_use]
    pub fn with_issues(mut self, language: &LanguageId, issues: Vec<SyntaxIssue>) -> Self {
        self.entry(language).issues = issues;
        self
    }

    /// Registers the matches of a pattern text.
    #[must_use]
    pub fn with_matches(
        mut self,
        language: &LanguageId,
        pattern: &str,
        matches: Vec<SyntaxMatch>,
    ) -> Self {
        self.entry(language)
            .matches
            .insert(pattern.to_owned(), matches);
        self
    }

    /// Registers the references of a name.
    #[must_use]
    pub fn with_references(
        mut self,
        language: &LanguageId,
        name: &str,
        references: Vec<SymbolReference>,
    ) -> Self {
        self.entry(language)
            .references
            .insert(name.to_owned(), references);
        self
    }

    /// Registers the function metrics.
    #[must_use]
    pub fn with_metrics(mut self, language: &LanguageId, metrics: Vec<FunctionMetrics>) -> Self {
        self.entry(language).metrics = metrics;
        self
    }

    /// Registers the tokens.
    #[must_use]
    pub fn with_tokens(mut self, language: &LanguageId, tokens: Vec<SyntaxToken>) -> Self {
        self.entry(language).tokens = tokens;
        self
    }

    fn data(&self, language: &LanguageId) -> Result<&LanguageData> {
        if self.languages.iter().any(|known| &known.id == language) {
            self.data
                .get(language)
                .ok_or_else(|| unsupported_language(language.as_str()))
        } else {
            Err(unsupported_language(language.as_str()))
        }
    }
}

fn qualified(symbol: &SymbolInfo) -> String {
    match &symbol.container {
        Some(container) => format!("{container}.{}", symbol.name),
        None => symbol.name.clone(),
    }
}

impl CodeService for StaticCodeService {
    fn languages(&self) -> Vec<LanguageInfo> {
        self.languages.clone()
    }

    fn language_of(&self, path: Option<&Path>, _bytes: &[u8]) -> Option<LanguageId> {
        let extension = path?.extension()?.to_str()?.to_ascii_lowercase();
        self.languages
            .iter()
            .find(|language| language.extensions.contains(&extension))
            .map(|language| language.id.clone())
    }

    fn outline(
        &self,
        _bytes: &[u8],
        language: &LanguageId,
        depth: Option<u32>,
    ) -> Result<Vec<SymbolInfo>> {
        Ok(self
            .data(language)?
            .symbols
            .iter()
            .filter(|symbol| depth.is_none_or(|depth| symbol.depth <= depth))
            .cloned()
            .collect())
    }

    fn find_symbol(
        &self,
        _bytes: &[u8],
        language: &LanguageId,
        name: &str,
    ) -> Result<Vec<SymbolInfo>> {
        Ok(self
            .data(language)?
            .symbols
            .iter()
            .filter(|symbol| symbol.name == name || qualified(symbol) == name)
            .cloned()
            .collect())
    }

    fn enclosing(
        &self,
        _bytes: &[u8],
        language: &LanguageId,
        offset: u64,
    ) -> Result<Option<SymbolInfo>> {
        Ok(self
            .data(language)?
            .symbols
            .iter()
            .filter(|symbol| symbol.span.bytes.contains(offset))
            .max_by_key(|symbol| symbol.depth)
            .cloned())
    }

    fn classify_spans(&self, _bytes: &[u8], language: &LanguageId) -> Result<Vec<ClassifiedSpan>> {
        Ok(self.data(language)?.spans.clone())
    }

    fn check_syntax(&self, _bytes: &[u8], language: &LanguageId) -> Result<Vec<SyntaxIssue>> {
        Ok(self.data(language)?.issues.clone())
    }

    fn compile_pattern(
        &self,
        pattern: &str,
        language: &LanguageId,
    ) -> Result<Box<dyn SyntaxPattern>> {
        self.data(language)?;
        if pattern.trim().is_empty() {
            return Err(Error::new(
                ErrorCode::PatternSyntax,
                "the structural pattern is empty; write the code to match",
            ));
        }
        Ok(Box::new(StaticPattern {
            language: language.clone(),
            source: pattern.to_owned(),
        }))
    }

    fn syntax_search(
        &self,
        _bytes: &[u8],
        pattern: &dyn SyntaxPattern,
    ) -> Result<Vec<SyntaxMatch>> {
        Ok(self
            .data(pattern.language())?
            .matches
            .get(pattern.source())
            .cloned()
            .unwrap_or_default())
    }

    fn render_template(&self, found: &SyntaxMatch, template: &str) -> Result<String> {
        let mut captures: Vec<_> = found.captures.iter().collect();
        captures.sort_by_key(|capture| std::cmp::Reverse(capture.name.len()));
        let mut out = String::with_capacity(template.len());
        let mut rest = template;
        while !rest.is_empty() {
            let found = ["$$$", "$"].into_iter().find_map(|sigil| {
                let after = rest.strip_prefix(sigil)?;
                captures
                    .iter()
                    .find(|capture| after.starts_with(capture.name.as_str()))
                    .map(|capture| (sigil.len() + capture.name.len(), capture.text.as_str()))
            });
            if let Some((length, text)) = found {
                out.push_str(text);
                rest = &rest[length..];
            } else if let Some(character) = rest.chars().next() {
                out.push(character);
                rest = &rest[character.len_utf8()..];
            }
        }
        Ok(out)
    }

    fn references(
        &self,
        _bytes: &[u8],
        language: &LanguageId,
        name: &str,
    ) -> Result<Vec<SymbolReference>> {
        Ok(self
            .data(language)?
            .references
            .get(name)
            .cloned()
            .unwrap_or_default())
    }

    fn function_metrics(
        &self,
        _bytes: &[u8],
        language: &LanguageId,
    ) -> Result<Vec<FunctionMetrics>> {
        Ok(self.data(language)?.metrics.clone())
    }

    fn tokens(&self, _bytes: &[u8], language: &LanguageId) -> Result<Vec<SyntaxToken>> {
        Ok(self.data(language)?.tokens.clone())
    }
}

/// Repository queries answering with registered data.
///
/// ```
/// use std::path::Path;
///
/// use nurt_contracts::services::GitQueryService;
/// use nurt_testkit::services::StaticGitQuery;
///
/// let git = StaticGitQuery::new("/repo")
///     .with_tracked(["src/main.rs"])
///     .with_ignored(["target/"]);
/// assert_eq!(git.repo_root(Path::new("/repo/src")).unwrap().as_deref(), Some(Path::new("/repo")));
/// assert_eq!(git.repo_root(Path::new("/other")).unwrap(), None);
/// assert!(git.is_ignored(Path::new("/repo"), "target/debug/app").unwrap());
/// ```
#[derive(Debug, Clone, Default)]
pub struct StaticGitQuery {
    root: Option<PathBuf>,
    statuses: Vec<GitPathStatus>,
    changed: Vec<String>,
    tracked: Vec<String>,
    ignored: Vec<String>,
    history: Vec<FileHistoryStat>,
}

fn strings<I, S>(items: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    items.into_iter().map(Into::into).collect()
}

impl StaticGitQuery {
    /// Repository with the working tree at `root` and no data.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: Some(root.into()),
            ..Self::default()
        }
    }

    /// Registers the status entries.
    #[must_use]
    pub fn with_status(mut self, statuses: Vec<GitPathStatus>) -> Self {
        self.statuses = statuses;
        self
    }

    /// Registers the changed paths.
    #[must_use]
    pub fn with_changed<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.changed = strings(paths);
        self
    }

    /// Registers the tracked paths.
    #[must_use]
    pub fn with_tracked<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tracked = strings(paths);
        self
    }

    /// Registers ignored paths; an entry ending with `/` ignores everything
    /// below it.
    #[must_use]
    pub fn with_ignored<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.ignored = strings(paths);
        self
    }

    /// Registers the file history.
    #[must_use]
    pub fn with_history(mut self, history: Vec<FileHistoryStat>) -> Self {
        self.history = history;
        self
    }
}

impl GitQueryService for StaticGitQuery {
    fn repo_root(&self, path: &Path) -> Result<Option<PathBuf>> {
        Ok(self.root.clone().filter(|root| path.starts_with(root)))
    }

    fn status(&self, _root: &Path, paths: &[String]) -> Result<Vec<GitPathStatus>> {
        Ok(self
            .statuses
            .iter()
            .filter(|status| {
                if paths.is_empty() {
                    status.index != GitChange::Unmodified
                        || status.worktree != GitChange::Unmodified
                } else {
                    paths.contains(&status.path)
                }
            })
            .cloned()
            .collect())
    }

    fn changed_files(&self, _root: &Path, _against: Option<&str>) -> Result<Vec<String>> {
        Ok(self.changed.clone())
    }

    fn tracked_files(&self, _root: &Path) -> Result<Vec<String>> {
        Ok(self.tracked.clone())
    }

    fn is_ignored(&self, _root: &Path, path: &str) -> Result<bool> {
        Ok(self.ignored.iter().any(|ignored| {
            ignored == path || (ignored.ends_with('/') && path.starts_with(ignored.as_str()))
        }))
    }

    fn file_history(
        &self,
        _root: &Path,
        _since_days: u32,
        limit: u32,
    ) -> Result<Vec<FileHistoryStat>> {
        let mut history = self.history.clone();
        history.sort_by_key(|stat| std::cmp::Reverse(stat.commits));
        history.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        Ok(history)
    }
}

/// Byte order mark of UTF-8.
const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// Label of UTF-8 in WHATWG form.
const UTF8_LABEL: &str = "utf-8";

/// Text codec for UTF-8 with and without byte order mark.
///
/// Content that is not valid UTF-8 or holds a NUL byte is detected as
/// binary; other encodings fail with `E_ENCODING`.
///
/// ```
/// use nurt_contracts::services::{LineEnding, TextCodecService, TextEncoding};
/// use nurt_testkit::services::Utf8TextCodec;
///
/// let detected = Utf8TextCodec.detect(b"\xEF\xBB\xBFa\r\nb\r\n");
/// assert_eq!(detected.line_ending, LineEnding::CrLf);
/// let encoding = detected.encoding.unwrap();
/// assert!(encoding.bom);
/// let text = Utf8TextCodec.decode(b"\xEF\xBB\xBFa", &encoding).unwrap();
/// assert_eq!(Utf8TextCodec.encode(&text, &encoding).unwrap(), b"\xEF\xBB\xBFa");
/// assert!(Utf8TextCodec.detect(&[0, 159, 146, 150]).binary);
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct Utf8TextCodec;

fn line_ending(text: &[u8]) -> LineEnding {
    let mut seen = [false; 3];
    let mut index = 0;
    while index < text.len() {
        match text[index] {
            b'\r' if text.get(index + 1) == Some(&b'\n') => {
                seen[1] = true;
                index += 1;
            }
            b'\r' => seen[2] = true,
            b'\n' => seen[0] = true,
            _ => {}
        }
        index += 1;
    }
    match seen {
        [false, false, false] => LineEnding::None,
        [true, false, false] => LineEnding::Lf,
        [false, true, false] => LineEnding::CrLf,
        [false, false, true] => LineEnding::Cr,
        _ => LineEnding::Mixed,
    }
}

fn not_utf8(label: &str) -> Error {
    Error::new(
        ErrorCode::Encoding,
        format!("encoding `{label}` is not available in this context; use utf-8"),
    )
}

impl TextCodecService for Utf8TextCodec {
    fn detect(&self, bytes: &[u8]) -> TextDetection {
        let (bom, body) = match bytes.strip_prefix(&UTF8_BOM) {
            Some(body) => (true, body),
            None => (false, bytes),
        };
        if body.contains(&0) || std::str::from_utf8(body).is_err() {
            return TextDetection {
                binary: true,
                encoding: None,
                line_ending: LineEnding::None,
            };
        }
        TextDetection {
            binary: false,
            encoding: Some(TextEncoding {
                label: UTF8_LABEL.to_owned(),
                bom,
            }),
            line_ending: line_ending(body),
        }
    }

    fn decode(&self, bytes: &[u8], encoding: &TextEncoding) -> Result<String> {
        if encoding.label != UTF8_LABEL {
            return Err(not_utf8(&encoding.label));
        }
        let body = bytes.strip_prefix(&UTF8_BOM).unwrap_or(bytes);
        String::from_utf8(body.to_vec()).map_err(|_| {
            Error::new(
                ErrorCode::Encoding,
                "the content is not valid utf-8; read it as binary",
            )
        })
    }

    fn encode(&self, text: &str, encoding: &TextEncoding) -> Result<Vec<u8>> {
        if encoding.label != UTF8_LABEL {
            return Err(not_utf8(&encoding.label));
        }
        let mut out = Vec::with_capacity(text.len() + UTF8_BOM.len());
        if encoding.bom {
            out.extend_from_slice(&UTF8_BOM);
        }
        out.extend_from_slice(text.as_bytes());
        Ok(out)
    }
}

/// File holder service answering with registered holders per path.
#[derive(Debug, Clone, Default)]
pub struct StaticFileHolders {
    holders: BTreeMap<PathBuf, Vec<FileHolder>>,
}

impl StaticFileHolders {
    /// Service without holders.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the holders of `path`.
    #[must_use]
    pub fn with_holders(mut self, path: impl Into<PathBuf>, holders: Vec<FileHolder>) -> Self {
        self.holders.insert(path.into(), holders);
        self
    }
}

impl FileHolderService for StaticFileHolders {
    fn holders(&self, path: &Path) -> Result<Vec<FileHolder>> {
        Ok(self.holders.get(path).cloned().unwrap_or_default())
    }
}

/// Review rule provider with a fixed list of rules.
#[derive(Debug, Clone, Default)]
pub struct StaticReviewRules(pub Vec<ReviewRule>);

impl ReviewRuleProvider for StaticReviewRules {
    fn rules(&self) -> Vec<ReviewRule> {
        self.0.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use nurt_contracts::env::Clock;
    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::services::{
        CodeService, DocFormatId, DocService, FileListService, GitChange, GitPathStatus,
        GitQueryService, LanguageId, LanguageInfo, ProcessHost, ProgramSpec, SerializeOptions,
        ServiceSpawnSpec, SyntaxCapture, SyntaxMatch, TextCodecService, TextEncoding, TextPosition,
        TextSpan, WalkSpec,
    };
    use nurt_contracts::stage::{CancelReason, CancelToken};

    use super::{
        EmptyFileList, KILL_SIGNAL, ProgramScript, ScriptedProcessHost, StaticCodeService,
        StaticDocService, StaticGitQuery, UnavailableProcessHost, UnsupportedCodeService,
        UnsupportedDocService, Utf8TextCodec,
    };
    use crate::clock::VirtualClock;
    use crate::vfs::{VirtualFs, test_path};

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

    #[test]
    fn document_service_detects_nothing() {
        assert!(UnsupportedDocService.formats().is_empty());
        assert_eq!(UnsupportedDocService.detect_format(None, b"{}"), None);
    }

    #[test]
    fn code_service_detects_nothing() {
        assert!(UnsupportedCodeService.languages().is_empty());
        assert_eq!(
            UnsupportedCodeService.language_of(Some(Path::new("main.rs")), b"fn main() {}"),
            None
        );
    }

    #[test]
    fn process_host_reports_missing_programs() {
        let error = UnavailableProcessHost
            .resolve_program("git", Path::new("/"))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ExternalMissing);
    }

    #[test]
    fn file_list_is_empty() {
        let entries = EmptyFileList.walk(&WalkSpec::new("/")).unwrap();
        assert_eq!(entries.count(), 0);
    }

    #[test]
    fn scripted_run_is_cut_by_the_time_limit() {
        let clock = Arc::new(VirtualClock::new());
        let host = ScriptedProcessHost::new(clock.clone());
        host.script(
            ProgramScript::new("sleep")
                .stdout(Duration::from_secs(1), "early\n")
                .stdout(Duration::from_secs(9), "late\n")
                .runs_for(Duration::from_secs(10)),
        );
        let mut spec = ProgramSpec::new("sleep", vec!["10".to_owned()], "/");
        spec.timeout_ms = Some(2_000);
        let outcome = host.run_program(&spec, &CancelToken::new()).unwrap();
        assert!(outcome.timed_out);
        assert_eq!(outcome.exit_code, None);
        assert_eq!(outcome.stdout, b"early\n");
        assert_eq!(clock.monotonic(), Duration::from_secs(2));
    }

    #[test]
    fn scripted_run_applies_file_changes_and_limits_output() {
        let clock = Arc::new(VirtualClock::new());
        let fs = Arc::new(VirtualFs::new(clock.clone()));
        let removed = test_path("/project/old.txt").unwrap();
        let written = test_path("/project/out/new.txt").unwrap();
        fs.write_file(&removed, b"x").unwrap();
        let host = ScriptedProcessHost::new(clock).with_fs(fs.clone());
        host.script(
            ProgramScript::new("gen")
                .with_args(["all"])
                .stderr(Duration::ZERO, "warning: long text\n")
                .writes(written.clone(), "made")
                .removes(removed.clone())
                .exits_with(3)
                .once(),
        );
        let mut spec = ProgramSpec::new("gen", vec!["all".to_owned()], "/project");
        spec.detect_changes = true;
        spec.output_limit = Some(7);
        let outcome = host.run_program(&spec, &CancelToken::new()).unwrap();
        assert_eq!(outcome.exit_code, Some(3));
        assert_eq!(outcome.stderr, b"warning");
        assert!(outcome.stderr_truncated);
        assert_eq!(outcome.changed_paths.len(), 2);
        assert_eq!(fs.read_file(&written).unwrap(), b"made");
        assert!(!fs.exists(&removed));
        let again = host.run_program(&spec, &CancelToken::new()).unwrap_err();
        assert_eq!(again.code(), ErrorCode::ExternalMissing);
    }

    #[test]
    fn scripted_run_respects_arguments_and_cancellation() {
        let host = ScriptedProcessHost::new(Arc::new(VirtualClock::new()));
        host.script(
            ProgramScript::new("git")
                .with_args(["status"])
                .at_path("/opt/git"),
        );
        assert_eq!(
            host.resolve_program("git", Path::new("/")).unwrap(),
            Path::new("/opt/git")
        );
        let other = ProgramSpec::new("git", vec!["log".to_owned()], "/");
        assert!(host.run_program(&other, &CancelToken::new()).is_err());
        let cancel = CancelToken::new();
        cancel.cancel(CancelReason::Requested);
        let spec = ProgramSpec::new("git", vec!["status".to_owned()], "/");
        assert!(host.run_program(&spec, &cancel).unwrap().cancelled);
    }

    #[test]
    fn scripted_child_streams_output_over_time() {
        let clock = Arc::new(VirtualClock::new());
        let host = ScriptedProcessHost::new(clock.clone());
        host.script(
            ProgramScript::new("server")
                .stdout(Duration::from_secs(2), "ready\n")
                .stdout(Duration::from_secs(5), "done\n")
                .runs_for(Duration::from_secs(6))
                .exits_with(0),
        );
        let mut child = host.spawn_service(&spawn_spec("server")).unwrap();
        let pid = child.pid();
        child.take_stdin().unwrap().write_all(b"ping").unwrap();
        assert_eq!(host.stdin_of(pid), b"ping");
        assert_eq!(child.try_wait().unwrap(), None);
        let mut stdout = child.take_stdout().unwrap();
        let mut buffer = [0_u8; 64];
        let count = stdout.read(&mut buffer).unwrap();
        assert_eq!(&buffer[..count], b"ready\n");
        assert_eq!(clock.monotonic(), Duration::from_secs(2));
        assert_eq!(child.wait_timeout(Duration::from_secs(1)).unwrap(), None);
        let exit = child
            .wait_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        assert_eq!(exit.exit_code, Some(0));
        assert_eq!(clock.monotonic(), Duration::from_secs(6));
        let mut rest = String::new();
        stdout.read_to_string(&mut rest).unwrap();
        assert_eq!(rest, "done\n");
        assert_eq!(host.spawned().len(), 1);
    }

    #[test]
    fn scripted_child_stops_on_kill() {
        let host = ScriptedProcessHost::new(Arc::new(VirtualClock::new()));
        host.script(
            ProgramScript::new("lsp")
                .stdout(Duration::from_secs(1), "x")
                .runs_for(Duration::from_secs(100)),
        );
        let mut child = host.spawn_service(&spawn_spec("lsp")).unwrap();
        let mut stdout = child.take_stdout().unwrap();
        child.kill_tree().unwrap();
        let exit = child.try_wait().unwrap().unwrap();
        assert_eq!(exit.signal, Some(KILL_SIGNAL));
        assert_eq!(stdout.read(&mut [0_u8; 4]).unwrap(), 0);
        assert!(child.take_stdout().is_none());
    }

    #[test]
    fn static_documents_serve_registered_and_built_in_answers() {
        let key = nurt_contracts::keypath::KeyPath::parse("a").unwrap();
        let doc = StaticDocService::json().with_items(DocFormatId::JSON, &key, Vec::new());
        assert_eq!(
            doc.detect_format(Some(Path::new("x.JSON")), b"not json"),
            Some(DocFormatId::JSON)
        );
        assert_eq!(doc.detect_format(None, b"not json"), None);
        let issues = doc
            .check_syntax(b"{\n  \"a\": }", &DocFormatId::JSON)
            .unwrap();
        assert_eq!(issues[0].span.start.line, 2);
        assert!(
            doc.check_syntax(b"[1]", &DocFormatId::JSON)
                .unwrap()
                .is_empty()
        );
        let bytes = doc
            .serialize_records(&[], &DocFormatId::JSON, &SerializeOptions::default())
            .unwrap();
        assert_eq!(bytes, b"[]");
        let error = doc
            .read_keys(b"", &DocFormatId::YAML, &key, None)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Unsupported);
    }

    #[test]
    fn static_code_renders_templates_and_rejects_unknown_languages() {
        let rust = LanguageId::new("rust").unwrap();
        let span = TextSpan {
            bytes: nurt_contracts::services::ByteRange::new(0, 3),
            start: TextPosition { line: 1, column: 1 },
            end: TextPosition { line: 1, column: 4 },
        };
        let found = SyntaxMatch {
            span,
            node_kind: "call".to_owned(),
            text: "f(a, b, c)".to_owned(),
            captures: vec![
                SyntaxCapture {
                    name: "X".to_owned(),
                    spans: vec![span],
                    text: "a".to_owned(),
                },
                SyntaxCapture {
                    name: "XS".to_owned(),
                    spans: vec![span],
                    text: "b, c".to_owned(),
                },
            ],
        };
        let code = StaticCodeService::new()
            .with_language(LanguageInfo {
                id: rust.clone(),
                name: "Rust".to_owned(),
                extensions: vec!["rs".to_owned()],
                grammar_version: None,
            })
            .with_matches(&rust, "f($X, $$$XS)", vec![found.clone()]);
        let pattern = code.compile_pattern("f($X, $$$XS)", &rust).unwrap();
        assert_eq!(code.syntax_search(b"", pattern.as_ref()).unwrap().len(), 1);
        assert_eq!(
            code.render_template(&found, "g($$$XS, $X)").unwrap(),
            "g(b, c, a)"
        );
        let mut literal = found.clone();
        literal.captures[1].text = "$X".to_owned();
        assert_eq!(
            code.render_template(&literal, "g($$$XS, $X)").unwrap(),
            "g($X, a)",
            "the text of a capture is not scanned for further captures"
        );
        let python = LanguageId::new("python").unwrap();
        assert_eq!(
            code.outline(b"", &python, None).unwrap_err().code(),
            ErrorCode::UnsupportedLanguage
        );
        assert_eq!(
            code.compile_pattern(" ", &rust).unwrap_err().code(),
            ErrorCode::PatternSyntax
        );
    }

    #[test]
    fn static_git_lists_changed_status_only_by_default() {
        let status = |path: &str, worktree| GitPathStatus {
            path: path.to_owned(),
            index: GitChange::Unmodified,
            worktree,
            original_path: None,
        };
        let git = StaticGitQuery::new("/repo").with_status(vec![
            status("a", GitChange::Modified),
            status("b", GitChange::Unmodified),
        ]);
        let root = Path::new("/repo");
        assert_eq!(git.status(root, &[]).unwrap().len(), 1);
        assert_eq!(git.status(root, &["b".to_owned()]).unwrap().len(), 1);
    }

    #[test]
    fn text_codec_rejects_other_encodings() {
        let latin = TextEncoding {
            label: "windows-1252".to_owned(),
            bom: false,
        };
        assert_eq!(
            Utf8TextCodec.decode(b"a", &latin).unwrap_err().code(),
            ErrorCode::Encoding
        );
        assert_eq!(
            Utf8TextCodec
                .decode(&[0xFF], &TextEncoding::utf8())
                .unwrap_err()
                .code(),
            ErrorCode::Encoding
        );
        assert_eq!(
            Utf8TextCodec.detect(b"a\nb\r\n").line_ending,
            nurt_contracts::services::LineEnding::Mixed
        );
    }
}
