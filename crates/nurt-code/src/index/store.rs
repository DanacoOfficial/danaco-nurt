// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Storage of symbol indexes in the state directory.
//!
//! The index of a root is the JSON file
//! `<state directory>/code-index/<key>.json`, where the key is the first 16
//! hexadecimal digits of the SHA-256 digest of the root path. A stored index
//! carries a stamp of the program version and the grammar versions; an index
//! with another stamp is ignored and rebuilt.
//!
//! Two processes may refresh the same index at once. Each writes a
//! temporary file of its own in the same directory and renames it over the
//! index, so a reader sees either the previous or the new index, never a
//! partial one; the last rename wins.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::IndexedFile;
use crate::registry::Registry;

/// Directory of the indexes inside the state directory.
pub const INDEX_DIRECTORY: &str = "code-index";

/// Version of the stored form. Version 2 adds the status change time and the
/// identity of every file; an index of an older version is ignored and
/// rebuilt.
const FORMAT: u32 = 2;

/// Hexadecimal digits of the root digest used in file names.
const KEY_DIGITS: usize = 16;

/// Sequence of temporary file names within the process.
static TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// Stored form of an index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Stored {
    pub(crate) format: u32,
    pub(crate) stamp: String,
    pub(crate) root: PathBuf,
    pub(crate) files: Vec<IndexedFile>,
}

/// File of the index of `root`.
#[must_use]
pub fn index_file(state_dir: &Path, root: &Path) -> PathBuf {
    let digest = Sha256::digest(root.to_string_lossy().as_bytes());
    let key: String = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
        .chars()
        .take(KEY_DIGITS)
        .collect();
    state_dir.join(INDEX_DIRECTORY).join(format!("{key}.json"))
}

/// Stamp of the program and grammar versions of `registry`.
pub(crate) fn stamp(registry: &Registry) -> String {
    let mut hasher = Sha256::new();
    for spec in registry.specs() {
        hasher.update(spec.id.as_str().as_bytes());
        hasher.update(b"=");
        hasher.update(spec.grammar_version.as_bytes());
        hasher.update(b";");
    }
    let digest = hasher.finalize();
    let grammars: String = digest
        .iter()
        .take(KEY_DIGITS / 2)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{}+{FORMAT}+{grammars}", env!("CARGO_PKG_VERSION"))
}

/// Stored files of the index at `file` when it exists, belongs to `root`
/// and carries `stamp`.
pub(crate) fn load(file: &Path, root: &Path, stamp: &str) -> Option<Vec<IndexedFile>> {
    let text = fs::read(file).ok()?;
    let stored: Stored = serde_json::from_slice(&text).ok()?;
    (stored.format == FORMAT && stored.stamp == stamp && stored.root == root)
        .then_some(stored.files)
}

/// Writes the index atomically.
pub(crate) fn save(
    file: &Path,
    root: &Path,
    stamp: &str,
    files: &[IndexedFile],
) -> std::io::Result<()> {
    let directory = file.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(directory)?;
    let stored = Stored {
        format: FORMAT,
        stamp: stamp.to_owned(),
        root: root.to_path_buf(),
        files: files.to_vec(),
    };
    let bytes = serde_json::to_vec(&stored).map_err(std::io::Error::other)?;
    let sequence = TEMPORARY.fetch_add(1, Ordering::Relaxed);
    let name = file
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let temporary = directory.join(format!(".{name}.{}.{sequence}.tmp", process::id()));
    let written = fs::File::create(&temporary).and_then(|mut handle| {
        handle.write_all(&bytes)?;
        handle.sync_all()
    });
    let renamed = written.and_then(|()| fs::rename(&temporary, file));
    if renamed.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    renamed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_file_depends_on_the_root_only() {
        let state = Path::new("/state");
        let first = index_file(state, Path::new("/repo/a"));
        assert_eq!(first, index_file(state, Path::new("/repo/a")));
        assert_ne!(first, index_file(state, Path::new("/repo/b")));
        assert!(first.starts_with("/state/code-index"));
        let stem = first.file_stem().unwrap().to_string_lossy().into_owned();
        assert_eq!(stem.len(), KEY_DIGITS);
    }

    #[test]
    fn stored_index_round_trips_and_rejects_other_stamps() {
        let directory = tempfile::tempdir().unwrap();
        let root = Path::new("/repo");
        let file = index_file(directory.path(), root);
        save(&file, root, "s1", &[]).unwrap();
        assert_eq!(load(&file, root, "s1"), Some(Vec::new()));
        assert_eq!(load(&file, root, "s2"), None);
        assert_eq!(load(&file, Path::new("/other"), "s1"), None);
        let leftovers = fs::read_dir(file.parent().unwrap()).unwrap().count();
        assert_eq!(leftovers, 1);
    }

    #[test]
    fn index_of_the_previous_format_is_ignored() {
        let directory = tempfile::tempdir().unwrap();
        let root = Path::new("/repo");
        let file = index_file(directory.path(), root);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let older = format!(
            "{{\"format\":1,\"stamp\":\"s1\",\"root\":\"/repo\",\"files\":[{{\"path\":\"/repo/a.rs\",\"language\":null,\"size\":1,\"modified_ns\":1,\"etag\":\"sha256:00\",\"symbols\":[]}}]}}"
        );
        fs::write(&file, older).unwrap();
        assert_eq!(load(&file, root, "s1"), None);
        let current =
            format!("{{\"format\":{FORMAT},\"stamp\":\"s1\",\"root\":\"/repo\",\"files\":[]}}");
        fs::write(&file, current).unwrap();
        assert_eq!(load(&file, root, "s1"), Some(Vec::new()));
        assert_eq!(FORMAT, 2);
    }

    #[test]
    fn stamp_names_the_program_version() {
        assert!(stamp(Registry::builtin()).starts_with(env!("CARGO_PKG_VERSION")));
    }
}
