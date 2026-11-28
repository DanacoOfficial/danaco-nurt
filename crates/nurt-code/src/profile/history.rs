// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-code::profile::history
// Responsibility:   Change hotspots: files changed most often in the history of the repository, uncommitted changes, and the current revision used to invalidate the cache.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Change hotspots: files changed most often in the history of the
//! repository, uncommitted changes, and the current revision used to
//! invalidate the cache.

use std::path::{Path, PathBuf};

use nurt_contracts::error::Result;
use nurt_contracts::services::{GitChange, GitQueryService};

use super::files::{TreeFiles, absolute};
use super::model::{Entry, Section, file_name};

/// History entries requested from the repository before files that are not
/// hotspots (lock files, build output) are removed.
const HISTORY_LIMIT: u32 = 500;

/// Most recently modified files reported outside a repository.
pub(crate) const RECENT_FILES_MAX: usize = 100;

/// Largest repository metadata file read to resolve the revision, in bytes.
const GIT_FILE_BYTES_MAX: u64 = 64 * 1024 * 1024;

/// Origin of an entry for a file with uncommitted changes.
pub(crate) const UNCOMMITTED: &str = "uncommitted";

/// Dependency lock files: changed by tools, not by people.
const LOCK_FILES: [&str; 16] = [
    "Cargo.lock",
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lock",
    "bun.lockb",
    "poetry.lock",
    "Pipfile.lock",
    "uv.lock",
    "Gemfile.lock",
    "composer.lock",
    "go.sum",
    "flake.lock",
    "packages.lock.json",
    "gradle.lockfile",
];

/// Directories with generated files.
const GENERATED_DIRS: [&str; 6] = ["dist", "build", "target", "out", "node_modules", "vendor"];

/// Returns `true` for files that change as a side effect of other changes.
pub(crate) fn is_generated(path: &str) -> bool {
    let name = file_name(path);
    LOCK_FILES.contains(&name)
        || name.ends_with(".min.js")
        || name.ends_with(".min.css")
        || path
            .split('/')
            .rev()
            .skip(1)
            .any(|dir| GENERATED_DIRS.contains(&dir))
}

/// Repository holding the profiled root.
pub(crate) struct Repository<'a> {
    git: &'a dyn GitQueryService,
    /// Root of the working tree.
    pub(crate) root: PathBuf,
    /// Path of the profiled root inside the working tree, with `/`
    /// separators; empty when the profiled root is the working tree root.
    prefix: String,
}

impl<'a> Repository<'a> {
    /// Repository containing `root`; `None` outside a repository.
    pub(crate) fn open(git: &'a dyn GitQueryService, root: &Path) -> Result<Option<Self>> {
        let Some(repo_root) = git.repo_root(root)? else {
            return Ok(None);
        };
        let prefix = root
            .strip_prefix(&repo_root)
            .map(|relative| {
                relative
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .unwrap_or_default();
        Ok(Some(Self {
            git,
            root: repo_root,
            prefix,
        }))
    }

    /// Path relative to the profiled root of a path relative to the working
    /// tree; `None` outside the profiled root.
    fn localize(&self, path: &str) -> Option<String> {
        if self.prefix.is_empty() {
            return Some(path.to_owned());
        }
        path.strip_prefix(&self.prefix)?
            .strip_prefix('/')
            .map(str::to_owned)
    }

    /// Files changed most often in the last `days` days.
    pub(crate) fn hotspots(&self, days: u32) -> Result<Vec<Entry>> {
        let origin = format!("history, {days} days");
        Ok(self
            .git
            .file_history(&self.root, days, HISTORY_LIMIT)?
            .into_iter()
            .filter(|stat| !is_generated(&stat.path))
            .filter_map(|stat| {
                let path = self.localize(&stat.path)?;
                let mut entry = Entry::new(Section::Hotspots, path.clone())
                    .path(path)
                    .origin(origin.clone());
                entry.changes = Some(u64::from(stat.commits));
                entry.last_change_ns = stat
                    .last_change
                    .and_then(|time| i64::try_from(time.unix_nanos()).ok());
                Some(entry)
            })
            .collect())
    }

    /// Files with changes in the index or the working tree.
    pub(crate) fn uncommitted(&self) -> Result<Vec<Entry>> {
        Ok(self
            .git
            .status(&self.root, &[])?
            .into_iter()
            .filter(|status| is_change(status.index) || is_change(status.worktree))
            .filter_map(|status| {
                let path = self.localize(&status.path)?;
                Some(
                    Entry::new(Section::Hotspots, path.clone())
                        .path(path)
                        .origin(UNCOMMITTED),
                )
            })
            .collect())
    }
}

const fn is_change(change: GitChange) -> bool {
    !matches!(change, GitChange::Unmodified | GitChange::Ignored)
}

/// Most recently modified files, for a tree outside a repository.
pub(crate) fn recent_files(root: &Path, files: &[String], tree: &dyn TreeFiles) -> Vec<Entry> {
    let mut dated: Vec<(i64, &String)> = files
        .iter()
        .filter(|path| !is_generated(path))
        .filter_map(|path| Some((tree.modified(&absolute(root, path))?, path)))
        .collect();
    dated.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    dated
        .into_iter()
        .take(RECENT_FILES_MAX)
        .map(|(modified, path)| {
            let mut entry = Entry::new(Section::Hotspots, path.clone())
                .path(path.clone())
                .origin("modification time");
            entry.last_change_ns = Some(modified);
            entry
        })
        .collect()
}

/// Commit the working tree is on, read from the repository metadata; `None`
/// when it cannot be resolved (for example before the first commit).
pub(crate) fn head_revision(repo_root: &Path, tree: &dyn TreeFiles) -> Option<String> {
    let read = |path: &Path| {
        tree.read(path, GIT_FILE_BYTES_MAX)
            .and_then(|bytes| String::from_utf8(bytes).ok())
    };
    let dot_git = repo_root.join(".git");
    let git_dir = match read(&dot_git) {
        Some(link) => {
            let target = link.trim().strip_prefix("gitdir:")?.trim().to_owned();
            repo_root.join(target)
        }
        None => dot_git,
    };
    let common_dir = read(&git_dir.join("commondir"))
        .map_or_else(|| git_dir.clone(), |dir| git_dir.join(dir.trim()));
    let head = read(&git_dir.join("HEAD"))?;
    let head = head.trim();
    let Some(reference) = head.strip_prefix("ref:").map(str::trim) else {
        return Some(head.to_owned());
    };
    let loose = reference
        .split('/')
        .fold(common_dir.clone(), |path, part| path.join(part));
    if let Some(id) = read(&loose) {
        return Some(id.trim().to_owned());
    }
    read(&common_dir.join("packed-refs"))?
        .lines()
        .filter(|line| !line.starts_with(['#', '^']))
        .find_map(|line| {
            let (id, name) = line.split_once(' ')?;
            (name.trim() == reference).then(|| id.to_owned())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_files_and_build_output_are_not_hotspots() {
        assert!(is_generated("Cargo.lock"));
        assert!(is_generated("web/package-lock.json"));
        assert!(is_generated("dist/app.js"));
        assert!(is_generated("static/app.min.js"));
        assert!(!is_generated("src/build.rs"));
        assert!(!is_generated("build.gradle"));
    }

    /// Files by absolute path.
    struct MapTree(std::collections::HashMap<PathBuf, Vec<u8>>);

    impl MapTree {
        fn of(files: &[(&str, &str)]) -> Self {
            Self(
                files
                    .iter()
                    .map(|(path, content)| (PathBuf::from(path), content.as_bytes().to_vec()))
                    .collect(),
            )
        }
    }

    impl TreeFiles for MapTree {
        fn list(&self, _root: &Path, _max: usize) -> Result<(Vec<String>, bool)> {
            Ok((Vec::new(), false))
        }

        fn read(&self, path: &Path, _max_bytes: u64) -> Option<Vec<u8>> {
            self.0.get(path).cloned()
        }

        fn modified(&self, _path: &Path) -> Option<i64> {
            None
        }

        fn is_dir(&self, _path: &Path) -> bool {
            false
        }
    }

    fn revision(files: &[(&str, &str)]) -> Option<String> {
        head_revision(Path::new("/r"), &MapTree::of(files))
    }

    #[test]
    fn detached_head_is_the_revision() {
        assert_eq!(
            revision(&[("/r/.git/HEAD", "0123abc\n")]).as_deref(),
            Some("0123abc")
        );
    }

    #[test]
    fn loose_reference_is_preferred_to_the_packed_one() {
        let files = [
            ("/r/.git/HEAD", "ref: refs/heads/main\n"),
            ("/r/.git/refs/heads/main", "loose\n"),
            ("/r/.git/packed-refs", "packed refs/heads/main\n"),
        ];
        assert_eq!(revision(&files).as_deref(), Some("loose"));
    }

    #[test]
    fn packed_reference_is_resolved_without_a_loose_file() {
        let files = [
            ("/r/.git/HEAD", "ref: refs/heads/main\n"),
            (
                "/r/.git/packed-refs",
                "# pack-refs with: peeled\naaa refs/heads/other\nbbb refs/heads/main\n^ccc\n",
            ),
        ];
        assert_eq!(revision(&files).as_deref(), Some("bbb"));
    }

    #[test]
    fn linked_worktree_reads_head_from_its_own_directory_and_references_from_the_common_one() {
        let files = [
            ("/r/.git", "gitdir: /r/wt/one\n"),
            ("/r/wt/one/HEAD", "ref: refs/heads/feature/x\n"),
            ("/r/wt/one/commondir", "/r/common\n"),
            ("/r/common/refs/heads/feature/x", "def456\n"),
        ];
        assert_eq!(revision(&files).as_deref(), Some("def456"));
    }

    #[test]
    fn unresolvable_head_has_no_revision() {
        assert_eq!(revision(&[]), None);
        assert_eq!(
            revision(&[("/r/.git/HEAD", "ref: refs/heads/main\n")]),
            None
        );
        assert_eq!(
            revision(&[("/r/.git", "not a link\n"), ("/r/.git/HEAD", "abc\n")]),
            None
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
