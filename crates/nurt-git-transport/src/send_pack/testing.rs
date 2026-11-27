// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Test support: repositories built without the `git` program, a recording
//! journal transaction and a process host for hooks.
//!
//! The module is compiled for tests and with the `testutil` feature only.

// Test support fails loudly on setup errors and on misuse of the journal
// protocol, so the panicking helpers are intended. The process host of the
// tests starts hook scripts as programs, which production code never does.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc,
    clippy::disallowed_methods
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use gix::ObjectId;
use gix::objs::tree::EntryKind;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::{
    AfterState, Applied, BeforeState, ExemptWrite, ExternalChange, Mutation, MutationTx, OpHandle,
    OpId, OpKind, PreserveMode, Preserved, RollbackReason, RollbackReport, TargetAction,
    TargetIndex, TargetPath, TargetSpec,
};
use nurt_contracts::report::IrreversibleEntry;
use nurt_contracts::services::{
    ProcessHost, ProgramInput, ProgramOutcome, ProgramSpec, ServiceChild, ServiceSpawnSpec,
};
use nurt_contracts::stage::CancelToken;

use super::refs::content_hash;

/// Creates a repository at `root` (bare when `bare` is set) and opens it
/// without the configuration of the machine that runs the tests (system and
/// global files, environment), so that a setting such as `core.autocrlf` of
/// Git for Windows does not change what a test sees.
pub fn init_isolated(root: &Path, bare: bool) -> gix::Repository {
    let kind = if bare {
        gix::create::Kind::Bare
    } else {
        gix::create::Kind::WithWorktree
    };
    let repo = gix::ThreadSafeRepository::init_opts(
        root,
        kind,
        gix::create::Options::default(),
        isolated_options(),
    )
    .expect("repository")
    .to_thread_local();
    // `git` writes `\n` line ends on every system; `gix` uses `\r\n` on
    // Windows.
    let config = repo.git_dir().join("config");
    let text = std::fs::read_to_string(&config).expect("configuration is readable");
    std::fs::write(&config, text.replace("\r\n", "\n")).expect("configuration is writable");
    repo
}

/// Opens the repository at `root` as [`init_isolated`] does.
pub fn open_isolated(root: &Path) -> gix::Repository {
    gix::open_opts(root, isolated_options()).expect("open")
}

/// Options of [`open_isolated`].
pub fn isolated_options() -> gix::open::Options {
    gix::open::Options::isolated().with_enforced_trust(gix::sec::Trust::Full)
}
/// Fixed time of fixture commits.
const FIXTURE_SECONDS: i64 = 1_700_000_000;

/// Repository built with `gix` in a directory.
pub struct Fixture {
    _dir: Option<tempfile::TempDir>,
    root: PathBuf,
    git_dir: PathBuf,
}

impl Fixture {
    /// Repository with a working tree in a new temporary directory.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("temporary directory");
        let root = dir.path().join("work");
        let mut fixture = Self::init(&root, false);
        fixture._dir = Some(dir);
        fixture
    }

    /// Bare repository in a new temporary directory.
    pub fn bare() -> Self {
        let dir = tempfile::tempdir().expect("temporary directory");
        let root = dir.path().join("remote.git");
        let mut fixture = Self::init(&root, true);
        fixture._dir = Some(dir);
        fixture
    }

    /// Repository at `root`, created when absent.
    pub fn init(root: &Path, bare: bool) -> Self {
        let repo = init_isolated(root, bare);
        let git_dir = repo.git_dir().to_path_buf();
        std::fs::write(git_dir.join("HEAD"), b"ref: refs/heads/main\n").expect("HEAD");
        Self {
            _dir: None,
            root: root.to_path_buf(),
            git_dir,
        }
    }

    /// Root directory (the working tree or the bare repository).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Repository directory.
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    /// Opened repository.
    pub fn repo(&self) -> gix::Repository {
        open_isolated(&self.root)
    }

    /// Appends configuration text to the repository configuration.
    pub fn config(&self, text: &str) {
        let path = self.git_dir.join("config");
        let mut content = std::fs::read_to_string(&path).expect("config");
        content.push_str(text);
        std::fs::write(path, content).expect("config");
    }

    /// Writes a blob and returns its identifier.
    pub fn write_blob(&self, content: &str) -> ObjectId {
        self.repo()
            .write_blob(content.as_bytes())
            .expect("blob")
            .detach()
    }

    /// Identifier of a blob with `content`.
    pub fn blob_id(&self, content: &str) -> ObjectId {
        gix::objs::compute_hash(
            gix::hash::Kind::Sha1,
            gix::objs::Kind::Blob,
            content.as_bytes(),
        )
        .expect("hash")
    }

    /// Commits `files` with `parents` and points `refs/heads/<branch>` at
    /// the commit.
    pub fn commit(&self, branch: &str, files: &[(&str, &str)], parents: &[ObjectId]) -> ObjectId {
        let repo = self.repo();
        let tree = write_tree(&repo, files);
        let signature = gix::actor::Signature {
            name: "Fixture".into(),
            email: "fixture@example.com".into(),
            time: gix::date::Time::new(FIXTURE_SECONDS + parents.len() as i64, 0),
        };
        let message = format!("commit {} on {branch}\n", files.len() + parents.len());
        let commit = gix::objs::Commit {
            tree,
            parents: parents.iter().copied().collect(),
            author: signature.clone(),
            committer: signature,
            encoding: None,
            message: message.into(),
            extra_headers: Vec::new(),
        };
        let id = repo.write_object(&commit).expect("commit").detach();
        self.set_ref(&format!("refs/heads/{branch}"), id);
        id
    }

    /// Creates the annotated tag `name` pointing at `target`.
    pub fn tag(&self, name: &str, target: ObjectId) -> ObjectId {
        let repo = self.repo();
        let tag = gix::objs::Tag {
            target,
            target_kind: gix::objs::Kind::Commit,
            name: name.into(),
            tagger: Some(gix::actor::Signature {
                name: "Fixture".into(),
                email: "fixture@example.com".into(),
                time: gix::date::Time::new(FIXTURE_SECONDS, 0),
            }),
            message: "tag\n".into(),
            signature: None,
        };
        let id = repo.write_object(&tag).expect("tag").detach();
        self.set_ref(&format!("refs/tags/{name}"), id);
        id
    }

    /// Points the loose reference `name` at `id`.
    pub fn set_ref(&self, name: &str, id: ObjectId) {
        let path = self.git_dir.join(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("directories");
        std::fs::write(path, format!("{id}\n")).expect("reference");
    }

    /// Value of a reference.
    pub fn ref_value(&self, name: &str) -> Option<ObjectId> {
        super::refs::read_ref(&self.git_dir, name.as_bytes()).expect("reference")
    }

    /// Removes a loose object.
    pub fn remove_object(&self, id: &ObjectId) {
        let hex = id.to_hex().to_string();
        let path = self.git_dir.join("objects").join(&hex[..2]).join(&hex[2..]);
        std::fs::remove_file(path).expect("loose object");
    }

    /// Indexes `pack` into the object store and returns the object count.
    pub fn index_pack(&self, pack: &[u8]) -> u32 {
        let directory = self.git_dir.join("objects").join("pack");
        let outcome = gix_pack::Bundle::write_to_directory(
            &mut std::io::BufReader::new(pack),
            Some(&directory),
            &mut gix::progress::Discard,
            &AtomicBool::new(false),
            None::<gix::OdbHandle>,
            gix::hash::Kind::Sha1,
            gix_pack::bundle::write::Options::default(),
        )
        .expect("index pack");
        outcome.index.num_objects
    }

    /// Writes an executable hook script.
    pub fn hook(&self, name: &str, script: &str) {
        let dir = self.git_dir.join("hooks");
        std::fs::create_dir_all(&dir).expect("hooks");
        let path = dir.join(name);
        std::fs::write(&path, script).expect("hook");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("mode");
        }
    }
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

fn write_tree(repo: &gix::Repository, files: &[(&str, &str)]) -> ObjectId {
    enum Node {
        File(ObjectId),
        Dir(BTreeMap<String, Node>),
    }
    fn insert(map: &mut BTreeMap<String, Node>, path: &str, id: ObjectId) {
        match path.split_once('/') {
            Some((dir, rest)) => {
                let entry = map
                    .entry(dir.to_owned())
                    .or_insert_with(|| Node::Dir(BTreeMap::new()));
                if let Node::Dir(children) = entry {
                    insert(children, rest, id);
                }
            }
            None => {
                map.insert(path.to_owned(), Node::File(id));
            }
        }
    }
    fn write(repo: &gix::Repository, map: &BTreeMap<String, Node>) -> ObjectId {
        let mut entries: Vec<gix::objs::tree::Entry> = map
            .iter()
            .map(|(name, node)| match node {
                Node::File(id) => gix::objs::tree::Entry {
                    mode: EntryKind::Blob.into(),
                    filename: name.as_str().into(),
                    oid: *id,
                },
                Node::Dir(children) => gix::objs::tree::Entry {
                    mode: EntryKind::Tree.into(),
                    filename: name.as_str().into(),
                    oid: write(repo, children),
                },
            })
            .collect();
        entries.sort();
        let tree = gix::objs::Tree { entries };
        repo.write_object(&tree).expect("tree").detach()
    }
    let mut root = BTreeMap::new();
    for (path, content) in files {
        let id = repo.write_blob(content.as_bytes()).expect("blob").detach();
        insert(&mut root, path, id);
    }
    write(repo, &root)
}

#[derive(Debug, Clone)]
enum Snapshot {
    Absent,
    File(Vec<u8>),
    Dir,
}

fn snapshot(path: &Path) -> Snapshot {
    if path.is_dir() {
        Snapshot::Dir
    } else {
        std::fs::read(path).map_or(Snapshot::Absent, Snapshot::File)
    }
}

fn restore(path: &Path, snapshot: &Snapshot) {
    match snapshot {
        Snapshot::Absent => {
            if path.is_dir() {
                let _ = std::fs::remove_dir(path);
            } else {
                let _ = std::fs::remove_file(path);
            }
        }
        Snapshot::File(bytes) => {
            std::fs::write(path, bytes).expect("restore file");
        }
        Snapshot::Dir => {
            let _ = std::fs::create_dir(path);
        }
    }
}

/// Journal transaction that performs the changes on disk, enforces the
/// protocol of [`MutationTx`] with panics and undoes the changes on request.
#[derive(Default)]
pub struct RecordingTx {
    targets: Vec<TargetSpec>,
    undo_log: Vec<(PathBuf, Snapshot)>,
    changed: bool,
    /// Writes recorded as exempt from the journal.
    pub exempt: Vec<ExemptWrite>,
    /// Irreversible markers.
    pub irreversible: Vec<IrreversibleEntry>,
}

impl RecordingTx {
    /// A transaction without targets.
    pub fn new() -> Self {
        Self::default()
    }

    /// Declared targets.
    pub fn targets(&self) -> &[TargetSpec] {
        &self.targets
    }

    /// Restores every change in reverse order.
    pub fn undo(&mut self) {
        while let Some((path, before)) = self.undo_log.pop() {
            restore(&path, &before);
        }
    }

    fn path(&self, index: TargetIndex) -> (PathBuf, TargetAction) {
        let target = self
            .targets
            .get(index.0 as usize)
            .unwrap_or_else(|| panic!("undeclared target {}", index.0));
        let TargetPath::Fs(path) = &target.path else {
            panic!("provider targets are not used")
        };
        (path.as_path().to_path_buf(), target.action.clone())
    }

    fn check_expect(&self, index: TargetIndex, path: &Path) -> Result<()> {
        let expect = &self.targets[index.0 as usize].expect;
        let matches = match expect {
            nurt_contracts::journal::Expect::Unchecked
            | nurt_contracts::journal::Expect::Etag(_) => true,
            nurt_contracts::journal::Expect::Absent => !path.exists(),
            nurt_contracts::journal::Expect::Hash(hash) => {
                std::fs::read(path).is_ok_and(|bytes| content_hash(&bytes) == *hash)
            }
        };
        if matches {
            Ok(())
        } else {
            Err(Error::new(
                ErrorCode::Stale,
                format!("{} changed", path.display()),
            ))
        }
    }

    fn record(&mut self, path: &Path) {
        self.undo_log.push((path.to_path_buf(), snapshot(path)));
    }
}

fn io(error: &std::io::Error) -> Error {
    Error::new(ErrorCode::Io, error.to_string())
}

impl MutationTx for RecordingTx {
    fn op(&self) -> Option<OpId> {
        (!self.targets.is_empty()).then_some(OpId(1))
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        assert!(!self.changed, "preserve after the first change");
        let mut preserved = Vec::new();
        for target in targets {
            let TargetPath::Fs(path) = &target.path else {
                panic!("provider targets are not used")
            };
            let path = path.as_path();
            match &target.action {
                TargetAction::Create if path.exists() => {
                    return Err(Error::new(
                        ErrorCode::Exists,
                        format!("{} exists", path.display()),
                    ));
                }
                TargetAction::Delete | TargetAction::Append if !path.exists() => {
                    return Err(Error::new(
                        ErrorCode::NotFound,
                        format!("{} is missing", path.display()),
                    ));
                }
                TargetAction::Rename { .. } if !path.exists() => {
                    return Err(Error::new(
                        ErrorCode::NotFound,
                        format!("{} is missing", path.display()),
                    ));
                }
                _ => {}
            }
            let index = TargetIndex(u32::try_from(self.targets.len()).expect("index"));
            self.targets.push(target.clone());
            preserved.push(Preserved {
                index,
                volume: None,
                mode: PreserveMode::Copy,
                bytes_held: 0,
                before: BeforeState {
                    exists: path.exists(),
                    ..BeforeState::default()
                },
                irreversible: None,
                notable: Vec::new(),
            });
        }
        Ok(preserved)
    }

    fn stage(&mut self, _target: TargetIndex) -> Result<Box<dyn nurt_contracts::fs::WriteHandle>> {
        panic!("staged writes are not used")
    }

    fn apply(&mut self, target: TargetIndex, mutation: Mutation) -> Result<Applied> {
        self.changed = true;
        let (path, action) = self.path(target);
        self.check_expect(target, &path)?;
        match (&action, mutation) {
            (TargetAction::Create, Mutation::CreateDir { .. }) => {
                self.record(&path);
                std::fs::create_dir(&path).map_err(|error| io(&error))?;
            }
            (TargetAction::Create | TargetAction::Modify, Mutation::WriteBytes(bytes)) => {
                self.record(&path);
                std::fs::write(&path, bytes).map_err(|error| io(&error))?;
            }
            (TargetAction::Append, Mutation::AppendBytes(bytes)) => {
                self.record(&path);
                let mut content = std::fs::read(&path).map_err(|error| io(&error))?;
                content.extend_from_slice(&bytes);
                std::fs::write(&path, content).map_err(|error| io(&error))?;
            }
            (TargetAction::Delete, Mutation::Remove) => {
                self.record(&path);
                std::fs::remove_file(&path).map_err(|error| io(&error))?;
            }
            (TargetAction::Rename { to, .. }, Mutation::Rename) => {
                let to = to.as_path().to_path_buf();
                self.record(&to);
                self.record(&path);
                std::fs::rename(&path, &to).map_err(|error| io(&error))?;
            }
            (action, mutation) => panic!("mutation {mutation:?} does not match action {action:?}"),
        }
        Ok(Applied {
            index: target,
            after: AfterState {
                exists: true,
                ..AfterState::default()
            },
        })
    }

    fn observe_external(&mut self, _changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        panic!("external writes are not used")
    }

    fn mark_irreversible(&mut self, marker: IrreversibleEntry) {
        self.irreversible.push(marker);
    }

    fn record_exempt(&mut self, write: ExemptWrite) {
        self.exempt.push(write);
    }

    fn commit(self: Box<Self>) -> Result<OpHandle> {
        Ok(OpHandle {
            op: self.op(),
            kind: OpKind::command("git.push"),
            bulk: self.targets.len() > 1,
            targets: Vec::new(),
            irreversible: self.irreversible.clone(),
            notable: Vec::new(),
            exempt: self.exempt.clone(),
        })
    }

    fn rollback(mut self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
        self.undo();
        Ok(RollbackReport {
            op: None,
            restored: Vec::new(),
            failed: Vec::new(),
        })
    }
}

/// Process host that runs hook scripts with `sh` and records the calls.
#[derive(Default)]
pub struct ScriptHost {
    /// Programs run, with their arguments.
    pub calls: Mutex<Vec<(String, Vec<String>)>>,
}

impl ProcessHost for ScriptHost {
    /// The file of `program` on the path, as a real host resolves it, or the
    /// name itself when no file is found.
    fn resolve_program(&self, program: &str, _cwd: &Path) -> Result<PathBuf> {
        let named = Path::new(program);
        if named.components().count() > 1 {
            return Ok(named.to_path_buf());
        }
        let suffixes: &[&str] = if cfg!(windows) { &[".exe", ""] } else { &[""] };
        let found = std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths).find_map(|dir| {
                suffixes
                    .iter()
                    .map(|suffix| dir.join(format!("{program}{suffix}")))
                    .find(|path| path.is_file())
            })
        });
        Ok(found.unwrap_or_else(|| PathBuf::from(program)))
    }

    fn run_program(&self, spec: &ProgramSpec, _cancel: &CancelToken) -> Result<ProgramOutcome> {
        use std::io::Write;
        self.calls
            .lock()
            .expect("calls")
            .push((spec.program.clone(), spec.args.clone()));
        let mut command = std::process::Command::new(&spec.program);
        command
            .args(&spec.args)
            .current_dir(&spec.cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        for variable in &spec.env {
            match &variable.value {
                Some(value) => command.env(&variable.name, value),
                None => command.env_remove(&variable.name),
            };
        }
        let mut child = command.spawn().map_err(|error| io(&error))?;
        if let (Some(mut stdin), ProgramInput::Bytes(bytes)) = (child.stdin.take(), &spec.stdin) {
            let _ = stdin.write_all(bytes);
        }
        let output = child.wait_with_output().map_err(|error| io(&error))?;
        Ok(ProgramOutcome {
            program_path: PathBuf::from(&spec.program),
            exit_code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
            ..ProgramOutcome::default()
        })
    }

    fn spawn_service(&self, _spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
        panic!("services are not used")
    }
}
