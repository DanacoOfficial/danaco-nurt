// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Repositories with exact histories for tests, written object by object
//! without an external program, and an execution context of test doubles.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gix::ObjectId;
use gix::objs::Write as _;
use gix::objs::tree::EntryKind;
use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::{
    ByteRange, ClassifiedSpan, CodeService, FunctionMetrics, LanguageId, LanguageInfo, Services,
    SymbolInfo, SymbolKind, SymbolReference, SyntaxIssue, SyntaxMatch, SyntaxPattern, SyntaxToken,
    TextPosition, TextSpan,
};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelReason, CancelToken, ExecCtx, ReadStage, StageData, StageInput,
};
use nurt_contracts::value::{Record, Value};

use super::repo::{OpenRepo, display_path};

/// Committer time of the first commit of a fixture.
const FIRST_TIME: i64 = 1_767_261_600;

/// Seconds between two commits of a fixture.
const STEP: i64 = 3600;

/// A repository under construction in a temporary directory, with the
/// branch `main` checked out.
pub(crate) struct Fixture {
    dir: tempfile::TempDir,
    root: PathBuf,
    repo: gix::Repository,
    clock: i64,
    frozen: bool,
    times: HashMap<ObjectId, i64>,
    messages: HashMap<ObjectId, String>,
}

impl Fixture {
    /// An empty repository.
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = gix::init(dir.path()).unwrap();
        std::fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let root = gix::path::realpath(dir.path()).unwrap();
        Self {
            dir,
            root,
            repo,
            clock: FIRST_TIME,
            frozen: false,
            times: HashMap::new(),
            messages: HashMap::new(),
        }
    }

    /// Root of the working tree in the canonical form of the language.
    pub(crate) fn root(&self) -> String {
        display_path(&self.root)
    }

    /// Absolute path of a file of the working tree.
    pub(crate) fn path(&self, relative: &str) -> String {
        format!("{}/{relative}", self.root())
    }

    /// The repository opened as the commands open it.
    pub(crate) fn open(&self) -> OpenRepo {
        OpenRepo::discover(&self.root(), &exec_ctx(None)).unwrap()
    }

    /// Gives every following commit the same time.
    pub(crate) fn freeze_clock(&mut self) {
        self.frozen = true;
    }

    /// Writes a file of the working tree without committing it.
    pub(crate) fn write_file(&self, relative: &str, content: &str) {
        write_bytes(&self.dir.path().join(relative), content.as_bytes());
    }

    /// Commits changes on top of `HEAD`, moves the current branch and writes
    /// the files to the working tree; `None` deletes a file.
    pub(crate) fn commit(&mut self, message: &str, files: &[(&str, Option<&str>)]) -> ObjectId {
        self.commit_as(("Author", "author@example.com"), message, files)
    }

    /// As [`Fixture::commit`], with another author.
    pub(crate) fn commit_as(
        &mut self,
        author: (&str, &str),
        message: &str,
        files: &[(&str, Option<&str>)],
    ) -> ObjectId {
        let parents: Vec<ObjectId> = self.head_commit().into_iter().collect();
        let id = self.write_commit(&parents, author, message, files, None);
        for (path, content) in files {
            let target = self.dir.path().join(path);
            match content {
                Some(text) => write_bytes(&target, text.as_bytes()),
                None => {
                    let _ = std::fs::remove_file(target);
                }
            }
        }
        self.move_current(id);
        id
    }

    /// Commits changes on top of the tree of the first of `parents` without
    /// moving any reference.
    pub(crate) fn commit_on(
        &mut self,
        parents: &[ObjectId],
        message: &str,
        files: &[(&str, Option<&str>)],
    ) -> ObjectId {
        self.write_commit(
            parents,
            ("Author", "author@example.com"),
            message,
            files,
            None,
        )
    }

    /// Commits a symbolic link at `path` on top of `HEAD`.
    pub(crate) fn commit_link(&mut self, message: &str, path: &str, target: &str) -> ObjectId {
        let parents: Vec<ObjectId> = self.head_commit().into_iter().collect();
        let base = parents
            .first()
            .map_or_else(|| self.empty_tree(), |id| self.tree(*id));
        let blob = self.repo.write_blob(target.as_bytes()).unwrap().detach();
        let mut editor = self.repo.edit_tree(base).unwrap();
        editor.upsert(path, EntryKind::Link, blob).unwrap();
        let tree = editor.write().unwrap().detach();
        let id = self.write_raw_commit(
            tree,
            &parents,
            ("Author", "author@example.com"),
            message,
            None,
        );
        self.move_current(id);
        id
    }

    /// Commits on top of `HEAD` without changes, with a `gpgsig` header.
    pub(crate) fn commit_signed(&mut self, message: &str, signature: &str) -> ObjectId {
        let parents: Vec<ObjectId> = self.head_commit().into_iter().collect();
        let id = self.write_commit(
            &parents,
            ("Author", "author@example.com"),
            message,
            &[],
            Some(signature),
        );
        self.move_current(id);
        id
    }

    /// Creates or moves the branch `name`.
    pub(crate) fn branch(&mut self, name: &str, id: ObjectId) {
        self.set_ref(&format!("refs/heads/{name}"), id);
    }

    /// Creates a lightweight tag.
    pub(crate) fn tag(&mut self, name: &str, id: ObjectId) {
        self.set_ref(&format!("refs/tags/{name}"), id);
    }

    /// Creates an annotated tag and returns the tag object.
    pub(crate) fn annotated_tag(&mut self, name: &str, id: ObjectId, message: &str) -> ObjectId {
        let time = self.tick();
        let body = format!(
            "object {id}\ntype commit\ntag {name}\ntagger Tagger <tagger@example.com> {time} +0000\n\n{message}\n"
        );
        let tag = self
            .repo
            .write_buf(gix::object::Kind::Tag, body.as_bytes())
            .unwrap();
        self.set_ref(&format!("refs/tags/{name}"), tag);
        tag
    }

    /// Points `HEAD` at a commit instead of a branch.
    pub(crate) fn detach_head(&mut self, id: ObjectId) {
        std::fs::write(self.dir.path().join(".git/HEAD"), format!("{id}\n")).unwrap();
    }

    /// The commit of `HEAD`.
    pub(crate) fn head(&self) -> ObjectId {
        self.head_commit().unwrap()
    }

    /// Resolves a revision.
    pub(crate) fn resolve(&self, spec: &str) -> ObjectId {
        self.open().resolve_commit(spec).unwrap()
    }

    /// Committer time of a commit made by the fixture.
    pub(crate) fn time_of(&self, id: ObjectId) -> i64 {
        self.times[&id]
    }

    /// First line of the message of a commit made by the fixture.
    pub(crate) fn message_of(&self, id: ObjectId) -> String {
        self.messages[&id].clone()
    }

    fn head_commit(&self) -> Option<ObjectId> {
        let repo = gix::open(self.dir.path()).ok()?;
        repo.head_id().ok().map(gix::Id::detach)
    }

    fn move_current(&mut self, id: ObjectId) {
        let head = std::fs::read_to_string(self.dir.path().join(".git/HEAD")).unwrap();
        match head.strip_prefix("ref: ") {
            Some(name) => self.set_ref(name.trim(), id),
            None => self.detach_head(id),
        }
    }

    fn set_ref(&mut self, name: &str, id: ObjectId) {
        write_bytes(
            &self.dir.path().join(".git").join(name),
            format!("{id}\n").as_bytes(),
        );
    }

    fn empty_tree(&self) -> ObjectId {
        ObjectId::empty_tree(self.repo.object_hash())
    }

    fn tree(&self, commit: ObjectId) -> ObjectId {
        self.repo
            .find_commit(commit)
            .unwrap()
            .tree_id()
            .unwrap()
            .detach()
    }

    fn tick(&mut self) -> i64 {
        let time = self.clock;
        if !self.frozen {
            self.clock += STEP;
        }
        time
    }

    fn write_commit(
        &mut self,
        parents: &[ObjectId],
        author: (&str, &str),
        message: &str,
        files: &[(&str, Option<&str>)],
        signature: Option<&str>,
    ) -> ObjectId {
        let base = parents
            .first()
            .map_or_else(|| self.empty_tree(), |id| self.tree(*id));
        let mut editor = self.repo.edit_tree(base).unwrap();
        for (path, content) in files {
            match content {
                Some(text) => {
                    let blob = self.repo.write_blob(text.as_bytes()).unwrap().detach();
                    editor.upsert(*path, EntryKind::Blob, blob).unwrap();
                }
                None => {
                    editor.remove(*path).unwrap();
                }
            }
        }
        let tree = editor.write().unwrap().detach();
        self.write_raw_commit(tree, parents, author, message, signature)
    }

    fn write_raw_commit(
        &mut self,
        tree: ObjectId,
        parents: &[ObjectId],
        author: (&str, &str),
        message: &str,
        signature: Option<&str>,
    ) -> ObjectId {
        let time = self.tick();
        let mut body = format!("tree {tree}\n");
        for parent in parents {
            writeln!(body, "parent {parent}").unwrap();
        }
        writeln!(body, "author {} <{}> {time} +0000", author.0, author.1).unwrap();
        writeln!(
            body,
            "committer Committer <committer@example.com> {time} +0000"
        )
        .unwrap();
        if let Some(signature) = signature {
            writeln!(body, "gpgsig {}", signature.replace('\n', "\n ")).unwrap();
        }
        body.push('\n');
        body.push_str(message);
        if !message.ends_with('\n') {
            body.push('\n');
        }
        let id = self
            .repo
            .write_buf(gix::object::Kind::Commit, body.as_bytes())
            .unwrap();
        self.times.insert(id, time);
        self.messages
            .insert(id, message.lines().next().unwrap_or_default().to_owned());
        id
    }
}

fn write_bytes(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> WallTime {
        WallTime::from_unix_nanos(1_800_000_000_000_000_000)
    }

    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: self.now(),
            boot: BootId("test".to_owned()),
            since_boot_with_suspend: Duration::ZERO,
            since_boot_active: Duration::ZERO,
        }
    }

    fn sleep(
        &self,
        _duration: Duration,
        cancel: &CancelToken,
    ) -> std::result::Result<(), CancelReason> {
        cancel.reason().map_or(Ok(()), Err)
    }
}

struct TestHost {
    info: HostInfo,
}

impl Host for TestHost {
    fn info(&self) -> &HostInfo {
        &self.info
    }

    fn env_var(&self, _name: &str) -> Option<OsString> {
        None
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        Vec::new()
    }
}

/// Execution context of test doubles, with `code` as the code service.
pub(crate) fn exec_ctx(code: Option<Arc<dyn CodeService>>) -> ExecCtx {
    let host = TestHost {
        info: HostInfo {
            os: Os::Linux,
            os_version: "6.8.0".to_owned(),
            architecture: "x86_64".to_owned(),
            hostname: "test".to_owned(),
            user: "user".to_owned(),
            home: PathBuf::from("/home/user"),
            temp: PathBuf::from("/tmp"),
            path_style: PathStyle::Posix,
            timezone: "UTC".to_owned(),
            locale: None,
            cpus: 1,
        },
    };
    let builder = ExecCtx::builder(Arc::new(FixedClock), Arc::new(host));
    match code {
        Some(code) => builder
            .services(Arc::new(Services::new().with_code(code)))
            .build(),
        None => builder.build(),
    }
}

/// Arguments of `command` with the given values.
pub(crate) fn args(command: &str, values: &[(&str, Value)]) -> BoundArgs {
    let mut args = BoundArgs::new(command);
    for (name, value) in values {
        args.insert(*name, ArgValue::Value(value.clone()), false);
    }
    args
}

/// Runs a read stage and collects its records.
pub(crate) fn run(stage: &dyn ReadStage, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
    let output = stage.execute(args, StageInput::none(), ctx)?;
    match output.data {
        StageData::Records(stream) => stream.collect(),
        StageData::None => Ok(Vec::new()),
        _ => panic!("a source stage returned a value instead of records"),
    }
}

/// Code service that knows functions written as `fn name(` at the start of
/// a line, ending at the next line that is `}`.
pub(crate) struct LineCode;

fn unsupported<T>() -> Result<T> {
    Err(Error::new(
        ErrorCode::UnsupportedLanguage,
        "not available in tests",
    ))
}

fn rust() -> LanguageId {
    LanguageId::new("rust").unwrap()
}

impl CodeService for LineCode {
    fn languages(&self) -> Vec<LanguageInfo> {
        Vec::new()
    }

    fn language_of(&self, path: Option<&Path>, _bytes: &[u8]) -> Option<LanguageId> {
        path.and_then(Path::extension)
            .is_some_and(|ext| ext == "rs")
            .then(rust)
    }

    fn outline(&self, _: &[u8], _: &LanguageId, _: Option<u32>) -> Result<Vec<SymbolInfo>> {
        unsupported()
    }

    fn find_symbol(&self, bytes: &[u8], _: &LanguageId, name: &str) -> Result<Vec<SymbolInfo>> {
        let text = String::from_utf8_lossy(bytes);
        let lines: Vec<&str> = text.lines().collect();
        let mut found = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            if line.starts_with(&format!("fn {name}(")) {
                let end = lines[index..]
                    .iter()
                    .position(|l| *l == "}")
                    .map_or(lines.len(), |p| index + p);
                let start = TextPosition {
                    line: u32::try_from(index + 1).unwrap(),
                    column: 1,
                };
                let stop = TextPosition {
                    line: u32::try_from(end + 2).unwrap(),
                    column: 1,
                };
                let span = TextSpan {
                    bytes: ByteRange::new(0, 0),
                    start,
                    end: stop,
                };
                found.push(SymbolInfo {
                    name: name.to_owned(),
                    kind: SymbolKind::Function,
                    container: None,
                    depth: 0,
                    span,
                    name_span: span,
                    signature: None,
                    exported: None,
                });
            }
        }
        Ok(found)
    }

    fn enclosing(&self, _: &[u8], _: &LanguageId, _: u64) -> Result<Option<SymbolInfo>> {
        unsupported()
    }

    fn classify_spans(&self, _: &[u8], _: &LanguageId) -> Result<Vec<ClassifiedSpan>> {
        unsupported()
    }

    fn check_syntax(&self, _: &[u8], _: &LanguageId) -> Result<Vec<SyntaxIssue>> {
        unsupported()
    }

    fn compile_pattern(&self, _: &str, _: &LanguageId) -> Result<Box<dyn SyntaxPattern>> {
        unsupported()
    }

    fn syntax_search(&self, _: &[u8], _: &dyn SyntaxPattern) -> Result<Vec<SyntaxMatch>> {
        unsupported()
    }

    fn render_template(&self, _: &SyntaxMatch, _: &str) -> Result<String> {
        unsupported()
    }

    fn references(&self, _: &[u8], _: &LanguageId, _: &str) -> Result<Vec<SymbolReference>> {
        unsupported()
    }

    fn function_metrics(&self, _: &[u8], _: &LanguageId) -> Result<Vec<FunctionMetrics>> {
        unsupported()
    }

    fn tokens(&self, _: &[u8], _: &LanguageId) -> Result<Vec<SyntaxToken>> {
        unsupported()
    }
}
