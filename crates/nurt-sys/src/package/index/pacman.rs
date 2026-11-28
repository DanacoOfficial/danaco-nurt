// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::package::index::pacman
// Responsibility:   Sync databases of pacman (`<DBPath>/sync/<repository>.db`).
// Dependencies:     nurt_programs
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Sync databases of pacman (`<DBPath>/sync/<repository>.db`).
//!
//! The database directory is `DBPath` of `pacman.conf`, by default
//! `/var/lib/pacman`.

use std::io::Read;

use nurt_programs::sources::pacman::db_path;

use super::{EntrySink, IndexEntry, IndexReport};
use crate::package::compress::open_decoded;
use crate::package::root::{HostEnv, SysPaths};

const SYNC_DIR: &str = "sync";
const DATABASE_EXTENSION: &str = "db";
const DESCRIPTOR: &str = "desc";

pub(super) fn read(env: &HostEnv, sink: EntrySink<'_>, report: &mut IndexReport) {
    let sync = db_path(env).join(SYNC_DIR);
    for path in env.list_dir(&sync.to_string_lossy()) {
        if path.extension().and_then(|ext| ext.to_str()) != Some(DATABASE_EXTENSION) {
            continue;
        }
        let repository = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_owned();
        report.sources_found += 1;
        let result = open_decoded(&path)
            .map_err(|error| error.to_string())
            .and_then(|reader| read_database(reader, &repository, sink));
        match result {
            Ok(()) => report.sources_read += 1,
            Err(detail) => report.warn("corrupt_index", format!("{repository}: {detail}")),
        }
    }
}

fn read_database(
    reader: Box<dyn Read + Send>,
    repository: &str,
    sink: EntrySink<'_>,
) -> Result<(), String> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries().map_err(|error| error.to_string())?;
    for entry in entries {
        let mut entry = entry.map_err(|error| error.to_string())?;
        let is_descriptor = entry
            .path()
            .map_err(|error| error.to_string())?
            .file_name()
            .is_some_and(|name| name == DESCRIPTOR);
        if !is_descriptor {
            continue;
        }
        let mut text = String::new();
        entry
            .read_to_string(&mut text)
            .map_err(|error| error.to_string())?;
        if let Some(parsed) = parse_descriptor(&text, repository) {
            sink(parsed);
        }
    }
    Ok(())
}

fn parse_descriptor(text: &str, repository: &str) -> Option<IndexEntry> {
    let mut entry = IndexEntry {
        source: repository.to_owned(),
        ..IndexEntry::default()
    };
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some(tag) = line
            .strip_prefix('%')
            .and_then(|rest| rest.strip_suffix('%'))
        else {
            continue;
        };
        let value = lines.next().unwrap_or_default().trim();
        match tag {
            "NAME" => entry.name = value.to_owned(),
            "VERSION" => entry.version = value.to_owned(),
            "DESC" => entry.description = Some(value.to_owned()),
            "ARCH" => entry.architecture = Some(value.to_owned()),
            "ISIZE" => entry.size = value.parse().ok(),
            "CSIZE" if entry.size.is_none() => entry.size = value.parse().ok(),
            _ => {}
        }
    }
    (!entry.name.is_empty() && !entry.version.is_empty()).then_some(entry)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
