// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Fingerprint of the state of the code.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use nurt_contracts::error::Result;
use nurt_contracts::services::GitQueryService;
use nurt_contracts::stage::ExecCtx;
use sha2::{Digest, Sha256};

/// Largest number of files a fingerprint covers.
pub(crate) const MANIFEST_MAX_FILES: usize = 20_000;

/// Size of the buffer used to hash file content.
const READ_BUFFER_BYTES: usize = 64 * 1024;

/// Prefix of the text form of a fingerprint.
const PREFIX: &str = "sha256:";

/// Fingerprint of a directory and the number of files it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fingerprint {
    /// `sha256:` followed by the digest in hexadecimal.
    pub(crate) id: String,
    /// Files covered.
    pub(crate) files: usize,
    /// More files exist than the limit allows; the rest are not covered.
    pub(crate) truncated: bool,
}

/// Fingerprint of the files under `dir`.
///
/// In a git repository the files are the tracked files and the changed
/// untracked files; elsewhere the files are found by walking the directory
/// under its ignore rules. Names in `exclude` are skipped at any depth.
///
/// # Errors
///
/// Fails when the repository service cannot answer.
pub(crate) fn compute(ctx: &ExecCtx, dir: &Path, exclude: &[String]) -> Result<Fingerprint> {
    let listed = match ctx.extensions().get::<dyn GitQueryService>() {
        Some(git) => match git.repo_root(dir)? {
            Some(root) => Some(repository_files(git.as_ref(), &root, dir)?),
            None => None,
        },
        None => None,
    };
    let (mut paths, truncated) = match listed {
        Some(paths) => paths,
        None => walk_files(dir),
    };
    paths.retain(|path| !excluded(path, exclude));
    let truncated = truncated || paths.len() > MANIFEST_MAX_FILES;
    paths.truncate(MANIFEST_MAX_FILES);
    let mut entries = BTreeMap::new();
    for path in paths {
        if let Some(hash) = file_hash(&dir.join(&path)) {
            entries.insert(path, hash);
        }
    }
    Ok(Fingerprint {
        id: digest(&entries),
        files: entries.len(),
        truncated,
    })
}

/// Digest of a list of paths and content hashes, independent of the order
/// the entries were found in.
pub(crate) fn digest(entries: &BTreeMap<String, String>) -> String {
    let mut hasher = Sha256::new();
    for (path, hash) in entries {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(hash.as_bytes());
        hasher.update(b"\n");
    }
    format!("{PREFIX}{}", hex::encode(hasher.finalize()))
}

fn excluded(path: &str, exclude: &[String]) -> bool {
    path.split('/')
        .any(|part| exclude.iter().any(|name| name == part))
}

fn repository_files(
    git: &dyn GitQueryService,
    root: &Path,
    dir: &Path,
) -> Result<(Vec<String>, bool)> {
    let mut all = git.tracked_files(root)?;
    all.extend(git.changed_files(root, None)?);
    all.sort();
    all.dedup();
    let prefix = dir
        .strip_prefix(root)
        .ok()
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .filter(|relative| !relative.is_empty());
    let files = all
        .into_iter()
        .filter_map(|path| match &prefix {
            Some(prefix) => path.strip_prefix(&format!("{prefix}/")).map(str::to_owned),
            None => Some(path),
        })
        .collect();
    Ok((files, false))
}

fn walk_files(dir: &Path) -> (Vec<String>, bool) {
    let mut files = Vec::new();
    let walker = WalkBuilder::new(dir)
        .hidden(false)
        .require_git(false)
        .sort_by_file_name(std::cmp::Ord::cmp)
        .build();
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let relative: PathBuf = entry
            .path()
            .strip_prefix(dir)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let text = relative.to_string_lossy().replace('\\', "/");
        if text.is_empty() || text == ".git" || text.starts_with(".git/") {
            continue;
        }
        files.push(text);
        if files.len() > MANIFEST_MAX_FILES {
            return (files, true);
        }
    }
    (files, false)
}

/// SHA-256 of the content of a regular file; `None` for anything else or
/// when the file cannot be read.
fn file_hash(path: &Path) -> Option<String> {
    if !fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let mut file = fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; READ_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Some(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::test::support::{ctx, write};

    fn fingerprint(dir: &Path, exclude: &[&str]) -> Fingerprint {
        let state = tempfile::tempdir().unwrap();
        let exclude: Vec<String> = exclude.iter().map(|name| (*name).to_owned()).collect();
        compute(&ctx(state.path()), dir, &exclude).unwrap()
    }

    #[test]
    fn same_content_gives_the_same_fingerprint() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        for dir in [first.path(), second.path()] {
            write(dir, "a.txt", "one");
            write(dir, "src/b.txt", "two");
        }
        let left = fingerprint(first.path(), &[]);
        assert_eq!(left, fingerprint(second.path(), &[]));
        assert!(left.id.starts_with("sha256:"));
        assert_eq!(left.files, 2);
    }

    #[test]
    fn changed_content_or_name_changes_the_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.txt", "one");
        let before = fingerprint(dir.path(), &[]);
        write(dir.path(), "a.txt", "two");
        let changed = fingerprint(dir.path(), &[]);
        assert_ne!(before.id, changed.id);
        std::fs::rename(dir.path().join("a.txt"), dir.path().join("b.txt")).unwrap();
        assert_ne!(changed.id, fingerprint(dir.path(), &[]).id);
    }

    #[test]
    fn excluded_names_and_ignored_files_do_not_count() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.txt", "one");
        write(dir.path(), ".gitignore", "skipped.txt\n");
        let before = fingerprint(dir.path(), &["node_modules"]);
        write(dir.path(), "skipped.txt", "noise");
        write(dir.path(), "node_modules/x.js", "noise");
        assert_eq!(before, fingerprint(dir.path(), &["node_modules"]));
    }

    #[test]
    fn digest_does_not_depend_on_insertion_order() {
        let mut first = BTreeMap::new();
        first.insert("b".to_owned(), "2".to_owned());
        first.insert("a".to_owned(), "1".to_owned());
        let mut second = BTreeMap::new();
        second.insert("a".to_owned(), "1".to_owned());
        second.insert("b".to_owned(), "2".to_owned());
        assert_eq!(digest(&first), digest(&second));
    }
}
