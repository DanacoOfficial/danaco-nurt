// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::windows::sys::tree
// Responsibility:   Recursive removal that never enters reparse points.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Recursive removal that never enters reparse points.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;

use nurt_contracts::fs::{
    AbsPath, FileKind, Follow, Fs, FsError, FsErrorKind, FsResult, RemoveStats,
};

use super::WindowsFs;
use super::handle::{RawEntry, WidePath, list_dir};
use crate::windows::meta::{classify_kind, is_leaf_for_recursion};

const OP: &str = "remove_tree";

fn add(total: &mut RemoveStats, part: RemoveStats) {
    total.files += part.files;
    total.directories += part.directories;
    total.bytes += part.bytes;
}

/// Statistics of one removed non-directory entry; links count as files
/// without bytes.
fn leaf_stats(kind: FileKind, size: u64) -> RemoveStats {
    let bytes = if kind == FileKind::File { size } else { 0 };
    RemoveStats {
        files: 1,
        directories: 0,
        bytes,
    }
}

fn child(dir: &AbsPath, entry: &RawEntry) -> FsResult<AbsPath> {
    AbsPath::new(dir.as_path().join(OsString::from_wide(&entry.name)))
}

impl WindowsFs {
    /// Removes the leaves of `dir` and returns its subdirectories.
    fn clear_leaves(&self, dir: &AbsPath, stats: &mut RemoveStats) -> FsResult<Vec<AbsPath>> {
        let wide = WidePath::of(dir)?;
        let entries = list_dir(&wide).map_err(|code| Self::error(code, OP, dir, &[]))?;
        let mut subdirectories = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|code| Self::error(code, OP, dir, &[]))?;
            let path = child(dir, &entry)?;
            if is_leaf_for_recursion(entry.attributes) {
                self.delete(&path, OP)?;
                let kind = classify_kind(entry.attributes, entry.reparse_tag);
                add(stats, leaf_stats(kind, entry.size));
            } else {
                subdirectories.push(path);
            }
        }
        Ok(subdirectories)
    }

    /// Removes the directory `root` bottom-up in the calling thread; stops
    /// early, without error, when `stop` is set by another worker.
    fn remove_subtree(&self, root: &AbsPath, stop: &AtomicBool) -> FsResult<RemoveStats> {
        let mut stats = RemoveStats::default();
        let mut pending = vec![(root.clone(), false)];
        while let Some((dir, emptied)) = pending.pop() {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            if emptied {
                self.delete(&dir, OP)?;
                stats.directories += 1;
                continue;
            }
            let subdirectories = self.clear_leaves(&dir, &mut stats)?;
            pending.push((dir, true));
            pending.extend(subdirectories.into_iter().map(|path| (path, false)));
        }
        Ok(stats)
    }

    /// Removes `path` and everything below it; subdirectories of `path` are
    /// distributed over up to `parallelism` threads.
    pub(super) fn remove_tree_at(
        &self,
        path: &AbsPath,
        parallelism: usize,
    ) -> FsResult<RemoveStats> {
        let meta = self.metadata(path, Follow::NoFollow)?;
        if is_leaf_for_recursion(meta.windows_attributes) {
            self.delete(path, OP)?;
            return Ok(leaf_stats(meta.kind, meta.size));
        }
        let mut stats = RemoveStats::default();
        let subdirectories = self.clear_leaves(path, &mut stats)?;
        let workers = parallelism.clamp(1, subdirectories.len().max(1));
        let next = AtomicUsize::new(0);
        let stop = AtomicBool::new(false);
        let work = || -> FsResult<RemoveStats> {
            let mut local = RemoveStats::default();
            while let Some(dir) = subdirectories.get(next.fetch_add(1, Ordering::Relaxed)) {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                match self.remove_subtree(dir, &stop) {
                    Ok(part) => add(&mut local, part),
                    Err(error) => {
                        stop.store(true, Ordering::Relaxed);
                        return Err(error);
                    }
                }
            }
            Ok(local)
        };
        let results: Vec<FsResult<RemoveStats>> = thread::scope(|scope| {
            let handles: Vec<_> = (0..workers).map(|_| scope.spawn(work)).collect();
            handles
                .into_iter()
                .map(|handle| {
                    handle.join().unwrap_or_else(|_| {
                        Err(FsError::new(FsErrorKind::Io, OP, Some(path.as_path())))
                    })
                })
                .collect()
        });
        for result in results {
            add(&mut stats, result?);
        }
        self.delete(path, OP)?;
        stats.directories += 1;
        Ok(stats)
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
