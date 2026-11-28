// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/core/location.rs
// Responsibility:   Check of the folder of nurt.exe: form of the path, scope rules, write access and free space.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Check of the folder of `nurt.exe`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::model::{Issue, LocationReport, Options, Scope, UserPaths};
use crate::paths;
use crate::sys;

/// Characters that a name of Windows cannot hold.
const FORBIDDEN: [char; 6] = ['<', '>', '"', '|', '?', '*'];

/// Resolves the folder of the options: variables are expanded, an empty value gives the default of the scope.
pub(crate) fn resolve(options: &Options, user: &UserPaths) -> PathBuf {
    let text = options.dir.trim().trim_matches('"');
    if text.is_empty() {
        return paths::default_bin(options.scope, user);
    }
    let expanded = paths::expand_env(text).replace('/', "\\");
    let trimmed = expanded.trim_end_matches('\\');
    if trimmed.len() == 2 && trimmed.ends_with(':') {
        return PathBuf::from(format!("{trimmed}\\"));
    }
    PathBuf::from(trimmed)
}

fn inside(path: &Path, root: &Path) -> bool {
    let left = path.to_string_lossy().to_ascii_lowercase();
    let right = root.to_string_lossy().to_ascii_lowercase();
    let right = right.trim_end_matches('\\');
    left == right || left.starts_with(&format!("{right}\\"))
}

fn writable(directory: &Path) -> bool {
    let probe = directory.join(format!(".nurt-setup-probe-{}", std::process::id()));
    match fs::File::create(&probe) {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// A full path with a drive letter and no character that Windows rejects.
pub(crate) fn is_full_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\' {
        return false;
    }
    let tail = &text[3..];
    !tail.is_empty()
        && !tail.chars().any(|character| {
            FORBIDDEN.contains(&character) || character == ':' || (character as u32) < 0x20
        })
        && !tail
            .split('\\')
            .any(|name| name.is_empty() || name.ends_with(' ') || name.ends_with('.'))
}

/// Checks the folder of the options.
///
/// `elevated` tells whether this process can write below `%ProgramFiles%`; for the scope of all
/// users the write access is checked in the elevated worker instead.
pub(crate) fn check(
    options: &Options,
    user: &UserPaths,
    elevated: bool,
    required: u64,
) -> LocationReport {
    let directory = resolve(options, user);
    let text = directory.to_string_lossy().into_owned();
    let mut issue = None;
    if !is_full_path(&text) {
        issue = Some(Issue::NotFullPath);
    } else if options.scope == Scope::Machine
        && !paths::program_files_roots()
            .iter()
            .any(|root| inside(&directory, root))
    {
        issue = Some(Issue::OutsideProgramFiles);
    } else if directory.is_file() {
        issue = Some(Issue::IsFile);
    }
    let mut free = 0;
    if issue.is_none()
        && let Some(existing) = paths::nearest_existing(&directory)
    {
        free = sys::free_space(&existing).unwrap_or(0);
        let checks_write = options.scope == Scope::User || elevated;
        if checks_write && existing.is_dir() && !writable(&existing) {
            issue = Some(Issue::NotWritable);
        } else if free != 0 && free < required {
            issue = Some(Issue::NoSpace);
        }
    }
    LocationReport {
        dir: text,
        issue,
        free,
        required,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> UserPaths {
        UserPaths {
            profile: PathBuf::from("C:\\Users\\user"),
            appdata: PathBuf::from("C:\\Users\\user\\AppData\\Roaming"),
            local_appdata: PathBuf::from("C:\\Users\\user\\AppData\\Local"),
            documents: PathBuf::from("C:\\Users\\user\\Documents"),
            desktop: PathBuf::from("C:\\Users\\user\\Desktop"),
        }
    }

    #[test]
    fn empty_directory_selects_the_default() {
        assert_eq!(
            resolve(&Options::default(), &user()),
            PathBuf::from("C:\\Users\\user\\AppData\\Local\\Programs\\Nurt\\bin")
        );
    }

    #[test]
    fn slashes_and_trailing_separators_are_normalised() {
        let options = Options {
            dir: "\"D:/Tools/Nurt/bin/\"".into(),
            ..Options::default()
        };
        assert_eq!(
            resolve(&options, &user()),
            PathBuf::from("D:\\Tools\\Nurt\\bin")
        );
    }

    #[test]
    fn full_paths_follow_the_rule_of_the_design() {
        assert!(is_full_path("C:\\Tools\\Nurt\\bin"));
        assert!(!is_full_path("tools\\nurt"));
        assert!(!is_full_path("C:\\a|b"));
        assert!(!is_full_path("C:\\"));
        assert!(!is_full_path("C:\\a\\"));
        assert!(!is_full_path("C:\\a\\b."));
    }

    fn scratch(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("nurt-location-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn with_dir(dir: &str, scope: Scope) -> Options {
        Options {
            dir: dir.into(),
            scope,
            ..Options::default()
        }
    }

    #[test]
    fn a_relative_folder_is_not_a_full_path() {
        let report = check(&with_dir("tools\\nurt", Scope::User), &user(), false, 1);
        assert_eq!(report.issue, Some(Issue::NotFullPath));
    }

    #[test]
    fn all_users_need_a_folder_inside_program_files() {
        let outside = check(
            &with_dir("D:\\Tools\\Nurt\\bin", Scope::Machine),
            &user(),
            true,
            1,
        );
        assert_eq!(outside.issue, Some(Issue::OutsideProgramFiles));
        let inside = check(
            &with_dir("%ProgramFiles%\\Nurt\\bin", Scope::Machine),
            &user(),
            false,
            1,
        );
        assert_ne!(inside.issue, Some(Issue::OutsideProgramFiles));
        assert!(inside.dir.to_ascii_lowercase().contains("program files"));
    }

    #[test]
    fn a_file_is_not_a_folder() {
        let directory = scratch("file");
        fs::create_dir_all(&directory).unwrap_or_default();
        let file = directory.join("nurt");
        fs::write(&file, b"x").unwrap_or_default();
        let report = check(
            &with_dir(&file.display().to_string(), Scope::User),
            &user(),
            false,
            1,
        );
        assert_eq!(report.issue, Some(Issue::IsFile));
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn free_space_decides_the_last_check() {
        let directory = scratch("space");
        let options = with_dir(
            &directory.join("Nurt\\bin").display().to_string(),
            Scope::User,
        );
        let enough = check(&options, &user(), false, 1024);
        assert_eq!(enough.issue, None);
        assert!(enough.free > 1024 && enough.required == 1024);
        let short = check(&options, &user(), false, u64::MAX);
        assert_eq!(short.issue, Some(Issue::NoSpace));
        assert!(!directory.exists(), "the check creates nothing");
    }

    #[test]
    fn a_folder_that_cannot_be_written_is_reported() {
        let directory = scratch("denied");
        fs::create_dir_all(&directory).unwrap_or_default();
        let denied = std::process::Command::new("icacls")
            .arg(&directory)
            .args(["/deny", "*S-1-1-0:(OI)(CI)(WD,AD)"])
            .output()
            .is_ok_and(|output| output.status.success());
        let report = check(
            &with_dir(&directory.join("bin").display().to_string(), Scope::User),
            &user(),
            false,
            1,
        );
        let _ = std::process::Command::new("icacls")
            .arg(&directory)
            .args(["/remove:d", "*S-1-1-0"])
            .output();
        let _ = fs::remove_dir_all(&directory);
        assert!(denied, "icacls refused the test folder");
        assert_eq!(report.issue, Some(Issue::NotWritable));
    }

    #[test]
    fn variables_of_the_environment_are_expanded() {
        let report = check(
            &with_dir("%TEMP%\\nurt-location-env\\bin", Scope::User),
            &user(),
            false,
            1,
        );
        assert!(!report.dir.contains('%'), "{}", report.dir);
        assert_eq!(report.issue, None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
