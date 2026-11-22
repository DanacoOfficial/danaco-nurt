// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conformance checks run against every file system back end.
//!
//! [`run`] exercises every method of [`Fs`] and [`FsWrite`] inside an empty
//! directory and returns one [`CheckResult`] per check. Each check works in a
//! subdirectory named after its identifier. A check is skipped only when the
//! volume declares that it lacks a capability ([`FsFeatures`]) and the method
//! fails with [`FsErrorKind::Unsupported`]; a method that contradicts the
//! declaration fails the check. Symbolic link checks are also skipped when
//! the account lacks the privilege to create links
//! ([`FsErrorKind::Privilege`]).

use std::fmt;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use nurt_contracts::fs::{
    AbsPath, ContentHash, Durability, ExtendedMeta, FileKind, Follow, FsError, FsErrorKind,
    FsFeatures, FsResult, FsWrite, LinkKind, MetaChange, MetaTransferReport, RenameMode, TxScope,
    Xattr,
};

const DIR_MODE: u32 = 0o755;
const CHECK_DIR_MODE: u32 = 0o750;
const RESTRICTED_MODE: u32 = 0o600;
const APPLIED_MODE: u32 = 0o640;
const PERMISSION_MASK: u32 = 0o777;
const LOCK_WAIT: Duration = Duration::from_millis(50);
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const TIME_SHIFT_NS: i64 = 3_600_000_000_000;
const EMPTY_SHA256: &str =
    "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const ABC_SHA256: &str = "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const TEST_XATTR: &str = "user.nurt-conformance";

/// Options of a conformance run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// Empty directory on a different volume, used to verify
    /// [`FsErrorKind::CrossDevice`]; the check is skipped without it.
    pub other_volume: Option<AbsPath>,
}

/// Outcome of one check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The behaviour conforms.
    Passed,
    /// The behaviour does not conform; the text describes the deviation.
    Failed(String),
    /// The check does not apply; the text gives the reason.
    Skipped(String),
}

/// Result of one check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    /// Stable identifier (`fs-conf-<name>`).
    pub id: &'static str,
    /// Outcome.
    pub outcome: Outcome,
}

/// Results of a conformance run, in check order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// One result per check.
    pub results: Vec<CheckResult>,
}

impl Report {
    /// Whether no check failed.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.failures().next().is_none()
    }

    /// Failed checks.
    pub fn failures(&self) -> impl Iterator<Item = &CheckResult> {
        self.results
            .iter()
            .filter(|result| matches!(result.outcome, Outcome::Failed(_)))
    }

    /// Outcome of the check with identifier `id`.
    #[must_use]
    pub fn outcome(&self, id: &str) -> Option<&Outcome> {
        self.results
            .iter()
            .find(|result| result.id == id)
            .map(|result| &result.outcome)
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for result in &self.results {
            match &result.outcome {
                Outcome::Passed => writeln!(f, "{}: passed", result.id)?,
                Outcome::Failed(reason) => writeln!(f, "{}: failed: {reason}", result.id)?,
                Outcome::Skipped(reason) => writeln!(f, "{}: skipped: {reason}", result.id)?,
            }
        }
        Ok(())
    }
}

/// Runs every check with default [`Options`].
///
/// `root` must be an empty, writable directory; `scope` is used for every
/// mutation. A back end test calls the function with a new temporary
/// directory and asserts `report.passed()`, formatting the report as the
/// failure message.
#[must_use]
pub fn run(fs: &dyn FsWrite, root: &AbsPath, scope: &TxScope) -> Report {
    run_with(fs, root, scope, &Options::default())
}

/// Runs every check.
///
/// `root` must be an empty, writable directory; `scope` is used for every
/// mutation.
///
/// The checks and their identifiers:
///
/// | Identifier | Verifies |
/// |---|---|
/// | `fs-conf-create-temp-in` | unique names derived from the hint, `NotFound` for a missing directory |
/// | `fs-conf-create-new` | creation and `Exists` |
/// | `fs-conf-rename-replace` | replacement keeps the identity of the source |
/// | `fs-conf-rename-noreplace` | `Exists` and unchanged entries |
/// | `fs-conf-exchange` | names exchanged |
/// | `fs-conf-replace-with-backup` | new content under the target, old content under the backup |
/// | `fs-conf-hard-link` | link count and shared identity |
/// | `fs-conf-open-in-place` | same identity, every name sees the new content |
/// | `fs-conf-handle-follows-inode` | open handles keep referring to the renamed file |
/// | `fs-conf-clone-file` | independent copy with equal content |
/// | `fs-conf-copy-file` | content, copied byte count, `Exists` |
/// | `fs-conf-remove-file` | removal, `NotFound`, `IsADirectory` |
/// | `fs-conf-remove-dir` | `NotEmpty`, `NotADirectory` |
/// | `fs-conf-remove-tree` | statistics; a link to a directory outside the tree is not followed |
/// | `fs-conf-create-dir` | permission bits, `Exists`, `NotFound` |
/// | `fs-conf-create-link` | file and directory links |
/// | `fs-conf-read-link` | stored target, `InvalidPath` for a non-link |
/// | `fs-conf-metadata-follow` | `Follow` and `NoFollow`, dangling links |
/// | `fs-conf-extended-metadata` | basic part and link target |
/// | `fs-conf-meta-round-trip` | `set_meta`, then `restore_meta` restores the previous state |
/// | `fs-conf-apply-meta` | metadata applied through a write handle |
/// | `fs-conf-truncate` | shrinking and zero-filled growth |
/// | `fs-conf-sync` | file and directory flushes |
/// | `fs-conf-lock-exclusive` | second lock waits and fails with `InUse` |
/// | `fs-conf-mark-hidden` | marking and `NotFound` |
/// | `fs-conf-read-dir` | names and kinds |
/// | `fs-conf-content-hash` | SHA-256 test vectors |
/// | `fs-conf-volume` | mount point and nearest existing ancestor |
/// | `fs-conf-space` | consistent figures |
/// | `fs-conf-path-key` | case variants share a key exactly on case-insensitive volumes |
/// | `fs-conf-holders` | query and `NotFound` |
/// | `fs-conf-can-write-dir` | own directory writable, error kinds |
/// | `fs-conf-error-kinds` | `NotFound`, `NotADirectory`, `IsADirectory` |
/// | `fs-conf-cross-device` | `CrossDevice` towards the second volume of [`Options`] |
/// | `fs-conf-streams` | alternate data streams, also through `copy_file` |
/// | `fs-conf-real-path` | links resolved |
#[must_use]
pub fn run_with(fs: &dyn FsWrite, root: &AbsPath, scope: &TxScope, options: &Options) -> Report {
    let features = match fs.volume(root) {
        Ok(volume) => volume.features,
        Err(error) => {
            return Report {
                results: vec![CheckResult {
                    id: "fs-conf-volume",
                    outcome: Outcome::Failed(format!("volume of the test root: {error}")),
                }],
            };
        }
    };
    let results = CHECKS
        .iter()
        .map(|(id, check)| CheckResult {
            id,
            outcome: run_check(fs, root, scope, options, features, id, *check),
        })
        .collect();
    Report { results }
}

fn run_check(
    fs: &dyn FsWrite,
    root: &AbsPath,
    scope: &TxScope,
    options: &Options,
    features: FsFeatures,
    id: &str,
    check: CheckFn,
) -> Outcome {
    let prepared = root
        .join_name(id)
        .and_then(|dir| fs.create_dir(scope, &dir, DIR_MODE).map(|()| dir));
    let dir = match prepared {
        Ok(dir) => dir,
        Err(error) => {
            return Outcome::Failed(format!("cannot create the check directory: {error}"));
        }
    };
    let ctx = Ctx {
        fs,
        scope,
        dir,
        features,
        other: options.other_volume.as_ref(),
    };
    match check(&ctx) {
        Ok(()) => Outcome::Passed,
        Err(Problem::Fail(reason)) => Outcome::Failed(reason),
        Err(Problem::Skip(reason)) => Outcome::Skipped(reason),
    }
}

type CheckFn = fn(&Ctx<'_>) -> Check;

const CHECKS: &[(&str, CheckFn)] = &[
    ("fs-conf-create-temp-in", check_create_temp_in),
    ("fs-conf-create-new", check_create_new),
    ("fs-conf-rename-replace", check_rename_replace),
    ("fs-conf-rename-noreplace", check_rename_noreplace),
    ("fs-conf-exchange", check_exchange),
    ("fs-conf-replace-with-backup", check_replace_with_backup),
    ("fs-conf-hard-link", check_hard_link),
    ("fs-conf-open-in-place", check_open_in_place),
    ("fs-conf-handle-follows-inode", check_handle_follows_inode),
    ("fs-conf-clone-file", check_clone_file),
    ("fs-conf-copy-file", check_copy_file),
    ("fs-conf-remove-file", check_remove_file),
    ("fs-conf-remove-dir", check_remove_dir),
    ("fs-conf-remove-tree", check_remove_tree),
    ("fs-conf-create-dir", check_create_dir),
    ("fs-conf-create-link", check_create_link),
    ("fs-conf-read-link", check_read_link),
    ("fs-conf-metadata-follow", check_metadata_follow),
    ("fs-conf-extended-metadata", check_extended_metadata),
    ("fs-conf-meta-round-trip", check_meta_round_trip),
    ("fs-conf-apply-meta", check_apply_meta),
    ("fs-conf-truncate", check_truncate),
    ("fs-conf-sync", check_sync),
    ("fs-conf-lock-exclusive", check_lock_exclusive),
    ("fs-conf-mark-hidden", check_mark_hidden),
    ("fs-conf-read-dir", check_read_dir),
    ("fs-conf-content-hash", check_content_hash),
    ("fs-conf-volume", check_volume),
    ("fs-conf-space", check_space),
    ("fs-conf-path-key", check_path_key),
    ("fs-conf-holders", check_holders),
    ("fs-conf-can-write-dir", check_can_write_dir),
    ("fs-conf-error-kinds", check_error_kinds),
    ("fs-conf-cross-device", check_cross_device),
    ("fs-conf-streams", check_streams),
    ("fs-conf-real-path", check_real_path),
];

struct Ctx<'a> {
    fs: &'a dyn FsWrite,
    scope: &'a TxScope,
    dir: AbsPath,
    features: FsFeatures,
    other: Option<&'a AbsPath>,
}

enum Problem {
    Fail(String),
    Skip(String),
}

type Check = Result<(), Problem>;

impl From<FsError> for Problem {
    fn from(error: FsError) -> Self {
        Self::Fail(format!("unexpected error: {error}"))
    }
}

impl From<io::Error> for Problem {
    fn from(error: io::Error) -> Self {
        Self::Fail(format!("unexpected input/output error: {error}"))
    }
}

fn ensure(condition: bool, reason: &str) -> Check {
    if condition {
        Ok(())
    } else {
        Err(Problem::Fail(reason.to_owned()))
    }
}

fn expect_kind<T>(result: FsResult<T>, kind: FsErrorKind, what: &str) -> Check {
    match result {
        Err(error) if error.kind == kind => Ok(()),
        Err(error) => Err(Problem::Fail(format!(
            "{what}: expected {}, got {error}",
            kind.as_str()
        ))),
        Ok(_) => Err(Problem::Fail(format!(
            "{what}: succeeded, expected {}",
            kind.as_str()
        ))),
    }
}

/// Result of a method whose availability the volume declares.
fn declared<T>(declared: bool, result: FsResult<T>, feature: &str) -> Result<T, Problem> {
    match (declared, result) {
        (true, Ok(value)) => Ok(value),
        (false, Err(error)) if error.kind == FsErrorKind::Unsupported => {
            Err(Problem::Skip(format!("the volume declares no {feature}")))
        }
        (false, Ok(_)) => Err(Problem::Fail(format!(
            "succeeded although the volume declares no {feature}"
        ))),
        (_, Err(error)) => Err(error.into()),
    }
}

/// Result of a link creation; the missing privilege skips the check.
fn link_created(result: FsResult<()>) -> Check {
    match result {
        Err(error) if error.kind == FsErrorKind::Privilege => Err(Problem::Skip(
            "the account lacks the privilege to create symbolic links".to_owned(),
        )),
        other => Ok(other?),
    }
}

impl Ctx<'_> {
    fn path(&self, name: &str) -> Result<AbsPath, Problem> {
        Ok(self.dir.join_name(name)?)
    }

    fn file(&self, name: &str, content: &[u8]) -> Result<AbsPath, Problem> {
        let path = self.path(name)?;
        self.write_new(&path, content)?;
        Ok(path)
    }

    fn write_new(&self, path: &AbsPath, content: &[u8]) -> Check {
        let mut handle = self.fs.create_new(self.scope, path)?;
        handle.write_all(content)?;
        handle.sync(Durability::Full)?;
        Ok(())
    }

    fn dir(&self, name: &str) -> Result<AbsPath, Problem> {
        let path = self.path(name)?;
        self.fs.create_dir(self.scope, &path, DIR_MODE)?;
        Ok(path)
    }

    fn read(&self, path: &AbsPath) -> Result<Vec<u8>, Problem> {
        let mut out = Vec::new();
        self.fs.open_read(path)?.read_to_end(&mut out)?;
        Ok(out)
    }

    fn overwrite(&self, path: &AbsPath, content: &[u8]) -> Check {
        let mut handle = self.fs.open_in_place(self.scope, path)?;
        handle.set_length(0)?;
        handle.write_all(content)?;
        handle.sync(Durability::Data)?;
        Ok(())
    }

    fn content_is(&self, path: &AbsPath, expected: &[u8], what: &str) -> Check {
        let actual = self.read(path)?;
        if actual == expected {
            Ok(())
        } else {
            Err(Problem::Fail(format!(
                "{what}: content {:?}, expected {:?}",
                String::from_utf8_lossy(&actual),
                String::from_utf8_lossy(expected)
            )))
        }
    }

    fn absent(&self, path: &AbsPath, what: &str) -> Check {
        expect_kind(
            self.fs.metadata(path, Follow::NoFollow),
            FsErrorKind::NotFound,
            what,
        )
    }

    fn link(&self, target: &Path, name: &str, kind: LinkKind) -> Result<AbsPath, Problem> {
        let path = self.path(name)?;
        link_created(self.fs.create_link(self.scope, target, &path, kind))?;
        Ok(path)
    }
}

fn check_create_temp_in(ctx: &Ctx<'_>) -> Check {
    let first = ctx.fs.create_temp_in(ctx.scope, &ctx.dir, "temp")?;
    let second = ctx.fs.create_temp_in(ctx.scope, &ctx.dir, "temp")?;
    ensure(
        first.path() != second.path(),
        "two temporary files received the same name",
    )?;
    for handle in [&first, &second] {
        ensure(
            handle.path().parent().as_ref() == Some(&ctx.dir),
            "temporary file created outside the requested directory",
        )?;
        let named = handle
            .path()
            .file_name()
            .is_some_and(|name| name.to_string_lossy().contains("temp"));
        ensure(named, "temporary file name does not contain the hint")?;
        let meta = ctx.fs.metadata(handle.path(), Follow::NoFollow)?;
        ensure(meta.kind == FileKind::File, "temporary entry is not a file")?;
    }
    let missing = ctx.path("missing")?;
    expect_kind(
        ctx.fs.create_temp_in(ctx.scope, &missing, "temp"),
        FsErrorKind::NotFound,
        "create_temp_in in a missing directory",
    )
}

fn check_create_new(ctx: &Ctx<'_>) -> Check {
    let path = ctx.file("new", b"content")?;
    ctx.content_is(&path, b"content", "created file")?;
    expect_kind(
        ctx.fs.create_new(ctx.scope, &path),
        FsErrorKind::Exists,
        "create_new over an existing file",
    )
}

fn check_rename_replace(ctx: &Ctx<'_>) -> Check {
    let source = ctx.file("source", b"one")?;
    let target = ctx.file("target", b"two")?;
    let identity = ctx.fs.metadata(&source, Follow::NoFollow)?.file_id;
    ctx.fs
        .rename(ctx.scope, &source, &target, RenameMode::Replace)?;
    ctx.content_is(&target, b"one", "rename target")?;
    let moved = ctx.fs.metadata(&target, Follow::NoFollow)?.file_id;
    ensure(moved == identity, "rename changed the file identity")?;
    ctx.absent(&source, "rename source")
}

fn check_rename_noreplace(ctx: &Ctx<'_>) -> Check {
    let source = ctx.file("source", b"one")?;
    let target = ctx.file("target", b"two")?;
    let result = ctx
        .fs
        .rename(ctx.scope, &source, &target, RenameMode::NoReplace);
    match result {
        Err(error) if error.kind == FsErrorKind::Unsupported && !ctx.features.rename_noreplace => {
            return Err(Problem::Skip(
                "the volume declares no rename without replacement".to_owned(),
            ));
        }
        other => expect_kind(other, FsErrorKind::Exists, "rename without replacement")?,
    }
    ctx.content_is(&source, b"one", "refused rename source")?;
    ctx.content_is(&target, b"two", "refused rename target")?;
    let free = ctx.path("free")?;
    ctx.fs
        .rename(ctx.scope, &source, &free, RenameMode::NoReplace)?;
    ctx.content_is(&free, b"one", "rename to a free name")
}

fn check_exchange(ctx: &Ctx<'_>) -> Check {
    let first = ctx.file("first", b"one")?;
    let second = ctx.file("second", b"two")?;
    let result = ctx.fs.exchange(ctx.scope, &first, &second);
    declared(ctx.features.exchange, result, "name exchange")?;
    ctx.content_is(&first, b"two", "first name after exchange")?;
    ctx.content_is(&second, b"one", "second name after exchange")
}

fn check_replace_with_backup(ctx: &Ctx<'_>) -> Check {
    let target = ctx.file("target", b"old")?;
    let replacement = ctx.file("replacement", b"new")?;
    let backup = ctx.path("backup")?;
    let result = ctx
        .fs
        .replace_with_backup(ctx.scope, &target, &replacement, &backup);
    declared(
        ctx.features.replace_with_backup,
        result,
        "replace with backup",
    )?;
    ctx.content_is(&target, b"new", "replaced target")?;
    ctx.content_is(&backup, b"old", "backup")?;
    ctx.absent(&replacement, "consumed replacement")
}

fn check_hard_link(ctx: &Ctx<'_>) -> Check {
    let first = ctx.file("first", b"shared")?;
    let second = ctx.path("second")?;
    let result = ctx.fs.hard_link(ctx.scope, &first, &second);
    declared(ctx.features.hardlink, result, "hard links")?;
    let a = ctx.fs.metadata(&first, Follow::NoFollow)?;
    let b = ctx.fs.metadata(&second, Follow::NoFollow)?;
    ensure(a.links == 2 && b.links == 2, "link count is not 2")?;
    ensure(
        a.file_id == b.file_id,
        "hard links have different identities",
    )?;
    expect_kind(
        ctx.fs.hard_link(ctx.scope, &first, &second),
        FsErrorKind::Exists,
        "hard link over an existing name",
    )
}

fn check_open_in_place(ctx: &Ctx<'_>) -> Check {
    let first = ctx.file("first", b"previous content")?;
    let second = ctx.path("second")?;
    let linked = ctx.features.hardlink;
    if linked {
        ctx.fs.hard_link(ctx.scope, &first, &second)?;
    }
    let identity = ctx.fs.metadata(&first, Follow::NoFollow)?.file_id;
    ctx.overwrite(&first, b"next")?;
    ctx.content_is(&first, b"next", "file written in place")?;
    if linked {
        ctx.content_is(&second, b"next", "second link of a file written in place")?;
    }
    let after = ctx.fs.metadata(&first, Follow::NoFollow)?.file_id;
    ensure(
        after == identity,
        "writing in place changed the file identity",
    )
}

fn check_handle_follows_inode(ctx: &Ctx<'_>) -> Check {
    let original = ctx.file("original", b"before")?;
    let moved = ctx.path("moved")?;
    let mut writer = ctx.fs.open_in_place(ctx.scope, &original)?;
    ctx.fs
        .rename(ctx.scope, &original, &moved, RenameMode::Replace)?;
    ctx.write_new(&original, b"fresh")?;
    writer.set_length(0)?;
    writer.write_all(b"after")?;
    drop(writer);
    ctx.content_is(
        &moved,
        b"after",
        "file written through a handle opened before the rename",
    )?;
    ctx.content_is(&original, b"fresh", "new file under the previous name")?;
    let mut reader = ctx.fs.open_read(&moved)?;
    let gone = ctx.path("gone")?;
    ctx.fs
        .rename(ctx.scope, &moved, &gone, RenameMode::Replace)?;
    let mut seen = Vec::new();
    reader.read_to_end(&mut seen)?;
    ensure(
        seen == b"after",
        "reading through a handle opened before the rename returned other content",
    )
}

fn check_clone_file(ctx: &Ctx<'_>) -> Check {
    let source = ctx.file("source", b"data")?;
    let clone = ctx.path("clone")?;
    let result = ctx.fs.clone_file(ctx.scope, &source, &clone);
    declared(ctx.features.reflink, result, "block clones")?;
    ctx.content_is(&clone, b"data", "clone")?;
    let a = ctx.fs.metadata(&source, Follow::NoFollow)?.file_id;
    let b = ctx.fs.metadata(&clone, Follow::NoFollow)?.file_id;
    ensure(a != b, "clone shares the identity of its source")?;
    ctx.overwrite(&clone, b"XX")?;
    ctx.content_is(&source, b"data", "source after the clone was modified")
}

fn check_copy_file(ctx: &Ctx<'_>) -> Check {
    let source = ctx.file("source", b"payload")?;
    let copy = ctx.path("copy")?;
    let copied = ctx.fs.copy_file(ctx.scope, &source, &copy)?;
    ensure(copied == 7, "copy_file reported a wrong byte count")?;
    ctx.content_is(&copy, b"payload", "copy")?;
    expect_kind(
        ctx.fs.copy_file(ctx.scope, &source, &copy),
        FsErrorKind::Exists,
        "copy over an existing file",
    )
}

fn check_remove_file(ctx: &Ctx<'_>) -> Check {
    let file = ctx.file("file", b"x")?;
    ctx.fs.remove_file(ctx.scope, &file)?;
    ctx.absent(&file, "removed file")?;
    expect_kind(
        ctx.fs.remove_file(ctx.scope, &file),
        FsErrorKind::NotFound,
        "removal of a missing file",
    )?;
    let dir = ctx.dir("dir")?;
    expect_kind(
        ctx.fs.remove_file(ctx.scope, &dir),
        FsErrorKind::IsADirectory,
        "remove_file on a directory",
    )
}

fn check_remove_dir(ctx: &Ctx<'_>) -> Check {
    let dir = ctx.dir("dir")?;
    let inner = dir.join_name("inner")?;
    ctx.write_new(&inner, b"x")?;
    expect_kind(
        ctx.fs.remove_dir(ctx.scope, &dir),
        FsErrorKind::NotEmpty,
        "removal of a non-empty directory",
    )?;
    ctx.fs.remove_file(ctx.scope, &inner)?;
    ctx.fs.remove_dir(ctx.scope, &dir)?;
    ctx.absent(&dir, "removed directory")?;
    let file = ctx.file("file", b"x")?;
    expect_kind(
        ctx.fs.remove_dir(ctx.scope, &file),
        FsErrorKind::NotADirectory,
        "remove_dir on a file",
    )
}

fn check_remove_tree(ctx: &Ctx<'_>) -> Check {
    let outside = ctx.dir("outside")?;
    let kept = outside.join_name("kept")?;
    ctx.write_new(&kept, b"keep")?;
    let tree = ctx.dir("tree")?;
    let sub = tree.join_name("sub")?;
    ctx.fs.create_dir(ctx.scope, &sub, DIR_MODE)?;
    ctx.write_new(&tree.join_name("one")?, b"12")?;
    ctx.write_new(&sub.join_name("two")?, b"345")?;
    let link = tree.join_name("link")?;
    let linked = match ctx.fs.create_link(
        ctx.scope,
        outside.as_path(),
        &link,
        LinkKind::SymlinkDirectory,
    ) {
        Ok(()) => true,
        Err(error) if error.kind == FsErrorKind::Privilege => false,
        Err(error) => return Err(error.into()),
    };
    let stats = ctx.fs.remove_tree(ctx.scope, &tree, 2)?;
    ctx.absent(&tree, "removed tree")?;
    ctx.content_is(&kept, b"keep", "file behind a link inside the removed tree")?;
    ensure(
        stats.directories == 2,
        "remove_tree counted a wrong number of directories",
    )?;
    let files = if linked { 3 } else { 2 };
    ensure(
        stats.files == files,
        "remove_tree counted a wrong number of files",
    )?;
    ensure(
        stats.bytes >= 5,
        "remove_tree counted fewer bytes than removed",
    )
}

fn check_create_dir(ctx: &Ctx<'_>) -> Check {
    let dir = ctx.path("dir")?;
    ctx.fs.create_dir(ctx.scope, &dir, CHECK_DIR_MODE)?;
    let meta = ctx.fs.metadata(&dir, Follow::NoFollow)?;
    ensure(
        meta.kind == FileKind::Directory,
        "created entry is not a directory",
    )?;
    if meta.uid.is_some() {
        ensure(
            meta.mode & PERMISSION_MASK == CHECK_DIR_MODE,
            "directory permission bits differ from the requested ones",
        )?;
    }
    expect_kind(
        ctx.fs.create_dir(ctx.scope, &dir, DIR_MODE),
        FsErrorKind::Exists,
        "creation of an existing directory",
    )?;
    let orphan = ctx.path("missing")?.join_name("child")?;
    expect_kind(
        ctx.fs.create_dir(ctx.scope, &orphan, DIR_MODE),
        FsErrorKind::NotFound,
        "creation below a missing directory",
    )
}

fn check_create_link(ctx: &Ctx<'_>) -> Check {
    let file = ctx.file("file", b"x")?;
    let dir = ctx.dir("dir")?;
    let file_link = ctx.link(file.as_path(), "file-link", LinkKind::SymlinkFile)?;
    let dir_link = ctx.link(dir.as_path(), "dir-link", LinkKind::SymlinkDirectory)?;
    for (link, kind) in [
        (&file_link, FileKind::File),
        (&dir_link, FileKind::Directory),
    ] {
        let own = ctx.fs.metadata(link, Follow::NoFollow)?.kind;
        ensure(
            own == FileKind::Symlink,
            "link is not reported as a symbolic link",
        )?;
        let target = ctx.fs.metadata(link, Follow::Follow)?.kind;
        ensure(target == kind, "followed link reports a wrong kind")?;
    }
    expect_kind(
        ctx.fs
            .create_link(ctx.scope, file.as_path(), &file_link, LinkKind::SymlinkFile),
        FsErrorKind::Exists,
        "link over an existing name",
    )?;
    let junction = ctx.path("junction")?;
    match ctx
        .fs
        .create_link(ctx.scope, dir.as_path(), &junction, LinkKind::Junction)
    {
        Ok(()) => {
            let kind = ctx.fs.metadata(&junction, Follow::NoFollow)?.kind;
            ensure(
                kind == FileKind::Junction,
                "junction is not reported as a junction",
            )
        }
        Err(error) if error.kind == FsErrorKind::Unsupported => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn check_read_link(ctx: &Ctx<'_>) -> Check {
    let target = PathBuf::from("relative-target");
    let link = ctx.link(&target, "link", LinkKind::SymlinkFile)?;
    ensure(
        ctx.fs.read_link(&link)? == target,
        "read_link returned a different target",
    )?;
    let file = ctx.file("file", b"x")?;
    expect_kind(
        ctx.fs.read_link(&file),
        FsErrorKind::InvalidPath,
        "read_link on a file",
    )?;
    expect_kind(
        ctx.fs.read_link(&ctx.path("missing")?),
        FsErrorKind::NotFound,
        "read_link on a missing entry",
    )
}

fn check_metadata_follow(ctx: &Ctx<'_>) -> Check {
    let file = ctx.file("file", b"12345")?;
    let link = ctx.link(file.as_path(), "link", LinkKind::SymlinkFile)?;
    let followed = ctx.fs.metadata(&link, Follow::Follow)?;
    ensure(
        followed.kind == FileKind::File && followed.size == 5,
        "followed link does not describe its target",
    )?;
    let own = ctx.fs.metadata(&link, Follow::NoFollow)?;
    ensure(
        own.kind == FileKind::Symlink,
        "link itself is not a symbolic link",
    )?;
    let missing = ctx.path("missing")?;
    let dangling = ctx.link(missing.as_path(), "dangling", LinkKind::SymlinkFile)?;
    expect_kind(
        ctx.fs.metadata(&dangling, Follow::Follow),
        FsErrorKind::NotFound,
        "followed dangling link",
    )?;
    ctx.fs.metadata(&dangling, Follow::NoFollow)?;
    Ok(())
}

fn check_extended_metadata(ctx: &Ctx<'_>) -> Check {
    let file = ctx.file("file", b"abc")?;
    let extended = ctx.fs.extended_metadata(&file, Follow::NoFollow)?;
    let basic = ctx.fs.metadata(&file, Follow::NoFollow)?;
    ensure(
        extended.basic.kind == basic.kind
            && extended.basic.size == basic.size
            && extended.basic.file_id == basic.file_id
            && extended.basic.mode == basic.mode,
        "extended metadata disagrees with basic metadata",
    )?;
    ensure(
        extended.link_target.is_none(),
        "a file reports a link target",
    )?;
    let link = ctx.link(file.as_path(), "link", LinkKind::SymlinkFile)?;
    let of_link = ctx.fs.extended_metadata(&link, Follow::NoFollow)?;
    ensure(
        of_link.link_target.as_deref() == Some(file.as_path()),
        "link target missing from extended metadata",
    )
}

fn shifted_change(before: &ExtendedMeta, mode: u32, xattr: bool) -> MetaChange {
    MetaChange {
        mode: Some(mode),
        modified_ns: Some(before.basic.modified_ns.saturating_sub(TIME_SHIFT_NS)),
        set_xattrs: if xattr {
            vec![Xattr {
                name: TEST_XATTR.into(),
                value: b"1".to_vec(),
            }]
        } else {
            Vec::new()
        },
        ..MetaChange::default()
    }
}

fn compare_restored(
    before: &ExtendedMeta,
    after: &ExtendedMeta,
    report: &MetaTransferReport,
) -> Check {
    let skipped = |element: &str| report.not_transferred.iter().any(|name| name == element);
    let unix = before.basic.uid.is_some();
    let mut differences = Vec::new();
    if unix && before.basic.mode != after.basic.mode {
        differences.push("mode");
    }
    if before.basic.modified_ns != after.basic.modified_ns {
        differences.push("modification time");
    }
    if !skipped("account") && before.basic.uid != after.basic.uid {
        differences.push("user id");
    }
    if !skipped("group") && before.basic.gid != after.basic.gid {
        differences.push("group id");
    }
    let xattrs_reported = report
        .not_transferred
        .iter()
        .any(|name| name.starts_with("xattr:"));
    if !xattrs_reported && before.xattrs != after.xattrs {
        differences.push("extended attributes");
    }
    if !skipped("acl") && before.acl != after.acl {
        differences.push("ACL");
    }
    if before.flags != after.flags
        || before.basic.windows_attributes != after.basic.windows_attributes
    {
        differences.push("flags");
    }
    if differences.is_empty() {
        Ok(())
    } else {
        Err(Problem::Fail(format!(
            "restore_meta left differences in: {}",
            differences.join(", ")
        )))
    }
}

fn check_meta_round_trip(ctx: &Ctx<'_>) -> Check {
    let file = ctx.file("file", b"x")?;
    let before = ctx.fs.extended_metadata(&file, Follow::NoFollow)?;
    let change = shifted_change(&before, RESTRICTED_MODE, ctx.features.xattr);
    ctx.fs.set_meta(ctx.scope, &file, &change)?;
    let changed = ctx.fs.extended_metadata(&file, Follow::NoFollow)?;
    ensure(
        changed.basic.modified_ns == before.basic.modified_ns.saturating_sub(TIME_SHIFT_NS),
        "set_meta did not change the modification time",
    )?;
    if ctx.features.xattr {
        ensure(
            changed.xattrs.iter().any(|xattr| xattr.name == TEST_XATTR),
            "set_meta did not set the extended attribute",
        )?;
    }
    let report = ctx.fs.restore_meta(ctx.scope, &file, &before)?;
    let after = ctx.fs.extended_metadata(&file, Follow::NoFollow)?;
    compare_restored(&before, &after, &report)
}

fn check_apply_meta(ctx: &Ctx<'_>) -> Check {
    let source = ctx.file("source", b"x")?;
    let before = ctx.fs.extended_metadata(&source, Follow::NoFollow)?;
    ctx.fs.set_meta(
        ctx.scope,
        &source,
        &shifted_change(&before, APPLIED_MODE, false),
    )?;
    let wanted = ctx.fs.extended_metadata(&source, Follow::NoFollow)?;
    let mut handle = ctx.fs.create_temp_in(ctx.scope, &ctx.dir, "apply")?;
    handle.write_all(b"x")?;
    let report = handle.apply_meta(&wanted)?;
    let path = handle.path().clone();
    drop(handle);
    let applied = ctx.fs.extended_metadata(&path, Follow::NoFollow)?;
    if wanted.basic.uid.is_some() {
        ensure(
            applied.basic.mode & PERMISSION_MASK == APPLIED_MODE,
            "apply_meta did not set the permission bits",
        )?;
    }
    let time_reported = report.not_transferred.iter().any(|name| name == "modified");
    ensure(
        time_reported || applied.basic.modified_ns == wanted.basic.modified_ns,
        "apply_meta did not set the modification time",
    )
}

fn check_truncate(ctx: &Ctx<'_>) -> Check {
    let file = ctx.file("file", b"0123456789")?;
    ctx.fs.truncate(ctx.scope, &file, 4)?;
    ctx.content_is(&file, b"0123", "shrunk file")?;
    ctx.fs.truncate(ctx.scope, &file, 8)?;
    ctx.content_is(&file, b"0123\0\0\0\0", "grown file")?;
    expect_kind(
        ctx.fs.truncate(ctx.scope, &ctx.path("missing")?, 1),
        FsErrorKind::NotFound,
        "truncation of a missing file",
    )
}

fn check_sync(ctx: &Ctx<'_>) -> Check {
    let path = ctx.path("file")?;
    let mut handle = ctx.fs.create_new(ctx.scope, &path)?;
    handle.write_all(b"data")?;
    handle.sync(Durability::Data)?;
    handle.sync(Durability::Full)?;
    drop(handle);
    ctx.fs.sync_dir(ctx.scope, &ctx.dir)?;
    ctx.content_is(&path, b"data", "flushed file")
}

fn check_lock_exclusive(ctx: &Ctx<'_>) -> Check {
    let path = ctx.path("lock")?;
    let first = ctx.fs.lock_exclusive(&path, LOCK_TIMEOUT)?;
    ensure(first.path() == &path, "lock reports a different path")?;
    ensure(
        ctx.fs.metadata(&path, Follow::NoFollow)?.kind == FileKind::File,
        "lock file was not created",
    )?;
    let started = std::time::Instant::now();
    expect_kind(
        ctx.fs.lock_exclusive(&path, LOCK_WAIT),
        FsErrorKind::InUse,
        "second exclusive lock",
    )?;
    ensure(
        started.elapsed() >= LOCK_WAIT / 2,
        "second exclusive lock failed without waiting",
    )?;
    drop(first);
    ctx.fs.lock_exclusive(&path, LOCK_TIMEOUT)?;
    Ok(())
}

fn check_mark_hidden(ctx: &Ctx<'_>) -> Check {
    let dir = ctx.dir("hidden")?;
    ctx.fs.mark_hidden(ctx.scope, &dir)?;
    ctx.fs.metadata(&dir, Follow::NoFollow)?;
    expect_kind(
        ctx.fs.mark_hidden(ctx.scope, &ctx.path("missing")?),
        FsErrorKind::NotFound,
        "marking a missing entry",
    )
}

fn check_read_dir(ctx: &Ctx<'_>) -> Check {
    for name in ["b", "a", "c"] {
        ctx.file(name, name.as_bytes())?;
    }
    ctx.dir("d")?;
    let mut entries = ctx.fs.read_dir(&ctx.dir)?.collect::<FsResult<Vec<_>>>()?;
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let names: Vec<String> = entries
        .iter()
        .map(|entry| entry.name.to_string_lossy().into_owned())
        .collect();
    ensure(
        names == ["a", "b", "c", "d"],
        "read_dir returned other names",
    )?;
    let kinds_ok = entries.iter().all(|entry| {
        let expected = if entry.name == "d" {
            FileKind::Directory
        } else {
            FileKind::File
        };
        entry.kind.is_none_or(|kind| kind == expected)
    });
    ensure(kinds_ok, "read_dir reported a wrong kind")?;
    expect_kind(
        ctx.fs.read_dir(&ctx.path("a")?).map(|_| ()),
        FsErrorKind::NotADirectory,
        "read_dir on a file",
    )?;
    expect_kind(
        ctx.fs.read_dir(&ctx.path("missing")?).map(|_| ()),
        FsErrorKind::NotFound,
        "read_dir on a missing directory",
    )
}

fn check_content_hash(ctx: &Ctx<'_>) -> Check {
    let empty = ctx.file("empty", b"")?;
    let abc = ctx.file("abc", b"abc")?;
    for (path, expected) in [(&empty, EMPTY_SHA256), (&abc, ABC_SHA256)] {
        let hash = ctx.fs.content_hash(path)?;
        let known =
            ContentHash::parse(expected).map_err(|error| Problem::Fail(error.to_string()))?;
        ensure(
            hash == known,
            "content_hash differs from the SHA-256 test vector",
        )?;
    }
    expect_kind(
        ctx.fs.content_hash(&ctx.dir),
        FsErrorKind::IsADirectory,
        "content_hash of a directory",
    )
}

fn check_volume(ctx: &Ctx<'_>) -> Check {
    let volume = ctx.fs.volume(&ctx.dir)?;
    ensure(
        ctx.dir.starts_with(&volume.mount),
        "test directory lies outside the reported mount point",
    )?;
    let deeper = ctx.path("missing")?.join_name("deeper")?;
    ensure(
        ctx.fs.volume(&deeper)?.id == volume.id,
        "a missing path is attributed to another volume",
    )
}

fn check_space(ctx: &Ctx<'_>) -> Check {
    let volume = ctx.fs.volume(&ctx.dir)?;
    let space = ctx.fs.space(&volume.id)?;
    ensure(space.total > 0, "volume reports no capacity")?;
    ensure(
        space.available <= space.total,
        "available space exceeds the capacity",
    )?;
    ensure(
        space
            .quota_remaining
            .is_none_or(|left| left >= space.available),
        "available space exceeds the remaining quota",
    )
}

fn check_path_key(ctx: &Ctx<'_>) -> Check {
    let mixed = ctx.fs.path_key(&ctx.path("Name\u{e9}")?)?;
    let upper = ctx.fs.path_key(&ctx.path("NAME\u{c9}")?)?;
    let insensitive = !ctx.features.case_sensitive;
    ensure(
        (mixed == upper) == insensitive,
        "case variants share a key exactly when the volume is case-sensitive",
    )?;
    if insensitive {
        let decomposed = ctx.fs.path_key(&ctx.path("name\u{65}\u{301}")?)?;
        ensure(
            decomposed == mixed,
            "normalisation variants differ on a case-insensitive volume",
        )?;
    }
    Ok(())
}

fn check_holders(ctx: &Ctx<'_>) -> Check {
    let file = ctx.file("file", b"x")?;
    ctx.fs.holders(&file)?;
    expect_kind(
        ctx.fs.holders(&ctx.path("missing")?),
        FsErrorKind::NotFound,
        "holders of a missing entry",
    )
}

fn check_can_write_dir(ctx: &Ctx<'_>) -> Check {
    ensure(
        ctx.fs.can_write_dir(&ctx.dir)?,
        "the test directory is reported as not writable",
    )?;
    let file = ctx.file("file", b"x")?;
    expect_kind(
        ctx.fs.can_write_dir(&file),
        FsErrorKind::NotADirectory,
        "can_write_dir on a file",
    )?;
    expect_kind(
        ctx.fs.can_write_dir(&ctx.path("missing")?),
        FsErrorKind::NotFound,
        "can_write_dir on a missing directory",
    )
}

fn check_error_kinds(ctx: &Ctx<'_>) -> Check {
    let missing = ctx.path("missing")?;
    expect_kind(
        ctx.fs.metadata(&missing, Follow::NoFollow),
        FsErrorKind::NotFound,
        "metadata of a missing entry",
    )?;
    let file = ctx.file("file", b"x")?;
    expect_kind(
        ctx.fs.metadata(&file.join_name("child")?, Follow::NoFollow),
        FsErrorKind::NotADirectory,
        "metadata below a file",
    )?;
    expect_kind(
        ctx.fs.open_read(&ctx.dir).map(|_| ()),
        FsErrorKind::IsADirectory,
        "open_read on a directory",
    )?;
    expect_kind(
        ctx.fs
            .create_new(ctx.scope, &missing.join_name("child")?)
            .map(|_| ()),
        FsErrorKind::NotFound,
        "create_new below a missing directory",
    )
}

fn check_cross_device(ctx: &Ctx<'_>) -> Check {
    let Some(other) = ctx.other else {
        return Err(Problem::Skip("no second volume configured".to_owned()));
    };
    let file = ctx.file("file", b"x")?;
    let remote = other.join_name("fs-conf-cross-device")?;
    expect_kind(
        ctx.fs
            .rename(ctx.scope, &file, &remote, RenameMode::Replace),
        FsErrorKind::CrossDevice,
        "rename to another volume",
    )?;
    if ctx.features.hardlink {
        expect_kind(
            ctx.fs.hard_link(ctx.scope, &file, &remote),
            FsErrorKind::CrossDevice,
            "hard link to another volume",
        )?;
    }
    let copied = ctx.fs.copy_file(ctx.scope, &file, &remote)?;
    ensure(
        copied == 1,
        "copy to another volume reported a wrong byte count",
    )
}

fn check_streams(ctx: &Ctx<'_>) -> Check {
    let file = ctx.file("file", b"1")?;
    let result = ctx.fs.open_stream_write(ctx.scope, &file, "extra");
    let mut handle = declared(ctx.features.alternate_streams, result, "alternate streams")?;
    handle.write_all(b"xyz")?;
    drop(handle);
    let mut seen = Vec::new();
    ctx.fs
        .open_stream_read(&file, "extra")?
        .read_to_end(&mut seen)?;
    ensure(seen == b"xyz", "stream content differs")?;
    let listed = ctx.fs.extended_metadata(&file, Follow::NoFollow)?;
    ensure(
        listed
            .streams
            .iter()
            .any(|stream| stream.name == "extra" && stream.size == 3),
        "stream missing from extended metadata",
    )?;
    let copy = ctx.path("copy")?;
    let copied = ctx.fs.copy_file(ctx.scope, &file, &copy)?;
    ensure(copied == 4, "copy_file did not count the stream bytes")?;
    expect_kind(
        ctx.fs.open_stream_read(&file, "absent").map(|_| ()),
        FsErrorKind::NotFound,
        "missing stream",
    )
}

fn check_real_path(ctx: &Ctx<'_>) -> Check {
    let dir = ctx.dir("Dir")?;
    let file = dir.join_name("file")?;
    ctx.write_new(&file, b"x")?;
    let link = ctx.link(dir.as_path(), "link", LinkKind::SymlinkDirectory)?;
    let direct = ctx.fs.real_path(&file)?;
    let through = ctx.fs.real_path(&link.join_name("file")?)?;
    ensure(direct == through, "real_path did not resolve the link")?;
    if !ctx.features.case_sensitive {
        let respelled = ctx.fs.real_path(&ctx.path("DIR")?.join_name("FILE")?)?;
        ensure(
            respelled == direct,
            "real_path did not return the stored spelling",
        )?;
    }
    Ok(())
}
