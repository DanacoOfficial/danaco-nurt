// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reference transactions with reflog entries written through a mutation transaction.
//!
//! A [`RefTx`] collects reference edits, [`RefTx::prepare`] reads the current
//! state of the repository and plans every file change, and
//! [`PreparedRefTx::apply`] performs the plan through a
//! [`MutationTx`], so that every change is journaled and undoable.
//!
//! The file layout is the layout of the files back end of Git:
//!
//! * every written file (a loose reference, `HEAD`, `packed-refs`) is one
//!   `modify` target whose expected previous content is checked by the
//!   journal atomically with the switch, so a change made after
//!   [`RefTx::prepare`] fails with `E_STALE`; a deleted loose reference is
//!   one `delete` target with the same check;
//! * the lock file `<file>.lock` of a running Git process fails the
//!   transaction with `E_GIT_LOCKED`; locks are checked when planning, when
//!   declaring and immediately before the first switch;
//! * `packed-refs` is rewritten only when a packed reference is deleted;
//! * reflog entries `<old> <new> <name> <<email>> <seconds> <+hhmm>\t<message>`
//!   are appended according to `core.logAllRefUpdates`; an update of the
//!   branch `HEAD` points to is also logged in the reflog of `HEAD`;
//! * deleting a reference deletes its reflog and the directories left empty
//!   below `refs/<category>/` and `logs/refs/<category>/`.
//!
//! When the transaction is part of a larger operation, the caller declares
//! the reference targets with [`PreparedRefTx::declare`] together with its own
//! targets, before the first change, and applies the references last.

mod plan;
mod read;
mod reflog;
#[cfg(test)]
pub(crate) mod test_journal;
#[cfg(test)]
mod tests;

use std::path::Path;

use bstr::{BStr, BString, ByteSlice};
use gix_ref::{FullName, FullNameRef, Target};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{MutationTx, TargetSpec};

pub use gix_ref::transaction::PreviousValue;
pub use reflog::{format_reflog_entry, normalize_reflog_message};

/// Which references get a reflog when none exists yet (`core.logAllRefUpdates`).
///
/// An existing reflog is always appended to, whatever the setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogAllRefUpdates {
    /// No reflog is created (`false`, the default of bare repositories).
    Never,
    /// Reflogs are created for `HEAD`, `refs/heads/`, `refs/remotes/` and
    /// `refs/notes/` (`true`, the default of repositories with a working tree).
    #[default]
    Standard,
    /// Reflogs are created for every reference (`always`).
    Always,
}

impl LogAllRefUpdates {
    /// Parses a value of `core.logAllRefUpdates` as Git does: `always`, a
    /// boolean word, or an integer where zero is `false` and any other value
    /// is `true`.
    ///
    /// Returns `None` for any other value.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git::refs_tx::LogAllRefUpdates;
    ///
    /// assert_eq!(LogAllRefUpdates::parse("always"), Some(LogAllRefUpdates::Always));
    /// assert_eq!(LogAllRefUpdates::parse("no"), Some(LogAllRefUpdates::Never));
    /// assert_eq!(LogAllRefUpdates::parse("2"), Some(LogAllRefUpdates::Standard));
    /// assert_eq!(LogAllRefUpdates::parse("sometimes"), None);
    /// ```
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim().to_ascii_lowercase();
        match value.as_str() {
            "always" => Some(Self::Always),
            "true" | "yes" | "on" => Some(Self::Standard),
            "false" | "no" | "off" | "" => Some(Self::Never),
            other => match parse_config_integer(other)? {
                0 => Some(Self::Never),
                _ => Some(Self::Standard),
            },
        }
    }

    /// Setting of `core.logAllRefUpdates` in `repo`; the default is
    /// [`Self::Standard`] for a repository with a working tree and
    /// [`Self::Never`] for a bare one. A key without a value means `true`.
    ///
    /// # Errors
    ///
    /// `E_GIT_UNSUPPORTED` with problem `invalid_config` when the value is
    /// one Git does not accept.
    pub fn of_repo(repo: &gix::Repository) -> Result<Self> {
        let config = repo.config_snapshot();
        match config.string("core.logAllRefUpdates") {
            Some(value) => {
                let text = value.to_str_lossy();
                Self::parse(&text).ok_or_else(|| {
                    Error::new(
                        ErrorCode::GitUnsupported,
                        format!("core.logAllRefUpdates has the invalid value `{text}`; use true, false or always"),
                    )
                    .with_problem("invalid_config")
                    .with_found(text.into_owned())
                })
            }
            None if config.boolean("core.logAllRefUpdates") == Some(true) => Ok(Self::Standard),
            None if repo.is_bare() => Ok(Self::Never),
            None => Ok(Self::Standard),
        }
    }

    /// Whether a missing reflog of `name` is created.
    fn creates_log_for(self, name: &FullNameRef) -> bool {
        self.creates_log_for_name(name.as_bstr())
    }

    /// Whether a missing reflog of the reference called `name` is created.
    pub(crate) fn creates_log_for_name(self, name: &[u8]) -> bool {
        match self {
            Self::Never => false,
            Self::Always => true,
            Self::Standard => {
                name == b"HEAD"
                    || name.starts_with(b"refs/heads/")
                    || name.starts_with(b"refs/remotes/")
                    || name.starts_with(b"refs/notes/")
            }
        }
    }
}

/// Integer of a configuration value as `git_parse_int` reads it: an optional
/// sign, decimal, octal (`0` prefix) or hexadecimal (`0x` prefix) digits and
/// an optional factor `k`, `m` or `g`; `None` when the text is not one or
/// the value overflows.
fn parse_config_integer(text: &str) -> Option<i64> {
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let (digits, factor) = match rest.chars().last()? {
        'k' => (&rest[..rest.len() - 1], 1u64 << 10),
        'm' => (&rest[..rest.len() - 1], 1u64 << 20),
        'g' => (&rest[..rest.len() - 1], 1u64 << 30),
        _ => (rest, 1),
    };
    let (digits, radix) = if let Some(hex) = digits.strip_prefix("0x") {
        (hex, 16)
    } else if digits.len() > 1 && digits.starts_with('0') {
        (&digits[1..], 8)
    } else {
        (digits, 10)
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return None;
    }
    let magnitude = i64::try_from(
        u64::from_str_radix(digits, radix)
            .ok()?
            .checked_mul(factor)?,
    )
    .ok()?;
    Some(if negative { -magnitude } else { magnitude })
}

/// Location and settings of the reference store of one working tree.
#[derive(Debug, Clone)]
pub struct RefLayout {
    git_dir: AbsPath,
    common_dir: AbsPath,
    object_hash: gix_hash::Kind,
    log_all: LogAllRefUpdates,
}

impl RefLayout {
    /// Layout with explicit directories and settings.
    ///
    /// `git_dir` is the repository directory of the working tree (it holds
    /// `HEAD` and the per-worktree references); `common_dir` holds the shared
    /// references, `packed-refs` and `worktrees/`. Both are the same directory
    /// for the main working tree.
    #[must_use]
    pub fn new(
        git_dir: AbsPath,
        common_dir: AbsPath,
        object_hash: gix_hash::Kind,
        log_all: LogAllRefUpdates,
    ) -> Self {
        Self {
            git_dir,
            common_dir,
            object_hash,
            log_all,
        }
    }

    /// Layout of an open repository, with `core.logAllRefUpdates` read from
    /// its configuration.
    ///
    /// # Errors
    ///
    /// `E_IO` when a repository directory is not an absolute path, and
    /// `E_GIT_UNSUPPORTED` with problem `invalid_config` when
    /// `core.logAllRefUpdates` has a value Git does not accept.
    pub fn from_repo(repo: &gix::Repository) -> Result<Self> {
        let log_all = LogAllRefUpdates::of_repo(repo)?;
        Ok(Self::new(
            abs_path(repo.git_dir())?,
            abs_path(repo.common_dir())?,
            repo.object_hash(),
            log_all,
        ))
    }

    /// Repository directory of the working tree.
    #[must_use]
    pub fn git_dir(&self) -> &AbsPath {
        &self.git_dir
    }

    /// Directory of the shared references.
    #[must_use]
    pub fn common_dir(&self) -> &AbsPath {
        &self.common_dir
    }

    /// Object hash of the repository.
    #[must_use]
    pub fn object_hash(&self) -> gix_hash::Kind {
        self.object_hash
    }

    /// Effective `core.logAllRefUpdates`.
    #[must_use]
    pub fn log_all(&self) -> LogAllRefUpdates {
        self.log_all
    }

    /// Directory holding the loose file and the reflog of `name`.
    fn base_for(&self, name: &FullNameRef) -> &AbsPath {
        let private = name
            .category()
            .is_some_and(|category| category.is_worktree_private());
        if private || !name.as_bstr().contains(&b'/') {
            &self.git_dir
        } else {
            &self.common_dir
        }
    }

    /// Path of the loose file of `name`.
    fn loose_path(&self, name: &FullNameRef) -> Result<AbsPath> {
        join_ref_path(self.base_for(name), None, name.as_bstr())
    }

    /// Path of the reflog of `name`.
    fn log_path(&self, name: &FullNameRef) -> Result<AbsPath> {
        join_ref_path(self.base_for(name), Some("logs"), name.as_bstr())
    }

    /// Path of `packed-refs`.
    fn packed_path(&self) -> Result<AbsPath> {
        join_ref_path(&self.common_dir, None, b"packed-refs".as_bstr())
    }
}

/// What an edit does to its reference.
#[derive(Debug, Clone, PartialEq, Eq)]
enum EditOp {
    Set(Target),
    Delete,
    Rename { to: FullName },
}

/// One change of a reference.
///
/// # Examples
///
/// ```
/// use gix_hash::ObjectId;
/// use gix_ref::{FullName, Target};
/// use nurt_git::refs_tx::{PreviousValue, RefEdit};
///
/// let name = FullName::try_from("refs/heads/topic").unwrap();
/// let id = ObjectId::from_hex(b"0123456789012345678901234567890123456789").unwrap();
/// let edit = RefEdit::set(name, Target::Object(id), PreviousValue::MustNotExist, "branch: Created from HEAD");
/// assert_eq!(edit.name().as_bstr(), "refs/heads/topic");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefEdit {
    name: FullName,
    op: EditOp,
    expected: PreviousValue,
    message: BString,
    deref: bool,
    log_only: bool,
    force_create_reflog: bool,
}

impl RefEdit {
    /// Sets `name` to `new` when its current value satisfies `expected`,
    /// recording `message` in the reflog.
    pub fn set(
        name: FullName,
        new: Target,
        expected: PreviousValue,
        message: impl Into<BString>,
    ) -> Self {
        Self::with_op(name, EditOp::Set(new), expected, message.into())
    }

    /// Deletes `name` and its reflog when its current value satisfies
    /// `expected`.
    ///
    /// `message` is recorded in the reflog of `HEAD` when `HEAD` points to
    /// the deleted branch.
    pub fn delete(name: FullName, expected: PreviousValue, message: impl Into<BString>) -> Self {
        Self::with_op(name, EditOp::Delete, expected, message.into())
    }

    /// Renames the branch `from` to `to`, keeping its value and reflog, and
    /// moves every `HEAD` that points to `from`.
    ///
    /// `to` must not exist. The reflog entries are those of `git branch -m`.
    pub fn rename(from: FullName, to: FullName, message: impl Into<BString>) -> Self {
        Self::with_op(
            from,
            EditOp::Rename { to },
            PreviousValue::MustExist,
            message.into(),
        )
    }

    fn with_op(name: FullName, op: EditOp, expected: PreviousValue, message: BString) -> Self {
        Self {
            name,
            op,
            expected,
            message,
            deref: false,
            log_only: false,
            force_create_reflog: false,
        }
    }

    /// Applies the edit to the reference a symbolic `name` points to, and
    /// records the reflog entry for both.
    #[must_use]
    pub fn deref(mut self) -> Self {
        self.deref = true;
        self
    }

    /// Writes only the reflog entry of a [`RefEdit::set`], leaving the
    /// reference unchanged.
    #[must_use]
    pub fn log_only(mut self) -> Self {
        self.log_only = true;
        self
    }

    /// Creates the reflog even when `core.logAllRefUpdates` would not.
    #[must_use]
    pub fn create_reflog(mut self) -> Self {
        self.force_create_reflog = true;
        self
    }

    /// Name of the edited reference.
    #[must_use]
    pub fn name(&self) -> &FullName {
        &self.name
    }
}

/// How a reference changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefChangeKind {
    /// The reference did not exist.
    Created,
    /// The reference had another value.
    Updated,
    /// The reference was deleted.
    Deleted,
    /// The reference already had the requested value.
    Unchanged,
}

/// Result of one reference edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefOutcome {
    /// Reference.
    pub name: FullName,
    /// Value before the transaction.
    pub previous: Option<Target>,
    /// Value after the transaction.
    pub new: Option<Target>,
    /// Kind of the change.
    pub kind: RefChangeKind,
}

/// Collected reference edits of one operation.
///
/// # Examples
///
/// ```no_run
/// use gix_ref::{FullName, Target};
/// use nurt_contracts::journal::MutationTx;
/// use nurt_git::refs_tx::{PreviousValue, RefEdit, RefLayout, RefTx};
///
/// fn create_branch(
///     repo: &gix::Repository,
///     committer: gix_actor::Signature,
///     tx: &mut dyn MutationTx,
/// ) -> nurt_contracts::error::Result<()> {
///     let head = repo.head_id().map_err(|e| {
///         nurt_contracts::error::Error::new(nurt_contracts::error::ErrorCode::GitRevision, e.to_string())
///     })?;
///     let mut refs = RefTx::new(RefLayout::from_repo(repo)?, committer);
///     refs.push(RefEdit::set(
///         FullName::try_from("refs/heads/topic").unwrap(),
///         Target::Object(head.detach()),
///         PreviousValue::MustNotExist,
///         "branch: Created from HEAD",
///     ));
///     refs.prepare()?.apply(tx)?;
///     Ok(())
/// }
/// ```
#[derive(Debug, Clone)]
pub struct RefTx {
    layout: RefLayout,
    committer: gix_actor::Signature,
    edits: Vec<RefEdit>,
}

impl RefTx {
    /// Empty transaction; `committer` is the identity and time of the reflog
    /// entries.
    #[must_use]
    pub fn new(layout: RefLayout, committer: gix_actor::Signature) -> Self {
        Self {
            layout,
            committer,
            edits: Vec::new(),
        }
    }

    /// Adds an edit.
    pub fn push(&mut self, edit: RefEdit) -> &mut Self {
        self.edits.push(edit);
        self
    }

    /// Edits collected so far.
    #[must_use]
    pub fn edits(&self) -> &[RefEdit] {
        &self.edits
    }

    /// Reference layout.
    #[must_use]
    pub fn layout(&self) -> &RefLayout {
        &self.layout
    }

    /// Reads the current state and plans every file change, without writing.
    ///
    /// # Errors
    ///
    /// * `E_GIT_REF` with problem `exists`, `missing` or `changed` when a
    ///   current value does not satisfy the expected one, `name_conflict` when
    ///   a new name collides with an existing reference as a directory,
    ///   `duplicate` when a reference is edited twice, `symref_loop` when a
    ///   symbolic chain is too deep, and `corrupt` for an unreadable reference
    ///   file;
    /// * `E_GIT_LOCKED` when a lock file of a written reference exists;
    /// * `E_GIT_UNSUPPORTED` for names of other working trees
    ///   (`main-worktree/`, `worktrees/<id>/`);
    /// * `E_IO` when a file cannot be read.
    pub fn prepare(self) -> Result<PreparedRefTx> {
        plan::prepare(self)
    }
}

/// Planned reference transaction, ready to be declared and applied.
#[derive(Debug)]
pub struct PreparedRefTx {
    plan: plan::Plan,
}

impl PreparedRefTx {
    /// Outcome of every edit as planned.
    #[must_use]
    pub fn outcomes(&self) -> &[RefOutcome] {
        &self.plan.outcomes
    }

    /// Whether the transaction changes no file.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plan.steps.is_empty()
    }

    /// Targets the transaction will change, in declaration order.
    #[must_use]
    pub fn targets(&self) -> Vec<TargetSpec> {
        self.plan.target_specs()
    }

    /// Declares and preserves the targets.
    ///
    /// Must be called before the first change of the mutation transaction;
    /// [`PreparedRefTx::apply`] calls it when the caller did not.
    ///
    /// # Errors
    ///
    /// `E_GIT_LOCKED` when a lock file appeared since [`RefTx::prepare`], and
    /// the errors of [`MutationTx::preserve`].
    pub fn declare(&mut self, tx: &mut dyn MutationTx) -> Result<()> {
        self.plan.declare(tx)
    }

    /// Performs the planned changes and returns the outcome of every edit.
    ///
    /// # Errors
    ///
    /// `E_STALE` when a reference or `packed-refs` changed since
    /// [`RefTx::prepare`], `E_GIT_LOCKED` as [`PreparedRefTx::declare`], and
    /// the errors of [`MutationTx::apply`]. After an error the mutation
    /// transaction is rolled back by its owner.
    pub fn apply(mut self, tx: &mut dyn MutationTx) -> Result<Vec<RefOutcome>> {
        self.plan.declare(tx)?;
        self.plan.apply(tx)?;
        Ok(self.plan.outcomes)
    }
}

/// Error of a reference with a problem name.
fn ref_error(problem: &str, name: &BStr, message: String) -> Error {
    Error::new(ErrorCode::GitRef, message)
        .with_problem(problem)
        .with_found(name.to_str_lossy().into_owned())
}

/// Input and output error of a file.
fn io_error(path: &Path, error: &std::io::Error) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("cannot read {}: {error}", path.display()),
    )
    .with_detail(
        "path",
        serde_json::Value::String(path.to_string_lossy().into_owned()),
    )
}

/// Absolute path of a repository directory.
fn abs_path(path: &Path) -> Result<AbsPath> {
    AbsPath::new(path).map_err(|error| {
        Error::new(
            ErrorCode::Io,
            format!(
                "repository path {} is not absolute: {error}",
                path.display()
            ),
        )
    })
}

/// Joins a slash-separated reference name below `base`.
fn join_ref_path(base: &AbsPath, prefix: Option<&str>, name: &BStr) -> Result<AbsPath> {
    let mut path = base.as_path().to_path_buf();
    if let Some(prefix) = prefix {
        path.push(prefix);
    }
    for component in name.split_str("/") {
        let component = gix_path::from_bstr(component.as_bstr()).map_err(|_| {
            ref_error(
                "invalid_name",
                name,
                format!("reference name {name} cannot be stored as a file name on this system; choose another name"),
            )
        })?;
        path.push(component);
    }
    abs_path(&path)
}
