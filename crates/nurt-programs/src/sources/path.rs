// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Executable files in `PATH`.
//!
//! Directories are read in `PATH` order; the first file with a name is the
//! program, later files with the same name are its alternatives. On Unix a
//! file is executable when any execute bit is set; on Windows when its
//! extension is listed in `PATHEXT`. A symbolic link to another program in
//! `PATH` becomes an alias of that program instead of a card of its own; a
//! link through `/etc/alternatives` is an alias of kind `alternative`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use nurt_contracts::env::Os;

use crate::model::{AliasKind, EntryKind, RawProgram, SourceData};
use crate::source::{
    Deadline, Fingerprint, FingerprintBuilder, HostEnv, ProgramSource, SourceError, SourceId,
    SourceKind,
};
use crate::sources::{canonical_dir_path, list_dir, normalize, path_text, resolve_links};

/// Directory of Debian alternatives links.
const ALTERNATIVES_DIR: &str = "/etc/alternatives/";

/// Provider of executable files in `PATH`.
#[derive(Debug, Clone)]
pub struct PathSource {
    id: SourceId,
}

impl PathSource {
    /// Creates the provider.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: SourceId::new(SourceId::PATH),
        }
    }
}

impl Default for PathSource {
    fn default() -> Self {
        Self::new()
    }
}

/// One executable found in `PATH`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathHit {
    /// Command name, without a `PATHEXT` extension on Windows.
    pub name: String,
    /// Host path of the file.
    pub path: PathBuf,
}

/// Result of resolving one name the way the system does.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Resolution {
    /// Name as given.
    pub name: String,
    /// Every matching file in resolution order; the first one is run.
    pub paths: Vec<PathBuf>,
    /// Program the first file links to, when it is another program in
    /// `PATH`.
    pub alias_of: Option<String>,
}

impl ProgramSource for PathSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Path
    }

    fn detect(&self, _env: &HostEnv) -> bool {
        true
    }

    fn fingerprint(&self, env: &HostEnv) -> Result<Fingerprint, SourceError> {
        let list = env
            .var("PATH")
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut builder = FingerprintBuilder::new(env)
            .text("PATH", &list)
            .text("PATHEXT", &env.path_extensions().join(";"));
        for dir in env.path_dirs() {
            builder = builder.path(dir);
        }
        Ok(builder.finish())
    }

    fn load(&self, env: &HostEnv, deadline: &Deadline<'_>) -> Result<SourceData, SourceError> {
        let hits = scan(env, deadline, &self.id)?;
        Ok(SourceData {
            programs: programs_from_hits(env, &hits),
            ..SourceData::default()
        })
    }
}

/// Every executable of every `PATH` directory in resolution order.
///
/// # Errors
///
/// Fails only when the deadline passes.
pub fn scan(
    env: &HostEnv,
    deadline: &Deadline<'_>,
    id: &SourceId,
) -> Result<Vec<PathHit>, SourceError> {
    let mut hits = Vec::new();
    for dir in env.path_dirs() {
        deadline.check(id)?;
        let start = hits.len();
        for (file_name, host) in list_dir(env, &dir) {
            if let Some(name) = executable_name(env, &file_name, &host) {
                hits.push(PathHit { name, path: host });
            }
        }
        order_by_pathext(env, &mut hits[start..]);
    }
    Ok(hits)
}

/// Orders the files of one directory that share a command name by the
/// position of their extension in `PATHEXT`, the order in which the system
/// tries them; without `PATHEXT` extensions the order is unchanged.
fn order_by_pathext(env: &HostEnv, hits: &mut [PathHit]) {
    let extensions = env.path_extensions();
    if extensions.is_empty() {
        return;
    }
    hits.sort_by_cached_key(|hit| {
        let file_name = hit
            .path
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_uppercase())
            .unwrap_or_default();
        let rank = extensions
            .iter()
            .position(|extension| file_name.ends_with(extension.as_str()));
        (env.name_key(&hit.name), rank)
    });
}

/// Command name of a directory entry when it is an executable file.
fn executable_name(env: &HostEnv, file_name: &str, host: &Path) -> Option<String> {
    let name = env.command_name(file_name)?;
    let meta = std::fs::metadata(env.sys_path(host)).ok()?;
    if !meta.is_file() {
        return None;
    }
    if env.os() != Os::Windows && !is_executable(&meta) {
        return None;
    }
    Some(name)
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    const ANY_EXECUTE: u32 = 0o111;
    meta.permissions().mode() & ANY_EXECUTE != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &std::fs::Metadata) -> bool {
    true
}

/// Groups hits by name: the first hit is the program, later hits are its
/// alternatives; links to another program become aliases.
fn programs_from_hits(env: &HostEnv, hits: &[PathHit]) -> Vec<RawProgram> {
    let mut order: Vec<String> = Vec::new();
    let mut by_key: HashMap<String, RawProgram> = HashMap::new();
    let mut first_paths: HashMap<String, PathBuf> = HashMap::new();
    for hit in hits {
        let key = env.name_key(&hit.name);
        if let Some(program) = by_key.get_mut(&key) {
            program.alternatives.push(path_text(&hit.path));
            continue;
        }
        let mut program = RawProgram::new(hit.name.clone());
        program.kind = EntryKind::Program;
        program.path = Some(path_text(&hit.path));
        program.installed = Some(true);
        first_paths.insert(key.clone(), hit.path.clone());
        order.push(key.clone());
        by_key.insert(key, program);
    }
    let mut targets: HashMap<PathBuf, String> = HashMap::new();
    for key in &order {
        let Some(path) = first_paths.get(key) else {
            continue;
        };
        let Some(resolved) = resolve_links(env, path) else {
            continue;
        };
        if std::fs::read_link(env.sys_path(path)).is_ok() {
            targets.entry(resolved).or_insert_with(|| key.clone());
        } else {
            targets.insert(resolved, key.clone());
        }
    }
    for key in &order {
        let Some(path) = first_paths.get(key) else {
            continue;
        };
        if let Some((target, kind)) = link_target(env, path, &targets, key)
            && let Some(program) = by_key.get_mut(key)
        {
            program.alias_of = Some(target);
            program.alias_kind = Some(kind);
        }
    }
    order
        .into_iter()
        .filter_map(|key| by_key.remove(&key))
        .collect()
}

/// Program in `PATH` that `path` links to, other than itself.
fn link_target(
    env: &HostEnv,
    path: &Path,
    targets: &HashMap<PathBuf, String>,
    own_key: &str,
) -> Option<(String, AliasKind)> {
    let link = std::fs::read_link(env.sys_path(path)).ok()?;
    let resolved = resolve_links(env, path)?;
    let target_key = targets
        .get(&resolved)
        .filter(|key| key.as_str() != own_key)
        .cloned()
        .or_else(|| {
            let name = resolved.file_name()?.to_str()?;
            let key = env.name_key(&env.command_name(name)?);
            (key != own_key && targets.values().any(|known| *known == key)).then_some(key)
        })?;
    let kind = if path_text(&link).starts_with(ALTERNATIVES_DIR) {
        AliasKind::Alternative
    } else {
        AliasKind::Symlink
    };
    Some((target_key, kind))
}

/// Resolves `name` the way the system starts it: every directory of `PATH`
/// in order; on Windows a name without extension is tried with each
/// `PATHEXT` extension. A name with a path separator is checked as it is,
/// without a search of `PATH`; a relative one is taken relative to the base
/// directory.
#[must_use]
pub fn resolve(env: &HostEnv, name: &str) -> Resolution {
    resolve_from(env, name, None)
}

/// As [`resolve`], with the directory a relative name that contains a path
/// separator is taken relative to; without it such a name is not found.
#[must_use]
pub fn resolve_from(env: &HostEnv, name: &str, base: Option<&Path>) -> Resolution {
    let mut resolution = Resolution {
        name: name.to_owned(),
        ..Resolution::default()
    };
    if name.contains('/') || (env.os() == Os::Windows && name.contains('\\')) {
        let written = PathBuf::from(name);
        let place = if env.is_absolute(&written) {
            Some(written)
        } else {
            base.map(|base| normalize(&base.join(&written)))
        };
        if let Some(place) = place {
            for candidate in candidate_names(env, &place.to_string_lossy()) {
                let host = PathBuf::from(candidate);
                if is_runnable(env, &host) && !resolution.paths.contains(&host) {
                    resolution.paths.push(host);
                }
            }
        }
        return resolution;
    }
    let candidates = candidate_names(env, name);
    for dir in env.path_dirs() {
        for candidate in &candidates {
            let host = dir.join(candidate);
            if is_runnable(env, &host) && !resolution.paths.contains(&host) {
                resolution.paths.push(host);
            }
        }
    }
    if let Some(first) = resolution.paths.first() {
        resolution.alias_of = alias_name(env, name, first);
    }
    resolution
}

/// File names tried for `name`: the name itself, and on Windows the name
/// with every `PATHEXT` extension when it has none of them.
fn candidate_names(env: &HostEnv, name: &str) -> Vec<String> {
    if env.os() != Os::Windows {
        return vec![name.to_owned()];
    }
    let upper = name.to_ascii_uppercase();
    let extensions = env.path_extensions();
    if extensions
        .iter()
        .any(|extension| upper.ends_with(extension.as_str()))
    {
        return vec![name.to_owned()];
    }
    extensions
        .iter()
        .map(|extension| format!("{name}{}", extension.to_ascii_lowercase()))
        .collect()
}

fn is_runnable(env: &HostEnv, host: &Path) -> bool {
    let target = resolve_links(env, host).unwrap_or_else(|| host.to_path_buf());
    let Ok(meta) = std::fs::metadata(env.sys_path(&target)) else {
        return false;
    };
    meta.is_file() && (env.os() == Os::Windows || is_executable(&meta))
}

/// Name of the program a link resolves to, when it differs from `name` and
/// the target lies in a directory of `PATH`.
fn alias_name(env: &HostEnv, name: &str, path: &Path) -> Option<String> {
    std::fs::read_link(env.sys_path(path)).ok()?;
    let resolved = resolve_links(env, path)?;
    let file_name = resolved.file_name()?.to_str()?;
    let target = env.command_name(file_name)?;
    (env.name_key(&target) != env.name_key(name) && in_path_dirs(env, &resolved)).then_some(target)
}

/// Returns `true` when `file` lies directly in a directory of `PATH`; links
/// in the directory part are resolved, so `/bin/ls` matches `/usr/bin`.
fn in_path_dirs(env: &HostEnv, file: &Path) -> bool {
    let Some(name) = file.file_name() else {
        return false;
    };
    let wanted = canonical_dir_path(env, file);
    env.path_dirs()
        .iter()
        .any(|dir| canonical_dir_path(env, &dir.join(name)) == wanted)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn executable(root: &Path, host: &str) {
        let file = root.join(host.trim_start_matches('/'));
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).unwrap_or_default();
        }
        std::fs::write(&file, b"#!/bin/sh\n").unwrap_or_default();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap_or_default();
    }

    #[test]
    fn scan_skips_files_without_execute_bit() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        executable(dir.path(), "/bin/a");
        std::fs::write(dir.path().join("bin/plain"), b"").unwrap_or_default();
        let env = HostEnv::new(Os::Linux)
            .with_root(dir.path())
            .with_var("PATH", "/bin");
        let hits = scan(&env, &Deadline::none(), &SourceId::new("path")).unwrap_or_default();
        let names: Vec<&str> = hits.iter().map(|hit| hit.name.as_str()).collect();
        assert_eq!(names, vec!["a"]);
    }

    #[test]
    fn links_to_one_program_all_name_the_real_file() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        executable(dir.path(), "/bin/vim.basic");
        let links = ["vi", "view", "ex", "rview", "rvim", "vimdiff"];
        for link in links {
            std::os::unix::fs::symlink("vim.basic", dir.path().join("bin").join(link))
                .unwrap_or_default();
        }
        let env = HostEnv::new(Os::Linux)
            .with_root(dir.path())
            .with_var("PATH", "/bin");
        let hits = scan(&env, &Deadline::none(), &SourceId::new("path")).unwrap_or_default();
        let programs = programs_from_hits(&env, &hits);
        assert_eq!(programs.len(), links.len() + 1);
        for program in &programs {
            if program.name == "vim.basic" {
                assert_eq!(program.alias_of, None);
            } else {
                assert_eq!(program.alias_of.as_deref(), Some("vim.basic"));
                assert_eq!(program.alias_kind, Some(AliasKind::Symlink));
            }
        }
    }

    #[test]
    fn alias_of_a_link_names_only_targets_in_path() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        executable(dir.path(), "/bin/vim");
        executable(dir.path(), "/opt/x/bar");
        std::os::unix::fs::symlink("vim", dir.path().join("bin/vi")).unwrap_or_default();
        std::os::unix::fs::symlink("../opt/x/bar", dir.path().join("bin/foo")).unwrap_or_default();
        let env = HostEnv::new(Os::Linux)
            .with_root(dir.path())
            .with_var("PATH", "/bin");
        assert_eq!(resolve(&env, "vi").alias_of.as_deref(), Some("vim"));
        let outside = resolve(&env, "foo");
        assert_eq!(outside.paths, vec![PathBuf::from("/bin/foo")]);
        assert_eq!(outside.alias_of, None);
    }

    #[test]
    fn scan_orders_files_of_one_name_by_pathext() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        for file in ["git.cmd", "git.bat", "git.exe", "git.txt"] {
            executable(dir.path(), &format!("/c/Tools/{file}"));
        }
        let env = HostEnv::new(Os::Windows)
            .with_root(dir.path())
            .with_var("PATH", "C:\\Tools");
        let hits = scan(&env, &Deadline::none(), &SourceId::new("path")).unwrap_or_default();
        let files: Vec<&str> = hits
            .iter()
            .filter_map(|hit| hit.path.file_name()?.to_str())
            .collect();
        assert_eq!(files, vec!["git.exe", "git.bat", "git.cmd"]);
        assert!(hits.iter().all(|hit| hit.name == "git"));
        let programs = programs_from_hits(&env, &hits);
        assert_eq!(programs.len(), 1);
        assert!(
            programs[0]
                .path
                .as_deref()
                .is_some_and(|path| path.ends_with("git.exe"))
        );
    }

    #[test]
    fn windows_candidates_use_pathext() {
        let env = HostEnv::new(Os::Windows).with_var("PATHEXT", ".EXE;.CMD");
        assert_eq!(candidate_names(&env, "git"), vec!["git.exe", "git.cmd"]);
        assert_eq!(candidate_names(&env, "git.EXE"), vec!["git.EXE"]);
    }
}
