// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Repository profile: build commands, entry points, change hotspots and manifests.
//!
//! A profile describes a project before its first change: the languages and
//! modules, the commands that build and test it, the programs it starts from,
//! the files with instructions for contributors, and the files that change
//! most often. Every command and entry point names the manifest it was read
//! from. Nothing in the project is run or changed.

mod cache;
mod files;
mod history;
mod manifests;
mod model;
mod overview;
mod source;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nurt_contracts::error::Result;
use nurt_contracts::services::{DocService, GitQueryService};

use crate::registry::Registry;

pub(crate) use files::{ContextFiles, TreeFiles};
pub(crate) use model::{Entry, Section};

use cache::Cached;
use files::not_a_directory;
use history::Repository;
use source::Source;

/// Files listed at most; a larger tree is profiled from the first files in
/// listing order.
pub(crate) const FILES_MAX: usize = 50_000;

/// Default history period of change hotspots, in days.
pub(crate) const DEFAULT_DAYS: u32 = 90;

/// What to profile and which part of the profile to return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Request {
    /// Root directory of the project.
    pub(crate) root: PathBuf,
    /// Sections to return, in output order.
    pub(crate) sections: Vec<Section>,
    /// Entries per section; `None` returns every entry.
    pub(crate) limit: Option<usize>,
    /// History period of change hotspots, in days.
    pub(crate) days: u32,
    /// Read the tree even when a valid cached profile exists.
    pub(crate) refresh: bool,
}

/// Services and state the profile is read with.
pub(crate) struct Env<'a> {
    /// Files of the tree.
    pub(crate) tree: &'a dyn TreeFiles,
    /// Reader of structured manifests.
    pub(crate) doc: Option<&'a dyn DocService>,
    /// Repository queries.
    pub(crate) git: Option<&'a dyn GitQueryService>,
    /// Directory of the profile cache.
    pub(crate) state_dir: Option<&'a Path>,
    /// Current time, nanoseconds since the Unix epoch.
    pub(crate) now_ns: i64,
    /// Languages known to the code group.
    pub(crate) registry: &'a Registry,
}

/// Profile of a tree, limited to the requested sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Profile {
    /// Entries of the requested sections, in section order.
    pub(crate) entries: Vec<Entry>,
    /// Commit the repository is on; `None` outside a repository.
    pub(crate) revision: Option<String>,
    /// The tree is in a repository.
    pub(crate) in_repository: bool,
    /// Number of files listed.
    pub(crate) files: usize,
    /// The listing stopped at [`FILES_MAX`].
    pub(crate) truncated: bool,
    /// Age of the cached profile in nanoseconds; `None` when the tree was
    /// read by this call.
    pub(crate) cache_age_ns: Option<i64>,
    /// Data sources that were unavailable, with the reason.
    pub(crate) unavailable: Vec<String>,
}

/// Profile of a tree, read from the tree or recalled from the cache.
///
/// The call fails with `E_NOT_FOUND` when the root is not a directory, and with the
/// error of the file listing when the tree cannot be listed. Unavailable
/// repository data and unreadable manifests are reported in
/// [`Profile::unavailable`] instead.
pub(crate) fn profile(request: &Request, env: &Env<'_>) -> Result<Profile> {
    let root = request.root.as_path();
    if !env.tree.is_dir(root) {
        return Err(not_a_directory(root));
    }
    let mut unavailable = Vec::new();
    let repository = match env.git.map(|git| Repository::open(git, root)).transpose() {
        Ok(repository) => repository.flatten(),
        Err(error) => {
            unavailable.push(format!("repository: {}", error.message));
            None
        }
    };
    let revision = repository
        .as_ref()
        .and_then(|repo| history::head_revision(&repo.root, env.tree));
    let cache_file = env
        .state_dir
        .map(|state_dir| cache::cache_path(state_dir, root));
    let recalled = cache_file
        .as_deref()
        .filter(|_| !request.refresh)
        .and_then(|path| cache::load(path, root))
        .filter(|cached| {
            cached.is_valid(revision.as_deref(), request.days, env.now_ns, |path| {
                env.tree.modified(&files::absolute(root, path))
            })
        });
    let (cached, cache_age_ns) = match recalled {
        Some(cached) => {
            let age = env.now_ns.saturating_sub(cached.created_ns);
            (cached, Some(age))
        }
        None => {
            let read = read_tree(request, env, repository.as_ref(), revision.clone())?;
            unavailable.extend(read.1);
            if let Some(path) = cache_file.as_deref() {
                cache::store(path, &read.0);
            }
            (read.0, None)
        }
    };
    let mut entries = cached.entries;
    if let Some(repo) = repository.as_ref() {
        match repo.uncommitted() {
            Ok(changed) => entries.extend(changed),
            Err(error) => unavailable.push(format!("uncommitted changes: {}", error.message)),
        }
    }
    Ok(Profile {
        entries: select(entries, &request.sections, request.limit),
        revision,
        in_repository: repository.is_some(),
        files: cached.files,
        truncated: cached.truncated,
        cache_age_ns,
        unavailable,
    })
}

/// Reads every section of the tree; returns the cache content and the
/// unavailable data sources.
fn read_tree(
    request: &Request,
    env: &Env<'_>,
    repository: Option<&Repository<'_>>,
    revision: Option<String>,
) -> Result<(Cached, Vec<String>)> {
    let root = request.root.as_path();
    let (listed, truncated) = env.tree.list(root, FILES_MAX)?;
    let source = Source::new(root, &listed, env.tree, env.doc);
    let found = manifests::find(&listed);
    let extracted = manifests::extract(&source, &found);
    let mut unavailable = Vec::new();
    let hotspots = match repository {
        Some(repo) => repo.hotspots(request.days).unwrap_or_else(|error| {
            unavailable.push(format!("history: {}", error.message));
            Vec::new()
        }),
        None => history::recent_files(root, &listed, env.tree),
    };
    let mut entries = overview::languages(&listed, env.registry);
    entries.extend(overview::modules(&source, &found, env.registry));
    entries.extend(extracted.entry_points);
    entries.extend(extracted.commands);
    entries.extend(overview::instructions(&source));
    entries.extend(hotspots);
    entries.extend(overview::manifest_entries(&found));
    unavailable.extend(
        source
            .unreadable()
            .into_iter()
            .map(|path| format!("{path}: content could not be read as a document")),
    );
    let mut cached = Cached::new(root, revision, request.days, env.now_ns);
    cached.manifests = found
        .iter()
        .filter_map(|manifest| {
            let modified = env
                .tree
                .modified(&files::absolute(root, &manifest.path))?;
            Some((manifest.path.clone(), modified))
        })
        .collect::<BTreeMap<_, _>>();
    cached.files = listed.len();
    cached.truncated = truncated;
    cached.entries = entries;
    Ok((cached, unavailable))
}

/// Entries of the requested sections in the requested order, at most
/// `limit` per section.
fn select(entries: Vec<Entry>, sections: &[Section], limit: Option<usize>) -> Vec<Entry> {
    let mut selected = Vec::new();
    for section in sections {
        let taken = entries
            .iter()
            .filter(|entry| entry.section == *section)
            .take(limit.unwrap_or(usize::MAX))
            .cloned();
        selected.extend(taken);
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_follows_section_order_and_limit() {
        let entries = vec![
            Entry::new(Section::Commands, "a"),
            Entry::new(Section::Hotspots, "h"),
            Entry::new(Section::Commands, "b"),
        ];
        let selected = select(entries, &[Section::Hotspots, Section::Commands], Some(1));
        let names: Vec<&str> = selected.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["h", "a"]);
    }
}
