// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Source index of winget (`index.db`, SQLite).
//!
//! The component interface of the package manager is not used: the index
//! file of the default source is read in read-only mode.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use super::{EntrySink, IndexEntry, IndexReport};
use crate::package::detect::WINGET_PACKAGE_DIR;
use crate::package::root::{HostEnv, SysPaths};

const STATE_DIR: &str = "LocalState";
const INDEX_FILE: &str = "index.db";
const SOURCE: &str = "winget";
const MAX_DEPTH: usize = 5;
const QUERY: &str = "SELECT ids.id, names.name, versions.version FROM manifest \
    JOIN ids ON manifest.id = ids.rowid \
    JOIN names ON manifest.name = names.rowid \
    JOIN versions ON manifest.version = versions.rowid";

pub(super) fn read(env: &HostEnv, sink: EntrySink<'_>, report: &mut IndexReport) {
    let Some(base) = env.var_text("LOCALAPPDATA") else {
        return;
    };
    let directory = env.resolve(&format!("{base}\\{WINGET_PACKAGE_DIR}\\{STATE_DIR}"));
    let Some(database) = find_index(&directory, MAX_DEPTH) else {
        return;
    };
    report.sources_found += 1;
    match read_database(&database, sink) {
        Ok(()) => report.sources_read += 1,
        Err(error) => report.warn("corrupt_index", format!("{INDEX_FILE}: {error}")),
    }
}

fn find_index(directory: &Path, depth: usize) -> Option<PathBuf> {
    let candidate = directory.join(INDEX_FILE);
    if candidate.is_file() {
        return Some(candidate);
    }
    if depth == 0 {
        return None;
    }
    let mut children: Vec<PathBuf> = std::fs::read_dir(directory)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    children.sort();
    children
        .into_iter()
        .find_map(|child| find_index(&child, depth - 1))
}

fn read_database(path: &Path, sink: EntrySink<'_>) -> rusqlite::Result<()> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let mut statement = connection.prepare(QUERY)?;
    let rows = statement.query_map([], |row| {
        Ok(IndexEntry {
            name: row.get(0)?,
            description: row.get(1)?,
            version: row.get(2)?,
            source: SOURCE.to_owned(),
            ..IndexEntry::default()
        })
    })?;
    for row in rows {
        sink(row?);
    }
    Ok(())
}
