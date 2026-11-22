// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Lookup of the program file in the search path of the child environment.
//!
//! The search uses `PATH` of the environment the program receives, not of the
//! console process, so a `PATH` given in `env` takes effect. Empty entries of
//! `PATH` are skipped; the working directory is searched only when the
//! program is written with a directory, such as `./tool`.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::error::{Candidate, Error, ErrorCode, Result};
use nurt_contracts::record::Launcher;

use super::{ChildEnv, Platform, program_text};

/// Extensions tried on Windows when `PATHEXT` is not set.
const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";
/// Largest number of similar names offered when a program is not found.
const MAX_CANDIDATES: usize = 5;
/// Edit distance accepted for every similar name, however short.
const MIN_DISTANCE_LIMIT: usize = 2;
/// A longer similar name may differ in one character of every this many.
const DISTANCE_RATIO: usize = 3;

/// Program file found for a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resolved {
    /// The program file.
    pub(crate) path: PathBuf,
    /// How the program is started.
    pub(crate) launcher: Launcher,
}

/// State of a possible program file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileState {
    Runnable,
    NotExecutable,
    Directory,
    Missing,
}

/// Finds the program named by the first element of `command`.
///
/// A name with a directory is taken relative to `cwd`; any other name is
/// looked up in `PATH` of `env`. On Windows the extensions of `PATHEXT` are
/// tried after the name as written, and `.bat` and `.cmd` files are started
/// through the command interpreter.
///
/// # Errors
///
/// Fails with `E_PROGRAM_NOT_FOUND`, listing similar names of the search path
/// as candidates, and with `E_SPAWN` when the named file is a directory or
/// lacks execute permission.
pub(crate) fn resolve(command: &[String], cwd: &Path, env: &ChildEnv) -> Result<Resolved> {
    let name = command.first().map_or("", String::as_str);
    let platform = env.platform();
    let extensions = extensions(env);
    let path = if has_directory(name, platform) {
        find_explicit(name, cwd, platform, &extensions)?
    } else {
        search_path(env)
            .iter()
            .flat_map(|dir| variants(&dir.join(name), platform, &extensions))
            .find(|candidate| file_state(candidate, platform) == FileState::Runnable)
            .ok_or_else(|| not_found(command, cwd, env, &extensions))?
    };
    Ok(Resolved {
        launcher: launcher_of(&path, platform),
        path,
    })
}

fn find_explicit(
    name: &str,
    cwd: &Path,
    platform: Platform,
    extensions: &[String],
) -> Result<PathBuf> {
    let base = cwd.join(name);
    let mut blocked = None;
    for candidate in variants(&base, platform, extensions) {
        match file_state(&candidate, platform) {
            FileState::Runnable => return Ok(candidate),
            FileState::Missing => {}
            state => {
                blocked.get_or_insert((candidate, state));
            }
        }
    }
    match blocked {
        Some((path, FileState::Directory)) => Err(Error::new(
            ErrorCode::Spawn,
            format!(
                "{} is a directory; give the program file as the first element",
                path.display()
            ),
        )
        .with_problem("program_is_directory")),
        Some((path, _)) => Err(Error::new(
            ErrorCode::Spawn,
            format!(
                "{} has no execute permission; grant it or start the file through its interpreter",
                path.display()
            ),
        )
        .with_problem("not_executable")),
        None => Err(Error::new(
            ErrorCode::ProgramNotFound,
            format!(
                "program {} does not exist; check the path relative to the working directory",
                base.display()
            ),
        )
        .with_found(name)
        .with_problem("missing_file")),
    }
}

/// Returns `true` when `name` names a directory, so `PATH` is not searched.
fn has_directory(name: &str, platform: Platform) -> bool {
    match platform {
        Platform::Unix => name.contains('/'),
        Platform::Windows => {
            name.contains(['/', '\\']) || name.as_bytes().get(1).is_some_and(|byte| *byte == b':')
        }
    }
}

/// Directories of `PATH` in the child environment.
fn search_path(env: &ChildEnv) -> Vec<PathBuf> {
    let Some(value) = env.get("PATH") else {
        return Vec::new();
    };
    match env.platform() {
        Platform::Unix => split_unix(value),
        Platform::Windows => value
            .to_string_lossy()
            .split(';')
            .map(|entry| entry.trim().trim_matches('"'))
            .filter(|entry| !entry.is_empty())
            .map(PathBuf::from)
            .collect(),
    }
}

#[cfg(unix)]
fn split_unix(value: &OsStr) -> Vec<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    value
        .as_bytes()
        .split(|byte| *byte == b':')
        .filter(|entry| !entry.is_empty())
        .map(|entry| PathBuf::from(OsStr::from_bytes(entry)))
        .collect()
}

#[cfg(not(unix))]
fn split_unix(value: &OsStr) -> Vec<PathBuf> {
    value
        .to_string_lossy()
        .split(':')
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Extensions of `PATHEXT` on Windows, in order; none on other systems.
fn extensions(env: &ChildEnv) -> Vec<String> {
    if env.platform() == Platform::Unix {
        return Vec::new();
    }
    let value = env
        .get("PATHEXT")
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_PATHEXT.to_owned());
    value
        .split(';')
        .map(str::trim)
        .filter(|extension| extension.starts_with('.') && extension.len() > 1)
        .map(str::to_owned)
        .collect()
}

/// Files tried for one program path, in order.
fn variants(base: &Path, platform: Platform, extensions: &[String]) -> Vec<PathBuf> {
    let mut list = Vec::new();
    if platform == Platform::Unix || base.extension().is_some() {
        list.push(base.to_path_buf());
    }
    if platform == Platform::Windows {
        for extension in extensions {
            let mut name = base.as_os_str().to_owned();
            name.push(extension);
            list.push(PathBuf::from(name));
        }
    }
    list
}

fn file_state(path: &Path, platform: Platform) -> FileState {
    match fs::metadata(path) {
        Err(_) => FileState::Missing,
        Ok(metadata) if metadata.is_dir() => FileState::Directory,
        Ok(metadata) if platform == Platform::Unix && !is_executable(&metadata) => {
            FileState::NotExecutable
        }
        Ok(_) => FileState::Runnable,
    }
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    /// Execute permission of the account, the group and others.
    const EXECUTE_BITS: u32 = 0o111;
    metadata.permissions().mode() & EXECUTE_BITS != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &fs::Metadata) -> bool {
    true
}

fn launcher_of(path: &Path, platform: Platform) -> Launcher {
    let batch = path.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("bat") || extension.eq_ignore_ascii_case("cmd")
    });
    if platform == Platform::Windows && batch {
        Launcher::Cmd
    } else {
        Launcher::Direct
    }
}

/// Name used to compare program names: on Windows without case and without
/// an extension of `PATHEXT`. `None` when the file name cannot name a
/// program on Windows.
fn comparable_name(file_name: &str, platform: Platform, extensions: &[String]) -> Option<String> {
    match platform {
        Platform::Unix => Some(file_name.to_owned()),
        Platform::Windows => {
            let lower = file_name.to_lowercase();
            extensions.iter().find_map(|extension| {
                lower
                    .strip_suffix(&extension.to_lowercase())
                    .filter(|stem| !stem.is_empty())
                    .map(str::to_owned)
            })
        }
    }
}

fn not_found(command: &[String], cwd: &Path, env: &ChildEnv, extensions: &[String]) -> Error {
    let name = command.first().map_or("", String::as_str);
    let platform = env.platform();
    let wanted = match platform {
        Platform::Unix => name.to_owned(),
        Platform::Windows => {
            comparable_name(name, platform, extensions).unwrap_or_else(|| name.to_lowercase())
        }
    };
    let mut error = Error::new(
        ErrorCode::ProgramNotFound,
        format!(
            "program `{name}` was not found in PATH; give its full path or add its directory to PATH in `env`"
        ),
    )
    .with_found(name)
    .with_problem("not_in_path");
    let local = variants(&cwd.join(name), platform, extensions)
        .into_iter()
        .any(|path| file_state(&path, platform) == FileState::Runnable);
    if local {
        let mut replaced = command.to_vec();
        replaced[0] = format!("./{name}");
        error = error.with_fix(program_text(&replaced));
    }
    for (rank, (display, path)) in similar_names(&wanted, env, extensions)
        .into_iter()
        .enumerate()
    {
        let mut replaced = command.to_vec();
        replaced[0].clone_from(&display);
        let program = program_text(&replaced);
        if rank == 0 && !local {
            error = error.with_fix(program.clone());
        }
        error = error.with_candidate(Candidate {
            rank: u32::try_from(rank + 1).unwrap_or(u32::MAX),
            reference: None,
            program,
            why: Some(format!(
                "similar name in the search path: {}",
                path.display()
            )),
        });
    }
    error
}

/// Up to [`MAX_CANDIDATES`] program names of the search path closest to
/// `wanted`, with their files, best first.
fn similar_names(wanted: &str, env: &ChildEnv, extensions: &[String]) -> Vec<(String, PathBuf)> {
    let platform = env.platform();
    let limit = MIN_DISTANCE_LIMIT.max(wanted.chars().count() / DISTANCE_RATIO);
    let mut scored: Vec<(usize, String, String, PathBuf)> = Vec::new();
    for dir in search_path(env) {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let Some(key) = comparable_name(&file_name, platform, extensions) else {
                continue;
            };
            let distance = edit_distance(wanted, &key);
            if distance == 0 || distance > limit {
                continue;
            }
            let path = entry.path();
            if file_state(&path, platform) != FileState::Runnable {
                continue;
            }
            let display = match platform {
                Platform::Unix => file_name,
                Platform::Windows => file_name
                    .get(..key.len())
                    .map_or_else(|| key.clone(), str::to_owned),
            };
            scored.push((distance, key, display, path));
        }
    }
    scored.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));
    scored.dedup_by(|later, earlier| later.1 == earlier.1);
    scored
        .into_iter()
        .take(MAX_CANDIDATES)
        .map(|(_, _, display, path)| (display, path))
        .collect()
}

/// Levenshtein distance between two texts, counted in characters.
fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];
    for (row, left_char) in left.chars().enumerate() {
        current[0] = row + 1;
        for (column, right_char) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(left_char != *right_char);
            current[column + 1] = substitution
                .min(previous[column + 1] + 1)
                .min(current[column] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

#[cfg(test)]
mod tests {
    use super::edit_distance;

    #[test]
    fn counts_insertions_deletions_and_substitutions() {
        assert_eq!(edit_distance("cargo", "cargo"), 0);
        assert_eq!(edit_distance("carg", "cargo"), 1);
        assert_eq!(edit_distance("cargoo", "cargo"), 1);
        assert_eq!(edit_distance("crago", "cargo"), 2);
        assert_eq!(edit_distance("", "ls"), 2);
        assert_eq!(edit_distance("naïve", "naive"), 1);
    }
}
