// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Profile cache in the state directory of the engine.
//!
//! A cached profile is used while it is younger than [`CACHE_TTL_NS`], the
//! revision and the history period are unchanged, and no manifest it was
//! read from changed its modification time or disappeared. Uncommitted
//! changes are never cached.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::model::Entry;

/// Lifetime of a cached profile: ten minutes, in nanoseconds.
pub(crate) const CACHE_TTL_NS: i64 = 600 * 1_000_000_000;

/// Version of the cache file layout; other versions are ignored.
const CACHE_FORMAT: u32 = 1;

/// Profile as stored in the cache.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Cached {
    format: u32,
    /// Profiled root.
    pub(crate) root: String,
    /// Revision the profile was read at.
    pub(crate) revision: Option<String>,
    /// History period in days.
    pub(crate) days: u32,
    /// Modification time of every manifest read, by relative path.
    pub(crate) manifests: BTreeMap<String, i64>,
    /// Time the profile was read, nanoseconds since the Unix epoch.
    pub(crate) created_ns: i64,
    /// Number of files listed.
    pub(crate) files: usize,
    /// The listing reached the file limit.
    pub(crate) truncated: bool,
    /// Entries of every section except uncommitted changes.
    pub(crate) entries: Vec<Entry>,
}

impl Cached {
    /// New cache content.
    pub(crate) fn new(root: &Path, revision: Option<String>, days: u32, created_ns: i64) -> Self {
        Self {
            format: CACHE_FORMAT,
            root: root.display().to_string(),
            revision,
            days,
            manifests: BTreeMap::new(),
            created_ns,
            files: 0,
            truncated: false,
            entries: Vec::new(),
        }
    }

    /// Returns `true` when the cached profile still describes the tree.
    pub(crate) fn is_valid(
        &self,
        revision: Option<&str>,
        days: u32,
        now_ns: i64,
        modified: impl Fn(&str) -> Option<i64>,
    ) -> bool {
        let age = now_ns.saturating_sub(self.created_ns);
        self.format == CACHE_FORMAT
            && (0..CACHE_TTL_NS).contains(&age)
            && self.revision.as_deref() == revision
            && self.days == days
            && self
                .manifests
                .iter()
                .all(|(path, time)| modified(path) == Some(*time))
    }
}

/// Location of the cache file of a root.
pub(crate) fn cache_path(state_dir: &Path, root: &Path) -> PathBuf {
    let digest = Sha256::digest(root.display().to_string().as_bytes());
    let name: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    state_dir
        .join("code")
        .join("profile")
        .join(format!("{name}.json"))
}

/// Cached profile of a root, when the file exists and parses.
pub(crate) fn load(path: &Path, root: &Path) -> Option<Cached> {
    let bytes = std::fs::read(path).ok()?;
    let cached: Cached = serde_json::from_slice(&bytes).ok()?;
    (cached.root == root.display().to_string()).then_some(cached)
}

/// Stores a profile; a temporary file is renamed over the cache file, so a
/// concurrent reader sees the old or the new content. Failure to store is
/// not an error of the command: the next call reads the tree again.
pub(crate) fn store(path: &Path, cached: &Cached) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let bytes = serde_json::to_vec(cached).map_err(std::io::Error::other)?;
        let temp = dir.join(format!(".{}.tmp", std::process::id()));
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    };
    write().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_expires_and_follows_revision_period_and_manifests() {
        let mut cached = Cached::new(Path::new("/r"), Some("abc".to_owned()), 90, 0);
        cached.manifests.insert("Cargo.toml".to_owned(), 7);
        let same = |_: &str| Some(7);
        assert!(cached.is_valid(Some("abc"), 90, 1, same));
        assert!(!cached.is_valid(Some("abd"), 90, 1, same));
        assert!(!cached.is_valid(Some("abc"), 30, 1, same));
        assert!(!cached.is_valid(Some("abc"), 90, CACHE_TTL_NS, same));
        assert!(!cached.is_valid(Some("abc"), 90, 1, |_| Some(8)));
        assert!(!cached.is_valid(Some("abc"), 90, 1, |_| None));
    }

    #[test]
    fn cache_files_are_named_by_root() {
        let a = cache_path(Path::new("/state"), Path::new("/a"));
        let b = cache_path(Path::new("/state"), Path::new("/b"));
        assert_ne!(a, b);
        assert!(a.starts_with("/state/code/profile"));
    }
}
