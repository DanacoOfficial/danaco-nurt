// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Listing and reading the files of the profiled tree.
//!
//! The file layer and the file listing service of the execution context are
//! used when they are registered; otherwise the tree is read directly, with
//! the exclusions of repository-wide source analysis.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, FileKind, Follow, Fs};
use nurt_contracts::services::{FileListService, WalkSpec};

/// Directories never descended into when the tree is read directly; the same
/// groups as the `vcs`, `deps` and `cache` exclusion presets.
const EXCLUDED_DIRS: [&str; 10] = [
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "venv",
    ".venv",
    "vendor",
    "__pycache__",
    ".cache",
    ".pytest_cache",
];

/// Files of the profiled tree.
pub(crate) trait TreeFiles {
    /// Files below `root` as relative paths with `/` separators, sorted, at
    /// most `max` of them; the flag is `true` when the listing was cut.
    fn list(&self, root: &Path, max: usize) -> Result<(Vec<String>, bool)>;

    /// Content of a file, up to `max_bytes`; `None` when it cannot be read.
    fn read(&self, path: &Path, max_bytes: u64) -> Option<Vec<u8>>;

    /// Modification time in nanoseconds since the Unix epoch.
    fn modified(&self, path: &Path) -> Option<i64>;

    /// Returns `true` when the path is a directory.
    fn is_dir(&self, path: &Path) -> bool;
}

/// Files read through the services of the execution context.
pub(crate) struct ContextFiles<'a> {
    fs: Option<&'a dyn Fs>,
    listing: Option<&'a dyn FileListService>,
}

impl<'a> ContextFiles<'a> {
    /// Uses the file layer and the listing service when they are present.
    pub(crate) fn new(fs: Option<&'a dyn Fs>, listing: Option<&'a dyn FileListService>) -> Self {
        Self { fs, listing }
    }
}

impl TreeFiles for ContextFiles<'_> {
    fn list(&self, root: &Path, max: usize) -> Result<(Vec<String>, bool)> {
        let absolute = match self.listing {
            Some(listing) => list_with_service(listing, root, max)?,
            None => list_directly(root, max)?,
        };
        let truncated = absolute.len() > max;
        let mut relative: Vec<String> = absolute
            .iter()
            .take(max)
            .filter_map(|path| relative_path(root, path))
            .collect();
        relative.sort();
        Ok((relative, truncated))
    }

    fn read(&self, path: &Path, max_bytes: u64) -> Option<Vec<u8>> {
        let mut content = Vec::new();
        match self.fs {
            Some(fs) => {
                let handle = fs.open_read(&AbsPath::new(path).ok()?).ok()?;
                handle.take(max_bytes).read_to_end(&mut content).ok()?;
            }
            None => {
                let file = std::fs::File::open(path).ok()?;
                file.take(max_bytes).read_to_end(&mut content).ok()?;
            }
        }
        Some(content)
    }

    fn modified(&self, path: &Path) -> Option<i64> {
        match self.fs {
            Some(fs) => {
                let meta = fs
                    .metadata(&AbsPath::new(path).ok()?, Follow::Follow)
                    .ok()?;
                Some(meta.modified_ns)
            }
            None => {
                let modified = std::fs::metadata(path).ok()?.modified().ok()?;
                let nanos = modified.duration_since(UNIX_EPOCH).ok()?.as_nanos();
                i64::try_from(nanos).ok()
            }
        }
    }

    fn is_dir(&self, path: &Path) -> bool {
        match self.fs {
            Some(fs) => AbsPath::new(path)
                .ok()
                .and_then(|path| fs.metadata(&path, Follow::Follow).ok())
                .is_some_and(|meta| meta.kind == FileKind::Directory),
            None => path.is_dir(),
        }
    }
}

/// Lists one entry more than `max`, so the caller can tell that the listing
/// was cut.
fn list_with_service(
    listing: &dyn FileListService,
    root: &Path,
    max: usize,
) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in listing.walk(&WalkSpec::source_tree(root))? {
        let entry = entry?;
        if !entry.is_dir {
            paths.push(entry.path);
        }
        if paths.len() > max {
            break;
        }
    }
    Ok(paths)
}

fn list_directly(root: &Path, max: usize) -> Result<Vec<PathBuf>> {
    if !root.is_dir() {
        return Err(not_a_directory(root));
    }
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !(entry.file_type().is_some_and(|kind| kind.is_dir())
                && EXCLUDED_DIRS.contains(&name.as_ref()))
        })
        .build();
    let mut paths = Vec::new();
    for entry in walker.flatten() {
        if entry.file_type().is_some_and(|kind| kind.is_file()) {
            paths.push(entry.into_path());
        }
        if paths.len() > max {
            break;
        }
    }
    Ok(paths)
}

fn relative_path(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let parts: Vec<String> = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Error for a root that is not a directory.
pub(crate) fn not_a_directory(root: &Path) -> Error {
    Error::new(
        ErrorCode::NotFound,
        format!(
            "profile root `{}` is not a directory; pass the root directory of a project",
            root.display()
        ),
    )
    .with_detail("path", serde_json::json!(root.display().to_string()))
}

/// Absolute path of a file given relative to the root.
pub(crate) fn absolute(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}
