// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Deterministic test repositories built without an external program.
//!
//! [`RepoBuilder`] creates repositories in temporary directories with the
//! repository library: files, commits with fixed identities and dates,
//! references, annotated tags and configuration. [`test_ctx`] gives an
//! execution context with a fixed clock and an environment that never
//! reaches the configuration of the machine running the tests.
//! [`RecordingJournal`] is a journal for tests that applies changes to the
//! real file system, panics on any change of an undeclared target, and
//! undoes committed operations byte for byte.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use bstr::ByteSlice;
use nurt_contracts::env::{
    BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime,
};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::fs::{AbsPath, ContentHash, WriteHandle};
use nurt_contracts::journal::{
    AfterState, Applied, BeforeState, ExemptWrite, Expect, ExternalChange, MutationTx, OpHandle,
    OpId, OpKind, Preserved, PreserveMode, RollbackReason, RollbackReport, TargetAction,
    TargetIndex, TargetOutcome, TargetPath, TargetSpec, Mutation,
};
use nurt_contracts::report::IrreversibleEntry;
use nurt_contracts::stage::{CancelReason, CancelToken, ExecCtx, StageFrame};
use sha2::{Digest, Sha256};

use crate::repo::open_options;

/// Clock that always reads the same wall time.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock {
    now: WallTime,
}

impl FixedClock {
    /// Clock at `seconds` after the Unix epoch.
    #[must_use]
    pub fn at_unix_seconds(seconds: i64) -> Self {
        const NANOS_PER_SECOND: i128 = 1_000_000_000;
        Self {
            now: WallTime::from_unix_nanos(i128::from(seconds) * NANOS_PER_SECOND),
        }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> WallTime {
        self.now
    }

    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: self.now,
            boot: BootId("test".to_owned()),
            since_boot_with_suspend: Duration::ZERO,
            since_boot_active: Duration::ZERO,
        }
    }

    fn sleep(&self, _duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
        cancel.reason().map_or(Ok(()), Err)
    }
}

/// Host with a chosen home directory and environment variables.
///
/// `GIT_CONFIG_NOSYSTEM` is set, so the system configuration of the machine
/// running the tests is never read.
#[derive(Debug, Clone)]
pub struct TestHost {
    info: HostInfo,
    vars: Vec<(OsString, OsString)>,
}

impl TestHost {
    /// Host whose `HOME` is `home`.
    #[must_use]
    pub fn new(home: impl Into<PathBuf>) -> Self {
        let home = home.into();
        let info = HostInfo {
            os: Os::Linux,
            os_version: "6.8.0".to_owned(),
            architecture: "x86_64".to_owned(),
            hostname: "test".to_owned(),
            user: "test".to_owned(),
            home: home.clone(),
            temp: std::env::temp_dir(),
            path_style: PathStyle::Posix,
            timezone: "UTC".to_owned(),
            locale: None,
            cpus: 1,
        };
        Self {
            info,
            vars: vec![
                ("HOME".into(), home.into_os_string()),
                ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            ],
        }
    }

    /// The same host with the variable `name` set to `value`.
    #[must_use]
    pub fn with_var(mut self, name: &str, value: impl Into<OsString>) -> Self {
        self.vars.retain(|(existing, _)| existing != name);
        self.vars.push((name.into(), value.into()));
        self
    }
}

impl Host for TestHost {
    fn info(&self) -> &HostInfo {
        &self.info
    }

    fn env_var(&self, name: &str) -> Option<OsString> {
        self.vars
            .iter()
            .find(|(existing, _)| existing == name)
            .map(|(_, value)| value.clone())
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        self.vars.clone()
    }
}

/// Execution context with [`FixedClock`] at [`RepoBuilder::EPOCH`] and a
/// [`TestHost`] whose home directory does not exist.
#[must_use]
pub fn test_ctx() -> ExecCtx {
    test_ctx_with(TestHost::new("/nonexistent-home"))
}

/// Execution context with [`FixedClock`] at [`RepoBuilder::EPOCH`] and the
/// given host, running the command `git.status`.
#[must_use]
pub fn test_ctx_with(host: TestHost) -> ExecCtx {
    let frame = StageFrame {
        command: "git.status".to_owned(),
        ..StageFrame::default()
    };
    ExecCtx::builder(
        Arc::new(FixedClock::at_unix_seconds(RepoBuilder::EPOCH)),
        Arc::new(host),
    )
    .frame(frame)
    .build()
}

/// Repository created by [`RepoBuilder`], removed with its temporary
/// directory when dropped.
#[derive(Debug)]
pub struct Fixture {
    dir: tempfile::TempDir,
    path: PathBuf,
}

impl Fixture {
    /// Root of the working tree, or the repository directory when bare.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Temporary directory holding the repository and nothing else.
    #[must_use]
    pub fn temp_dir(&self) -> &Path {
        self.dir.path()
    }

    /// Repository directory (`.git`, or the root when bare).
    #[must_use]
    pub fn git_dir(&self) -> PathBuf {
        let dot_git = self.path.join(".git");
        if dot_git.is_dir() { dot_git } else { self.path.clone() }
    }

    /// Opens the repository with the options of the console.
    ///
    /// # Panics
    ///
    /// Panics when the repository cannot be opened.
    #[must_use]
    pub fn open(&self) -> gix::Repository {
        gix::open_opts(&self.path, open_options()).expect("fixture repository opens")
    }

    /// Content of every entry below the temporary directory, for byte-level
    /// comparison of states.
    #[must_use]
    pub fn snapshot(&self) -> BTreeMap<PathBuf, TreeEntry> {
        snapshot(self.dir.path())
    }
}

/// One entry of a [`snapshot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeEntry {
    /// Directory.
    Directory,
    /// File with its bytes and executable bit.
    File {
        /// Content.
        bytes: Vec<u8>,
        /// The executable bit is set.
        executable: bool,
    },
    /// Symbolic link with its target.
    Link(PathBuf),
}

/// Content of every entry below `root`, keyed by the path relative to it.
///
/// # Panics
///
/// Panics when an entry cannot be read.
#[must_use]
pub fn snapshot(root: &Path) -> BTreeMap<PathBuf, TreeEntry> {
    let mut entries = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("readable directory") {
            let path = entry.expect("readable entry").path();
            let relative = path.strip_prefix(root).expect("below root").to_path_buf();
            let meta = std::fs::symlink_metadata(&path).expect("readable metadata");
            if meta.file_type().is_symlink() {
                entries.insert(relative, TreeEntry::Link(std::fs::read_link(&path).expect("link")));
            } else if meta.is_dir() {
                entries.insert(relative, TreeEntry::Directory);
                pending.push(path);
            } else {
                entries.insert(
                    relative,
                    TreeEntry::File {
                        bytes: std::fs::read(&path).expect("readable file"),
                        executable: is_executable(&meta),
                    },
                );
            }
        }
    }
    entries
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    const EXECUTE_BITS: u32 = 0o111;
    meta.permissions().mode() & EXECUTE_BITS != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &std::fs::Metadata) -> bool {
    false
}

#[derive(Debug, Clone)]
enum Step {
    File(String, Vec<u8>, bool),
    Remove(String),
    Commit(String),
    Reference(String, String),
    Symbolic(String, String),
    Tag(String, String, String),
    Config(String, String),
    WorktreeOnly(String, Vec<u8>),
}

/// Builder of a repository in a temporary directory.
///
/// Files are written to the tree of the next commit and to the working
/// tree; [`commit`](RepoBuilder::commit) records them with
/// [`RepoBuilder::AUTHOR_NAME`] at [`RepoBuilder::EPOCH`] plus one minute per
/// commit and updates the index to match. The default branch is `main`.
#[derive(Debug, Clone, Default)]
pub struct RepoBuilder {
    bare: bool,
    steps: Vec<Step>,
}

impl RepoBuilder {
    /// Author and committer name of fixture commits.
    pub const AUTHOR_NAME: &'static str = "Ada Lovelace";
    /// Author and committer e-mail of fixture commits.
    pub const AUTHOR_EMAIL: &'static str = "ada@example.com";
    /// Time of the first fixture commit: 2026-01-01T12:00:00Z.
    pub const EPOCH: i64 = 1_767_268_800;
    /// Seconds between consecutive fixture commits.
    pub const STEP_SECONDS: i64 = 60;

    /// Builder of a repository with a working tree.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder of a bare repository.
    #[must_use]
    pub fn bare() -> Self {
        Self {
            bare: true,
            steps: Vec::new(),
        }
    }

    /// Writes `content` to `path` for the next commit.
    #[must_use]
    pub fn file(mut self, path: &str, content: impl AsRef<[u8]>) -> Self {
        self.steps.push(Step::File(path.to_owned(), content.as_ref().to_vec(), false));
        self
    }

    /// Writes an executable file for the next commit.
    #[must_use]
    pub fn executable(mut self, path: &str, content: impl AsRef<[u8]>) -> Self {
        self.steps.push(Step::File(path.to_owned(), content.as_ref().to_vec(), true));
        self
    }

    /// Writes `content` to `path` in the working tree only, after every
    /// earlier step.
    #[must_use]
    pub fn worktree_file(mut self, path: &str, content: impl AsRef<[u8]>) -> Self {
        self.steps.push(Step::WorktreeOnly(path.to_owned(), content.as_ref().to_vec()));
        self
    }

    /// Removes `path` from the next commit and from the working tree.
    #[must_use]
    pub fn remove(mut self, path: &str) -> Self {
        self.steps.push(Step::Remove(path.to_owned()));
        self
    }

    /// Commits the current files on the branch `HEAD` points to.
    #[must_use]
    pub fn commit(mut self, message: &str) -> Self {
        self.steps.push(Step::Commit(message.to_owned()));
        self
    }

    /// Creates or moves the reference `name` to the object `revision` names.
    #[must_use]
    pub fn reference(mut self, name: &str, revision: &str) -> Self {
        self.steps.push(Step::Reference(name.to_owned(), revision.to_owned()));
        self
    }

    /// Points `HEAD` symbolically at `name`, for example
    /// `refs/heads/feature`.
    #[must_use]
    pub fn head(mut self, name: &str) -> Self {
        self.steps.push(Step::Symbolic("HEAD".to_owned(), name.to_owned()));
        self
    }

    /// Creates the annotated tag `name` on `revision` with `message`.
    #[must_use]
    pub fn annotated_tag(mut self, name: &str, revision: &str, message: &str) -> Self {
        self.steps
            .push(Step::Tag(name.to_owned(), revision.to_owned(), message.to_owned()));
        self
    }

    /// Adds `key = value` to the repository configuration.
    #[must_use]
    pub fn config(mut self, key: &str, value: &str) -> Self {
        self.steps.push(Step::Config(key.to_owned(), value.to_owned()));
        self
    }

    /// Creates the repository.
    ///
    /// # Panics
    ///
    /// Panics when the repository cannot be created.
    #[must_use]
    pub fn build(self) -> Fixture {
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join(if self.bare { "repo.git" } else { "repo" });
        let kind = if self.bare {
            gix::create::Kind::Bare
        } else {
            gix::create::Kind::WithWorktree
        };
        gix::create::into(&path, kind, gix::create::Options::default()).expect("repository created");
        let mut state = BuildState::new(&path, self.bare);
        for step in self.steps {
            state.run(step);
        }
        Fixture { dir, path }
    }
}

struct BuildState {
    root: PathBuf,
    bare: bool,
    files: BTreeMap<String, (Vec<u8>, bool)>,
    commits: i64,
}

impl BuildState {
    fn new(root: &Path, bare: bool) -> Self {
        Self {
            root: root.to_path_buf(),
            bare,
            files: BTreeMap::new(),
            commits: 0,
        }
    }

    fn repo(&self) -> gix::Repository {
        gix::open_opts(&self.root, open_options()).expect("fixture repository opens")
    }

    fn run(&mut self, step: Step) {
        match step {
            Step::File(path, bytes, executable) => {
                self.write_worktree(&path, &bytes, executable);
                self.files.insert(path, (bytes, executable));
            }
            Step::WorktreeOnly(path, bytes) => self.write_worktree(&path, &bytes, false),
            Step::Remove(path) => {
                self.files.remove(&path);
                if !self.bare {
                    std::fs::remove_file(self.root.join(&path)).ok();
                }
            }
            Step::Commit(message) => self.commit(&message),
            Step::Reference(name, revision) => {
                let repo = self.repo();
                let id = repo.rev_parse_single(revision.as_str()).expect("revision").detach();
                repo.reference(
                    name.as_str(),
                    id,
                    gix::refs::transaction::PreviousValue::Any,
                    "fixture",
                )
                .expect("reference created");
            }
            Step::Symbolic(name, target) => {
                let file = self.git_dir().join(name);
                std::fs::write(file, format!("ref: {target}\n")).expect("symbolic reference");
            }
            Step::Tag(name, revision, message) => self.tag(&name, &revision, &message),
            Step::Config(key, value) => self.config(&key, &value),
        }
    }

    fn git_dir(&self) -> PathBuf {
        if self.bare { self.root.clone() } else { self.root.join(".git") }
    }

    fn write_worktree(&self, path: &str, bytes: &[u8], executable: bool) {
        if self.bare {
            return;
        }
        let file = self.root.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).expect("parent directory");
        }
        std::fs::write(&file, bytes).expect("worktree file");
        set_executable(&file, executable);
    }

    fn time(&self) -> String {
        format!("{} +0000", RepoBuilder::EPOCH + self.commits * RepoBuilder::STEP_SECONDS)
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "fixtures are built with the repository library outside any journal"
    )]
    fn commit(&mut self, message: &str) {
        let repo = self.repo();
        let mut editor = repo.empty_tree().edit().expect("tree editor");
        for (path, (bytes, executable)) in &self.files {
            let blob = repo.write_blob(bytes).expect("blob written").detach();
            let kind = if *executable {
                gix::object::tree::EntryKind::BlobExecutable
            } else {
                gix::object::tree::EntryKind::Blob
            };
            editor.upsert(path.as_str(), kind, blob).expect("tree entry");
        }
        let tree = editor.write().expect("tree written").detach();
        let time = self.time();
        let signature = gix::actor::SignatureRef {
            name: RepoBuilder::AUTHOR_NAME.as_bytes().as_bstr(),
            email: RepoBuilder::AUTHOR_EMAIL.as_bytes().as_bstr(),
            time: &time,
        };
        let parents: Vec<gix::ObjectId> = repo.head_id().ok().map(gix::Id::detach).into_iter().collect();
        repo.commit_as(signature, signature, "HEAD", message, tree, parents)
            .expect("commit written");
        self.commits += 1;
        if !self.bare {
            let index = repo.index_from_tree(&tree).expect("index from tree");
            let mut index = index;
            index.write(gix::index::write::Options::default()).expect("index written");
        }
    }

    fn tag(&self, name: &str, revision: &str, message: &str) {
        let repo = self.repo();
        let target = repo.rev_parse_single(revision).expect("revision").detach();
        let kind = repo.find_object(target).expect("object").kind;
        let time = self.time();
        let tagger = gix::actor::SignatureRef {
            name: RepoBuilder::AUTHOR_NAME.as_bytes().as_bstr(),
            email: RepoBuilder::AUTHOR_EMAIL.as_bytes().as_bstr(),
            time: &time,
        };
        repo.tag(
            name,
            target,
            kind,
            Some(tagger),
            message,
            gix::refs::transaction::PreviousValue::MustNotExist,
        )
        .expect("tag created");
    }

    fn config(&self, key: &str, value: &str) {
        let path = self.git_dir().join("config");
        let mut file = gix::config::File::from_path_no_includes(
            path.clone(),
            gix::config::Source::Local,
        )
        .expect("configuration read");
        let (section_key, name) = key.rsplit_once('.').expect("key with a section");
        let (section, subsection) = match section_key.split_once('.') {
            Some((section, subsection)) => (section, Some(subsection.as_bytes().as_bstr())),
            None => (section_key, None),
        };
        let mut section = file
            .section_mut_or_create_new(section, subsection)
            .expect("configuration section");
        section.push(gix::config::parse::section::ValueName::try_from(name).expect("value name"), Some(value.as_bytes().as_bstr()));
        let mut out = Vec::new();
        file.write_to(&mut out).expect("configuration serialized");
        std::fs::write(path, out).expect("configuration written");
    }
}

#[cfg(unix)]
fn set_executable(path: &Path, executable: bool) {
    use std::os::unix::fs::PermissionsExt;
    let mode = if executable { 0o755 } else { 0o644 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("permissions");
}

#[cfg(not(unix))]
fn set_executable(_path: &Path, _executable: bool) {}

#[derive(Debug, Clone)]
enum Saved {
    Absent,
    Entries(BTreeMap<PathBuf, TreeEntry>),
}

#[derive(Debug)]
struct Declared {
    spec: TargetSpec,
    before: Vec<(PathBuf, Saved)>,
    applied: bool,
}

#[derive(Debug, Default)]
struct JournalState {
    next_op: u64,
    committed: Vec<Vec<Declared>>,
    exempt: Vec<ExemptWrite>,
}

/// Journal for tests: transactions write to the real file system, keep the
/// previous content of every declared target, and can be undone after
/// commit.
///
/// A change of a target that was not declared, or a mutation that does not
/// match the declared action, panics.
#[derive(Debug, Clone, Default)]
pub struct RecordingJournal {
    state: Arc<Mutex<JournalState>>,
}

impl RecordingJournal {
    /// Empty journal.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens a transaction.
    #[must_use]
    pub fn begin(&self) -> Box<dyn MutationTx> {
        Box::new(RecordingTx {
            journal: self.clone(),
            targets: Vec::new(),
            changed: false,
            exempt: Vec::new(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, JournalState> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Number of committed operations that are not undone.
    #[must_use]
    pub fn committed(&self) -> usize {
        self.lock().committed.len()
    }

    /// Exempt writes recorded by committed transactions.
    #[must_use]
    pub fn exempt(&self) -> Vec<ExemptWrite> {
        self.lock().exempt.clone()
    }

    /// Paths of the targets of the last committed operation, in
    /// declaration order.
    #[must_use]
    pub fn last_targets(&self) -> Vec<PathBuf> {
        self.lock()
            .committed
            .last()
            .map(|op| op.iter().map(|target| fs_path(&target.spec.path)).collect())
            .unwrap_or_default()
    }

    /// Restores the previous state of every target of the last committed
    /// operation.
    ///
    /// # Panics
    ///
    /// Panics when there is no committed operation or a target cannot be
    /// restored.
    pub fn undo_last(&self) {
        let op = self.lock().committed.pop().expect("a committed operation");
        restore(&op);
    }
}

struct RecordingTx {
    journal: RecordingJournal,
    targets: Vec<Declared>,
    changed: bool,
    exempt: Vec<ExemptWrite>,
}

fn fs_path(path: &TargetPath) -> PathBuf {
    match path {
        TargetPath::Fs(path) => path.as_path().to_path_buf(),
        TargetPath::Provider(_) => panic!("provider targets are not supported by RecordingJournal"),
    }
}

fn save(path: &Path) -> Saved {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Saved::Absent;
    };
    let mut entries = BTreeMap::new();
    if meta.file_type().is_symlink() {
        entries.insert(PathBuf::new(), TreeEntry::Link(std::fs::read_link(path).expect("link")));
    } else if meta.is_dir() {
        entries.insert(PathBuf::new(), TreeEntry::Directory);
        entries.extend(snapshot(path));
    } else {
        entries.insert(
            PathBuf::new(),
            TreeEntry::File {
                bytes: std::fs::read(path).expect("readable file"),
                executable: is_executable(&meta),
            },
        );
    }
    Saved::Entries(entries)
}

fn remove_any(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
            std::fs::remove_dir_all(path).expect("directory removed");
        }
        Ok(_) => std::fs::remove_file(path).expect("file removed"),
        Err(_) => {}
    }
}

fn restore(op: &[Declared]) {
    for target in op.iter().rev().filter(|target| target.applied) {
        for (path, saved) in &target.before {
            remove_any(path);
            let Saved::Entries(entries) = saved else {
                continue;
            };
            for (relative, entry) in entries {
                let full = if relative.as_os_str().is_empty() {
                    path.clone()
                } else {
                    path.join(relative)
                };
                write_entry(&full, entry);
            }
        }
    }
}

fn write_entry(path: &Path, entry: &TreeEntry) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent directory");
    }
    match entry {
        TreeEntry::Directory => std::fs::create_dir_all(path).expect("directory"),
        TreeEntry::File { bytes, executable } => {
            std::fs::write(path, bytes).expect("file restored");
            set_executable(path, *executable);
        }
        TreeEntry::Link(target) => make_link(target, path),
    }
}

#[cfg(unix)]
fn make_link(target: &Path, path: &Path) {
    std::os::unix::fs::symlink(target, path).expect("link restored");
}

#[cfg(not(unix))]
fn make_link(_target: &Path, _path: &Path) {
    panic!("symbolic links are restored only on Unix in tests");
}

fn after_state(path: &Path) -> AfterState {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            let bytes = if meta.is_file() { std::fs::read(path).ok() } else { None };
            AfterState {
                exists: true,
                kind: None,
                size: Some(meta.len()),
                hash: bytes.map(|bytes| ContentHash(Sha256::digest(&bytes).into())),
                mode: None,
            }
        }
        Err(_) => AfterState::default(),
    }
}

fn check_expect(spec: &TargetSpec, path: &Path) -> Result<(), Error> {
    let stale = || Error::new(ErrorCode::Stale, format!("{} changed after it was read", path.display()));
    match &spec.expect {
        Expect::Unchecked => Ok(()),
        Expect::Absent => {
            if std::fs::symlink_metadata(path).is_ok() {
                Err(stale())
            } else {
                Ok(())
            }
        }
        Expect::Hash(expected) => {
            let bytes = std::fs::read(path).map_err(|_| stale())?;
            if ContentHash(Sha256::digest(&bytes).into()) == *expected {
                Ok(())
            } else {
                Err(stale())
            }
        }
        Expect::Etag(etag) => {
            let meta = std::fs::metadata(path).map_err(|_| stale())?;
            if meta.len() == etag.size { Ok(()) } else { Err(stale()) }
        }
    }
}

impl RecordingTx {
    fn target(&mut self, index: TargetIndex) -> &mut Declared {
        let position = usize::try_from(index.0).expect("index fits");
        self.targets
            .get_mut(position)
            .unwrap_or_else(|| panic!("change of an undeclared target {index:?}"))
    }

    fn perform(spec: &TargetSpec, path: &Path, mutation: Mutation) -> Result<(), Error> {
        let io = |error: std::io::Error| crate::repo::error::io(path, &error);
        match (&spec.action, mutation) {
            (
                TargetAction::Create | TargetAction::Modify | TargetAction::Replace,
                Mutation::WriteBytes(bytes),
            ) => {
                if matches!(spec.action, TargetAction::Replace) {
                    remove_any(path);
                }
                std::fs::write(path, bytes).map_err(io)
            }
            (TargetAction::Append, Mutation::AppendBytes(bytes)) => {
                use std::io::Write as _;
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .and_then(|mut file| file.write_all(&bytes))
                    .map_err(io)
            }
            (TargetAction::Truncate, Mutation::SetLength(length)) => std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .and_then(|file| file.set_len(length))
                .map_err(io),
            (TargetAction::Delete, Mutation::Remove) => {
                remove_any(path);
                Ok(())
            }
            (TargetAction::Rename { to, replace }, Mutation::Rename) => {
                if !replace && std::fs::symlink_metadata(to.as_path()).is_ok() {
                    return Err(Error::new(ErrorCode::Exists, "rename destination exists"));
                }
                std::fs::rename(path, to.as_path()).map_err(io)
            }
            (TargetAction::Create | TargetAction::CreateTree, Mutation::CreateDir { .. }) => {
                std::fs::create_dir(path).map_err(io)
            }
            (TargetAction::Link | TargetAction::Create, Mutation::CreateLink { target, .. }) => {
                make_link(&target, path);
                Ok(())
            }
            (action, mutation) => {
                panic!("mutation {mutation:?} does not match the declared action {action:?}")
            }
        }
    }
}

impl MutationTx for RecordingTx {
    fn op(&self) -> Option<OpId> {
        None
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> nurt_contracts::error::Result<Vec<Preserved>> {
        assert!(!self.changed, "preserve after the first change");
        let mut preserved = Vec::new();
        for spec in targets {
            let path = fs_path(&spec.path);
            let exists = std::fs::symlink_metadata(&path).is_ok();
            if matches!(spec.action, TargetAction::Create | TargetAction::CreateTree) && exists {
                return Err(Error::new(ErrorCode::Exists, format!("{} exists", path.display())));
            }
            let mut before = vec![(path.clone(), save(&path))];
            if let TargetAction::Rename { to, .. } = &spec.action {
                before.push((to.as_path().to_path_buf(), save(to.as_path())));
            }
            let index = TargetIndex(u32::try_from(self.targets.len()).expect("index fits"));
            preserved.push(Preserved {
                index,
                volume: None,
                mode: PreserveMode::Copy,
                bytes_held: 0,
                before: BeforeState {
                    exists,
                    ..BeforeState::default()
                },
                irreversible: None,
                notable: Vec::new(),
            });
            self.targets.push(Declared {
                spec: spec.clone(),
                before,
                applied: false,
            });
        }
        Ok(preserved)
    }

    fn stage(&mut self, _target: TargetIndex) -> nurt_contracts::error::Result<Box<dyn WriteHandle>> {
        Err(Error::new(
            ErrorCode::StageFailed,
            "staged writes are not supported by RecordingJournal; write bytes instead",
        ))
    }

    fn apply(&mut self, index: TargetIndex, mutation: Mutation) -> nurt_contracts::error::Result<Applied> {
        self.changed = true;
        let target = self.target(index);
        let path = fs_path(&target.spec.path);
        target.applied = true;
        let spec = target.spec.clone();
        check_expect(&spec, &path)?;
        Self::perform(&spec, &path, mutation)?;
        let observed = match &spec.action {
            TargetAction::Rename { to, .. } => to.as_path().to_path_buf(),
            _ => path,
        };
        Ok(Applied {
            index,
            after: after_state(&observed),
        })
    }

    fn observe_external(&mut self, changes: &[ExternalChange]) -> nurt_contracts::error::Result<Vec<Applied>> {
        self.changed = true;
        let mut applied = Vec::new();
        for change in changes {
            let index = change
                .declared
                .unwrap_or_else(|| panic!("undeclared external change of {}", change.path.as_path().display()));
            let target = self.target(index);
            assert!(
                matches!(target.spec.action, TargetAction::ExternalWrite),
                "external change of a target not declared as an external write"
            );
            target.applied = true;
            applied.push(Applied {
                index,
                after: after_state(change.path.as_path()),
            });
        }
        Ok(applied)
    }

    fn mark_irreversible(&mut self, _marker: IrreversibleEntry) {}

    fn record_exempt(&mut self, write: ExemptWrite) {
        self.exempt.push(write);
    }

    fn commit(self: Box<Self>) -> nurt_contracts::error::Result<OpHandle> {
        let mut state = self.journal.lock();
        state.next_op += 1;
        let op = OpId(state.next_op);
        state.exempt.extend(self.exempt.iter().cloned());
        let outcomes = self
            .targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.applied)
            .map(|(position, target)| TargetOutcome {
                index: TargetIndex(u32::try_from(position).expect("index fits")),
                path: target.spec.path.clone(),
                action: target.spec.action.clone(),
                mode: PreserveMode::Copy,
                bytes_held: 0,
                before: BeforeState::default(),
                after: None,
                irreversible: None,
                notable: Vec::new(),
            })
            .collect();
        state.committed.push(self.targets);
        Ok(OpHandle {
            op: Some(op),
            kind: OpKind::command("test"),
            bulk: false,
            targets: outcomes,
            irreversible: Vec::new(),
            notable: Vec::new(),
            exempt: self.exempt,
        })
    }

    fn rollback(self: Box<Self>, _reason: RollbackReason) -> nurt_contracts::error::Result<RollbackReport> {
        restore(&self.targets);
        Ok(RollbackReport {
            op: None,
            restored: self
                .targets
                .iter()
                .filter(|target| target.applied)
                .map(|target| target.spec.path.clone())
                .collect(),
            failed: Vec::new(),
        })
    }
}

/// Declaration of a file target with the given action and no expectation.
///
/// # Panics
///
/// Panics when `path` is not absolute.
#[must_use]
pub fn target(path: &Path, action: TargetAction) -> TargetSpec {
    TargetSpec {
        path: TargetPath::Fs(AbsPath::new(path).expect("absolute path")),
        action,
        expect: Expect::Unchecked,
        size_hint: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_creates_commits_with_fixed_identity_and_time() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "1\n")
            .commit("one")
            .file("b/c.txt", "2\n")
            .commit("two")
            .build();
        let repo = fixture.open();
        let head = repo.head_commit().unwrap();
        assert_eq!(head.time().unwrap().seconds, RepoBuilder::EPOCH + RepoBuilder::STEP_SECONDS);
        assert_eq!(head.parent_ids().count(), 1);
        assert!(fixture.path().join("b/c.txt").is_file());
        let index = repo.open_index().unwrap();
        assert_eq!(index.entries().len(), 2);
    }

    #[test]
    fn identical_builders_produce_identical_commits() {
        let build = || RepoBuilder::new().file("a.txt", "x\n").commit("same").build();
        let (first, second) = (build(), build());
        assert_eq!(
            first.open().head_id().unwrap().detach(),
            second.open().head_id().unwrap().detach()
        );
    }

    #[test]
    fn bare_builder_has_no_working_tree() {
        let fixture = RepoBuilder::bare().file("a.txt", "1\n").commit("one").build();
        assert!(fixture.open().workdir().is_none());
        assert_eq!(fixture.git_dir(), fixture.path());
    }

    #[test]
    fn recording_journal_undoes_writes_byte_for_byte() {
        let fixture = RepoBuilder::new().file("a.txt", "1\n").commit("one").build();
        let before = fixture.snapshot();
        let journal = RecordingJournal::new();
        let mut tx = journal.begin();
        let file = fixture.path().join("a.txt");
        let created = fixture.path().join("new.txt");
        tx.preserve(&[
            target(&file, TargetAction::Modify),
            target(&created, TargetAction::Create),
        ])
        .unwrap();
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"2\n".to_vec())).unwrap();
        tx.apply(TargetIndex(1), Mutation::WriteBytes(b"n\n".to_vec())).unwrap();
        tx.commit().unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"2\n");
        journal.undo_last();
        assert_eq!(fixture.snapshot(), before);
    }

    #[test]
    fn rename_with_replace_is_undone() {
        let fixture = RepoBuilder::new().file("a.txt", "1\n").commit("one").build();
        let before = fixture.snapshot();
        let journal = RecordingJournal::new();
        let mut tx = journal.begin();
        let lock = fixture.path().join("a.txt.lock");
        let file = fixture.path().join("a.txt");
        tx.preserve(&[
            target(&lock, TargetAction::Create),
            target(
                &lock,
                TargetAction::Rename {
                    to: AbsPath::new(&file).unwrap(),
                    replace: true,
                },
            ),
        ])
        .unwrap();
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"new\n".to_vec())).unwrap();
        tx.apply(TargetIndex(1), Mutation::Rename).unwrap();
        tx.commit().unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"new\n");
        assert!(!lock.exists());
        journal.undo_last();
        assert_eq!(fixture.snapshot(), before);
    }

    #[test]
    #[should_panic(expected = "undeclared target")]
    fn change_of_an_undeclared_target_panics() {
        let journal = RecordingJournal::new();
        let mut tx = journal.begin();
        let _ = tx.apply(TargetIndex(0), Mutation::Remove);
    }

    #[test]
    fn create_of_an_existing_target_fails_with_exists() {
        let fixture = RepoBuilder::new().file("a.txt", "1\n").commit("one").build();
        let journal = RecordingJournal::new();
        let mut tx = journal.begin();
        let error = tx
            .preserve(&[target(&fixture.path().join("a.txt"), TargetAction::Create)])
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Exists);
    }

    #[test]
    fn test_host_reports_variables_and_home() {
        let host = TestHost::new("/home/t").with_var("X", "1");
        assert_eq!(host.env_var("X"), Some("1".into()));
        assert_eq!(host.env_var("HOME"), Some("/home/t".into()));
        assert_eq!(host.info().home, PathBuf::from("/home/t"));
    }
}
