// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-space::cost
// Responsibility:   Reclaim cost model and calibration.
// Dependencies:     nurt_contracts, nurt_journal
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Reclaim cost model and calibration.
//!
//! Reclaim deletes data, and its time grows mainly with the number of files:
//! `cost = files · c_f + directories · c_d + bytes / v_b`. Entries are released
//! in decreasing order of `score = bytes / cost`, so that the bytes the user
//! needs are freed first. The coefficients are measured per volume by
//! removing a small tree and smoothed over successive calibrations.

use std::io::Write;
use std::time::Duration;

use nurt_contracts::env::Clock;
use nurt_contracts::fs::{AbsPath, FsError, FsErrorKind, FsResult, FsWrite, TxScope};

use nurt_journal::store::EntrySummary;

use crate::admission::CostModel;

/// Files removed by one calibration.
pub(crate) const CALIBRATION_FILES: u32 = 2_000;

/// Directories holding the calibration files.
const CALIBRATION_DIRS: u32 = 20;

/// The file whose removal measures the cost per byte is written in this
/// many chunks of [`CALIBRATION_CHUNK`] (4 MiB in total).
const CALIBRATION_CHUNKS: u32 = 64;
static CALIBRATION_CHUNK: [u8; 64 << 10] = [0; 64 << 10];

/// Wall time a calibration may take; file creation stops when it runs out.
pub(crate) const CALIBRATION_BUDGET: Duration = Duration::from_millis(150);

/// Weight of a new calibration in the smoothed model.
const SMOOTHING: f64 = 0.3;

/// Lowest cost in seconds, so that the score of an empty entry stays finite.
const MIN_COST_SECONDS: f64 = 1e-9;

/// Initial per-file removal time on solid-state media; measured values for
/// ext4 and XFS lie between 20 and 60 µs, for NTFS between 100 and 300 µs.
const SSD_PER_FILE: f64 = 40e-6;
const SSD_PER_DIR: f64 = 60e-6;
const SSD_BYTES_PER_SECOND: f64 = 4e9;

/// Initial coefficients on rotational media, where every metadata update may
/// cost a seek.
const HDD_PER_FILE: f64 = 400e-6;
const HDD_PER_DIR: f64 = 800e-6;
const HDD_BYTES_PER_SECOND: f64 = 400e6;

/// Mode of the calibration directories.
const CALIBRATION_DIR_MODE: u32 = 0o700;

#[allow(
    clippy::cast_precision_loss,
    reason = "byte and file counts enter a time estimate where 53 bits of precision suffice"
)]
fn approx(value: u64) -> f64 {
    value as f64
}

/// Coefficients measured by one calibration; a coefficient is absent when
/// its interval was too short for the clock to resolve.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Sample {
    pub(crate) per_file: Option<f64>,
    pub(crate) per_dir: Option<f64>,
    pub(crate) bytes_per_second: Option<f64>,
}

impl CostModel {
    /// Model used before the first calibration of a volume.
    pub(crate) fn initial(rotational: bool) -> Self {
        if rotational {
            Self {
                per_file: HDD_PER_FILE,
                per_dir: HDD_PER_DIR,
                bytes_per_second: HDD_BYTES_PER_SECOND,
            }
        } else {
            Self {
                per_file: SSD_PER_FILE,
                per_dir: SSD_PER_DIR,
                bytes_per_second: SSD_BYTES_PER_SECOND,
            }
        }
    }

    /// Estimated seconds to remove an entry.
    pub(crate) fn seconds(&self, entry: &EntrySummary) -> f64 {
        let bytes = if self.bytes_per_second > 0.0 {
            approx(entry.bytes) / self.bytes_per_second
        } else {
            0.0
        };
        approx(entry.files) * self.per_file + approx(entry.directories) * self.per_dir + bytes
    }

    /// Bytes released per second of removal.
    pub(crate) fn score(&self, entry: &EntrySummary) -> f64 {
        approx(entry.bytes) / self.seconds(entry).max(MIN_COST_SECONDS)
    }

    /// Model moved towards a new calibration by exponential smoothing.
    #[must_use]
    pub(crate) fn smoothed(&self, sample: &Sample) -> Self {
        let blend = |old: f64, new: Option<f64>| {
            new.filter(|value| value.is_finite() && *value > 0.0)
                .map_or(old, |value| old + SMOOTHING * (value - old))
        };
        Self {
            per_file: blend(self.per_file, sample.per_file),
            per_dir: blend(self.per_dir, sample.per_dir),
            bytes_per_second: blend(self.bytes_per_second, sample.bytes_per_second),
        }
    }
}

fn widen(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

fn per_unit(elapsed: Duration, units: u32) -> Option<f64> {
    (units > 0 && !elapsed.is_zero()).then(|| elapsed.as_secs_f64() / f64::from(units))
}

struct Tree {
    dirs: Vec<AbsPath>,
    files: Vec<AbsPath>,
    large: Option<AbsPath>,
}

fn build(
    fs: &dyn FsWrite,
    clock: &dyn Clock,
    scope: &TxScope,
    root: &AbsPath,
    tree: &mut Tree,
) -> FsResult<()> {
    let deadline = clock.monotonic().saturating_add(CALIBRATION_BUDGET / 2);
    let large = root.join_name("large")?;
    let mut handle = fs.create_new(scope, &large)?;
    tree.large = Some(large.clone());
    for _ in 0..CALIBRATION_CHUNKS {
        handle
            .write_all(&CALIBRATION_CHUNK)
            .map_err(|_| FsError::new(FsErrorKind::Io, "write", Some(large.as_path())))?;
    }
    drop(handle);
    let per_dir = CALIBRATION_FILES / CALIBRATION_DIRS;
    for d in 0..CALIBRATION_DIRS {
        let dir = root.join_name(&format!("d{d}"))?;
        fs.create_dir(scope, &dir, CALIBRATION_DIR_MODE)?;
        tree.dirs.push(dir.clone());
        for f in 0..per_dir {
            if clock.monotonic() >= deadline {
                return Ok(());
            }
            let file = dir.join_name(&format!("f{f}"))?;
            drop(fs.create_new(scope, &file)?);
            tree.files.push(file);
        }
    }
    Ok(())
}

fn timed(clock: &dyn Clock, mut step: impl FnMut() -> FsResult<()>) -> FsResult<Duration> {
    let start = clock.monotonic();
    step()?;
    Ok(clock.monotonic().saturating_sub(start))
}

fn measure_removal(
    fs: &dyn FsWrite,
    clock: &dyn Clock,
    scope: &TxScope,
    tree: &Tree,
) -> FsResult<Sample> {
    let files = timed(clock, || {
        tree.files
            .iter()
            .try_for_each(|file| fs.remove_file(scope, file))
    })?;
    let dirs = timed(clock, || {
        tree.dirs
            .iter()
            .try_for_each(|dir| fs.remove_dir(scope, dir))
    })?;
    let large = match &tree.large {
        Some(large) => timed(clock, || fs.remove_file(scope, large))?,
        None => Duration::ZERO,
    };
    let file_count = u32::try_from(tree.files.len()).unwrap_or(u32::MAX);
    let dir_count = u32::try_from(tree.dirs.len()).unwrap_or(u32::MAX);
    let per_file = per_unit(files, file_count);
    let byte_time = large.as_secs_f64() - per_file.unwrap_or(0.0);
    let bytes = u64::from(CALIBRATION_CHUNKS) * widen(CALIBRATION_CHUNK.len());
    let bytes_per_second =
        (tree.large.is_some() && byte_time > 0.0).then(|| approx(bytes) / byte_time);
    Ok(Sample {
        per_file,
        per_dir: per_unit(dirs, dir_count),
        bytes_per_second,
    })
}

/// Calibrates the cost model on a volume by creating and removing a tree of
/// up to 2000 empty files in 20 directories and one 4 MiB file under `root`,
/// which must not exist and must lie on the volume measured.
///
/// File creation stops when half of the 150 ms budget is spent, so that the
/// whole calibration stays within the budget on ordinary media.
pub(crate) fn calibrate(
    fs: &dyn FsWrite,
    clock: &dyn Clock,
    scope: &TxScope,
    root: &AbsPath,
) -> FsResult<Sample> {
    fs.create_dir(scope, root, CALIBRATION_DIR_MODE)?;
    let mut tree = Tree {
        dirs: Vec::new(),
        files: Vec::new(),
        large: None,
    };
    let sample = build(fs, clock, scope, root, &mut tree)
        .and_then(|()| measure_removal(fs, clock, scope, &tree));
    let cleanup = fs.remove_tree(scope, root, 1).map(|_| ());
    let sample = sample?;
    cleanup?;
    Ok(sample)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nurt_contracts::env::Clock;
    use nurt_contracts::journal::{EntryState, OpId};

    use super::*;
    use crate::measure::testing::{RealClock, TestVolume, path};
    use crate::reclaim::maintenance_scope;

    fn entry(bytes: u64, files: u64, directories: u64) -> EntrySummary {
        EntrySummary {
            op: OpId(1),
            state: EntryState::Committed,
            bytes,
            files,
            directories,
            pinned: false,
            overdue: None,
        }
    }

    #[test]
    fn cost_calibration_score_follows_the_model() {
        let model = CostModel {
            per_file: 1e-3,
            per_dir: 2e-3,
            bytes_per_second: 1e6,
        };
        let one_large = entry(10_000_000, 1, 1);
        let many_small = entry(10_000_000, 10_000, 100);
        assert!((model.seconds(&one_large) - (1e-3 + 2e-3 + 10.0)).abs() < 1e-9);
        assert!((model.seconds(&many_small) - (10.0 + 0.2 + 10.0)).abs() < 1e-9);
        assert!(model.score(&one_large) > model.score(&many_small));
        assert!((model.score(&one_large) - 10_000_000.0 / 10.003).abs() < 1e-3);
    }

    #[test]
    fn score_of_an_empty_entry_is_finite() {
        let model = CostModel {
            per_file: 0.0,
            per_dir: 0.0,
            bytes_per_second: 0.0,
        };
        assert!(model.score(&entry(0, 0, 0)).is_finite());
        assert!(model.score(&entry(100, 0, 0)).is_finite());
    }

    #[test]
    fn initial_model_is_slower_on_rotational_media() {
        let ssd = CostModel::initial(false);
        let hdd = CostModel::initial(true);
        assert!(hdd.per_file > ssd.per_file);
        assert!(hdd.per_dir > ssd.per_dir);
        assert!(hdd.bytes_per_second < ssd.bytes_per_second);
    }

    #[test]
    fn cost_calibration_ewma_converges_to_repeated_samples() {
        let target = Sample {
            per_file: Some(20e-6),
            per_dir: Some(30e-6),
            bytes_per_second: Some(1e9),
        };
        let mut model = CostModel::initial(true);
        for _ in 0..60 {
            model = model.smoothed(&target);
        }
        assert!((model.per_file - 20e-6).abs() < 1e-9);
        assert!((model.per_dir - 30e-6).abs() < 1e-9);
        assert!((model.bytes_per_second - 1e9).abs() < 1.0);
    }

    #[test]
    fn smoothing_keeps_coefficients_missing_from_the_sample() {
        let model = CostModel::initial(false);
        let partial = Sample {
            per_file: Some(f64::NAN),
            per_dir: None,
            bytes_per_second: Some(-1.0),
        };
        assert_eq!(model.smoothed(&partial), model);
    }

    #[test]
    fn cost_calibration_measures_removal_times() {
        let volume = TestVolume::new(1 << 30);
        volume.set_per_file(Duration::from_micros(25));
        volume.put_file(&path("vol/j/reap/x"), 0);
        let scope = maintenance_scope();
        let root = path("vol/j/reap/calibration");
        let sample = calibrate(&volume, volume.clock.as_ref(), &scope, &root).expect("calibrate");
        let per_file = sample.per_file.expect("per file");
        assert!((per_file - 25e-6).abs() < 1e-9);
        assert_eq!(sample.per_dir, None);
        assert!(!volume.exists(&root));
    }

    #[test]
    fn cost_calibration_two_thousand_files_within_budget() {
        let volume = TestVolume::new(1 << 30);
        volume.put_file(&path("vol/j/reap/x"), 0);
        let clock = RealClock::new();
        let scope = maintenance_scope();
        let root = path("vol/j/reap/calibration");
        let start = clock.monotonic();
        let sample = calibrate(&volume, &clock, &scope, &root).expect("calibrate");
        let elapsed = clock.monotonic().saturating_sub(start);
        assert!(
            elapsed <= CALIBRATION_BUDGET,
            "calibration took {elapsed:?}"
        );
        assert!(sample.per_file.is_some());
        let removed = volume
            .log()
            .iter()
            .filter(|line| line.starts_with("remove_file") && line.contains("/d"))
            .count();
        assert_eq!(removed, CALIBRATION_FILES as usize);
    }

    #[test]
    fn calibration_fails_when_root_exists_and_leaves_it() {
        let volume = TestVolume::new(1 << 30);
        let root = path("vol/j/reap/calibration");
        volume.put_file(&root.join_name("keep").expect("name"), 1);
        let scope = maintenance_scope();
        assert!(calibrate(&volume, volume.clock.as_ref(), &scope, &root).is_err());
        assert!(volume.exists(&root));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
