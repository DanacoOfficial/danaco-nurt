// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-report::assemble::bulk
// Responsibility:   Summary of a bulk operation: the largest directories and extensions and a.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Summary of a bulk operation: the largest directories and extensions and a
//! sample of the first and last targets.
//!
//! The summary is computed in one pass over the changes. Only the groups
//! that enter the ranking are sorted, so the cost grows linearly with the
//! number of changes.

use std::cmp::Ordering;
use std::collections::HashMap;

use nurt_contracts::record::Change;
use nurt_contracts::report::{Bulk, DirectoryCount, ExtensionCount};
use nurt_contracts::value::{Bytes, Size};

use super::input::TargetChange;
use crate::rules::paths::parent;

/// Number of directories and of extensions in a bulk summary.
pub const BULK_GROUPS: usize = 10;

/// Number of first targets in the sample.
pub const SAMPLE_HEAD: usize = 5;

/// Number of last targets in the sample.
pub const SAMPLE_TAIL: usize = 2;

/// Summarizes the changes of a bulk operation.
///
/// Directories and extensions count the files and links among the changes,
/// grouped by the directory that contains them and by the extension of their
/// name (with the leading dot; empty for names without an extension). Groups
/// are ordered by the number of targets, then by bytes, both largest first,
/// then by name. The bytes of a target are its size after the change, or its
/// size before the change when it was deleted. The sample lists the first
/// [`SAMPLE_HEAD`] and the last [`SAMPLE_TAIL`] targets in enumeration order,
/// without repeating a target.
#[must_use]
pub fn summarize_bulk(changes: &[TargetChange]) -> Bulk {
    let mut directories: HashMap<&str, Tally> = HashMap::new();
    let mut extensions: HashMap<&str, Tally> = HashMap::new();
    for target in changes.iter().filter(|target| target.entry.is_file()) {
        let path = target.change.path.as_str();
        let size = target.change.size_after.or(target.change.size_before);
        if let Some(directory) = parent(path) {
            let tally = directories.entry(directory).or_default();
            tally.add(size);
            if tally.raw.is_none() {
                tally.raw = raw_directory(&target.change);
            }
        }
        extensions.entry(extension(path)).or_default().add(size);
    }
    let head = changes.len().min(SAMPLE_HEAD);
    let tail = (changes.len() - head).min(SAMPLE_TAIL);
    Bulk {
        by_directory: largest(directories)
            .into_iter()
            .map(|(directory, tally)| DirectoryCount {
                directory: directory.to_owned(),
                raw_directory: tally.raw.clone(),
                count: tally.count,
                bytes: tally.bytes(),
            })
            .collect(),
        by_extension: largest(extensions)
            .into_iter()
            .map(|(extension, tally)| ExtensionCount {
                extension: extension.to_owned(),
                count: tally.count,
                bytes: tally.bytes(),
            })
            .collect(),
        sample_head: sample(&changes[..head]),
        raw_sample_head: raw_sample(&changes[..head]),
        sample_tail: sample(&changes[changes.len() - tail..]),
        raw_sample_tail: raw_sample(&changes[changes.len() - tail..]),
    }
}

fn sample(changes: &[TargetChange]) -> Vec<String> {
    changes
        .iter()
        .map(|target| target.change.path.as_str().to_owned())
        .collect()
}

/// Native bytes of the sampled targets that are not valid Unicode, item by
/// item; empty when every sampled target is.
fn raw_sample(changes: &[TargetChange]) -> Vec<Option<Bytes>> {
    if changes
        .iter()
        .all(|target| target.change.raw_path.is_none())
    {
        return Vec::new();
    }
    changes
        .iter()
        .map(|target| target.change.raw_path.clone())
        .collect()
}

/// Native bytes of the directory of a change, when the directory is not valid
/// Unicode.
fn raw_directory(change: &Change) -> Option<Bytes> {
    let raw = change.raw_path.as_ref()?.as_slice();
    let cut = raw.iter().rposition(|byte| *byte == b'/')?;
    let directory = raw.get(..cut.max(1))?;
    std::str::from_utf8(directory)
        .is_err()
        .then(|| Bytes::new(directory.to_vec()))
}

/// Extension of the last component of `path`, with the leading dot; empty
/// for names without an extension and for names that start with their only
/// dot, such as `.bashrc`.
fn extension(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rfind('.') {
        Some(dot) if dot > 0 && dot + 1 < name.len() => &name[dot..],
        _ => "",
    }
}

#[derive(Debug, Clone, Default)]
struct Tally {
    count: u64,
    bytes: u64,
    sized: bool,
    /// Native bytes of the group, for a directory that is not valid Unicode.
    raw: Option<Bytes>,
}

impl Tally {
    fn add(&mut self, size: Option<Size>) {
        self.count += 1;
        if let Some(size) = size {
            self.bytes = self.bytes.saturating_add(size.bytes());
            self.sized = true;
        }
    }

    fn bytes(self) -> Option<u64> {
        self.sized.then_some(self.bytes)
    }
}

/// The [`BULK_GROUPS`] largest groups in ranking order.
fn largest(groups: HashMap<&str, Tally>) -> Vec<(&str, Tally)> {
    let mut groups: Vec<(&str, Tally)> = groups.into_iter().collect();
    if groups.len() > BULK_GROUPS {
        groups.select_nth_unstable_by(BULK_GROUPS - 1, rank);
        groups.truncate(BULK_GROUPS);
    }
    groups.sort_unstable_by(rank);
    groups
}

fn rank(left: &(&str, Tally), right: &(&str, Tally)) -> Ordering {
    right
        .1
        .count
        .cmp(&left.1.count)
        .then(right.1.bytes.cmp(&left.1.bytes))
        .then(left.0.cmp(right.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_of_names() {
        assert_eq!(extension("/srv/app/a.tmp"), ".tmp");
        assert_eq!(extension("/srv/app/archive.tar.gz"), ".gz");
        assert_eq!(extension("/srv/app/.bashrc"), "");
        assert_eq!(extension("/srv/app/Makefile"), "");
        assert_eq!(extension("/srv/app/name."), "");
        assert_eq!(extension("/srv/app.d/file"), "");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
