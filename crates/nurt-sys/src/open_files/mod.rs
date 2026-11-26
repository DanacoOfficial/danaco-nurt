// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Processes that hold a file or directory open: the `open_files` command
//! and the [`SystemFileHolders`] service.
//!
//! | System | Source | Access kinds in `os_raw.access` |
//! |---|---|---|
//! | Linux | `/proc/<pid>/fd/*`, `cwd`, `root`, `exe` and `maps` | `fd`, `cwd`, `root`, `exe`, `mmap` |
//! | macOS | `proc_pidinfo` (`PROC_PIDLISTFDS`, `PROC_PIDVNODEPATHINFO`, `PROC_PIDREGIONPATHINFO`), `proc_pidfdinfo`, `proc_pidpath` | `fd`, `cwd`, `root`, `exe`, `mmap` |
//! | Windows | Restart Manager (`RmStartSession`, `RmRegisterResources`, `RmGetList`) | not reported; `os_raw.kind` holds the application type |
//!
//! The path must be absolute and must exist. A file matches the processes
//! that use that file; a directory matches the processes that use the
//! directory or any path below it. A file deleted while a process keeps it
//! open matches a directory that contained it and is marked with
//! `os_raw.deleted`. Processes whose open files cannot be read are skipped;
//! the report carries one `permission_denied_skipped` note with their number.
//! On Windows a directory is checked through the files below it, up to
//! 10 000; beyond that the report carries a `target_count_high` note.
//!
//! [`SystemFileHolders`] answers the same question for the file layer, which
//! reports the holders of a file with `E_IN_USE`.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows;

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::Process;
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::services::{FileHolder, FileHolderService};
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Record, Value};

#[cfg(target_os = "linux")]
use self::linux as platform;
#[cfg(target_os = "macos")]
use self::macos as platform;
#[cfg(windows)]
use self::windows as platform;
use crate::common::{REQUIRED_PRIVILEGE, SysRoot, source_error};
use crate::processes::model::{ProcessSource, ReadRequest};
use crate::processes::source_under;

/// Name of the command.
pub(crate) const COMMAND: &str = "open_files";
/// Parameter with the file or directory.
pub(crate) const PATH: &str = "path";
/// Marker the kernel appends to the path of a deleted file.
#[cfg(unix)]
const DELETED_SUFFIX: &str = " (deleted)";

/// How a process uses a matching path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(
    windows,
    expect(
        dead_code,
        reason = "the Restart Manager does not report the kind of access"
    )
)]
pub(crate) enum Access {
    /// Open file descriptor.
    Fd,
    /// Working directory.
    Cwd,
    /// Root directory.
    Root,
    /// Executable file.
    Exe,
    /// File mapped into memory.
    Mmap,
}

impl Access {
    /// Name used in `os_raw.access`.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Fd => "fd",
            Self::Cwd => "cwd",
            Self::Root => "root",
            Self::Exe => "exe",
            Self::Mmap => "mmap",
        }
    }
}

/// File or directory whose holders are searched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    /// Absolute path with symbolic links resolved.
    pub(crate) path: PathBuf,
    /// The path is a directory and matches its subtree.
    pub(crate) directory: bool,
}

impl Target {
    /// Target at `path`.
    ///
    /// # Errors
    ///
    /// Fails with `E_PATH_RELATIVE` for a relative path, `E_NOT_FOUND` when
    /// the path does not exist and `E_PRIVILEGE` or `E_IO` when it cannot be
    /// resolved.
    pub(crate) fn resolve(path: &Path) -> Result<Self> {
        if !path.is_absolute() {
            return Err(Error::new(
                ErrorCode::PathRelative,
                format!("`{}` is relative; give an absolute path", path.display()),
            )
            .with_found(path.display().to_string()));
        }
        let resolved = std::fs::canonicalize(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Error::new(
                    ErrorCode::NotFound,
                    format!("`{}` does not exist; check the path", path.display()),
                )
                .with_found(path.display().to_string())
            } else {
                source_error(&error, &path.display().to_string())
            }
        })?;
        let directory = resolved.is_dir();
        Ok(Self {
            path: resolved,
            directory,
        })
    }

    /// Returns `true` when `candidate` is the target or, for a directory,
    /// lies below it.
    pub(crate) fn matches(&self, candidate: &Path) -> bool {
        if self.directory {
            candidate.starts_with(&self.path)
        } else {
            candidate == self.path
        }
    }
}

/// Process found to hold the target.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Holder {
    /// Process identifier.
    pub(crate) pid: u32,
    /// Process or application name.
    pub(crate) name: Option<String>,
    /// Application type reported by the system.
    pub(crate) kind: Option<String>,
    /// Kinds of access to matching paths.
    pub(crate) access: BTreeSet<Access>,
    /// Matching paths in use.
    pub(crate) files: BTreeSet<String>,
    /// One of the matching files was deleted while open.
    pub(crate) deleted: bool,
}

impl Holder {
    /// Process `pid` with no matching path yet.
    pub(crate) fn new(pid: u32) -> Self {
        Self {
            pid,
            ..Self::default()
        }
    }

    /// Records `link`, a path used by the process with `access`, when it
    /// matches `target`.
    ///
    /// A deleted file matches only a directory target: a file now at the
    /// same path is another file.
    ///
    /// The link is compared as bytes, so paths that are not valid UTF-8
    /// match as well.
    #[cfg(unix)]
    pub(crate) fn check(
        &mut self,
        target: &Target,
        access: Access,
        link: impl AsRef<std::ffi::OsStr>,
    ) {
        use std::os::unix::ffi::OsStrExt;

        let bytes = link.as_ref().as_bytes();
        let (path, deleted) = bytes
            .strip_suffix(DELETED_SUFFIX.as_bytes())
            .map_or((bytes, false), |path| (path, true));
        let path = Path::new(std::ffi::OsStr::from_bytes(path));
        if (deleted && !target.directory) || !target.matches(path) {
            return;
        }
        self.access.insert(access);
        self.files.insert(path.to_string_lossy().into_owned());
        self.deleted |= deleted;
    }

    /// Returns `true` when the process holds the target.
    pub(crate) fn is_holding(&self) -> bool {
        !self.access.is_empty() || self.kind.is_some()
    }

    /// Attributes added to `os_raw` of the `process` record.
    pub(crate) fn raw(&self) -> IndexMap<String, Value> {
        let mut raw = IndexMap::new();
        if !self.access.is_empty() {
            let access = self
                .access
                .iter()
                .map(|access| Value::Text(access.as_str().to_owned()))
                .collect();
            raw.insert("access".to_owned(), Value::List(access));
        }
        if !self.files.is_empty() {
            let files = self.files.iter().cloned().map(Value::Path).collect();
            raw.insert("files".to_owned(), Value::List(files));
        }
        if self.deleted {
            raw.insert("deleted".to_owned(), Value::Bool(true));
        }
        if let Some(kind) = &self.kind {
            raw.insert("kind".to_owned(), Value::Text(kind.clone()));
        }
        raw
    }
}

/// Result of one search for holders.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Scan {
    /// Processes that hold the target, by identifier.
    pub(crate) holders: Vec<Holder>,
    /// Number of processes whose open files could not be read.
    pub(crate) denied: u64,
    /// Number of files checked when a directory had more files than the
    /// system can check at once.
    pub(crate) truncated: Option<u64>,
}

/// The `open_files` command.
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_sys::open_files::OpenFilesStage;
///
/// assert_eq!(OpenFilesStage::new().descriptor().command, "open_files");
/// ```
#[derive(Debug, Clone, Default)]
pub struct OpenFilesStage {
    root: SysRoot,
}

impl OpenFilesStage {
    /// The command inspecting the processes of the running system.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The command reading process directories under `root`.
    #[must_use]
    pub fn with_root(root: SysRoot) -> Self {
        Self { root }
    }
}

impl StageImpl for OpenFilesStage {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for OpenFilesStage {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let target = target_from_args(args)?;
        ctx.checkpoint()?;
        let scan = platform::scan(&self.root, &target)?;
        report(ctx, &scan);
        let source = source_under(&self.root);
        let records: Vec<Record> =
            with_raw(holder_processes(source.as_ref(), &scan.holders)?, &scan)
                .into_iter()
                .map(Record::from)
                .collect();
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

/// Processes holding files, for the file layer and for embedding
/// applications.
///
/// ```
/// use std::sync::Arc;
///
/// use nurt_contracts::services::Services;
/// use nurt_sys::open_files::SystemFileHolders;
///
/// let services = Services::new().with_file_holders(Arc::new(SystemFileHolders::new()));
/// assert!(services.file_holders().is_some());
/// ```
#[derive(Debug, Clone, Default)]
pub struct SystemFileHolders {
    root: SysRoot,
}

impl SystemFileHolders {
    /// The service inspecting the processes of the running system.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The service reading process directories under `root`.
    #[must_use]
    pub fn with_root(root: SysRoot) -> Self {
        Self { root }
    }
}

impl FileHolderService for SystemFileHolders {
    fn holders(&self, path: &Path) -> Result<Vec<FileHolder>> {
        let target = Target::resolve(path)?;
        let scan = platform::scan(&self.root, &target)?;
        Ok(scan
            .holders
            .into_iter()
            .map(|holder| FileHolder {
                pid: holder.pid,
                name: holder.name.unwrap_or_default(),
                kind: holder.kind,
            })
            .collect())
    }
}

fn target_from_args(args: &BoundArgs) -> Result<Target> {
    let path = args
        .require(PATH)?
        .as_str()
        .ok_or_else(|| Error::new(ErrorCode::Type, "`open_files` expects a path for `path`"))?;
    Target::resolve(Path::new(path))
}

/// `process` records of `holders`, read by the process reader of the
/// `processes` command; processes that ended meanwhile are left out.
///
/// # Errors
///
/// Fails when the process table cannot be read on this system.
pub(crate) fn holder_processes(
    source: &dyn ProcessSource,
    holders: &[Holder],
) -> Result<Vec<Process>> {
    if holders.is_empty() {
        return Ok(Vec::new());
    }
    let mut reader = source.open(ReadRequest { cwd: true })?;
    Ok(holders
        .iter()
        .filter_map(|holder| reader.read(u64::from(holder.pid)))
        .map(|process| process.into_record(None, None))
        .collect())
}

/// Adds the attributes of each holder to `os_raw` of its record.
fn with_raw(mut processes: Vec<Process>, scan: &Scan) -> Vec<Process> {
    let raws: HashMap<u64, IndexMap<String, Value>> = scan
        .holders
        .iter()
        .map(|holder| (u64::from(holder.pid), holder.raw()))
        .collect();
    for process in &mut processes {
        if let Some(raw) = raws.get(&process.pid) {
            process
                .os_raw
                .get_or_insert_with(IndexMap::new)
                .extend(raw.clone());
        }
    }
    processes
}

/// Notes about processes that were skipped and directories checked in part.
pub(crate) fn report(ctx: &ExecCtx, scan: &Scan) {
    if scan.denied > 0 {
        ctx.note(NotableEntry {
            count: Some(scan.denied),
            detail: Some(format!(
                "the open files of {} processes could not be read; run as {REQUIRED_PRIVILEGE} \
                 to include them",
                scan.denied
            )),
            ..NotableEntry::new(Notable::PermissionDeniedSkipped)
        });
    }
    if let Some(checked) = scan.truncated {
        ctx.note(NotableEntry {
            count: Some(checked),
            detail: Some(format!(
                "only the first {checked} files below the directory were checked"
            )),
            ..NotableEntry::new(Notable::TargetCountHigh)
        });
    }
}
