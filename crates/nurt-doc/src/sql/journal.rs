// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Files of a database declared to the operation journal.
//!
//! SQLite writes the database file and, depending on the journal mode, the
//! files `-wal`, `-shm` and `-journal` next to it. All four are declared as
//! external writes before the first change, so that undo restores the
//! database together with its journal files and removes journal files that
//! the change created.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use nurt_contracts::error::Result;
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{
    Expect, ExternalChange, ExternalChangeKind, Preserved, TargetAction, TargetIndex, TargetPath,
    TargetSpec,
};
use nurt_contracts::report::IrreversibleEntry;

use super::SqlRequest;
use super::error;

/// Suffixes of the files SQLite keeps next to a database.
const SIDE_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];

/// Observable state of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FileState {
    exists: bool,
    len: u64,
    modified: Option<SystemTime>,
}

impl FileState {
    /// Size in bytes; zero for a missing file.
    pub(super) fn len(&self) -> u64 {
        self.len
    }
}

/// The database file followed by its journal files.
#[derive(Debug, Clone)]
pub(super) struct DatabaseFiles {
    paths: Vec<AbsPath>,
    labels: Vec<String>,
}

impl DatabaseFiles {
    /// Locates the files of the database at `request.path`. Symbolic links
    /// are resolved, because SQLite names the journal files after the file
    /// the link points to.
    pub(super) fn locate(request: &SqlRequest) -> Result<Self> {
        let database = resolve(&request.path).map_err(|e| error::file_error(&e, request))?;
        let mut paths = vec![AbsPath::new(database.clone())?];
        let mut labels = vec![request.display.clone()];
        for suffix in SIDE_SUFFIXES {
            let mut name = OsString::from(database.as_os_str());
            name.push(suffix);
            paths.push(AbsPath::new(PathBuf::from(name))?);
            labels.push(format!("{}{suffix}", request.display));
        }
        Ok(Self { paths, labels })
    }

    /// Current state of every file, in declaration order.
    pub(super) fn snapshot(&self) -> Vec<FileState> {
        self.paths
            .iter()
            .map(|path| state(path.as_path()))
            .collect()
    }

    /// Declarations of all files as external writes.
    pub(super) fn specs(&self, before: &[FileState]) -> Vec<TargetSpec> {
        self.paths
            .iter()
            .zip(before)
            .map(|(path, state)| TargetSpec {
                path: TargetPath::Fs(path.clone()),
                action: TargetAction::ExternalWrite,
                expect: Expect::Unchecked,
                size_hint: state.exists.then_some(state.len),
            })
            .collect()
    }

    /// Irreversible markers of files whose previous content the journal did
    /// not keep.
    pub(super) fn irreversible(&self, preserved: &[Preserved]) -> Vec<IrreversibleEntry> {
        preserved
            .iter()
            .zip(&self.labels)
            .filter_map(|(item, label)| {
                let mut entry = IrreversibleEntry::new(item.irreversible?);
                entry.path = Some(label.clone());
                Some(entry)
            })
            .collect()
    }

    /// Changes between two snapshots. The database file counts as modified
    /// whenever a changing statement ran, because its size and time may
    /// stay equal within the resolution of the file system.
    pub(super) fn changes(
        &self,
        before: &[FileState],
        after: &[FileState],
        wrote: bool,
    ) -> Vec<ExternalChange> {
        (0_u32..)
            .zip(self.paths.iter().zip(before.iter().zip(after)))
            .filter_map(|(index, (path, (old, new)))| {
                let kind = match (old.exists, new.exists) {
                    (false, true) => ExternalChangeKind::Created,
                    (true, false) => ExternalChangeKind::Deleted,
                    (true, true) if old != new || (index == 0 && wrote) => {
                        ExternalChangeKind::Modified
                    }
                    _ => return None,
                };
                Some(ExternalChange {
                    path: path.clone(),
                    kind,
                    declared: Some(TargetIndex(index)),
                })
            })
            .collect()
    }
}

fn state(path: &Path) -> FileState {
    fs::metadata(path).map_or(
        FileState {
            exists: false,
            len: 0,
            modified: None,
        },
        |metadata| FileState {
            exists: true,
            len: metadata.len(),
            modified: metadata.modified().ok(),
        },
    )
}

fn resolve(path: &Path) -> std::io::Result<PathBuf> {
    if cfg!(windows) {
        std::path::absolute(path)
    } else {
        fs::canonicalize(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn present(len: u64) -> FileState {
        FileState {
            exists: true,
            len,
            modified: None,
        }
    }

    fn absent() -> FileState {
        FileState {
            exists: false,
            len: 0,
            modified: None,
        }
    }

    fn files() -> DatabaseFiles {
        let root = std::env::temp_dir();
        let paths = ["app.db", "app.db-wal", "app.db-shm", "app.db-journal"]
            .iter()
            .filter_map(|name| AbsPath::new(root.join(name)).ok())
            .collect();
        DatabaseFiles {
            paths,
            labels: vec!["/home/user/app.db".to_owned(); 4],
        }
    }

    #[test]
    fn classifies_changes_of_each_file() {
        let before = [present(4096), absent(), present(10), present(5)];
        let after = [present(4096), present(8), present(10), absent()];
        let kinds: Vec<_> = files()
            .changes(&before, &after, true)
            .into_iter()
            .map(|change| (change.declared, change.kind))
            .collect();
        assert_eq!(
            kinds,
            [
                (Some(TargetIndex(0)), ExternalChangeKind::Modified),
                (Some(TargetIndex(1)), ExternalChangeKind::Created),
                (Some(TargetIndex(3)), ExternalChangeKind::Deleted),
            ]
        );
    }

    #[test]
    fn reports_nothing_without_changes() {
        let before = [present(4096), absent(), absent(), absent()];
        assert!(files().changes(&before, &before, false).is_empty());
    }

    #[test]
    fn declares_size_of_existing_files_only() {
        let before = [present(4096), absent(), absent(), absent()];
        let specs = files().specs(&before);
        assert_eq!(specs.len(), 4);
        assert_eq!(specs[0].size_hint, Some(4096));
        assert_eq!(specs[1].size_hint, None);
        assert!(
            specs
                .iter()
                .all(|spec| spec.action == TargetAction::ExternalWrite)
        );
    }
}
