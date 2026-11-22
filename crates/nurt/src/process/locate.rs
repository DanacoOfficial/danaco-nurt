// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Location of the `nurt` executable.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::error::{Error, ErrorKind};

/// Variable that names the `nurt` executable.
pub(crate) const EXECUTABLE_VARIABLE: &str = "NURT_EXECUTABLE";

/// Finds the executable: the explicit path, then the path in
/// [`EXECUTABLE_VARIABLE`], then the directories of `PATH`.
///
/// An explicit path or a path in the variable must name an existing file;
/// the search does not fall back to `PATH` when it does not, so that a
/// mistake in the configuration is reported instead of starting another
/// executable.
pub(crate) fn locate(explicit: Option<&Path>) -> Result<PathBuf, Error> {
    locate_with(
        explicit,
        std::env::var_os(EXECUTABLE_VARIABLE),
        std::env::var_os("PATH"),
    )
}

pub(crate) fn locate_with(
    explicit: Option<&Path>,
    variable: Option<OsString>,
    search_path: Option<OsString>,
) -> Result<PathBuf, Error> {
    if let Some(path) = explicit {
        return existing(path.to_path_buf(), "the configured executable");
    }
    if let Some(path) = variable.filter(|value| !value.is_empty()) {
        return existing(PathBuf::from(path), EXECUTABLE_VARIABLE);
    }
    let name = format!("nurt{}", std::env::consts::EXE_SUFFIX);
    let candidates: Vec<PathBuf> = search_path
        .iter()
        .flat_map(std::env::split_paths)
        .filter(|directory| !directory.as_os_str().is_empty())
        .map(|directory| directory.join(&name))
        .collect();
    if let Some(found) = candidates.iter().find(|candidate| candidate.is_file()) {
        return Ok(found.clone());
    }
    let checked = if candidates.is_empty() {
        "PATH is empty".to_owned()
    } else {
        join_paths(&candidates)
    };
    Err(Error::new(
        ErrorKind::NotFound,
        format!(
            "nurt executable not found; checked {EXECUTABLE_VARIABLE} (not set) and PATH: \
             {checked}; set {EXECUTABLE_VARIABLE} or pass the path of the executable"
        ),
    ))
}

fn existing(path: PathBuf, source: &str) -> Result<PathBuf, Error> {
    if path.is_file() {
        Ok(path)
    } else {
        Err(Error::new(
            ErrorKind::NotFound,
            format!(
                "nurt executable not found at {} given by {source}; correct the path",
                path.display()
            ),
        ))
    }
}

fn join_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn executable_in(directory: &Path) -> PathBuf {
        let path = directory.join(format!("nurt{}", std::env::consts::EXE_SUFFIX));
        fs::write(&path, b"").unwrap();
        path
    }

    #[test]
    fn explicit_path_comes_first() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let explicit = executable_in(first.path());
        let from_variable = executable_in(second.path());
        let found = locate_with(
            Some(&explicit),
            Some(from_variable.into_os_string()),
            Some(second.path().as_os_str().to_owned()),
        )
        .unwrap();
        assert_eq!(found, explicit);
    }

    #[test]
    fn variable_comes_before_search_path() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let from_variable = executable_in(first.path());
        executable_in(second.path());
        let found = locate_with(
            None,
            Some(from_variable.clone().into_os_string()),
            Some(second.path().as_os_str().to_owned()),
        )
        .unwrap();
        assert_eq!(found, from_variable);
    }

    #[test]
    fn search_path_is_scanned_in_order() {
        let empty = tempfile::tempdir().unwrap();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let expected = executable_in(first.path());
        executable_in(second.path());
        let joined = std::env::join_paths([empty.path(), first.path(), second.path()]).unwrap();
        let found = locate_with(None, None, Some(joined)).unwrap();
        assert_eq!(found, expected);
    }

    #[test]
    fn empty_variable_is_ignored() {
        let directory = tempfile::tempdir().unwrap();
        let expected = executable_in(directory.path());
        let found = locate_with(
            None,
            Some(OsString::new()),
            Some(directory.path().as_os_str().to_owned()),
        )
        .unwrap();
        assert_eq!(found, expected);
    }

    #[test]
    fn missing_explicit_path_is_an_error_without_fallback() {
        let directory = tempfile::tempdir().unwrap();
        executable_in(directory.path());
        let missing = directory.path().join("missing").join("nurt");
        let error = locate_with(
            Some(&missing),
            None,
            Some(directory.path().as_os_str().to_owned()),
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::NotFound);
        assert!(error.message().contains("the configured executable"));
    }

    #[test]
    fn missing_variable_path_names_the_variable() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("nurt-missing");
        let error = locate_with(None, Some(missing.into_os_string()), None).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::NotFound);
        assert!(error.message().contains(EXECUTABLE_VARIABLE));
    }

    #[test]
    fn directory_is_not_an_executable() {
        let directory = tempfile::tempdir().unwrap();
        let error = locate_with(Some(directory.path()), None, None).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn not_found_lists_every_checked_location() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let joined = std::env::join_paths([first.path(), second.path()]).unwrap();
        let error = locate_with(None, None, Some(joined)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::NotFound);
        for directory in [first.path(), second.path()] {
            assert!(error.message().contains(&directory.display().to_string()));
        }
    }

    #[test]
    fn missing_search_path_is_reported() {
        let error = locate_with(None, None, None).unwrap_err();
        assert!(error.message().contains("PATH is empty"));
    }
}
