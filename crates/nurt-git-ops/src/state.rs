// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! State files of operations in progress.
//!
//! Merges, rebases, cherry-picks and reverts that stop for conflict
//! resolution leave files in the Git directory that later commands read:
//! `MERGE_HEAD`, `MERGE_MSG`, `MERGE_MODE`, `AUTO_MERGE`, `ORIG_HEAD`,
//! `CHERRY_PICK_HEAD`, `REVERT_HEAD`, `REBASE_HEAD` and the `rebase-merge/`
//! and `rebase-apply/` directories. The names and formats match Git, so an
//! operation stopped by Nurt can be concluded by Git and the reverse.
//!
//! Changes of state files are described by [`StateChange`] values; each one
//! converts to a journal target and a mutation, so that commands write them
//! in the same transaction as the index and the references.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use gix_hash::ObjectId;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{Expect, Mutation, TargetAction, TargetPath, TargetSpec};
use nurt_contracts::record::GitInProgress;
use nurt_git::apply::StateFilePlan;

/// A state file in the Git directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StateFile {
    /// `MERGE_HEAD`: the commits being merged, one per line.
    MergeHead,
    /// `MERGE_MSG`: the prepared message of the merge commit.
    MergeMsg,
    /// `MERGE_MODE`: `no-ff` when the merge must create a commit, otherwise
    /// empty.
    MergeMode,
    /// `MERGE_RR`: recorded resolutions of the stopped merge.
    MergeRr,
    /// `AUTO_MERGE`: the tree of the merge result with conflict markers.
    AutoMerge,
    /// `ORIG_HEAD`: the value of `HEAD` before a history changing operation.
    OrigHead,
    /// `SQUASH_MSG`: the prepared message of a squashed merge.
    SquashMsg,
    /// `CHERRY_PICK_HEAD`: the commit being picked.
    CherryPickHead,
    /// `REVERT_HEAD`: the commit being reverted.
    RevertHead,
    /// `REBASE_HEAD`: the commit at which a rebase stopped.
    RebaseHead,
    /// `BISECT_LOG`: the log of a bisection.
    BisectLog,
}

impl StateFile {
    /// Files removed when the state of a branch is cleared, as by
    /// `git reset` and by concluding or aborting a merge.
    pub const BRANCH_STATE: &'static [StateFile] = &[
        StateFile::MergeHead,
        StateFile::MergeRr,
        StateFile::MergeMsg,
        StateFile::MergeMode,
        StateFile::AutoMerge,
        StateFile::SquashMsg,
        StateFile::CherryPickHead,
        StateFile::RevertHead,
    ];

    /// Files removed when a merge is concluded by a commit or aborted.
    pub const MERGE_STATE: &'static [StateFile] = &[
        StateFile::MergeHead,
        StateFile::MergeRr,
        StateFile::MergeMsg,
        StateFile::MergeMode,
        StateFile::AutoMerge,
    ];

    /// Name of the file in the Git directory.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git_ops::state::StateFile;
    ///
    /// assert_eq!(StateFile::MergeHead.file_name(), "MERGE_HEAD");
    /// ```
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::MergeHead => "MERGE_HEAD",
            Self::MergeMsg => "MERGE_MSG",
            Self::MergeMode => "MERGE_MODE",
            Self::MergeRr => "MERGE_RR",
            Self::AutoMerge => "AUTO_MERGE",
            Self::OrigHead => "ORIG_HEAD",
            Self::SquashMsg => "SQUASH_MSG",
            Self::CherryPickHead => "CHERRY_PICK_HEAD",
            Self::RevertHead => "REVERT_HEAD",
            Self::RebaseHead => "REBASE_HEAD",
            Self::BisectLog => "BISECT_LOG",
        }
    }

    /// Path of the file in `git_dir`.
    #[must_use]
    pub fn path(self, git_dir: &Path) -> PathBuf {
        git_dir.join(self.file_name())
    }
}

/// An operation in progress in a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpState {
    /// A merge stopped before its commit (`MERGE_HEAD` exists).
    Merge,
    /// A rebase in the `merge` back end (`rebase-merge/`).
    Rebase {
        /// The rebase was started as interactive.
        interactive: bool,
    },
    /// A rebase in the `apply` back end or a mailbox application
    /// (`rebase-apply/`).
    RebaseApply,
    /// A cherry-pick stopped for conflict resolution.
    CherryPick,
    /// A revert stopped for conflict resolution.
    Revert,
    /// A bisection.
    Bisect,
}

impl OpState {
    /// Stable name of the operation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebase { .. } | Self::RebaseApply => "rebase",
            Self::CherryPick => "cherry_pick",
            Self::Revert => "revert",
            Self::Bisect => "bisect",
        }
    }

    /// Value of the `in_progress` field of the `effect.git` record.
    #[must_use]
    pub const fn to_record(self) -> GitInProgress {
        match self {
            Self::Merge => GitInProgress::Merge,
            Self::Rebase { .. } | Self::RebaseApply => GitInProgress::Rebase,
            Self::CherryPick => GitInProgress::CherryPick,
            Self::Revert => GitInProgress::Revert,
            Self::Bisect => GitInProgress::Bisect,
        }
    }

    /// `E_GIT_IN_PROGRESS` for a command that cannot run while this
    /// operation is in progress, with the calls that conclude or abort it.
    #[must_use]
    pub fn in_progress_error(self, repo: &str) -> Error {
        let error = Error::new(
            ErrorCode::GitInProgress,
            format!(
                "a {} is in progress; conclude or abort it first",
                self.as_str().replace('_', "-")
            ),
        )
        .with_problem(self.as_str());
        match self {
            Self::Merge => error
                .with_fix(format!("git.merge {} action: continue", quote(repo)))
                .with_fix(format!("git.merge {} action: abort", quote(repo))),
            Self::Rebase { .. } | Self::RebaseApply => error
                .with_fix(format!("git.rebase {} action: continue", quote(repo)))
                .with_fix(format!("git.rebase {} action: abort", quote(repo))),
            Self::CherryPick | Self::Revert | Self::Bisect => error.with_fix(format!(
                "git.reset {} revision: \"HEAD\" mode: hard",
                quote(repo)
            )),
        }
    }
}

/// Quotes a path for a Nurt call shown in `fix` and `next_steps`.
pub(crate) fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        if matches!(c, '"' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// Detects the operation in progress in `git_dir`, in the order Git uses
/// for its status: merge, rebase, cherry-pick, bisection, revert.
///
/// # Errors
///
/// Fails with `E_IO` when the Git directory cannot be read.
///
/// # Examples
///
/// ```
/// use nurt_git_ops::state::{detect, OpState};
///
/// let dir = tempfile::tempdir().unwrap();
/// assert_eq!(detect(dir.path()).unwrap(), None);
/// std::fs::write(dir.path().join("MERGE_HEAD"), b"0000000000000000000000000000000000000001\n").unwrap();
/// assert_eq!(detect(dir.path()).unwrap(), Some(OpState::Merge));
/// ```
pub fn detect(git_dir: &Path) -> Result<Option<OpState>> {
    if is_file(&StateFile::MergeHead.path(git_dir))? {
        return Ok(Some(OpState::Merge));
    }
    if is_dir(&git_dir.join("rebase-apply"))? {
        return Ok(Some(OpState::RebaseApply));
    }
    let rebase_merge = git_dir.join("rebase-merge");
    if is_dir(&rebase_merge)? {
        let interactive = is_file(&rebase_merge.join("interactive"))?;
        return Ok(Some(OpState::Rebase { interactive }));
    }
    if is_file(&StateFile::CherryPickHead.path(git_dir))? {
        return Ok(Some(OpState::CherryPick));
    }
    if is_file(&StateFile::BisectLog.path(git_dir))? {
        return Ok(Some(OpState::Bisect));
    }
    if is_file(&StateFile::RevertHead.path(git_dir))? {
        return Ok(Some(OpState::Revert));
    }
    Ok(None)
}

/// State of a merge stopped before its commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeState {
    /// Commits being merged (`MERGE_HEAD`), in file order.
    pub heads: Vec<ObjectId>,
    /// Prepared message (`MERGE_MSG`), when present.
    pub message: Option<Vec<u8>>,
    /// The merge must create a commit even when a fast-forward is possible
    /// (`MERGE_MODE` contains `no-ff`).
    pub no_ff: bool,
    /// Tree of the merge result with conflict markers (`AUTO_MERGE`).
    pub auto_merge: Option<ObjectId>,
}

/// Reads the state of a stopped merge from `git_dir`; `None` when
/// `MERGE_HEAD` does not exist.
///
/// # Errors
///
/// Fails with `E_IO` when a state file cannot be read and with
/// `E_GIT_REVISION` when `MERGE_HEAD` or `AUTO_MERGE` holds no valid object
/// identifier.
pub fn read_merge_state(git_dir: &Path) -> Result<Option<MergeState>> {
    let Some(head_text) = read_optional(&StateFile::MergeHead.path(git_dir))? else {
        return Ok(None);
    };
    let heads = parse_ids(&head_text, StateFile::MergeHead)?;
    let message = read_optional(&StateFile::MergeMsg.path(git_dir))?;
    let no_ff = read_optional(&StateFile::MergeMode.path(git_dir))?.is_some_and(|mode| {
        mode.split(u8::is_ascii_whitespace)
            .any(|word| word == b"no-ff")
    });
    let auto_merge = match read_optional(&StateFile::AutoMerge.path(git_dir))? {
        Some(text) => parse_ids(&text, StateFile::AutoMerge)?.into_iter().next(),
        None => None,
    };
    Ok(Some(MergeState {
        heads,
        message,
        no_ff,
        auto_merge,
    }))
}

/// Content of `MERGE_HEAD` for `heads`: one hexadecimal identifier per
/// line.
#[must_use]
pub fn merge_head_content(heads: &[ObjectId]) -> Vec<u8> {
    let mut out = Vec::with_capacity(heads.len() * 41);
    for head in heads {
        out.extend_from_slice(head.to_hex().to_string().as_bytes());
        out.push(b'\n');
    }
    out
}

/// Content of a state file holding one object identifier (`ORIG_HEAD`,
/// `AUTO_MERGE`, `REBASE_HEAD`).
#[must_use]
pub fn id_content(id: &ObjectId) -> Vec<u8> {
    merge_head_content(std::slice::from_ref(id))
}

/// A change of one state file, written through the journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateChange {
    /// Write the file with the given content, creating it when absent.
    Write {
        /// File.
        file: StateFile,
        /// Complete new content.
        content: Vec<u8>,
    },
    /// Remove the file.
    Remove(StateFile),
}

impl StateChange {
    /// File concerned.
    #[must_use]
    pub const fn file(&self) -> StateFile {
        match self {
            Self::Write { file, .. } | Self::Remove(file) => *file,
        }
    }

    /// Journal target of the change in `git_dir`.
    ///
    /// # Errors
    ///
    /// Fails with `E_PATH_RELATIVE` when `git_dir` is not absolute.
    pub fn target(&self, git_dir: &Path) -> Result<TargetSpec> {
        let path = abs_path(&self.file().path(git_dir))?;
        let action = match self {
            Self::Write { .. } => TargetAction::Modify,
            Self::Remove(_) => TargetAction::Delete,
        };
        Ok(TargetSpec {
            path: TargetPath::Fs(path),
            action,
            expect: Expect::Unchecked,
            size_hint: None,
        })
    }

    /// Mutation that performs the change on the declared target.
    #[must_use]
    pub fn mutation(&self) -> Mutation {
        match self {
            Self::Write { content, .. } => Mutation::WriteBytes(content.clone()),
            Self::Remove(_) => Mutation::Remove,
        }
    }

    /// The change as a state file of a working tree update plan.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git_ops::state::{StateChange, StateFile};
    ///
    /// let plan = StateChange::Remove(StateFile::MergeHead).to_plan();
    /// assert_eq!(plan.path, "MERGE_HEAD");
    /// assert!(plan.content.is_none());
    /// ```
    #[must_use]
    pub fn to_plan(&self) -> StateFilePlan {
        StateFilePlan {
            path: self.file().file_name().to_owned(),
            content: match self {
                Self::Write { content, .. } => Some(content.clone()),
                Self::Remove(_) => None,
            },
            expect: Expect::Unchecked,
        }
    }
}

/// Removal of every file of `files` that exists in `git_dir`, in the order
/// given.
///
/// # Errors
///
/// Fails with `E_IO` when the Git directory cannot be read.
pub fn removals(git_dir: &Path, files: &[StateFile]) -> Result<Vec<StateChange>> {
    let mut out = Vec::new();
    for file in files {
        if exists(&file.path(git_dir))? {
            out.push(StateChange::Remove(*file));
        }
    }
    Ok(out)
}

pub(crate) fn abs_path(path: &Path) -> Result<AbsPath> {
    AbsPath::new(path).map_err(|_| {
        Error::new(
            ErrorCode::PathRelative,
            format!(
                "path {} is not absolute; give an absolute repository path",
                path.display()
            ),
        )
    })
}

fn parse_ids(text: &[u8], file: StateFile) -> Result<Vec<ObjectId>> {
    let mut out = Vec::new();
    for line in text.split(|byte| *byte == b'\n') {
        let line = line.trim_ascii();
        if line.is_empty() {
            continue;
        }
        let id = ObjectId::from_hex(line).map_err(|_| {
            Error::new(
                ErrorCode::GitRevision,
                format!(
                    "{} holds an invalid object identifier; remove the file or reset the merge",
                    file.file_name()
                ),
            )
            .with_found(String::from_utf8_lossy(line).into_owned())
        })?;
        out.push(id);
    }
    Ok(out)
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error(path, &error)),
    }
}

fn is_file(path: &Path) -> Result<bool> {
    match fs::metadata(path) {
        Ok(meta) => Ok(meta.is_file()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(path, &error)),
    }
}

fn is_dir(path: &Path) -> Result<bool> {
    match fs::metadata(path) {
        Ok(meta) => Ok(meta.is_dir()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(path, &error)),
    }
}

fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(path, &error)),
    }
}

pub(crate) fn io_error(path: &Path, error: &io::Error) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("cannot read {}: {error}", path.display()),
    )
    .with_found(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().expect("temporary directory")
    }

    const ONE: &str = "1111111111111111111111111111111111111111";
    const TWO: &str = "2222222222222222222222222222222222222222";

    #[test]
    fn detects_nothing_in_clean_git_directory() {
        let dir = temp();
        assert_eq!(detect(dir.path()).unwrap(), None);
    }

    #[test]
    fn detects_operations_in_git_status_order() {
        let dir = temp();
        fs::write(dir.path().join("REVERT_HEAD"), ONE).unwrap();
        assert_eq!(detect(dir.path()).unwrap(), Some(OpState::Revert));
        fs::write(dir.path().join("BISECT_LOG"), "").unwrap();
        assert_eq!(detect(dir.path()).unwrap(), Some(OpState::Bisect));
        fs::write(dir.path().join("CHERRY_PICK_HEAD"), ONE).unwrap();
        assert_eq!(detect(dir.path()).unwrap(), Some(OpState::CherryPick));
        fs::create_dir(dir.path().join("rebase-merge")).unwrap();
        assert_eq!(
            detect(dir.path()).unwrap(),
            Some(OpState::Rebase { interactive: false })
        );
        fs::write(dir.path().join("rebase-merge/interactive"), "").unwrap();
        assert_eq!(
            detect(dir.path()).unwrap(),
            Some(OpState::Rebase { interactive: true })
        );
        fs::create_dir(dir.path().join("rebase-apply")).unwrap();
        assert_eq!(detect(dir.path()).unwrap(), Some(OpState::RebaseApply));
        fs::write(dir.path().join("MERGE_HEAD"), ONE).unwrap();
        assert_eq!(detect(dir.path()).unwrap(), Some(OpState::Merge));
    }

    #[test]
    fn reads_merge_state_files() {
        let dir = temp();
        assert_eq!(read_merge_state(dir.path()).unwrap(), None);
        fs::write(dir.path().join("MERGE_HEAD"), format!("{ONE}\n{TWO}\n")).unwrap();
        fs::write(dir.path().join("MERGE_MSG"), "Merge branch 'x'\n").unwrap();
        fs::write(dir.path().join("MERGE_MODE"), "no-ff").unwrap();
        fs::write(dir.path().join("AUTO_MERGE"), format!("{TWO}\n")).unwrap();
        let state = read_merge_state(dir.path()).unwrap().unwrap();
        assert_eq!(state.heads.len(), 2);
        assert_eq!(state.heads[1].to_hex().to_string(), TWO);
        assert_eq!(state.message.as_deref(), Some(&b"Merge branch 'x'\n"[..]));
        assert!(state.no_ff);
        assert_eq!(
            state.auto_merge.map(|id| id.to_hex().to_string()),
            Some(TWO.to_owned())
        );
    }

    #[test]
    fn empty_merge_mode_means_fast_forward_allowed() {
        let dir = temp();
        fs::write(dir.path().join("MERGE_HEAD"), ONE).unwrap();
        fs::write(dir.path().join("MERGE_MODE"), "").unwrap();
        let state = read_merge_state(dir.path()).unwrap().unwrap();
        assert!(!state.no_ff);
        assert!(state.message.is_none());
        assert!(state.auto_merge.is_none());
    }

    #[test]
    fn rejects_invalid_merge_head() {
        let dir = temp();
        fs::write(dir.path().join("MERGE_HEAD"), "not-an-id\n").unwrap();
        let error = read_merge_state(dir.path()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::GitRevision);
    }

    #[test]
    fn writes_identifiers_one_per_line() {
        let one = ObjectId::from_hex(ONE.as_bytes()).unwrap();
        let two = ObjectId::from_hex(TWO.as_bytes()).unwrap();
        assert_eq!(
            merge_head_content(&[one, two]),
            format!("{ONE}\n{TWO}\n").into_bytes()
        );
        assert_eq!(id_content(&one), format!("{ONE}\n").into_bytes());
    }

    #[test]
    fn lists_removals_of_existing_files_only() {
        let dir = temp();
        fs::write(dir.path().join("MERGE_HEAD"), ONE).unwrap();
        fs::write(dir.path().join("AUTO_MERGE"), ONE).unwrap();
        let changes = removals(dir.path(), StateFile::MERGE_STATE).unwrap();
        assert_eq!(
            changes,
            vec![
                StateChange::Remove(StateFile::MergeHead),
                StateChange::Remove(StateFile::AutoMerge)
            ]
        );
    }

    #[test]
    fn converts_changes_to_targets_and_mutations() {
        let dir = temp();
        let write = StateChange::Write {
            file: StateFile::MergeMode,
            content: Vec::new(),
        };
        let target = write.target(dir.path()).unwrap();
        assert_eq!(target.action, TargetAction::Modify);
        assert_eq!(write.mutation(), Mutation::WriteBytes(Vec::new()));
        let remove = StateChange::Remove(StateFile::OrigHead);
        assert_eq!(
            remove.target(dir.path()).unwrap().action,
            TargetAction::Delete
        );
        assert_eq!(remove.mutation(), Mutation::Remove);
        assert!(remove.target(Path::new("relative")).is_err());
    }

    #[test]
    fn in_progress_error_lists_concluding_calls() {
        let error = OpState::Merge.in_progress_error("/r");
        assert_eq!(error.code(), ErrorCode::GitInProgress);
        assert_eq!(
            error.data().fix,
            vec![
                "git.merge \"/r\" action: continue".to_owned(),
                "git.merge \"/r\" action: abort".to_owned()
            ]
        );
        assert_eq!(
            OpState::Rebase { interactive: true }.to_record(),
            GitInProgress::Rebase
        );
        assert_eq!(OpState::CherryPick.to_record(), GitInProgress::CherryPick);
        assert_eq!(OpState::Bisect.to_record(), GitInProgress::Bisect);
        assert_eq!(OpState::Revert.to_record(), GitInProgress::Revert);
        let plan = StateChange::Write {
            file: StateFile::OrigHead,
            content: b"x\n".to_vec(),
        }
        .to_plan();
        assert_eq!(plan.path, "ORIG_HEAD");
        assert_eq!(plan.content.as_deref(), Some(&b"x\n"[..]));
        assert_eq!(OpState::Bisect.as_str(), "bisect");
        assert!(OpState::Revert.in_progress_error("/r").data().fix[0].starts_with("git.reset"));
    }

    #[test]
    fn quotes_paths_for_calls() {
        assert_eq!(quote(r#"a "b"\c"#), r#""a \"b\"\\c""#);
    }
}
