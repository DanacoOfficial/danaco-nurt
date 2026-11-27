// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syslog::journald::discover
// Responsibility:   Selection of the journal files of the local machine.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Selection of the journal files of the local machine.
//!
//! The persistent journal lives in `/var/log/journal/<machine-id>/`, the
//! volatile one in `/run/log/journal/<machine-id>/`. Directories of other
//! journal namespaces (`<machine-id>.<namespace>`), of remote hosts and of
//! containers are not read. Files named `*.journal` (active and archived) and
//! `*.journal~` (renamed after an unclean shutdown) are read; every other
//! file is ignored. A file whose header names another machine is skipped.
//! Without `/etc/machine-id` every directory named with 32 hexadecimal
//! digits is read.

use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use nurt_contracts::error::{Error, ErrorCode};

use crate::cursor::Id128;

use super::file::JournalFile;
use super::header::Header;
use super::reader::Source;

const JOURNAL_DIRECTORIES: [&str; 2] = ["var/log/journal", "run/log/journal"];
const MACHINE_ID_FILE: &str = "etc/machine-id";
const ACTIVE_SUFFIX: &str = ".journal";
const DIRTY_SUFFIX: &str = ".journal~";
const HEADER_CACHE_BYTES: u64 = 0;

/// Readable journal file with its header.
#[derive(Debug, Clone)]
pub(super) struct Candidate {
    /// Path of the file.
    pub(super) path: PathBuf,
    /// Header read when the file was selected.
    pub(super) header: Header,
}

/// Result of the selection of files.
#[derive(Debug, Default)]
pub(super) struct Discovery {
    /// Files with a readable header.
    pub(super) candidates: Vec<Candidate>,
    /// Files and directories that could not be opened for lack of
    /// permission.
    pub(super) denied: u64,
    /// Files skipped with the class of their defect.
    pub(super) skipped: Vec<(String, &'static str)>,
}

/// Selects the journal files under the system root `root`.
pub(super) fn discover(root: &Path) -> Result<Discovery, Error> {
    let machine_id = fs::read_to_string(root.join(MACHINE_ID_FILE))
        .ok()
        .and_then(|text| text.trim().parse::<Id128>().ok());
    let mut directories = Vec::new();
    for base in JOURNAL_DIRECTORIES.map(|base| root.join(base)) {
        match machine_id {
            Some(id) => directories.push(base.join(id.to_string())),
            None => directories.extend(machine_directories(&base)),
        }
    }
    directories.retain(|directory| directory.is_dir());
    if directories.is_empty() {
        return Err(Error::new(
            ErrorCode::NotFound,
            format!(
                "no systemd journal under `{}`; read text logs such as /var/log/syslog with `read`",
                root.display()
            ),
        )
        .with_help("logs"));
    }
    let mut discovery = Discovery::default();
    for directory in directories {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                discovery.denied += 1;
                continue;
            }
            Err(_) => {
                discovery
                    .skipped
                    .push((directory.display().to_string(), "io"));
                continue;
            }
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| is_journal_name(path))
            .collect();
        paths.sort();
        for path in paths {
            inspect(path, machine_id, &mut discovery);
        }
    }
    Ok(discovery)
}

/// Reads the headers of the given files without checking the machine.
pub(super) fn from_paths(paths: &[PathBuf]) -> Discovery {
    let mut discovery = Discovery::default();
    for path in paths {
        inspect(path.clone(), None, &mut discovery);
    }
    discovery
}

fn inspect(path: PathBuf, machine_id: Option<Id128>, discovery: &mut Discovery) {
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::PermissionDenied => {
            discovery.denied += 1;
            return;
        }
        Err(_) => {
            discovery.skipped.push((path.display().to_string(), "io"));
            return;
        }
    };
    match JournalFile::open(Source::File(file), HEADER_CACHE_BYTES) {
        Ok(journal) => {
            let header = journal.header().clone();
            if machine_id.is_none_or(|id| id == header.machine_id) {
                discovery.candidates.push(Candidate { path, header });
            }
        }
        Err(fault) => discovery
            .skipped
            .push((path.display().to_string(), fault.class())),
    }
}

fn machine_directories(base: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(base) else {
        return Vec::new();
    };
    let mut directories: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.parse::<Id128>().is_ok())
        })
        .map(|entry| entry.path())
        .collect();
    directories.sort();
    directories
}

fn is_journal_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(ACTIVE_SUFFIX) || name.ends_with(DIRTY_SUFFIX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_active_archived_and_dirty_names() {
        for name in [
            "system.journal",
            "user-1000.journal",
            "system@0123456789abcdef0123456789abcdef-0000000000000001-0005f0a1b2c3d4e5.journal",
            "system@0005f0a1b2c3d4e5-0123456789abcdef.journal~",
        ] {
            assert!(is_journal_name(Path::new(name)), "{name}");
        }
        for name in ["system.journal.tmp", "notes.txt", "journal"] {
            assert!(!is_journal_name(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn missing_journal_is_not_found() {
        let root = tempfile::tempdir().unwrap();
        let error = discover(root.path()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
    }

    #[test]
    fn without_machine_id_reads_machine_directories_only() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().join("var/log/journal");
        let machine = base.join("0123456789abcdef0123456789abcdef");
        fs::create_dir_all(&machine).unwrap();
        fs::create_dir_all(base.join("0123456789abcdef0123456789abcdef.ns")).unwrap();
        fs::create_dir_all(base.join("remote")).unwrap();
        fs::write(machine.join("system.journal"), b"not a journal").unwrap();
        fs::write(machine.join("readme.txt"), b"ignored").unwrap();
        let discovery = discover(root.path()).unwrap();
        assert!(discovery.candidates.is_empty());
        assert_eq!(discovery.skipped.len(), 1);
        assert_eq!(discovery.skipped[0].1, "unsupported_format");
    }

    #[test]
    fn unreadable_paths_are_reported_as_io() {
        let discovery = from_paths(&[PathBuf::from("/nonexistent/system.journal")]);
        assert_eq!(discovery.skipped[0].1, "io");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
