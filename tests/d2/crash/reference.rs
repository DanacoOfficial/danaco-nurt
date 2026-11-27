// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           tests/d2/crash/reference.rs
// Responsibility:   Reference replacement protocol used to test the harness itself.
// Dependencies:     nurt_contracts, nurt_crashfs, nurt_fs
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Reference replacement protocol used to test the harness itself.
//!
//! The protocol replaces `data/target` and keeps the previous content as a
//! hard link `keep/previous` until a commit marker is durable. Recovery
//! restores the previous content when the marker is missing. The variant
//! without the flush of `keep` loses the previous content after some crashes,
//! which the oracle must detect.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nurt_contracts::fs::{
    AbsPath, Durability, Follow, FsError, FsErrorKind, FsResult, FsWrite, RenameMode,
};
use nurt_crashfs::crash::{CrashFs, VolumeSpec};
use nurt_crashfs::oracle::{
    Change, Expected, OpOutcome, TEMP_PREFIX, TreeSnapshot, Violation, check_complete,
    check_explicit, check_no_leftovers, check_preserved,
};
use nurt_crashfs::points::{Phase, Scenario};
use nurt_fs::failpoint::{FailAction, FailPoint, Failpoints, SeededFailpoints};

use crate::support::{scope, volume_root};

/// Content before the operation.
pub const OLD: &[u8] = b"previous content of the target";
/// Content written by the operation.
pub const NEW: &[u8] = b"new content of the target, longer than the old one";

fn path(relative: &str) -> AbsPath {
    let mut path = volume_root().as_path().to_path_buf();
    path.extend(relative.split('/'));
    AbsPath::new(path).unwrap()
}

fn target() -> AbsPath {
    path("data/target")
}

fn previous() -> AbsPath {
    path("keep/previous")
}

fn marker() -> AbsPath {
    path("keep/committed")
}

fn crashed() -> FsError {
    FsError::new(FsErrorKind::Io, "crash", None)
}

/// Stops on a crash decision; an error decision fails the step.
fn hit(failpoints: &dyn Failpoints, point: FailPoint, n: Option<u32>) -> FsResult<()> {
    match failpoints.hit(point, n) {
        FailAction::Continue => Ok(()),
        FailAction::Error(kind) => Err(FsError::new(kind, point.as_str(), None)),
        FailAction::Crash => Err(crashed()),
    }
}

/// Creates the initial state: `data/target` with [`OLD`] and an empty
/// `keep` directory.
pub fn setup(fs: &dyn FsWrite) -> FsResult<()> {
    fs.create_dir(&scope(), &path("data"), 0o755)?;
    fs.create_dir(&scope(), &path("keep"), 0o755)?;
    let mut file = fs.create_new(&scope(), &target())?;
    file.write_all(OLD).map_err(|_| crashed())?;
    Ok(())
}

/// Performs the replacement; `flush_keep` selects the correct protocol.
pub fn replace(fs: &dyn FsWrite, failpoints: &dyn Failpoints, flush_keep: bool) -> OpOutcome {
    let data = path("data");
    let staged = match stage(fs, failpoints, &data) {
        Ok(staged) => staged,
        Err(err) => return outcome_of(&err, failpoints, || cleanup(fs, &data)),
    };
    if let Err(err) = preserve(fs, failpoints, flush_keep) {
        return outcome_of(&err, failpoints, || {
            fs.remove_file(&scope(), &staged).ok();
            fs.remove_file(&scope(), &previous()).ok();
            Ok(())
        });
    }
    if let Err(err) = apply_and_commit(fs, failpoints, &staged, &data) {
        return outcome_of(&err, failpoints, || {
            if fs.metadata(&marker(), Follow::NoFollow).is_ok() {
                fs.remove_file(&scope(), &marker())?;
            }
            restore(fs)
        });
    }
    OpOutcome::Committed {
        irreversible: false,
    }
}

fn stage(fs: &dyn FsWrite, failpoints: &dyn Failpoints, data: &AbsPath) -> FsResult<AbsPath> {
    hit(failpoints, FailPoint::PlanDecided, None)?;
    let mut file = fs.create_temp_in(&scope(), data, TEMP_PREFIX)?;
    file.write_all(NEW).map_err(|err| {
        err.into_inner()
            .and_then(|inner| inner.downcast::<FsError>().ok())
            .map_or_else(crashed, |inner| *inner)
    })?;
    hit(failpoints, FailPoint::StageWritten, Some(0))?;
    file.sync(Durability::Full)?;
    hit(failpoints, FailPoint::StageSynced, Some(0))?;
    Ok(file.path().clone())
}

fn preserve(fs: &dyn FsWrite, failpoints: &dyn Failpoints, flush_keep: bool) -> FsResult<()> {
    hit(failpoints, FailPoint::PreserveBefore, Some(0))?;
    fs.hard_link(&scope(), &target(), &previous())?;
    hit(failpoints, FailPoint::PreserveMoved, Some(0))?;
    if flush_keep {
        fs.sync_dir(&scope(), &path("keep"))?;
    }
    hit(failpoints, FailPoint::PreserveDurable, Some(0))
}

fn apply_and_commit(
    fs: &dyn FsWrite,
    failpoints: &dyn Failpoints,
    staged: &AbsPath,
    data: &AbsPath,
) -> FsResult<()> {
    fs.rename(&scope(), staged, &target(), RenameMode::Replace)?;
    hit(failpoints, FailPoint::ApplySwitched, Some(0))?;
    fs.sync_dir(&scope(), data)?;
    hit(failpoints, FailPoint::ApplyDone, None)?;
    let mut file = fs.create_new(&scope(), &marker())?;
    hit(failpoints, FailPoint::CommitWritten, None)?;
    file.sync(Durability::Full)?;
    fs.sync_dir(&scope(), &path("keep"))?;
    hit(failpoints, FailPoint::CommitDurable, None)
}

/// Maps a failed step to the reported outcome: a crash stops at once, any
/// other error rolls back with `undo`.
fn outcome_of(
    err: &FsError,
    failpoints: &dyn Failpoints,
    undo: impl FnOnce() -> FsResult<()>,
) -> OpOutcome {
    let stopped =
        err.op == "crash" || failpoints.hit(FailPoint::RollbackStarted, None) == FailAction::Crash;
    if stopped {
        return OpOutcome::Crashed;
    }
    match undo() {
        Ok(()) => OpOutcome::RolledBack,
        Err(_) => OpOutcome::FailedSilently,
    }
}

fn restore(fs: &dyn FsWrite) -> FsResult<()> {
    if fs.metadata(&previous(), Follow::NoFollow).is_ok() {
        fs.rename(&scope(), &previous(), &target(), RenameMode::Replace)?;
        fs.sync_dir(&scope(), &path("data"))?;
        fs.sync_dir(&scope(), &path("keep"))?;
    }
    cleanup(fs, &path("data"))
}

fn cleanup(fs: &dyn FsWrite, data: &AbsPath) -> FsResult<()> {
    let names: Vec<_> = fs.read_dir(data)?.collect::<FsResult<_>>()?;
    let mut removed = false;
    for entry in names {
        if entry.name.to_string_lossy().starts_with(TEMP_PREFIX) {
            fs.remove_file(&scope(), &data.join_name(&entry.name.to_string_lossy())?)?;
            removed = true;
        }
    }
    if removed {
        fs.sync_dir(&scope(), data)?;
    }
    Ok(())
}

/// Recovery after a crash: keeps a committed result, otherwise restores the
/// previous content, and removes temporary files.
pub fn recover(fs: &dyn FsWrite, failpoints: &dyn Failpoints) -> FsResult<()> {
    hit(failpoints, FailPoint::RecoveryStarted, None)?;
    if fs.metadata(&marker(), Follow::NoFollow).is_err() {
        restore(fs)?;
    } else {
        cleanup(fs, &path("data"))?;
    }
    hit(failpoints, FailPoint::RecoveryEntry, None)
}

/// Snapshot of the state before the operation.
pub fn before_tree() -> TreeSnapshot {
    TreeSnapshot::default()
        .with(&Change::CreateDir {
            path: PathBuf::from("data"),
            mode: 0o755,
        })
        .with(&Change::Write {
            path: Path::new("data").join("target"),
            content: OLD.to_vec(),
        })
}

/// Snapshot of the state after the operation.
pub fn after_tree() -> TreeSnapshot {
    before_tree().with(&Change::Write {
        path: Path::new("data").join("target"),
        content: NEW.to_vec(),
    })
}

/// Checks preservation, completeness and leftovers of the reference state.
pub fn check_state(fs: &dyn FsWrite, phase: Phase, outcome: OpOutcome) -> Result<(), Violation> {
    check_explicit(outcome)?;
    let volume = TreeSnapshot::capture(fs, &volume_root(), &[])?;
    check_preserved(&before_tree(), &[Path::new("data").join("target")], &volume)?;
    if phase == Phase::AfterCrash {
        return Ok(());
    }
    check_no_leftovers(&volume)?;
    let tree = TreeSnapshot::capture(fs, &volume_root(), &[Path::new("keep")])?;
    let expected = match outcome {
        OpOutcome::Committed { .. } => Expected::After,
        OpOutcome::RolledBack => Expected::Before,
        _ => Expected::Either,
    };
    check_complete(&tree, &before_tree(), &after_tree(), expected).map(|_| ())
}

/// Reference protocol as a crash scenario.
#[derive(Clone, Copy, Debug)]
pub struct ReferenceReplace {
    /// Whether the directory holding the previous content is flushed.
    pub flush_keep: bool,
}

impl Scenario for ReferenceReplace {
    fn name(&self) -> &str {
        if self.flush_keep {
            "reference_replace"
        } else {
            "reference_replace_without_keep_flush"
        }
    }

    fn volumes(&self) -> Vec<VolumeSpec> {
        vec![VolumeSpec::new(volume_root())]
    }

    fn setup(&self, fs: &CrashFs) -> FsResult<()> {
        setup(fs)
    }

    fn run(&self, fs: &CrashFs, failpoints: &Arc<SeededFailpoints>) -> OpOutcome {
        replace(fs, failpoints.as_ref(), self.flush_keep)
    }

    fn recover(&self, fs: &CrashFs, failpoints: &Arc<SeededFailpoints>) -> FsResult<()> {
        recover(fs, failpoints.as_ref())
    }

    fn check(&self, fs: &CrashFs, phase: Phase, outcome: OpOutcome) -> Result<(), Violation> {
        check_state(fs, phase, outcome)
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
