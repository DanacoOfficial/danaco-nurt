// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-code::index::walk
// Responsibility:   Listing of the source files of a directory tree.
// Dependencies:     globset, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Listing of the source files of a directory tree.
//!
//! The listing goes through the file listing service of the context, so
//! repository-wide code commands skip the same directories as `find`
//! ([`WalkSpec::source_tree`]: ignore files, version control, dependency and
//! cache directories). Without the service the tree is read through the file
//! system of the context; ignore files are not applied then, the named
//! exclusion groups are applied by directory name and exclusions are
//! matched as globs relative to the root.

use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, FileKind, Follow};
use nurt_contracts::services::{ExcludePreset, WalkKind, WalkSpec};
use nurt_contracts::stage::ExecCtx;

/// Directories skipped by every listing.
pub const ALWAYS_SKIPPED: &[&str] = &[".git", "node_modules", "__pycache__", "venv", ".venv"];

/// Selection of the files of a listing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceFilter {
    /// Glob patterns relative to the root; empty lists every file.
    pub globs: Vec<String>,
    /// Exclusions in gitignore syntax.
    pub exclude: Vec<String>,
    /// Named exclusion groups added to those of a source tree.
    pub presets: Vec<ExcludePreset>,
}

/// Result of a listing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    /// Files in path order.
    pub files: Vec<PathBuf>,
    /// The listing stopped at the file limit.
    pub truncated: bool,
    /// Directories that could not be read.
    pub unreadable: u64,
}

/// Files under `root` selected by `filter` and accepted by `accept`, in
/// path order, no more than `max`.
///
/// # Errors
///
/// Fails when the root cannot be read, when a glob of the filter is not
/// valid (`E_INVALID_VALUE`) and when the execution is cancelled.
pub fn list_sources(
    ctx: &ExecCtx,
    root: &Path,
    filter: &SourceFilter,
    accept: &dyn Fn(&Path) -> bool,
    max: usize,
) -> Result<Listing> {
    match ctx.files() {
        Some(service) => {
            let spec = walk_spec(root, filter);
            let mut listing = Listing::default();
            for entry in service.walk(&spec)? {
                ctx.checkpoint()?;
                let Ok(entry) = entry else {
                    listing.unreadable += 1;
                    continue;
                };
                if entry.is_dir || !accept(&entry.path) {
                    continue;
                }
                if listing.files.len() >= max {
                    listing.truncated = true;
                    break;
                }
                listing.files.push(entry.path);
            }
            listing.files.sort();
            Ok(listing)
        }
        None => FsWalk::new(ctx, root, filter, accept, max)?.run(),
    }
}

/// Request of the file listing service for `root` and `filter`.
#[must_use]
pub fn walk_spec(root: &Path, filter: &SourceFilter) -> WalkSpec {
    let mut spec = WalkSpec::source_tree(root);
    spec.kind = WalkKind::File;
    if !filter.globs.is_empty() {
        spec.globs.clone_from(&filter.globs);
    }
    spec.exclude.clone_from(&filter.exclude);
    for preset in &filter.presets {
        if !spec.exclude_presets.contains(preset) {
            spec.exclude_presets.push(*preset);
        }
    }
    spec
}

fn preset_directories(preset: ExcludePreset) -> &'static [&'static str] {
    match preset {
        ExcludePreset::Vcs => &[".git", ".hg", ".svn"],
        ExcludePreset::Deps => &["node_modules", "venv", ".venv", "vendor"],
        ExcludePreset::Build => &["target", "build", "dist"],
        ExcludePreset::Cache => &["__pycache__", ".cache", ".pytest_cache"],
        _ => &[],
    }
}

fn glob_set(patterns: &[String]) -> Result<Option<GlobSet>> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = Glob::new(pattern).map_err(|error| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("invalid glob `{pattern}`: {error}"),
            )
        })?;
        builder.add(glob);
    }
    builder
        .build()
        .map(Some)
        .map_err(|error| Error::new(ErrorCode::InvalidValue, format!("invalid glob: {error}")))
}

/// Globs equivalent to exclusions in gitignore syntax: a pattern without
/// `/` matches at any depth, a pattern with `/` relative to the root, and
/// both match everything below a matched directory. Negations are not
/// supported by this form and are left out.
fn exclusion_globs(patterns: &[String]) -> Vec<String> {
    let mut globs = Vec::new();
    for pattern in patterns {
        let pattern = pattern.trim();
        if pattern.is_empty() || pattern.starts_with('!') || pattern.starts_with('#') {
            continue;
        }
        let trimmed = pattern.trim_end_matches('/');
        let base = match trimmed.strip_prefix('/') {
            Some(anchored) => anchored.to_owned(),
            None if trimmed.contains('/') => trimmed.to_owned(),
            None => format!("**/{trimmed}"),
        };
        globs.push(format!("{base}/**"));
        globs.push(base);
    }
    globs
}

struct FsWalk<'a> {
    ctx: &'a ExecCtx,
    root: PathBuf,
    accept: &'a dyn Fn(&Path) -> bool,
    max: usize,
    skipped: Vec<&'static str>,
    include: Option<GlobSet>,
    exclude: Option<GlobSet>,
}

impl<'a> FsWalk<'a> {
    fn new(
        ctx: &'a ExecCtx,
        root: &Path,
        filter: &SourceFilter,
        accept: &'a dyn Fn(&Path) -> bool,
        max: usize,
    ) -> Result<Self> {
        let mut skipped: Vec<&'static str> = ALWAYS_SKIPPED.to_vec();
        for preset in &filter.presets {
            skipped.extend(preset_directories(*preset));
        }
        Ok(Self {
            ctx,
            root: root.to_path_buf(),
            accept,
            max,
            skipped,
            include: glob_set(&filter.globs)?,
            exclude: glob_set(&exclusion_globs(&filter.exclude))?,
        })
    }

    fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn excluded(&self, path: &Path) -> bool {
        self.exclude
            .as_ref()
            .is_some_and(|set| set.is_match(self.relative(path)))
    }

    fn included(&self, path: &Path) -> bool {
        self.include
            .as_ref()
            .is_none_or(|set| set.is_match(self.relative(path)))
    }

    fn run(self) -> Result<Listing> {
        let fs = self.ctx.require_fs()?;
        let root = AbsPath::new(&self.root).map_err(Error::from)?;
        drop(fs.read_dir(&root).map_err(Error::from)?);
        let mut listing = Listing::default();
        let mut pending = vec![self.root.clone()];
        while let Some(directory) = pending.pop() {
            self.ctx.checkpoint()?;
            let Some(entries) = self.entries(&directory) else {
                listing.unreadable += 1;
                continue;
            };
            let mut directories = Vec::new();
            for (path, kind) in entries {
                if self.excluded(&path) {
                    continue;
                }
                match kind {
                    Some(FileKind::Directory) => directories.push(path),
                    Some(FileKind::File) if self.included(&path) && (self.accept)(&path) => {
                        if listing.files.len() >= self.max {
                            listing.truncated = true;
                            listing.files.sort();
                            return Ok(listing);
                        }
                        listing.files.push(path);
                    }
                    _ => {}
                }
            }
            pending.extend(directories.into_iter().rev());
        }
        listing.files.sort();
        Ok(listing)
    }

    /// Entries of a directory in name order, without skipped directories;
    /// `None` when the directory cannot be read.
    fn entries(&self, directory: &Path) -> Option<Vec<(PathBuf, Option<FileKind>)>> {
        let fs = self.ctx.fs()?;
        let absolute = AbsPath::new(directory).ok()?;
        let mut entries: Vec<_> = fs
            .read_dir(&absolute)
            .ok()?
            .filter_map(std::result::Result::ok)
            .collect();
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Some(
            entries
                .into_iter()
                .filter(|entry| {
                    !self
                        .skipped
                        .iter()
                        .any(|name| entry.name.to_str() == Some(name))
                })
                .map(|entry| {
                    let path = directory.join(&entry.name);
                    let kind = entry.kind.or_else(|| {
                        let absolute = AbsPath::new(&path).ok()?;
                        fs.metadata(&absolute, Follow::NoFollow)
                            .ok()
                            .map(|meta| meta.kind)
                    });
                    (path, kind)
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusions_match_at_any_depth_unless_anchored() {
        let globs = exclusion_globs(&[
            "generated".to_owned(),
            "/docs/".to_owned(),
            "!keep".to_owned(),
        ]);
        let set = glob_set(&globs).unwrap().unwrap();
        assert!(set.is_match("src/generated/a.rs"));
        assert!(set.is_match("generated"));
        assert!(set.is_match("docs/index.md"));
        assert!(!set.is_match("src/docs/index.md"));
        assert!(!set.is_match("keep"));
    }

    #[test]
    fn walk_spec_adds_presets_once() {
        let filter = SourceFilter {
            globs: vec!["**/*.rs".to_owned()],
            exclude: vec!["gen".to_owned()],
            presets: vec![ExcludePreset::Build, ExcludePreset::Vcs],
        };
        let spec = walk_spec(Path::new("/repo"), &filter);
        assert!(spec.gitignore);
        assert_eq!(spec.globs, ["**/*.rs"]);
        assert_eq!(spec.exclude, ["gen"]);
        let vcs = spec
            .exclude_presets
            .iter()
            .filter(|preset| **preset == ExcludePreset::Vcs)
            .count();
        assert_eq!(vcs, 1);
        assert!(spec.exclude_presets.contains(&ExcludePreset::Build));
    }

    #[test]
    fn invalid_glob_is_rejected() {
        let error = glob_set(&["a[".to_owned()]).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
