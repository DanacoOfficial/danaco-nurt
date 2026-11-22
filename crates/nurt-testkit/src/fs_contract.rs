// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conformance checks of the file system interface.
//!
//! The checks exercise the behaviour that the contract of [`Fs`] and
//! [`FsWrite`] fixes for every implementation: error kinds of missing
//! entries and wrong entry kinds, directory listings without `.` and `..`,
//! exclusive creation, renames, links, entity tags, volumes and locks. They
//! run on the virtual file system of this crate and on the platform back
//! ends, so that both are held to the same rules.
//!
//! [`FsContract::run`] executes every check of [`CHECKS`] in its own
//! subdirectory of a root directory that exists when the run starts.
//! Operations the volume does not offer (hard links, symbolic links without
//! the privilege) make a check report [`CheckOutcome::Skipped`] instead of a
//! failure.
//!
//! ```
//! use std::sync::Arc;
//! use nurt_testkit::clock::VirtualClock;
//! use nurt_testkit::fs_contract::FsContract;
//! use nurt_testkit::vfs::{VirtualFs, test_path};
//!
//! let fs = VirtualFs::new(Arc::new(VirtualClock::new()));
//! let root = test_path("/contract")?;
//! fs.create_dir_all(&root)?;
//! let scope = FsContract::scope();
//! let report = FsContract::new(&fs, root, &scope).run();
//! assert!(report.is_success(), "{report}");
//! # Ok::<(), nurt_contracts::fs::FsError>(())
//! ```

use std::fmt;
use std::io::{Read, Seek, SeekFrom, Write};
use std::time::Duration;

use nurt_contracts::fs::{
    AbsPath, Durability, FileKind, Follow, FsErrorKind, FsResult, FsWrite, LinkKind, MetaChange,
    RenameMode, ScopePurpose, TxScope,
};
use nurt_contracts::journal::OpId;

use crate::hash::sha256;

/// Content written by the checks; eight bytes.
const CONTENT: &[u8] = b"contract";

/// Modification time set by the check of [`FsWrite::set_meta`]: a whole
/// second, representable on every supported file system.
const FIXED_MODIFIED_NS: i64 = 1_600_000_000_000_000_000;

/// Time a second lock request waits before it reports the lock as held.
const LOCK_WAIT: Duration = Duration::from_millis(50);

/// Permission bits of the directories created by the checks.
const DIR_MODE: u32 = 0o755;

/// Result of one check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// The implementation behaves as the contract requires.
    Passed,
    /// The volume does not offer the checked operation.
    Skipped {
        /// Why the check did not run.
        reason: String,
    },
    /// The implementation deviates from the contract.
    Failed {
        /// The deviation.
        message: String,
    },
}

/// Outcome of one named check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    /// Name of the check, an entry of [`CHECKS`].
    pub name: &'static str,
    /// What the check found.
    pub outcome: CheckOutcome,
}

/// Outcomes of a run of the checks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContractReport {
    /// One entry per check, in the order of [`CHECKS`].
    pub checks: Vec<CheckReport>,
}

impl ContractReport {
    /// Returns `true` when no check failed.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.failures().next().is_none()
    }

    /// Failed checks.
    pub fn failures(&self) -> impl Iterator<Item = &CheckReport> {
        self.checks
            .iter()
            .filter(|check| matches!(check.outcome, CheckOutcome::Failed { .. }))
    }

    /// Names of the skipped checks.
    pub fn skipped(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.checks
            .iter()
            .filter(|check| matches!(check.outcome, CheckOutcome::Skipped { .. }))
            .map(|check| check.name)
    }

    /// Number of passed checks.
    #[must_use]
    pub fn passed(&self) -> usize {
        self.checks
            .iter()
            .filter(|check| check.outcome == CheckOutcome::Passed)
            .count()
    }
}

impl fmt::Display for ContractReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} passed, {} skipped, {} failed",
            self.passed(),
            self.skipped().count(),
            self.failures().count()
        )?;
        for check in self.failures() {
            if let CheckOutcome::Failed { message } = &check.outcome {
                write!(f, "\n{}: {message}", check.name)?;
            }
        }
        Ok(())
    }
}

/// Working directory of one check.
pub struct CheckArea<'a> {
    fs: &'a dyn FsWrite,
    scope: &'a TxScope,
    dir: AbsPath,
}

impl fmt::Debug for CheckArea<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckArea")
            .field("dir", &self.dir)
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

/// Early end of a check: a skip or a failure.
type Stop = CheckOutcome;

/// Result of a step of a check.
type Step<T> = Result<T, Stop>;

/// A check of the contract.
pub type CheckFn = fn(&CheckArea<'_>) -> CheckOutcome;

/// Every check, by name, in the order of a run.
pub const CHECKS: &[(&str, CheckFn)] = &[
    ("files_round_trip", |area| finish(files_round_trip(area))),
    ("create_new_refuses_existing", |area| {
        finish(create_new_refuses_existing(area))
    }),
    ("missing_entries", |area| finish(missing_entries(area))),
    ("directories", |area| finish(directories(area))),
    ("remove_file", |area| finish(remove_file(area))),
    ("rename", |area| finish(rename(area))),
    ("temporary_files", |area| finish(temporary_files(area))),
    ("in_place_writes", |area| finish(in_place_writes(area))),
    ("copy_file", |area| finish(copy_file(area))),
    ("symbolic_links", |area| finish(symbolic_links(area))),
    ("hard_links", |area| finish(hard_links(area))),
    ("modification_time", |area| finish(modification_time(area))),
    ("volumes", |area| finish(volumes(area))),
    ("remove_tree", |area| finish(remove_tree(area))),
    ("exclusive_locks", |area| finish(exclusive_locks(area))),
];

fn finish(result: Step<()>) -> CheckOutcome {
    match result {
        Ok(()) => CheckOutcome::Passed,
        Err(stop) => stop,
    }
}

fn failed(message: impl Into<String>) -> Stop {
    CheckOutcome::Failed {
        message: message.into(),
    }
}

fn op<T>(what: &str, result: FsResult<T>) -> Step<T> {
    result.map_err(|error| failed(format!("{what} failed: {error}")))
}

fn ensure(condition: bool, message: impl FnOnce() -> String) -> Step<()> {
    if condition {
        Ok(())
    } else {
        Err(failed(message()))
    }
}

fn expect_kind<T>(what: &str, result: FsResult<T>, kind: FsErrorKind) -> Step<()> {
    match result {
        Ok(_) => Err(failed(format!(
            "{what} succeeded; expected {}",
            kind.as_str()
        ))),
        Err(error) if error.kind == kind => Ok(()),
        Err(error) => Err(failed(format!(
            "{what} failed with {}; expected {}",
            error.kind.as_str(),
            kind.as_str()
        ))),
    }
}

impl CheckArea<'_> {
    fn path(&self, name: &str) -> Step<AbsPath> {
        op(&format!("joining `{name}`"), self.dir.join_name(name))
    }

    fn child(&self, dir: &AbsPath, name: &str) -> Step<AbsPath> {
        op(&format!("joining `{name}`"), dir.join_name(name))
    }

    fn write_new(&self, path: &AbsPath, content: &[u8]) -> Step<()> {
        let mut handle = op(
            &format!("create_new {path}"),
            self.fs.create_new(self.scope, path),
        )?;
        handle
            .write_all(content)
            .map_err(|error| failed(format!("writing {path} failed: {error}")))?;
        op(&format!("sync {path}"), handle.sync(Durability::Data))
    }

    fn read(&self, path: &AbsPath) -> Step<Vec<u8>> {
        let mut handle = op(&format!("open_read {path}"), self.fs.open_read(path))?;
        let mut content = Vec::new();
        handle
            .read_to_end(&mut content)
            .map_err(|error| failed(format!("reading {path} failed: {error}")))?;
        Ok(content)
    }

    fn mkdir(&self, path: &AbsPath) -> Step<()> {
        op(
            &format!("create_dir {path}"),
            self.fs.create_dir(self.scope, path, DIR_MODE),
        )
    }

    fn kind(&self, path: &AbsPath, follow: Follow) -> Step<FileKind> {
        op(
            &format!("metadata {path}"),
            self.fs.metadata(path, follow).map(|meta| meta.kind),
        )
    }

    fn names(&self, dir: &AbsPath) -> Step<Vec<String>> {
        let entries = op(&format!("read_dir {dir}"), self.fs.read_dir(dir))?;
        let mut names = Vec::new();
        for entry in entries {
            let entry = op(&format!("an entry of {dir}"), entry)?;
            names.push(entry.name.to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }

    fn link(&self, target: &AbsPath, link: &AbsPath, kind: LinkKind) -> Step<()> {
        match self
            .fs
            .create_link(self.scope, target.as_path(), link, kind)
        {
            Ok(()) => Ok(()),
            Err(error)
                if matches!(
                    error.kind,
                    FsErrorKind::Privilege | FsErrorKind::Unsupported
                ) =>
            {
                Err(CheckOutcome::Skipped {
                    reason: format!("symbolic links are not available: {error}"),
                })
            }
            Err(error) => Err(failed(format!("create_link {link} failed: {error}"))),
        }
    }
}

fn files_round_trip(area: &CheckArea<'_>) -> Step<()> {
    let file = area.path("a.txt")?;
    area.write_new(&file, CONTENT)?;
    let meta = op("metadata", area.fs.metadata(&file, Follow::NoFollow))?;
    ensure(meta.kind == FileKind::File, || {
        format!("a new file has the kind {:?}", meta.kind)
    })?;
    ensure(meta.size == CONTENT.len() as u64, || {
        format!("a file of 8 bytes reports {} bytes", meta.size)
    })?;
    ensure(meta.links == 1, || {
        format!("a new file reports {} links", meta.links)
    })?;
    let content = area.read(&file)?;
    ensure(content == CONTENT, || {
        "the content read differs from the content written".to_owned()
    })?;
    let handle = op("open_read", area.fs.open_read(&file))?;
    let handle_meta = op("meta of a read handle", handle.meta())?;
    ensure(handle_meta.size == meta.size, || {
        "the metadata of a read handle differs from the metadata of the path".to_owned()
    })?;
    let hash = op("content_hash", area.fs.content_hash(&file))?;
    ensure(hash == sha256(CONTENT), || {
        format!("content_hash returned {hash}, not the SHA-256 of the content")
    })?;
    let etag = op("etag", area.fs.etag(&file, Follow::Follow))?;
    ensure(etag.matches(&meta.etag()), || {
        "the entity tag differs from the tag of the metadata".to_owned()
    })
}

fn create_new_refuses_existing(area: &CheckArea<'_>) -> Step<()> {
    let file = area.path("lock")?;
    area.write_new(&file, CONTENT)?;
    expect_kind(
        "create_new of an existing file",
        area.fs.create_new(area.scope, &file),
        FsErrorKind::Exists,
    )?;
    let content = area.read(&file)?;
    ensure(content == CONTENT, || {
        "a refused create_new changed the existing file".to_owned()
    })
}

fn missing_entries(area: &CheckArea<'_>) -> Step<()> {
    let missing = area.path("missing")?;
    let file = area.path("file")?;
    area.write_new(&file, CONTENT)?;
    let below_file = area.child(&file, "child")?;
    let below_missing = area.child(&missing, "child")?;
    expect_kind(
        "metadata of a missing entry",
        area.fs.metadata(&missing, Follow::NoFollow),
        FsErrorKind::NotFound,
    )?;
    expect_kind(
        "metadata below a missing directory",
        area.fs.metadata(&below_missing, Follow::NoFollow),
        FsErrorKind::NotFound,
    )?;
    expect_kind(
        "open_read of a missing file",
        area.fs.open_read(&missing),
        FsErrorKind::NotFound,
    )?;
    expect_kind(
        "read_dir of a missing directory",
        area.fs.read_dir(&missing).map(|_| ()),
        FsErrorKind::NotFound,
    )?;
    expect_kind(
        "metadata with a file as a directory component",
        area.fs.metadata(&below_file, Follow::NoFollow),
        FsErrorKind::NotADirectory,
    )?;
    expect_kind(
        "read_dir of a file",
        area.fs.read_dir(&file).map(|_| ()),
        FsErrorKind::NotADirectory,
    )?;
    expect_kind(
        "read_link of a file",
        area.fs.read_link(&file),
        FsErrorKind::InvalidPath,
    )
}

fn directories(area: &CheckArea<'_>) -> Step<()> {
    let sub = area.path("sub")?;
    area.mkdir(&sub)?;
    ensure(
        area.kind(&sub, Follow::NoFollow)? == FileKind::Directory,
        || "a new directory does not report the kind directory".to_owned(),
    )?;
    expect_kind(
        "create_dir of an existing directory",
        area.fs.create_dir(area.scope, &sub, DIR_MODE),
        FsErrorKind::Exists,
    )?;
    let orphan = area.child(&area.path("missing")?, "orphan")?;
    expect_kind(
        "create_dir with a missing parent",
        area.fs.create_dir(area.scope, &orphan, DIR_MODE),
        FsErrorKind::NotFound,
    )?;
    expect_kind(
        "open_read of a directory",
        area.fs.open_read(&sub),
        FsErrorKind::IsADirectory,
    )?;
    area.write_new(&area.child(&sub, "b")?, CONTENT)?;
    area.write_new(&area.child(&sub, "a")?, CONTENT)?;
    area.mkdir(&area.child(&sub, "c")?)?;
    let names = area.names(&sub)?;
    ensure(names == ["a", "b", "c"], || {
        format!("read_dir listed {names:?}; expected the entries a, b and c")
    })?;
    expect_kind(
        "remove_dir of a directory with entries",
        area.fs.remove_dir(area.scope, &sub),
        FsErrorKind::NotEmpty,
    )?;
    expect_kind(
        "remove_dir of a file",
        area.fs.remove_dir(area.scope, &area.child(&sub, "a")?),
        FsErrorKind::NotADirectory,
    )?;
    let empty = area.child(&sub, "c")?;
    op("remove_dir", area.fs.remove_dir(area.scope, &empty))?;
    expect_kind(
        "metadata of a removed directory",
        area.fs.metadata(&empty, Follow::NoFollow),
        FsErrorKind::NotFound,
    )
}

fn remove_file(area: &CheckArea<'_>) -> Step<()> {
    let file = area.path("gone")?;
    area.write_new(&file, CONTENT)?;
    op("remove_file", area.fs.remove_file(area.scope, &file))?;
    expect_kind(
        "metadata of a removed file",
        area.fs.metadata(&file, Follow::NoFollow),
        FsErrorKind::NotFound,
    )?;
    expect_kind(
        "remove_file of a removed file",
        area.fs.remove_file(area.scope, &file),
        FsErrorKind::NotFound,
    )?;
    let dir = area.path("dir")?;
    area.mkdir(&dir)?;
    expect_kind(
        "remove_file of a directory",
        area.fs.remove_file(area.scope, &dir),
        FsErrorKind::IsADirectory,
    )
}

fn rename(area: &CheckArea<'_>) -> Step<()> {
    let first = area.path("first")?;
    let second = area.path("second")?;
    let third = area.path("third")?;
    area.write_new(&first, CONTENT)?;
    op(
        "rename to a new name",
        area.fs
            .rename(area.scope, &first, &second, RenameMode::Replace),
    )?;
    expect_kind(
        "metadata of a renamed source",
        area.fs.metadata(&first, Follow::NoFollow),
        FsErrorKind::NotFound,
    )?;
    ensure(area.read(&second)? == CONTENT, || {
        "a renamed file lost its content".to_owned()
    })?;
    area.write_new(&third, b"third")?;
    let features = op("volume", area.fs.volume(&third))?.features;
    if features.rename_noreplace {
        expect_kind(
            "rename without replacement onto an existing file",
            area.fs
                .rename(area.scope, &third, &second, RenameMode::NoReplace),
            FsErrorKind::Exists,
        )?;
        ensure(area.read(&second)? == CONTENT, || {
            "a refused rename changed the destination".to_owned()
        })?;
    }
    op(
        "rename onto an existing file",
        area.fs
            .rename(area.scope, &third, &second, RenameMode::Replace),
    )?;
    ensure(area.read(&second)? == b"third", || {
        "a replacing rename kept the old content".to_owned()
    })?;
    let names = area.names(&area.dir)?;
    ensure(names == ["second"], || {
        format!("after the renames the directory lists {names:?}")
    })
}

fn temporary_files(area: &CheckArea<'_>) -> Step<()> {
    let mut first = op(
        "create_temp_in",
        area.fs.create_temp_in(area.scope, &area.dir, "part"),
    )?;
    let second = op(
        "create_temp_in",
        area.fs.create_temp_in(area.scope, &area.dir, "part"),
    )?;
    let path = first.path().clone();
    ensure(&path != second.path(), || {
        "two temporary files received the same name".to_owned()
    })?;
    ensure(path.parent().as_ref() == Some(&area.dir), || {
        format!("the temporary file {path} is not in the requested directory")
    })?;
    first
        .write_all(CONTENT)
        .map_err(|error| failed(format!("writing a temporary file failed: {error}")))?;
    op("sync", first.sync(Durability::Full))?;
    drop(first);
    drop(second);
    ensure(area.read(&path)? == CONTENT, || {
        "a temporary file lost its content".to_owned()
    })?;
    let names = area.names(&area.dir)?;
    ensure(names.len() == 2, || {
        format!("two temporary files appear as {names:?}")
    })
}

fn in_place_writes(area: &CheckArea<'_>) -> Step<()> {
    let file = area.path("data")?;
    area.write_new(&file, CONTENT)?;
    let before = op("etag", area.fs.etag(&file, Follow::NoFollow))?;
    {
        let mut handle = op("open_in_place", area.fs.open_in_place(area.scope, &file))?;
        handle
            .seek(SeekFrom::Start(0))
            .and_then(|_| handle.write_all(b"XY"))
            .map_err(|error| failed(format!("writing in place failed: {error}")))?;
        op("sync", handle.sync(Durability::Data))?;
    }
    ensure(area.read(&file)? == b"XYntract", || {
        "an in-place write did not replace the first bytes only".to_owned()
    })?;
    op("truncate", area.fs.truncate(area.scope, &file, 2))?;
    let meta = op("metadata", area.fs.metadata(&file, Follow::NoFollow))?;
    ensure(meta.size == 2, || {
        format!("a file truncated to 2 bytes reports {} bytes", meta.size)
    })?;
    ensure(!before.matches(&meta.etag()), || {
        "the entity tag did not change with the size".to_owned()
    })?;
    {
        let mut handle = op("open_in_place", area.fs.open_in_place(area.scope, &file))?;
        op("set_length", handle.set_length(0))?;
    }
    ensure(area.read(&file)?.is_empty(), || {
        "set_length(0) left content".to_owned()
    })?;
    expect_kind(
        "open_in_place of a missing file",
        area.fs.open_in_place(area.scope, &area.path("missing")?),
        FsErrorKind::NotFound,
    )
}

fn copy_file(area: &CheckArea<'_>) -> Step<()> {
    let source = area.path("source")?;
    let copy = area.path("copy")?;
    area.write_new(&source, CONTENT)?;
    let copied = op("copy_file", area.fs.copy_file(area.scope, &source, &copy))?;
    ensure(copied == CONTENT.len() as u64, || {
        format!("copy_file reported {copied} bytes for a file of 8 bytes")
    })?;
    ensure(area.read(&copy)? == CONTENT, || {
        "the copy differs from the source".to_owned()
    })?;
    let source_id = op("metadata", area.fs.metadata(&source, Follow::NoFollow))?.file_id;
    let copy_id = op("metadata", area.fs.metadata(&copy, Follow::NoFollow))?.file_id;
    ensure(source_id != copy_id, || {
        "the copy shares the file identity of the source".to_owned()
    })?;
    expect_kind(
        "copy_file onto an existing file",
        area.fs.copy_file(area.scope, &source, &copy),
        FsErrorKind::Exists,
    )
}

fn symbolic_links(area: &CheckArea<'_>) -> Step<()> {
    let target = area.path("target")?;
    let link = area.path("link")?;
    area.write_new(&target, CONTENT)?;
    area.link(&target, &link, LinkKind::SymlinkFile)?;
    ensure(
        area.kind(&link, Follow::NoFollow)? == FileKind::Symlink,
        || "a symbolic link does not report the kind symlink".to_owned(),
    )?;
    ensure(area.kind(&link, Follow::Follow)? == FileKind::File, || {
        "following a link to a file does not report a file".to_owned()
    })?;
    let stored = op("read_link", area.fs.read_link(&link))?;
    ensure(stored == target.as_path(), || {
        format!("read_link returned {}", stored.display())
    })?;
    let extended = op(
        "extended_metadata",
        area.fs.extended_metadata(&link, Follow::NoFollow),
    )?;
    ensure(
        extended.link_target.as_deref() == Some(target.as_path()),
        || "the extended metadata of a link lacks its target".to_owned(),
    )?;
    let real = op("real_path", area.fs.real_path(&link))?;
    let real_target = op("real_path", area.fs.real_path(&target))?;
    ensure(real == real_target, || {
        format!("real_path of the link is {real}, of the target {real_target}")
    })?;
    ensure(area.read(&link)? == CONTENT, || {
        "reading through a link does not read the target".to_owned()
    })?;
    let looping = area.path("loop")?;
    area.link(&looping, &looping, LinkKind::SymlinkFile)?;
    expect_kind(
        "metadata following a link to itself",
        area.fs.metadata(&looping, Follow::Follow),
        FsErrorKind::Loop,
    )?;
    op(
        "remove_file of a link",
        area.fs.remove_file(area.scope, &link),
    )?;
    ensure(area.read(&target)? == CONTENT, || {
        "removing a link changed its target".to_owned()
    })
}

fn hard_links(area: &CheckArea<'_>) -> Step<()> {
    let first = area.path("first")?;
    let second = area.path("second")?;
    area.write_new(&first, CONTENT)?;
    if !op("volume", area.fs.volume(&first))?.features.hardlink {
        return Err(CheckOutcome::Skipped {
            reason: "the volume has no hard links".to_owned(),
        });
    }
    op("hard_link", area.fs.hard_link(area.scope, &first, &second))?;
    let one = op("metadata", area.fs.metadata(&first, Follow::NoFollow))?;
    let two = op("metadata", area.fs.metadata(&second, Follow::NoFollow))?;
    ensure(one.links == 2 && two.links == 2, || {
        format!(
            "two names of a file report {} and {} links",
            one.links, two.links
        )
    })?;
    ensure(one.file_id == two.file_id, || {
        "two names of a file report different identities".to_owned()
    })?;
    {
        let mut handle = op("open_in_place", area.fs.open_in_place(area.scope, &second))?;
        handle
            .write_all(b"C")
            .map_err(|error| failed(format!("writing in place failed: {error}")))?;
    }
    ensure(area.read(&first)? == b"Contract", || {
        "an in-place write through one name is not visible through the other".to_owned()
    })?;
    expect_kind(
        "hard_link onto an existing name",
        area.fs.hard_link(area.scope, &first, &second),
        FsErrorKind::Exists,
    )?;
    op("remove_file", area.fs.remove_file(area.scope, &second))?;
    let left = op("metadata", area.fs.metadata(&first, Follow::NoFollow))?;
    ensure(left.links == 1, || {
        format!(
            "after removing one name the file reports {} links",
            left.links
        )
    })
}

fn modification_time(area: &CheckArea<'_>) -> Step<()> {
    let file = area.path("dated")?;
    area.write_new(&file, CONTENT)?;
    let before = op("etag", area.fs.etag(&file, Follow::NoFollow))?;
    let change = MetaChange {
        modified_ns: Some(FIXED_MODIFIED_NS),
        ..MetaChange::default()
    };
    op("set_meta", area.fs.set_meta(area.scope, &file, &change))?;
    let meta = op("metadata", area.fs.metadata(&file, Follow::NoFollow))?;
    ensure(meta.modified_ns == FIXED_MODIFIED_NS, || {
        format!(
            "set_meta stored the modification time {}; expected {FIXED_MODIFIED_NS}",
            meta.modified_ns
        )
    })?;
    ensure(
        before.modified_ns == FIXED_MODIFIED_NS || !before.matches(&meta.etag()),
        || "the entity tag did not change with the modification time".to_owned(),
    )
}

fn volumes(area: &CheckArea<'_>) -> Step<()> {
    let file = area.path("file")?;
    let other = area.path("other")?;
    area.write_new(&file, CONTENT)?;
    area.write_new(&other, CONTENT)?;
    let deep = area.child(&area.path("missing")?, "deeper")?;
    let volume = op(
        "volume of a path below a missing directory",
        area.fs.volume(&deep),
    )?;
    let same = op("same_volume", area.fs.same_volume(&area.dir, &file))?;
    ensure(same, || {
        "a file and its directory are reported on different volumes".to_owned()
    })?;
    let space = op("space", area.fs.space(&volume.id))?;
    ensure(space.available <= space.total, || {
        format!(
            "the volume reports {} bytes available of {} in total",
            space.available, space.total
        )
    })?;
    let key = op("path_key", area.fs.path_key(&file))?;
    ensure(key == op("path_key", area.fs.path_key(&file))?, || {
        "path_key is not stable for one path".to_owned()
    })?;
    ensure(key != op("path_key", area.fs.path_key(&other))?, || {
        "two files have the same path key".to_owned()
    })?;
    ensure(
        op("can_write_dir", area.fs.can_write_dir(&area.dir))?,
        || "the check directory is reported as not writable".to_owned(),
    )?;
    expect_kind(
        "can_write_dir of a file",
        area.fs.can_write_dir(&file),
        FsErrorKind::NotADirectory,
    )?;
    op("holders", area.fs.holders(&file))?;
    expect_kind(
        "holders of a missing entry",
        area.fs.holders(&area.path("missing")?),
        FsErrorKind::NotFound,
    )
}

fn remove_tree(area: &CheckArea<'_>) -> Step<()> {
    let keep = area.path("keep")?;
    let kept = area.child(&keep, "kept")?;
    let tree = area.path("tree")?;
    let sub = area.child(&tree, "sub")?;
    area.mkdir(&keep)?;
    area.write_new(&kept, CONTENT)?;
    area.mkdir(&tree)?;
    area.mkdir(&sub)?;
    area.write_new(&area.child(&tree, "one")?, b"abc")?;
    area.write_new(&area.child(&sub, "two")?, b"defgh")?;
    let linked = match area.link(
        &keep,
        &area.child(&tree, "outside")?,
        LinkKind::SymlinkDirectory,
    ) {
        Ok(()) => true,
        Err(CheckOutcome::Skipped { .. }) => false,
        Err(stop) => return Err(stop),
    };
    let stats = op("remove_tree", area.fs.remove_tree(area.scope, &tree, 2))?;
    expect_kind(
        "metadata of a removed tree",
        area.fs.metadata(&tree, Follow::NoFollow),
        FsErrorKind::NotFound,
    )?;
    ensure(stats.directories == 2, || {
        format!(
            "remove_tree reported {} directories; expected 2",
            stats.directories
        )
    })?;
    ensure(stats.files >= 2, || {
        format!(
            "remove_tree reported {} files; expected at least 2",
            stats.files
        )
    })?;
    ensure(!linked || area.read(&kept)? == CONTENT, || {
        "remove_tree followed a symbolic link out of the tree".to_owned()
    })
}

fn exclusive_locks(area: &CheckArea<'_>) -> Step<()> {
    let path = area.path("lock")?;
    let held = op("lock_exclusive", area.fs.lock_exclusive(&path, LOCK_WAIT))?;
    ensure(held.path() == &path, || {
        format!("the lock reports the path {}", held.path())
    })?;
    expect_kind(
        "a second lock_exclusive of a held lock",
        area.fs.lock_exclusive(&path, LOCK_WAIT).map(|_| ()),
        FsErrorKind::InUse,
    )?;
    drop(held);
    op(
        "lock_exclusive after release",
        area.fs.lock_exclusive(&path, LOCK_WAIT).map(|_| ()),
    )
}

/// Runner of the conformance checks on one implementation.
pub struct FsContract<'a> {
    fs: &'a dyn FsWrite,
    root: AbsPath,
    scope: &'a TxScope,
}

impl fmt::Debug for FsContract<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FsContract")
            .field("root", &self.root)
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl<'a> FsContract<'a> {
    /// Runner for `fs` below the existing directory `root`, with mutations
    /// made under `scope`.
    #[must_use]
    pub fn new(fs: &'a dyn FsWrite, root: AbsPath, scope: &'a TxScope) -> Self {
        Self { fs, root, scope }
    }

    /// Scope for a run outside a journal, belonging to the operation 0.
    // The checks stand in for the journal, the only holder of scopes in
    // production, so they create the scope with the reserved constructor.
    #[allow(clippy::disallowed_methods)]
    #[must_use]
    pub fn scope() -> TxScope {
        TxScope::new_for_journal(OpId(0), ScopePurpose::Operation)
    }

    /// Runs every check of [`CHECKS`], each in a new subdirectory of the
    /// root named after the check.
    #[must_use]
    pub fn run(&self) -> ContractReport {
        ContractReport {
            checks: CHECKS
                .iter()
                .map(|(name, check)| CheckReport {
                    name,
                    outcome: self.run_one(name, *check),
                })
                .collect(),
        }
    }

    /// Runs the check `name` in a new subdirectory of the root; `None` when
    /// no check has this name.
    #[must_use]
    pub fn run_check(&self, name: &str) -> Option<CheckOutcome> {
        CHECKS
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(name, check)| self.run_one(name, *check))
    }

    fn run_one(&self, name: &str, check: CheckFn) -> CheckOutcome {
        let dir = match self.root.join_name(name) {
            Ok(dir) => dir,
            Err(error) => return failed(format!("joining `{name}` failed: {error}")),
        };
        if let Err(error) = self.fs.create_dir(self.scope, &dir, DIR_MODE) {
            return failed(format!("create_dir {dir} failed: {error}"));
        }
        check(&CheckArea {
            fs: self.fs,
            scope: self.scope,
            dir,
        })
    }
}

impl CheckArea<'_> {
    /// Working directory of the check.
    #[must_use]
    pub fn dir(&self) -> &AbsPath {
        &self.dir
    }

    /// File system under check.
    #[must_use]
    pub fn fs(&self) -> &dyn FsWrite {
        self.fs
    }

    /// Scope of the mutations.
    #[must_use]
    pub fn scope(&self) -> &TxScope {
        self.scope
    }
}
