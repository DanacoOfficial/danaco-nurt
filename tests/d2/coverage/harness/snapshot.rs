// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Snapshot of a directory tree with the content hash and the complete
//! metadata of every entry, compared before an operation and after its undo.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

use nurt_contracts::fs::{AbsPath, ContentHash, FileKind, Follow, Fs};

/// State of one entry that undo must restore exactly.
///
/// Access and status change times are excluded: reading changes the former,
/// and the system sets the latter on every rename. The file identity is
/// excluded because exchange and copy strategies restore content under a new
/// identity. Modification times of directories are excluded because creating
/// and removing names inside a directory updates them; undo restores entries,
/// not the times of their parent directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EntryState {
    kind: FileKind,
    size: Option<u64>,
    hash: Option<ContentHash>,
    mode: u32,
    uid: Option<u32>,
    gid: Option<u32>,
    links: u64,
    modified_ns: Option<i64>,
    xattrs: Vec<(OsString, Vec<u8>)>,
    acl: Option<Vec<u8>>,
    account_sid: Option<String>,
    flags: Option<u64>,
    streams: Vec<(String, u64)>,
    link_target: Option<PathBuf>,
}

/// Entries of a tree keyed by their path relative to the root, in byte
/// order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TreeSnapshot {
    entries: BTreeMap<String, EntryState>,
}

impl TreeSnapshot {
    /// Snapshot of the tree below `root`, skipping directories named in
    /// `skip` (the journal directory).
    pub(crate) fn take(fs: &dyn Fs, root: &AbsPath, skip: &[&str]) -> Self {
        let mut entries = BTreeMap::new();
        let mut pending = vec![(root.clone(), String::new())];
        while let Some((dir, prefix)) = pending.pop() {
            let mut names: Vec<OsString> = fs
                .read_dir(&dir)
                .unwrap_or_else(|error| panic!("cannot list {dir:?}: {error}"))
                .map(|entry| entry.expect("directory entry is readable").name)
                .collect();
            names.sort();
            for name in names {
                let text = name.to_string_lossy().into_owned();
                if prefix.is_empty() && skip.contains(&text.as_str()) {
                    continue;
                }
                let path = dir
                    .join_name(&text)
                    .expect("entry name is a single component");
                let relative = format!("{prefix}{text}");
                let state = entry_state(fs, &path);
                if state.kind == FileKind::Directory {
                    pending.push((path, format!("{relative}/")));
                }
                entries.insert(relative, state);
            }
        }
        Self { entries }
    }

    /// Number of entries.
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Human-readable differences from `other`, empty when equal.
    pub(crate) fn differences(&self, other: &Self) -> Vec<String> {
        let mut out = Vec::new();
        for (path, state) in &self.entries {
            match other.entries.get(path) {
                None => out.push(format!("{path}: missing")),
                Some(found) if found != state => {
                    out.push(format!("{path}: expected {state:?}, found {found:?}"));
                }
                Some(_) => {}
            }
        }
        for path in other.entries.keys() {
            if !self.entries.contains_key(path) {
                out.push(format!("{path}: unexpected"));
            }
        }
        out
    }

    /// Copy of the snapshot with every link count set to one. The entry of an
    /// undone link creation keeps the removed name, so the count of the
    /// linked file stays raised until the entry is released.
    pub(crate) fn without_link_counts(&self) -> Self {
        let mut copy = self.clone();
        for state in copy.entries.values_mut() {
            state.links = 1;
        }
        copy
    }

    /// Panics with the differences when `actual` differs from `self`.
    pub(crate) fn assert_same(&self, actual: &Self, context: &str) {
        let differences = self.differences(actual);
        assert!(
            differences.is_empty(),
            "{context}: tree differs:\n{}",
            differences.join("\n")
        );
    }
}

fn entry_state(fs: &dyn Fs, path: &AbsPath) -> EntryState {
    let meta = fs
        .extended_metadata(path, Follow::NoFollow)
        .unwrap_or_else(|error| panic!("cannot read metadata of {path:?}: {error}"));
    let basic = &meta.basic;
    let is_file = basic.kind == FileKind::File;
    let hash = is_file.then(|| {
        fs.content_hash(path)
            .unwrap_or_else(|error| panic!("cannot hash {path:?}: {error}"))
    });
    let mut xattrs: Vec<(OsString, Vec<u8>)> = meta
        .xattrs
        .iter()
        .map(|xattr| (xattr.name.clone(), xattr.value.clone()))
        .collect();
    xattrs.sort();
    let mut streams: Vec<(String, u64)> = meta
        .streams
        .iter()
        .map(|stream| (stream.name.clone(), stream.size))
        .collect();
    streams.sort();
    EntryState {
        kind: basic.kind,
        size: is_file.then_some(basic.size),
        hash,
        mode: basic.mode,
        uid: basic.uid,
        gid: basic.gid,
        links: basic.links,
        modified_ns: (basic.kind != FileKind::Directory).then_some(basic.modified_ns),
        xattrs,
        acl: meta.acl.clone(),
        account_sid: meta.account_sid.clone(),
        flags: meta.flags,
        streams,
        link_target: meta.link_target.clone(),
    }
}
