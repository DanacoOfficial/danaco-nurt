// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Repository access: opening without external programs, configuration, path specifications, attributes, content filters, error mapping and record conversion.
//!
//! [`GitRepo`] is the handle every command of the `git` group works with. It
//! opens a repository with the repository library configured so that no
//! program is started and no variable of the process environment is read:
//! the system, global and environment configuration come from the execution
//! context ([`env::GitEnv`], [`config`]), `safe.directory` is checked as
//! `git` checks it, and repository features the implementation does not
//! support are rejected with `E_GIT_UNSUPPORTED` before anything is read.

pub mod config;
pub mod env;
pub mod error;
pub mod filter;
pub mod index;
pub mod lock;
pub mod pathspec;
pub mod records;
pub mod revision;

use std::path::{Path, PathBuf};

use nurt_contracts::error::Error;
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::ExecCtx;

use self::env::GitEnv;

/// Largest number of nearby repositories offered when a path is not inside
/// a repository.
const MAX_NEARBY: usize = error::MAX_CANDIDATES;

/// Depth below the given path searched for nearby repositories.
const NEARBY_DEPTH: usize = 2;

/// Open repository with its effective configuration and the execution
/// context it was opened in.
pub struct GitRepo {
    repo: gix::Repository,
    env: GitEnv,
    ctx: ExecCtx,
    outer: gix::config::File,
}

impl std::fmt::Debug for GitRepo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitRepo")
            .field("git_dir", &self.repo.git_dir())
            .field("workdir", &self.repo.workdir())
            .finish_non_exhaustive()
    }
}

impl GitRepo {
    /// Opens the repository at `path`: a working tree root, a `.git`
    /// directory or file, or a bare repository.
    ///
    /// # Errors
    ///
    /// `E_GIT_NOT_REPOSITORY` when `path` does not exist, is not a repository
    /// or is a repository of another account not listed in
    /// `safe.directory`; `E_GIT_UNSUPPORTED` for repository extensions that
    /// are not implemented, such as `reftable`; `E_IO` when the
    /// configuration cannot be read.
    pub fn open(path: &Path, ctx: &ExecCtx) -> Result<Self, Error> {
        if !path.exists() {
            return Err(error::path_not_found(path));
        }
        let repo = gix::open_opts(path, open_options())
            .map_err(|_| error::not_repository(path, &nearby_repositories(path), command(ctx)))?;
        Self::finish(repo, ctx)
    }

    /// Opens the repository containing `path`, searching `path` and its
    /// parents up to the directories listed in `GIT_CEILING_DIRECTORIES`.
    ///
    /// # Errors
    ///
    /// As [`GitRepo::open`].
    pub fn discover(path: &Path, ctx: &ExecCtx) -> Result<Self, Error> {
        if !path.exists() {
            return Err(error::path_not_found(path));
        }
        let env = GitEnv::from_host(ctx.host());
        let upwards = gix::discover::upwards::Options {
            trust: gix::discover::upwards::TrustPolicy::Assume(gix::sec::Trust::Full),
            ceiling_dirs: env.ceiling_dirs().to_vec(),
            match_ceiling_dir_or_error: false,
            ..Default::default()
        };
        let options = open_options();
        let mapping = gix::sec::trust::Mapping {
            full: options.clone(),
            reduced: options,
        };
        let repo = gix::ThreadSafeRepository::discover_opts(path, upwards, mapping)
            .map_err(|_| error::not_repository(path, &nearby_repositories(path), command(ctx)))?
            .to_thread_local();
        if crosses_ceiling(path, root_of(&repo), env.ceiling_dirs()) {
            return Err(error::not_repository(
                path,
                &nearby_repositories(path),
                command(ctx),
            ));
        }
        Self::finish(repo, ctx)
    }

    fn finish(mut repo: gix::Repository, ctx: &ExecCtx) -> Result<Self, Error> {
        let env = GitEnv::from_host(ctx.host());
        {
            let local = repo.config_snapshot();
            config::check_extensions(&local)?;
        }
        if repo.common_dir().join("reftable").is_dir() {
            return Err(error::unsupported(
                "extensions.refStorage=reftable",
                "only the files reference backend is implemented",
            ));
        }
        let head = repo.head_name().ok().flatten();
        let outer =
            config::load_outer_layers(&env, repo.git_dir(), head.as_ref().map(AsRef::as_ref))?;
        check_ownership(&repo, &outer, &env)?;
        let plan = {
            let local = repo.config_snapshot();
            config::merge_overrides(&local, &outer, &env)
        };
        plan.apply(&mut repo)?;
        if config::fsmonitor_configured(&repo.config_snapshot()) {
            let mut note = NotableEntry::new(Notable::FsmonitorIgnored);
            note.path = Some(root_of(&repo).to_string_lossy().into_owned());
            note.detail =
                Some("core.fsmonitor is set; every status checks the files directly".to_owned());
            ctx.note(note);
        }
        Ok(Self {
            repo,
            env,
            ctx: ctx.clone(),
            outer,
        })
    }

    /// The repository of the underlying library.
    #[must_use]
    pub fn gix(&self) -> &gix::Repository {
        &self.repo
    }

    /// Mutable access to the repository of the underlying library, for
    /// object writes and in-memory configuration changes.
    pub fn gix_mut(&mut self) -> &mut gix::Repository {
        &mut self.repo
    }

    /// Execution context the repository was opened in.
    #[must_use]
    pub fn ctx(&self) -> &ExecCtx {
        &self.ctx
    }

    /// Environment variables that shaped the configuration.
    #[must_use]
    pub fn env(&self) -> &GitEnv {
        &self.env
    }

    /// System and global configuration layers, without the repository-local
    /// layers.
    #[must_use]
    pub fn outer_config(&self) -> &gix::config::File {
        &self.outer
    }

    /// Effective configuration: system, global, repository, worktree and
    /// environment layers.
    #[must_use]
    pub fn config(&self) -> gix::config::Snapshot<'_> {
        self.repo.config_snapshot()
    }

    /// Root of the working tree, or the repository directory of a bare
    /// repository; paths of the `git` commands are relative to it.
    #[must_use]
    pub fn root(&self) -> &Path {
        root_of(&self.repo)
    }

    /// Root of the working tree; `None` for a bare repository.
    #[must_use]
    pub fn workdir(&self) -> Option<&Path> {
        self.repo.workdir()
    }

    /// Repository directory of this working tree (`.git`, or
    /// `.git/worktrees/<name>` for a linked working tree).
    #[must_use]
    pub fn git_dir(&self) -> &Path {
        self.repo.git_dir()
    }

    /// Directory shared by all working trees: objects, references and the
    /// common configuration.
    #[must_use]
    pub fn common_dir(&self) -> &Path {
        self.repo.common_dir()
    }

    /// Whether the repository has no working tree.
    #[must_use]
    pub fn is_bare(&self) -> bool {
        self.repo.workdir().is_none()
    }

    /// Path of the index file of this working tree.
    #[must_use]
    pub fn index_path(&self) -> PathBuf {
        self.repo.index_path()
    }

    /// Commit `HEAD` points to; `None` on an unborn branch.
    ///
    /// # Errors
    ///
    /// `E_IO` when `HEAD` or the reference it names cannot be read.
    pub fn head_id(&self) -> Result<Option<gix::ObjectId>, Error> {
        let head = self
            .repo
            .head()
            .map_err(|source| error::from_gix("read HEAD", &source))?;
        Ok(head.id().map(gix::Id::detach))
    }

    /// Full name of the branch `HEAD` points to; `None` when detached.
    ///
    /// # Errors
    ///
    /// `E_IO` when `HEAD` cannot be read.
    pub fn head_ref(&self) -> Result<Option<gix::refs::FullName>, Error> {
        self.repo
            .head_name()
            .map_err(|source| error::from_gix("read HEAD", &source))
    }

    /// The index of this working tree as an owned file, empty when the
    /// repository has no index yet.
    ///
    /// # Errors
    ///
    /// `E_IO` when the index exists but cannot be read.
    pub fn index_file(&self) -> Result<gix::index::File, Error> {
        let path = self.repo.index_path();
        if !path.exists() {
            return Ok(gix::index::File::from_state(
                gix::index::State::new(self.repo.object_hash()),
                path,
            ));
        }
        self.repo
            .open_index()
            .map_err(|source| error::from_gix("read the index", &source))
    }

    /// Abbreviated identifier of `id` with the length of `core.abbrev`,
    /// extended until it is unique in the repository.
    #[must_use]
    pub fn short_id(&self, id: &gix::oid) -> String {
        gix::prelude::ObjectIdExt::attach(id.to_owned(), &self.repo)
            .shorten_or_id()
            .to_string()
    }

    /// Value of a configuration key, `section.name` or
    /// `section.subsection.name`.
    #[must_use]
    pub fn config_string(&self, key: &str) -> Option<String> {
        self.repo
            .config_snapshot()
            .string(key)
            .map(|value| String::from_utf8_lossy(value.as_ref()).into_owned())
    }

    /// Truth value of a configuration key; `None` when unset or not a truth
    /// value.
    #[must_use]
    pub fn config_bool(&self, key: &str) -> Option<bool> {
        self.repo.config_snapshot().boolean(key)
    }
}

/// Options of the repository library used for every repository: no
/// configuration of the `git` installation, no variables of the process
/// environment, includes followed, and transports that start programs
/// disabled.
#[must_use]
pub fn open_options() -> gix::open::Options {
    let mut permissions = gix::open::Permissions::isolated();
    permissions.config.includes = true;
    gix::open::Options::isolated()
        .permissions(permissions)
        .config_overrides(config::OPEN_OVERRIDES)
        .with(gix::sec::Trust::Full)
}

fn root_of(repo: &gix::Repository) -> &Path {
    repo.workdir().unwrap_or_else(|| repo.git_dir())
}

/// Whether discovery from `start` reached `found` by entering a directory
/// of `GIT_CEILING_DIRECTORIES`: `git` searches upwards from a path below a
/// ceiling only as far as the directories strictly below that ceiling.
fn crosses_ceiling(start: &Path, found: &Path, ceilings: &[PathBuf]) -> bool {
    let canonical =
        |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let start = canonical(start);
    let found = canonical(found);
    ceilings
        .iter()
        .map(|ceiling| canonical(ceiling))
        .any(|ceiling| {
            start.starts_with(&ceiling)
                && start != ceiling
                && !(found.starts_with(&ceiling) && found != ceiling)
        })
}

fn command(ctx: &ExecCtx) -> &str {
    &ctx.frame().command
}

fn check_ownership(
    repo: &gix::Repository,
    outer: &gix::config::File,
    env: &GitEnv,
) -> Result<(), Error> {
    let root = root_of(repo);
    let owned = [repo.git_dir(), root]
        .iter()
        .all(|path| gix::sec::identity::is_path_owned_by_current_user(path).unwrap_or(true));
    if owned || config::is_safe_directory(outer, env, root) {
        return Ok(());
    }
    let global = env.global_config_files().pop();
    Err(error::unsafe_repository(root, global.as_deref()))
}

fn nearby_repositories(path: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut level = vec![path.to_path_buf()];
    for _ in 0..NEARBY_DEPTH {
        let mut next = Vec::new();
        for dir in level {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut children: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .map(|entry| entry.path())
                .collect();
            children.sort();
            for child in children {
                if child.join(".git").exists() {
                    found.push(child);
                } else {
                    next.push(child);
                }
            }
        }
        level = next;
    }
    found.truncate(MAX_NEARBY);
    found
}

#[cfg(test)]
mod tests;
