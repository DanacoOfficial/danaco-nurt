// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Data types of a working tree update plan and of its execution report.

use bstr::BString;
use gix::ObjectId;
use nurt_contracts::journal::Expect;

/// Change of one working tree entry, in the vocabulary of the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryAction {
    /// A regular file is created where no entry exists.
    Create,
    /// The content of an existing regular file is replaced; the mode may
    /// change as well.
    Modify,
    /// A file, symbolic link or ignored directory tree is removed.
    Delete,
    /// A directory is created.
    Mkdir,
    /// An empty directory is removed.
    Rmdir,
    /// A symbolic link is created where no entry exists.
    Symlink,
    /// Only the executable bit of a regular file changes.
    Chmod,
    /// An entry is replaced by an entry of another kind (file, directory or
    /// symbolic link).
    TypeChange,
}

impl EntryAction {
    /// Stable name used in reports and previews.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Modify => "modify",
            Self::Delete => "delete",
            Self::Mkdir => "mkdir",
            Self::Rmdir => "rmdir",
            Self::Symlink => "symlink",
            Self::Chmod => "chmod",
            Self::TypeChange => "type_change",
        }
    }
}

/// Kind of the entry a plan entry leaves in the working tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryKind {
    /// Regular file without the executable bit.
    File,
    /// Regular file with the executable bit.
    Executable,
    /// Symbolic link; written as a regular file holding the link text when
    /// `core.symlinks` is false.
    Symlink,
    /// Directory, also the placeholder directory of a submodule.
    Directory,
}

impl EntryKind {
    /// Kind of the working tree entry for an index or tree entry mode;
    /// `None` for modes that have no working tree representation.
    pub fn from_index_mode(mode: gix_index::entry::Mode) -> Option<Self> {
        match mode {
            gix_index::entry::Mode::FILE => Some(Self::File),
            gix_index::entry::Mode::FILE_EXECUTABLE => Some(Self::Executable),
            gix_index::entry::Mode::SYMLINK => Some(Self::Symlink),
            gix_index::entry::Mode::COMMIT | gix_index::entry::Mode::DIR => Some(Self::Directory),
            _ => None,
        }
    }
}

/// New content of a plan entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// A blob of the object database, converted to the working tree form by
    /// the smudge filters of its path when it is written.
    Blob(ObjectId),
    /// Bytes already in working tree form, written verbatim.
    Bytes(Vec<u8>),
}

/// One change of the working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEntry {
    /// Path relative to the root of the working tree, with `/` separators.
    pub path: BString,
    /// Change.
    pub action: EntryAction,
    /// State the entry must have immediately before the change, observed
    /// when the plan was made.
    pub expect: Expect,
    /// New content; present for every action that writes a file or a link.
    pub content: Option<Content>,
    /// Kind of the resulting entry; `None` for removals.
    pub kind: Option<EntryKind>,
}

/// The index that the execution writes.
#[derive(Debug, Clone)]
pub struct IndexPlan {
    /// New index state. Statistics of entries whose files the plan writes are
    /// refreshed after the write.
    pub state: gix_index::State,
    /// Expected state of the index file, observed when the plan was made.
    pub expect: Expect,
    /// Whether conflict stages resolved by the plan are recorded in the
    /// resolve-undo extension. Set for the resolution of single paths; plans
    /// that reset the index to a tree leave it unset, as `git` clears the
    /// extension then.
    pub record_resolve_undo: bool,
}

/// New value of a reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefTarget {
    /// The reference points directly at an object.
    Object(ObjectId),
    /// The reference is symbolic and names another reference, for example
    /// `refs/heads/main`.
    Symbolic(String),
}

/// One line appended to a reference log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflogLine {
    /// Previous object id; the null id when the reference did not exist.
    pub old: ObjectId,
    /// New object id.
    pub new: ObjectId,
    /// Identity and time of the change.
    pub committer: gix::actor::Signature,
    /// Message; line breaks are folded into spaces as by `git`.
    pub message: String,
}

/// Update of one loose reference and of its log.
///
/// The reference file is written under a `<name>.lock` file that is renamed
/// over the reference, as `git` does; a packed value of the same reference is
/// shadowed by the loose file and left unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefPlan {
    /// Full reference name, for example `HEAD` or `refs/heads/main`.
    pub name: String,
    /// New value.
    pub target: RefTarget,
    /// Expected state of the loose reference file.
    pub expect: Expect,
    /// Line appended to the reference log, when the log is kept.
    pub reflog: Option<ReflogLine>,
}

/// Change of one file of the repository directory other than the index and
/// the references, such as `ORIG_HEAD` or `MERGE_MSG`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateFilePlan {
    /// Path relative to the repository directory (`.git`), with `/`
    /// separators.
    pub path: String,
    /// New content; `None` removes the file.
    pub content: Option<Vec<u8>>,
    /// Expected state of the file.
    pub expect: Expect,
}

/// Unusual effect found while planning or applying, shown in the report.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Notable {
    /// Stable name, for example `filter_not_applied`.
    pub kind: String,
    /// Path concerned, relative to the root of the working tree.
    pub path: Option<BString>,
}

impl Notable {
    /// Notable effect concerning one path.
    pub fn at(kind: &str, path: impl Into<BString>) -> Self {
        Self {
            kind: kind.to_owned(),
            path: Some(path.into()),
        }
    }
}

/// Complete plan of a change of the working tree, the index, references and
/// state files, computed without changing anything.
///
/// The plan is executed by [`apply_plan`](super::apply_plan) in one mutation
/// transaction; a preview reports it without executing it.
#[derive(Debug, Clone)]
pub struct ApplyPlan {
    /// Working tree changes in execution order: removals, removals of empty
    /// directories (deepest first), new directories (shallowest first),
    /// files, links and mode changes.
    pub entries: Vec<PlanEntry>,
    /// New index; `None` leaves the index unchanged.
    pub index: Option<IndexPlan>,
    /// Reference updates, applied after the index.
    pub refs: Vec<RefPlan>,
    /// State file changes, applied last.
    pub state_files: Vec<StateFilePlan>,
    /// Paths whose local changes stay in place, as listed by `git` after a
    /// branch switch.
    pub carried: Vec<BString>,
    /// Unusual effects found while planning.
    pub notable: Vec<Notable>,
}

impl ApplyPlan {
    /// Whether executing the plan changes nothing.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
            && self.index.is_none()
            && self.refs.is_empty()
            && self.state_files.is_empty()
    }
}

/// Result of executing a plan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyReport {
    /// Paths written (files and links), relative to the working tree root.
    pub written: Vec<BString>,
    /// Paths removed (files, links and directories).
    pub removed: Vec<BString>,
    /// Directories created.
    pub created_dirs: Vec<BString>,
    /// The index was written.
    pub index_written: bool,
    /// References updated, by full name.
    pub refs_updated: Vec<String>,
    /// Unusual effects found while planning and applying.
    pub notable: Vec<Notable>,
}
