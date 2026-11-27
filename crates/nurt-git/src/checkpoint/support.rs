// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Repository access, identity, content filters, trees and journaled writes
//! shared by the checkpoint and working tree commands.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Display;
use std::path::{Component, Path, PathBuf};

use gix::bstr::{BStr, BString, ByteSlice};
use gix::hash::ObjectId;
use gix::objs::tree::EntryKind;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, ContentHash};
use nurt_contracts::journal::{
    Expect, Mutation, MutationTx, TargetAction, TargetIndex, TargetPath, TargetSpec,
};
use nurt_contracts::stage::ExecCtx;
use sha2::{Digest, Sha256};

use crate::commit::identity::sanitize as sanitize_identity;
use crate::refs_tx::normalize_reflog_message;

/// Permission bits of directories created inside the repository.
pub(crate) const DIR_MODE: u32 = 0o755;

/// Name recorded when neither the environment nor the configuration names
/// the committer.
const FALLBACK_NAME: &str = "nurt";

/// E-mail recorded when neither the environment nor the configuration gives
/// one.
const FALLBACK_EMAIL: &str = "nurt@localhost";

/// Suffix of the lock files of `git`.
const LOCK_SUFFIX: &str = ".lock";

/// Builds an `E_IO` error for a failure of the repository library.
pub(crate) fn repo_error(what: &str, err: impl Display) -> Error {
    Error::new(ErrorCode::Io, format!("cannot {what}: {err}"))
}

/// Builds an error with a problem code.
pub(crate) fn problem(code: ErrorCode, problem: &str, message: impl Into<String>) -> Error {
    Error::new(code, message).with_problem(problem)
}

/// Quotes a value as a text literal of the language.
pub(crate) fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        if ch == '"' || ch == '\\' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// Opens the repository that contains `path`, searching upwards.
///
/// Only the configuration of the repository itself is read and no program is
/// started; the environment of the process is not consulted.
pub(crate) fn open_repo(path: &Path) -> Result<gix::Repository> {
    if !path.exists() {
        return Err(Error::new(
            ErrorCode::NotFound,
            format!(
                "`{}` does not exist; pass an existing directory",
                path.display()
            ),
        ));
    }
    let mut current = Some(path);
    while let Some(dir) = current {
        if dir.join(".git").exists() {
            let options = gix::open::Options::isolated();
            return gix::open_opts(dir, options).map_err(|err| {
                repo_error(&format!("open the repository at `{}`", dir.display()), err)
            });
        }
        current = dir.parent();
    }
    Err(Error::new(
        ErrorCode::GitNotRepository,
        format!(
            "`{}` is not inside a git repository; pass a path inside a repository",
            path.display()
        ),
    ))
}

/// Root of the working tree; fails for a bare repository.
pub(crate) fn workdir(repo: &gix::Repository) -> Result<PathBuf> {
    repo.workdir().map(Path::to_path_buf).ok_or_else(|| {
        problem(
            ErrorCode::GitUnsupported,
            "bare_repository",
            format!(
                "`{}` has no working tree; pass a repository with a working tree",
                repo.git_dir().display()
            ),
        )
    })
}

/// Converts a native path to a journal path.
pub(crate) fn abs(path: &Path) -> Result<AbsPath> {
    AbsPath::new(path).map_err(|err| {
        Error::new(
            ErrorCode::Io,
            format!(
                "`{}` is not a usable absolute path: {err:?}",
                path.display()
            ),
        )
    })
}

/// Text of a path for records and messages, with `/` separators.
pub(crate) fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text.into_owned()
    }
}

/// Native location of the repository path `rela` below `root`.
pub(crate) fn worktree_path(root: &Path, rela: &BStr) -> Result<PathBuf> {
    gix::path::from_bstr(rela)
        .map(|path| root.join(path))
        .map_err(|err| repo_error(&format!("convert the path `{rela}`"), err))
}

/// Repository form of a native relative path, with `/` separators.
pub(crate) fn repo_path(rela: &Path) -> Result<BString> {
    gix::path::into_bstr(rela)
        .map(|bytes| gix::path::to_unix_separators_on_windows(bytes).into_owned())
        .map_err(|err| repo_error(&format!("convert the path `{}`", rela.display()), err))
}

/// Converts a path given by the user into a path relative to the root of the
/// working tree.
///
/// Relative paths are taken relative to the root; absolute paths must lie
/// inside it. `.` names the whole working tree and yields an empty path.
pub(crate) fn repo_relative(root: &Path, input: &str) -> Result<BString> {
    let given = Path::new(input);
    let relative = if given.is_absolute() {
        let normalized = abs(given)?;
        let root = abs(root)?;
        normalized
            .as_path()
            .strip_prefix(root.as_path())
            .map(Path::to_path_buf)
            .map_err(|_| outside_repo(input))?
    } else {
        given.to_path_buf()
    };
    let mut parts: Vec<String> = Vec::new();
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(name) => {
                let name = name.to_string_lossy().into_owned();
                if name.eq_ignore_ascii_case(".git") {
                    return Err(problem(
                        ErrorCode::Arg,
                        "git_directory",
                        format!(
                            "`{input}` names the repository directory; pass a path in the working tree"
                        ),
                    ));
                }
                parts.push(name);
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(outside_repo(input));
            }
        }
    }
    Ok(BString::from(parts.join("/")))
}

fn outside_repo(input: &str) -> Error {
    problem(
        ErrorCode::GitNotRepository,
        "path_outside_repo",
        format!("`{input}` is outside the repository; pass a path inside the working tree"),
    )
}

/// Whether `path` is selected by `scopes`; an empty list or an empty scope
/// selects everything.
pub(crate) fn in_scope(path: &BStr, scopes: &[BString]) -> bool {
    scopes.is_empty()
        || scopes.iter().any(|scope| {
            scope.is_empty()
                || path == scope.as_bstr()
                || (path.starts_with(scope.as_slice()) && path.get(scope.len()) == Some(&b'/'))
        })
}

/// Whether a path component of a tree must never be written to the working
/// tree: an empty name, `.`, `..`, and every name that the NTFS or the HFS+
/// file system resolves to the repository directory (trailing dots and
/// spaces are ignored there, and `git~1` is its short name).
fn unsafe_component(name: &[u8]) -> bool {
    let trimmed = name.trim_end_with(|ch| ch == '.' || ch == ' ');
    trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case(b".git")
        || trimmed.eq_ignore_ascii_case(b"git~1")
        || (cfg!(windows) && name.iter().any(|byte| matches!(byte, b'\\' | b':')))
}

/// Refuses paths of a tree that cannot be written below the working tree
/// root: a component that is empty, `.`, `..` or names the repository
/// directory, and a path whose parent directory is also an entry.
///
/// # Errors
///
/// `E_GIT_UNSUPPORTED` with the problem `unsafe_path`.
pub(crate) fn verify_checkout_paths(entries: &BTreeMap<BString, TreeEntry>) -> Result<()> {
    for path in entries.keys() {
        let unsafe_name = path.split(|byte| *byte == b'/').any(unsafe_component);
        let clash = path
            .iter()
            .enumerate()
            .any(|(at, byte)| *byte == b'/' && entries.contains_key(path[..at].as_bstr()));
        if unsafe_name || clash {
            return Err(problem(
                ErrorCode::GitUnsupported,
                "unsafe_path",
                format!(
                    "the tree contains the path `{path}` that cannot be written to the working tree; remove the reference that holds the tree"
                ),
            )
            .with_found(path.to_string()));
        }
    }
    Ok(())
}

/// Committer identity of new objects and reflog entries.
///
/// The name, e-mail and date come from `GIT_COMMITTER_NAME`,
/// `GIT_COMMITTER_EMAIL` and `GIT_COMMITTER_DATE` of the execution context,
/// then from `user.name` and `user.email` of the repository and of the
/// configuration file in the home directory. The time comes from the clock of
/// the context, with the UTC offset of the host.
pub(crate) fn committer(repo: &gix::Repository, ctx: &ExecCtx) -> gix::actor::Signature {
    let global = global_config(ctx);
    let lookup = |env: &str, key: &str, fallback: &str| -> BString {
        let found = match ctx.host().env_var(env).filter(|value| !value.is_empty()) {
            Some(value) => Some(value.to_string_lossy().into_owned()),
            None => repo
                .config_snapshot()
                .string(key)
                .or_else(|| global.as_ref().and_then(|file| file.string(key)))
                .filter(|value| !value.is_empty())
                .map(|value| value.to_str_lossy().into_owned()),
        };
        let clean = found
            .map(|text| sanitize_identity(&text))
            .filter(|text| !text.is_empty());
        BString::from(clean.unwrap_or_else(|| fallback.to_owned()))
    };
    gix::actor::Signature {
        name: lookup("GIT_COMMITTER_NAME", "user.name", FALLBACK_NAME),
        email: lookup("GIT_COMMITTER_EMAIL", "user.email", FALLBACK_EMAIL),
        time: commit_time(ctx),
    }
}

fn global_config(ctx: &ExecCtx) -> Option<gix::config::File> {
    let path = ctx.host().info().home.join(".gitconfig");
    if !path.is_file() {
        return None;
    }
    gix::config::File::from_path_no_includes(path, gix::config::Source::User).ok()
}

fn commit_time(ctx: &ExecCtx) -> gix::date::Time {
    if let Some(parsed) = ctx
        .host()
        .env_var("GIT_COMMITTER_DATE")
        .and_then(|value| gix::date::parse(&value.to_string_lossy(), None).ok())
    {
        return parsed;
    }
    const NANOSECONDS_PER_SECOND: i128 = 1_000_000_000;
    let seconds =
        i64::try_from(ctx.clock().now().unix_nanos() / NANOSECONDS_PER_SECOND).unwrap_or(0);
    let offset = jiff::tz::TimeZone::get(&ctx.host().info().timezone)
        .ok()
        .zip(jiff::Timestamp::from_second(seconds).ok())
        .map_or(0, |(zone, stamp)| zone.to_offset(stamp).seconds());
    gix::date::Time { seconds, offset }
}

/// Formats one reflog line in the format of `git`.
pub(crate) fn reflog_line(
    old: ObjectId,
    new: ObjectId,
    who: &gix::actor::Signature,
    message: &str,
) -> Vec<u8> {
    let mut line = Vec::new();
    line.extend_from_slice(old.to_hex().to_string().as_bytes());
    line.push(b' ');
    line.extend_from_slice(new.to_hex().to_string().as_bytes());
    line.push(b' ');
    line.extend_from_slice(&who.name);
    line.extend_from_slice(b" <");
    line.extend_from_slice(&who.email);
    line.extend_from_slice(b"> ");
    line.extend_from_slice(format_time(who.time).as_bytes());
    let message = normalize_reflog_message(message.as_bytes().as_bstr());
    if !message.is_empty() {
        line.push(b'\t');
        line.extend_from_slice(&message);
    }
    line.push(b'\n');
    line
}

fn format_time(time: gix::date::Time) -> String {
    const SECONDS_PER_MINUTE: i32 = 60;
    const MINUTES_PER_HOUR: i32 = 60;
    let sign = if time.offset < 0 { '-' } else { '+' };
    let minutes = time.offset.abs() / SECONDS_PER_MINUTE;
    format!(
        "{} {sign}{:02}{:02}",
        time.seconds,
        minutes / MINUTES_PER_HOUR,
        minutes % MINUTES_PER_HOUR
    )
}

/// Conversion of file content between the working tree and the object
/// database with the built-in filters of `git` (line endings, `ident`,
/// working tree encoding). Content filter drivers configured by the
/// repository are not run.
pub(crate) struct Filters<'repo> {
    repo: &'repo gix::Repository,
    pipeline: gix::filter::plumbing::Pipeline,
    attributes: gix::AttributeStack<'repo>,
    drivers_skipped: bool,
}

impl<'repo> Filters<'repo> {
    /// Prepares the filters for `repo` with attributes from the working tree
    /// and from `index`.
    pub(crate) fn new(repo: &'repo gix::Repository, index: &gix::index::State) -> Result<Self> {
        Self::with_source(
            repo,
            index,
            gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping,
        )
    }

    /// Prepares the filters for `repo` with attributes read from `source`;
    /// a checkout into an empty directory reads them from `index` only.
    pub(crate) fn with_source(
        repo: &'repo gix::Repository,
        index: &gix::index::State,
        source: gix::worktree::stack::state::attributes::Source,
    ) -> Result<Self> {
        let mut options = gix::filter::Pipeline::options(repo)
            .map_err(|err| repo_error("read the filter configuration", err))?;
        let drivers_skipped = !options.drivers.is_empty();
        options.drivers.clear();
        let pipeline = gix::filter::plumbing::Pipeline::new(
            gix::command::Context::default(),
            repo.object_hash(),
            options,
        );
        let attributes = repo
            .attributes_only(index, source)
            .map_err(|err| repo_error("read the attributes", err))?;
        Ok(Self {
            repo,
            pipeline,
            attributes,
            drivers_skipped,
        })
    }

    /// Whether the repository configures filter drivers that are not run.
    pub(crate) fn drivers_skipped(&self) -> bool {
        self.drivers_skipped
    }

    /// Converts working tree content of `rela_path` to its form in the object
    /// database.
    pub(crate) fn convert_to_git(
        &mut self,
        rela_path: &BStr,
        content: &[u8],
        index: &gix::index::State,
    ) -> Result<Vec<u8>> {
        use gix::filter::plumbing::pipeline::convert::ToGitOutcome;
        let path = gix::path::from_bstr(rela_path)
            .map_err(|err| repo_error(&format!("convert the path `{rela_path}`"), err))?
            .into_owned();
        let platform = self
            .attributes
            .at_entry(rela_path, None)
            .map_err(|err| repo_error("read the attributes", err))?;
        let mut matcher = |_: &BStr, out: &mut gix::attrs::search::Outcome| {
            platform.matching_attributes(out);
        };
        let objects = &self.repo.objects;
        let mut index_object = |buf: &mut Vec<u8>| -> gix::Result<Option<()>> {
            let Some(entry) = index.entry_by_path(rela_path) else {
                return Ok(None);
            };
            use gix::objs::Find;
            let found = objects.try_find(&entry.id, buf)?;
            Ok(found
                .filter(|object| object.kind == gix::object::Kind::Blob)
                .map(|_| ()))
        };
        let outcome = self
            .pipeline
            .convert_to_git(content, &path, &mut matcher, &mut index_object)
            .map_err(|err| repo_error(&format!("convert `{rela_path}`"), err))?;
        let mut out = Vec::new();
        match outcome {
            ToGitOutcome::Unchanged(bytes) => out.extend_from_slice(bytes),
            ToGitOutcome::Buffer(bytes) => out.extend_from_slice(bytes),
            ToGitOutcome::Process(mut reader) => {
                std::io::Read::read_to_end(&mut reader, &mut out)
                    .map_err(|err| repo_error(&format!("convert `{rela_path}`"), err))?;
            }
        }
        Ok(out)
    }

    /// Converts content of `rela_path` from the object database to its form
    /// in the working tree.
    pub(crate) fn convert_to_worktree(
        &mut self,
        rela_path: &BStr,
        content: &[u8],
    ) -> Result<Vec<u8>> {
        use gix::filter::plumbing::pipeline::convert::{ToWorktreeOutcome, to_worktree};
        let platform = self
            .attributes
            .at_entry(rela_path, None)
            .map_err(|err| repo_error("read the attributes", err))?;
        let mut matcher = |_: &BStr, out: &mut gix::attrs::search::Outcome| {
            platform.matching_attributes(out);
        };
        let options = to_worktree::Options {
            can_delay: gix::filter::plumbing::driver::apply::Delay::Forbid,
            unknown_encoding: to_worktree::UnknownEncoding::default(),
        };
        let outcome = self
            .pipeline
            .convert_to_worktree(content, rela_path, &mut matcher, options)
            .map_err(|err| repo_error(&format!("convert `{rela_path}`"), err))?;
        match outcome {
            ToWorktreeOutcome::Unchanged(bytes) | ToWorktreeOutcome::Buffer(bytes) => {
                Ok(bytes.to_vec())
            }
            ToWorktreeOutcome::Process(_) => Err(problem(
                ErrorCode::GitUnsupported,
                "filter_driver",
                format!("`{rela_path}` needs a filter program, which is not run"),
            )),
        }
    }
}

/// Entry of a flattened tree: kind and object of one path.
pub(crate) type TreeEntry = (EntryKind, ObjectId);

/// Lists every non-tree entry of `tree`, by path.
pub(crate) fn flatten_tree(
    repo: &gix::Repository,
    tree: ObjectId,
) -> Result<BTreeMap<BString, TreeEntry>> {
    let mut out = BTreeMap::new();
    let mut pending = vec![(BString::default(), tree)];
    while let Some((prefix, id)) = pending.pop() {
        let tree = repo
            .find_tree(id)
            .map_err(|err| repo_error(&format!("read tree {id}"), err))?;
        let decoded = tree
            .decode()
            .map_err(|err| repo_error(&format!("read tree {id}"), err))?;
        for entry in &decoded.entries {
            let mut path = prefix.clone();
            if !path.is_empty() {
                path.push(b'/');
            }
            path.extend_from_slice(entry.filename);
            let kind = entry.mode.kind();
            if kind == EntryKind::Tree {
                pending.push((path, entry.oid.to_owned()));
            } else {
                out.insert(path, (kind, entry.oid.to_owned()));
            }
        }
    }
    Ok(out)
}

/// Writes a tree with the given entries and returns its identifier.
pub(crate) fn write_tree(
    repo: &gix::Repository,
    entries: &BTreeMap<BString, TreeEntry>,
) -> Result<ObjectId> {
    let mut editor = repo
        .edit_tree(ObjectId::empty_tree(repo.object_hash()))
        .map_err(|err| repo_error("prepare a tree", err))?;
    for (path, (kind, id)) in entries {
        editor
            .upsert(path.as_bstr(), *kind, *id)
            .map_err(|err| repo_error(&format!("add `{path}` to a tree"), err))?;
    }
    let tree = editor
        .write()
        .map_err(|err| repo_error("write a tree", err))?;
    Ok(tree.detach())
}

/// SHA-256 of content, as used by the journal to check expected state.
pub(crate) fn content_hash(bytes: &[u8]) -> ContentHash {
    let digest = Sha256::digest(bytes);
    let mut out = [0_u8; 32];
    out.copy_from_slice(&digest);
    ContentHash(out)
}

/// Expected state of a file from its current content.
pub(crate) fn expect_current(path: &Path) -> Result<Expect> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Expect::Hash(content_hash(&bytes))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Expect::Absent),
        Err(err) => Err(repo_error(&format!("read `{}`", path.display()), err)),
    }
}

/// Lock file of `path` as `git` names it.
pub(crate) fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(LOCK_SUFFIX);
    PathBuf::from(name)
}

/// Fails with `E_GIT_LOCKED` when another process holds the lock of `path`.
pub(crate) fn ensure_unlocked(path: &Path) -> Result<()> {
    let lock = lock_path(path);
    if lock.exists() {
        return Err(Error::new(
            ErrorCode::GitLocked,
            format!(
                "`{}` exists: another git process is changing `{}`; retry after it finishes or remove the stale lock file",
                lock.display(),
                path.display()
            ),
        )
        .with_fix(format!("remove {}", quote(&display_path(&lock)))));
    }
    Ok(())
}

enum Step {
    Write(Vec<u8>),
    CreateDir,
    ReplaceDir,
    Remove,
}

/// Changes of one operation, declared in full before the first change and
/// applied in declaration order through the journal.
#[derive(Default)]
pub(crate) struct TxPlan {
    specs: Vec<TargetSpec>,
    steps: Vec<Step>,
    guarded: HashMap<usize, PathBuf>,
    created_dirs: BTreeSet<PathBuf>,
    removed: BTreeSet<PathBuf>,
    next: usize,
}

impl TxPlan {
    fn push(
        &mut self,
        path: &Path,
        action: TargetAction,
        expect: Expect,
        step: Step,
    ) -> Result<usize> {
        self.specs.push(TargetSpec {
            path: TargetPath::Fs(abs(path)?),
            action,
            expect,
            size_hint: None,
        });
        self.steps.push(step);
        Ok(self.steps.len() - 1)
    }

    /// Declares the creation of every missing directory from `dir` upwards.
    /// Entries removed earlier in the same plan count as missing.
    pub(crate) fn create_dirs(&mut self, dir: &Path) -> Result<()> {
        let mut missing = Vec::new();
        let mut current = Some(dir);
        while let Some(path) = current {
            let gone = self.removed.iter().any(|removed| path.starts_with(removed));
            if (path.exists() && !gone) || self.created_dirs.contains(path) {
                break;
            }
            missing.push(path.to_path_buf());
            current = path.parent();
        }
        for path in missing.into_iter().rev() {
            self.push(&path, TargetAction::Create, Expect::Absent, Step::CreateDir)?;
            self.created_dirs.insert(path);
        }
        Ok(())
    }

    /// Declares a new directory whose content is written by the operation.
    pub(crate) fn create_tree(&mut self, dir: &Path) -> Result<usize> {
        if let Some(parent) = dir.parent() {
            self.create_dirs(parent)?;
        }
        let index = self.push(
            dir,
            TargetAction::CreateTree,
            Expect::Absent,
            Step::CreateDir,
        )?;
        self.created_dirs.insert(dir.to_path_buf());
        Ok(index)
    }

    /// Declares a new directory populated by the operation in place of an
    /// existing empty directory; the previous directory is preserved.
    pub(crate) fn replace_with_tree(&mut self, dir: &Path) -> Result<usize> {
        self.removed.insert(dir.to_path_buf());
        let index = self.push(
            dir,
            TargetAction::Replace,
            Expect::Unchecked,
            Step::ReplaceDir,
        )?;
        self.created_dirs.insert(dir.to_path_buf());
        Ok(index)
    }

    /// Declares a new file with `content`.
    pub(crate) fn create_file(&mut self, path: &Path, content: Vec<u8>) -> Result<usize> {
        if let Some(parent) = path.parent() {
            self.create_dirs(parent)?;
        }
        self.push(
            path,
            TargetAction::Create,
            Expect::Absent,
            Step::Write(content),
        )
    }

    /// Declares a new content of an existing or new file of the working tree.
    pub(crate) fn write_file(
        &mut self,
        path: &Path,
        content: Vec<u8>,
        expect: Expect,
    ) -> Result<usize> {
        if let Some(parent) = path.parent() {
            self.create_dirs(parent)?;
        }
        self.push(path, TargetAction::Modify, expect, Step::Write(content))
    }

    /// Declares the deletion of a file or a directory tree.
    pub(crate) fn delete(&mut self, path: &Path, expect: Expect) -> Result<usize> {
        self.removed.insert(path.to_path_buf());
        self.push(path, TargetAction::Delete, expect, Step::Remove)
    }

    /// Declares a write of `file` that honours the lock protocol of `git`:
    /// the target is `file` itself, expecting `expected` (`None` means
    /// absent), and the lock file `<file>.lock` is checked when the target is
    /// declared and again before the write.
    pub(crate) fn write_locked(
        &mut self,
        file: &Path,
        content: Vec<u8>,
        expected: Option<Vec<u8>>,
    ) -> Result<()> {
        ensure_unlocked(file)?;
        if let Some(parent) = file.parent() {
            self.create_dirs(parent)?;
        }
        let expect = expected
            .as_deref()
            .map_or(Expect::Absent, |old| Expect::Hash(content_hash(old)));
        let index = self.push(file, TargetAction::Modify, expect, Step::Write(content))?;
        self.guarded.insert(index, file.to_path_buf());
        Ok(())
    }

    /// Replaces the content of a declared write before it is applied.
    pub(crate) fn set_content(&mut self, index: usize, content: Vec<u8>) {
        if let Some(Step::Write(bytes)) = self.steps.get_mut(index) {
            *bytes = content;
        }
    }

    /// Preserves the previous state of every declared target.
    pub(crate) fn preserve(&self, tx: &mut dyn MutationTx) -> Result<()> {
        if self.specs.is_empty() {
            return Ok(());
        }
        tx.preserve(&self.specs).map(|_| ())
    }

    /// Applies declared changes up to, but excluding, target `end`.
    pub(crate) fn apply_until(&mut self, tx: &mut dyn MutationTx, end: usize) -> Result<()> {
        while self.next < end.min(self.steps.len()) {
            let index = self.next;
            self.next += 1;
            let target = TargetIndex(u32::try_from(index).unwrap_or(u32::MAX));
            if matches!(self.steps[index], Step::ReplaceDir) {
                tx.apply(target, Mutation::Remove)?;
                tx.apply(target, Mutation::CreateDir { mode: DIR_MODE })?;
                continue;
            }
            let mutation = match &mut self.steps[index] {
                Step::Write(bytes) => Mutation::WriteBytes(std::mem::take(bytes)),
                Step::CreateDir | Step::ReplaceDir => Mutation::CreateDir { mode: DIR_MODE },
                Step::Remove => Mutation::Remove,
            };
            if let Some(file) = self.guarded.get(&index) {
                ensure_unlocked(file)?;
            }
            tx.apply(target, mutation)?;
        }
        Ok(())
    }

    /// Applies all remaining declared changes.
    pub(crate) fn apply_rest(&mut self, tx: &mut dyn MutationTx) -> Result<()> {
        self.apply_until(tx, self.steps.len())
    }
}

/// Location of a reference file in the common directory of the repository.
pub(crate) fn ref_file(repo: &gix::Repository, name: &str) -> PathBuf {
    let mut path = repo.common_dir().to_path_buf();
    for part in name.split('/') {
        path.push(part);
    }
    path
}

/// Content of a loose reference file pointing at `id`.
pub(crate) fn ref_content(id: ObjectId) -> Vec<u8> {
    format!("{id}\n").into_bytes()
}

/// Whether `name` is listed in the `packed-refs` file of the repository.
pub(crate) fn packed_ref_line(repo: &gix::Repository, name: &str) -> Result<Option<String>> {
    let path = repo.common_dir().join("packed-refs");
    let content = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(repo_error("read `packed-refs`", err)),
    };
    Ok(content
        .lines()
        .map(|line| line.to_str_lossy().into_owned())
        .find(|line| {
            line.split_once(' ')
                .is_some_and(|(_, refname)| refname == name)
        }))
}

/// Declares the deletion of reference `name`: its loose file, its line in
/// `packed-refs` and its reflog.
pub(crate) fn delete_ref(plan: &mut TxPlan, repo: &gix::Repository, name: &str) -> Result<()> {
    let loose = ref_file(repo, name);
    ensure_unlocked(&loose)?;
    if loose.is_file() {
        let expect = expect_current(&loose)?;
        plan.delete(&loose, expect)?;
    }
    if packed_ref_line(repo, name)?.is_some() {
        let packed = repo.common_dir().join("packed-refs");
        let old = std::fs::read(&packed).map_err(|err| repo_error("read `packed-refs`", err))?;
        let new = remove_packed(&old, name);
        plan.write_locked(&packed, new, Some(old))?;
    }
    let log = repo.common_dir().join("logs").join(Path::new(name));
    if log.is_file() {
        let expect = expect_current(&log)?;
        plan.delete(&log, expect)?;
    }
    Ok(())
}

fn remove_packed(content: &[u8], name: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len());
    let mut skip_peeled = false;
    for line in content.lines_with_terminator() {
        if skip_peeled && line.starts_with(b"^") {
            continue;
        }
        skip_peeled = false;
        let text = line.trim_end_with(|ch| ch == '\n' || ch == '\r');
        let matches = text
            .split_once_str(" ")
            .is_some_and(|(_, refname)| refname == name.as_bytes());
        if matches && !text.starts_with(b"#") {
            skip_peeled = true;
            continue;
        }
        out.extend_from_slice(line);
    }
    out
}

/// Path without `.` and `..` components, resolved lexically.
pub(crate) fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Length in bytes of a state token before hex encoding.
const STATE_TOKEN_BYTES: usize = 16;

/// Adds `bytes` to `hasher` followed by a separator, so that adjacent fields
/// cannot run together.
pub(crate) fn feed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update(bytes);
    hasher.update([0]);
}

/// Hex text of a finished state token hash.
pub(crate) fn finish_token(hasher: Sha256) -> String {
    let digest = hasher.finalize();
    hex::encode(&digest[..STATE_TOKEN_BYTES])
}

/// Fails with `E_STALE` when `expected` is given and differs from `actual`.
pub(crate) fn check_expected_state(
    expected: Option<&str>,
    actual: &str,
    repo: &Path,
) -> Result<()> {
    let Some(expected) = expected else {
        return Ok(());
    };
    if expected.trim() == actual {
        return Ok(());
    }
    Err(problem(
        ErrorCode::Stale,
        "state_changed",
        "the repository changed since the state token was read; read the state again and repeat the call",
    )
    .with_found(expected.to_owned())
    .with_next_step(format!("git.status {}", quote(&display_path(repo)))))
}

/// Serializes an index state in the format read by `git`.
pub(crate) fn serialize_index(state: gix::index::State, path: &Path) -> Result<Vec<u8>> {
    let mut state = state;
    state.remove_tree();
    let file = gix::index::File::from_state(state, path.to_path_buf());
    let mut out = Vec::new();
    file.write_to(&mut out, gix::index::write::Options::default())
        .map_err(|err| repo_error("serialize the index", err))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_matches_files_and_directories() {
        let scopes = vec![BString::from("src")];
        assert!(in_scope(b"src/a.rs".as_bstr(), &scopes));
        assert!(in_scope(b"src".as_bstr(), &scopes));
        assert!(!in_scope(b"srcx/a.rs".as_bstr(), &scopes));
        assert!(in_scope(b"anything".as_bstr(), &[]));
        assert!(in_scope(b"anything".as_bstr(), &[BString::default()]));
    }

    #[test]
    fn relative_paths_reject_escapes_and_git_directory() {
        let root = &nurt_testkit::paths::abs_path("/srv/repo");
        assert_eq!(repo_relative(root, "src/./a.rs").unwrap(), "src/a.rs");
        assert_eq!(
            repo_relative(root, nurt_testkit::paths::abs("/srv/repo/src").as_str()).unwrap(),
            "src"
        );
        assert_eq!(repo_relative(root, ".").unwrap(), "");
        assert_eq!(
            repo_relative(root, "../x").unwrap_err().code(),
            ErrorCode::GitNotRepository
        );
        assert_eq!(
            repo_relative(root, nurt_testkit::paths::abs("/etc/hosts").as_str())
                .unwrap_err()
                .code(),
            ErrorCode::GitNotRepository
        );
        assert_eq!(
            repo_relative(root, ".GIT/config").unwrap_err().code(),
            ErrorCode::Arg
        );
    }

    #[test]
    fn reflog_line_matches_git_format() {
        let who = gix::actor::Signature {
            name: "A U Thor".into(),
            email: "a@example.com".into(),
            time: gix::date::Time {
                seconds: 1_767_265_200,
                offset: 3600,
            },
        };
        let null = ObjectId::null(gix::hash::Kind::Sha1);
        let line = reflog_line(null, null, &who, "branch: Created from HEAD");
        let expected = format!(
            "{null} {null} A U Thor <a@example.com> 1767265200 +0100\tbranch: Created from HEAD\n"
        );
        assert_eq!(line, expected.into_bytes());
        let bare = reflog_line(null, null, &who, "");
        assert!(bare.ends_with(b"+0100\n"));
    }

    #[test]
    fn packed_refs_line_and_its_peeled_value_are_removed() {
        let content = b"# pack-refs with: peeled fully-peeled sorted \n\
1111111111111111111111111111111111111111 refs/nurt/checkpoints/cp-1\n\
^2222222222222222222222222222222222222222\n\
3333333333333333333333333333333333333333 refs/nurt/checkpoints/cp-2\n";
        let out = remove_packed(content, "refs/nurt/checkpoints/cp-1");
        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains("cp-1"));
        assert!(!text.contains("^2222"));
        assert!(text.contains("cp-2"));
        assert!(text.starts_with("# pack-refs"));
    }

    fn entries(paths: &[&str]) -> BTreeMap<BString, TreeEntry> {
        let id = ObjectId::null(gix::hash::Kind::Sha1);
        paths
            .iter()
            .map(|path| (BString::from(*path), (EntryKind::Blob, id)))
            .collect()
    }

    #[test]
    fn checkout_paths_accept_ordinary_names() {
        let ordinary = entries(&[
            "a.txt",
            "src/main.rs",
            ".gitignore",
            ".github/workflows/ci.yml",
            "dir/.gitattributes",
            "zażółć/gęślą.txt",
        ]);
        verify_checkout_paths(&ordinary).unwrap();
        verify_checkout_paths(&BTreeMap::new()).unwrap();
    }

    #[test]
    fn checkout_paths_refuse_escapes_and_the_repository_directory() {
        let refused = [
            "..",
            "../x",
            "a/../x",
            "a/./x",
            "/etc/x",
            "a//x",
            ".git/config",
            ".GIT/hooks/pre-commit",
            "sub/.git/config",
            ".git./config",
            ".git /config",
            "git~1/config",
            "GIT~1/config",
            "...",
        ];
        for path in refused {
            let error = verify_checkout_paths(&entries(&[path])).unwrap_err();
            assert_eq!(error.code(), ErrorCode::GitUnsupported, "{path}");
            assert_eq!(
                error.data().problem.as_deref(),
                Some("unsafe_path"),
                "{path}"
            );
        }
    }

    #[test]
    fn checkout_paths_refuse_a_file_that_is_also_a_directory() {
        let error = verify_checkout_paths(&entries(&["a", "a/b"])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::GitUnsupported);
        verify_checkout_paths(&entries(&["a", "ab/b", "a.b"])).unwrap();
    }

    #[test]
    fn empty_identity_variables_fall_back_to_the_configuration() {
        use crate::testutil::{TestHost, test_ctx_with};
        let dir = tempfile::tempdir().unwrap();
        crate::testutil::init_isolated(dir.path()).unwrap();
        let config = dir.path().join(".git/config");
        let mut text = std::fs::read_to_string(&config).unwrap();
        text.push_str("[user]\n\tname = Config Name\n\temail = config@example.com\n");
        std::fs::write(&config, text).unwrap();
        let repo = gix::open_opts(dir.path(), gix::open::Options::isolated()).unwrap();
        let host = TestHost::new("/nonexistent-home")
            .with_var("GIT_COMMITTER_NAME", "")
            .with_var("GIT_COMMITTER_EMAIL", "");
        let who = committer(&repo, &test_ctx_with(host));
        assert_eq!(who.name, "Config Name");
        assert_eq!(who.email, "config@example.com");
    }

    #[test]
    fn identity_characters_that_break_a_log_line_are_removed() {
        use crate::testutil::{TestHost, test_ctx_with};
        let dir = tempfile::tempdir().unwrap();
        crate::testutil::init_isolated(dir.path()).unwrap();
        let repo = gix::open_opts(dir.path(), gix::open::Options::isolated()).unwrap();
        let host = TestHost::new("/nonexistent-home")
            .with_var("GIT_COMMITTER_NAME", "Ada <Lovelace>\n")
            .with_var("GIT_COMMITTER_EMAIL", "<ada@example.com>");
        let who = committer(&repo, &test_ctx_with(host));
        assert_eq!(who.name, "Ada Lovelace");
        assert_eq!(who.email, "ada@example.com");
        let host = TestHost::new("/nonexistent-home")
            .with_var("GIT_COMMITTER_NAME", "<>")
            .with_var("GIT_COMMITTER_EMAIL", "\n");
        let who = committer(&repo, &test_ctx_with(host));
        assert_eq!(who.name, FALLBACK_NAME);
        assert_eq!(who.email, FALLBACK_EMAIL);
    }

    #[test]
    fn reflog_line_folds_the_message_to_one_line() {
        let who = gix::actor::Signature {
            name: "A".into(),
            email: "a@example.com".into(),
            time: gix::date::Time {
                seconds: 1,
                offset: 0,
            },
        };
        let null = ObjectId::null(gix::hash::Kind::Sha1);
        let line = reflog_line(null, null, &who, "branch: Created from\n a  b \n");
        assert!(line.ends_with(b"\tbranch: Created from a b\n"));
    }
    #[test]
    fn lexical_resolves_dots_without_touching_the_file_system() {
        assert_eq!(lexical(Path::new("/a/./b/../c")), PathBuf::from("/a/c"));
        assert_eq!(lexical(Path::new("/../x")), PathBuf::from("/x"));
        assert_eq!(lexical(Path::new("/a/b/")), PathBuf::from("/a/b"));
    }

    #[test]
    fn reflog_times_carry_the_sign_and_minutes_of_the_offset() {
        let time = |offset| gix::date::Time { seconds: 5, offset };
        assert_eq!(format_time(time(0)), "5 +0000");
        assert_eq!(format_time(time(-(5 * 3600 + 30 * 60))), "5 -0530");
        assert_eq!(format_time(time(5 * 3600 + 45 * 60)), "5 +0545");
    }

    #[test]
    fn lock_files_are_named_after_the_file_and_block_the_write() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("index");
        assert_eq!(lock_path(&file), dir.path().join("index.lock"));
        ensure_unlocked(&file).unwrap();
        std::fs::write(lock_path(&file), b"").unwrap();
        let error = ensure_unlocked(&file).unwrap_err();
        assert_eq!(error.code(), ErrorCode::GitLocked);
        assert!(!error.data().fix.is_empty());
    }

    #[test]
    fn expected_state_must_match_after_trimming() {
        let repo = Path::new("/srv/repo");
        check_expected_state(None, "abc", repo).unwrap();
        check_expected_state(Some(" abc\n"), "abc", repo).unwrap();
        let error = check_expected_state(Some("other"), "abc", repo).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert_eq!(error.data().problem.as_deref(), Some("state_changed"));
    }

    #[test]
    fn current_content_is_expected_by_hash_and_a_missing_file_as_absent() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        assert_eq!(expect_current(&file).unwrap(), Expect::Absent);
        std::fs::write(&file, b"content").unwrap();
        assert_eq!(
            expect_current(&file).unwrap(),
            Expect::Hash(content_hash(b"content"))
        );
    }

    #[test]
    fn packed_reference_lookup_matches_whole_names_only() {
        let dir = tempfile::tempdir().unwrap();
        let repo = crate::testutil::init_isolated(dir.path()).unwrap();
        let id = "1".repeat(40);
        std::fs::write(
            repo.common_dir().join("packed-refs"),
            format!("# pack-refs with: peeled \n{id} refs/heads/ab\n^{id}\n{id} refs/heads/a\n"),
        )
        .unwrap();
        assert_eq!(
            packed_ref_line(&repo, "refs/heads/a").unwrap(),
            Some(format!("{id} refs/heads/a"))
        );
        assert_eq!(packed_ref_line(&repo, "refs/heads/b").unwrap(), None);
        assert_eq!(packed_ref_line(&repo, "refs/heads/").unwrap(), None);
    }

    #[test]
    fn packed_line_removal_keeps_other_lines_and_a_missing_name_changes_nothing() {
        let content = b"# header\n1111111111111111111111111111111111111111 refs/heads/a\n2222222222222222222222222222222222222222 refs/heads/ab\n";
        let out = remove_packed(content, "refs/heads/a");
        assert_eq!(
            out,
            b"# header\n2222222222222222222222222222222222222222 refs/heads/ab\n"
        );
        assert_eq!(remove_packed(content, "refs/heads/zz"), content);
        assert_eq!(remove_packed(b"", "refs/heads/a"), b"");
    }

    #[test]
    fn quote_escapes_quotes_and_backslashes() {
        assert_eq!(quote(r#"a"b\c"#), r#""a\"b\\c""#);
    }
}
