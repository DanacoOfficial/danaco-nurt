// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Files covered by an analysis or a formatting run.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::{GitQueryService, WalkSpec};
use nurt_contracts::stage::ExecCtx;

/// Directories skipped when no file listing service is available.
const SKIPPED_DIRECTORIES: &[&str] = &[".git", "node_modules", "target", ".venv", "__pycache__"];

/// Files covered by a command.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Scope {
    /// Files relative to the project directory, sorted, with forward slashes.
    pub(crate) files: Vec<String>,
    /// The scope is narrower than the project.
    pub(crate) scoped: bool,
    /// Remarks for the report.
    pub(crate) notes: Vec<String>,
}

impl Scope {
    /// Extensions of the files in scope, lowercase and without the period.
    pub(crate) fn extensions(&self) -> BTreeSet<String> {
        self.files
            .iter()
            .filter_map(|file| extension_of(file))
            .collect()
    }

    /// Files in scope with one of `extensions`.
    pub(crate) fn with_extensions(&self, extensions: &[&str]) -> Vec<String> {
        self.files
            .iter()
            .filter(|file| extension_of(file).is_some_and(|ext| extensions.contains(&ext.as_str())))
            .cloned()
            .collect()
    }
}

/// Lowercase extension of a file name.
pub(crate) fn extension_of(file: &str) -> Option<String> {
    let name = file.rsplit('/').next().unwrap_or(file);
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty()).then(|| ext.to_ascii_lowercase())
}

/// Resolves the files covered by the paths of a command.
///
/// # Errors
///
/// Fails with `E_EXTERNAL_MISSING` when `changed_only` is set and no
/// repository query service is registered.
pub(crate) fn resolve(
    ctx: &ExecCtx,
    dir: &Path,
    paths: &[String],
    changed_only: bool,
) -> Result<Scope> {
    let mut scope = Scope {
        files: list_files(ctx, dir),
        ..Scope::default()
    };
    let wanted: Vec<String> = paths
        .iter()
        .filter_map(|path| relative(dir, path))
        .collect();
    if !wanted.is_empty() {
        scope.files.retain(|file| {
            wanted
                .iter()
                .any(|path| file == path || file.starts_with(&format!("{path}/")))
        });
        scope.scoped = true;
    }
    if changed_only {
        restrict_to_changed(ctx, dir, &mut scope)?;
    }
    Ok(scope)
}

fn restrict_to_changed(ctx: &ExecCtx, dir: &Path, scope: &mut Scope) -> Result<()> {
    let git = ctx
        .extensions()
        .get::<dyn GitQueryService>()
        .ok_or_else(|| {
            Error::new(
                ErrorCode::ExternalMissing,
                "changed files are not available",
            )
            .with_problem("no repository query service is registered")
            .with_fix("analyze.check changed_only: false")
        })?;
    let Some(root) = git.repo_root(dir)? else {
        scope.notes.push(
            "changed_only: the directory is not in a repository, the whole scope is used"
                .to_owned(),
        );
        return Ok(());
    };
    let changed: BTreeSet<String> = git
        .changed_files(&root, None)?
        .into_iter()
        .filter_map(|file| relative(dir, &root.join(file).to_string_lossy()))
        .collect();
    scope.files.retain(|file| changed.contains(file));
    scope.scoped = true;
    Ok(())
}

/// Path relative to `dir` with forward slashes; `None` outside the
/// directory.
fn relative(dir: &Path, path: &str) -> Option<String> {
    let candidate = Path::new(path);
    let local = if candidate.is_absolute() {
        candidate.strip_prefix(dir).ok()?.to_path_buf()
    } else {
        PathBuf::from(path)
    };
    let text = local.to_string_lossy().replace('\\', "/");
    let text = text.trim_start_matches("./").trim_end_matches('/');
    (!text.is_empty() && text != ".").then(|| text.to_owned())
}

fn list_files(ctx: &ExecCtx, dir: &Path) -> Vec<String> {
    let mut files: Vec<String> = match ctx.files() {
        Some(service) => service
            .walk(&WalkSpec::source_tree(dir))
            .map(|walk| {
                walk.filter_map(|entry| entry.ok())
                    .filter(|entry| !entry.is_dir)
                    .filter_map(|entry| relative(dir, &entry.path.to_string_lossy()))
                    .collect()
            })
            .unwrap_or_default(),
        None => {
            let mut found = Vec::new();
            walk_directory(dir, dir, &mut found);
            found
        }
    };
    files.sort();
    files
}

fn walk_directory(root: &Path, current: &Path, found: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(current) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if !SKIPPED_DIRECTORIES.contains(&name.as_str()) {
                walk_directory(root, &path, found);
            }
        } else if let Some(file) = relative(root, &path.to_string_lossy()) {
            found.push(file);
        }
    }
}
