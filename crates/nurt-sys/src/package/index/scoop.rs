// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::package::index::scoop
// Responsibility:   Scoop bucket manifests (`buckets/<bucket>/bucket/*.json`).
// Dependencies:     nurt_programs, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Scoop bucket manifests (`buckets/<bucket>/bucket/*.json`).
//!
//! Buckets are read below the account root and the global root of Scoop.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use nurt_programs::sources::scoop::roots;
use serde_json::Value;

use super::{EntrySink, IndexEntry, IndexReport};
use crate::package::root::HostEnv;

const BUCKETS_DIR: &str = "buckets";
const MANIFEST_DIR: &str = "bucket";
const MANIFEST_EXTENSION: &str = "json";

pub(super) fn read(env: &HostEnv, sink: EntrySink<'_>, report: &mut IndexReport) {
    let mut skipped = 0_usize;
    for root in roots(env) {
        for bucket in sorted_entries(&env.sys_path(root.join(BUCKETS_DIR))) {
            if !bucket.is_dir() {
                continue;
            }
            let Some(bucket_name) = bucket.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let manifests = bucket.join(MANIFEST_DIR);
            let directory = if manifests.is_dir() {
                manifests
            } else {
                bucket.clone()
            };
            report.sources_found += 1;
            for path in sorted_entries(&directory) {
                if path.extension().and_then(|ext| ext.to_str()) != Some(MANIFEST_EXTENSION) {
                    continue;
                }
                match manifest(&path, bucket_name) {
                    Some(entry) => sink(entry),
                    None => skipped += 1,
                }
            }
            report.sources_read += 1;
        }
    }
    if skipped > 0 {
        report.warn(
            "corrupt_index",
            format!("{skipped} Scoop manifests could not be read"),
        );
    }
}

fn sorted_entries(directory: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(directory)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    paths.sort();
    paths
}

fn manifest(path: &Path, bucket: &str) -> Option<IndexEntry> {
    let document: Value = serde_json::from_reader(BufReader::new(File::open(path).ok()?)).ok()?;
    Some(IndexEntry {
        name: path.file_stem()?.to_str()?.to_owned(),
        version: document.get("version")?.as_str()?.to_owned(),
        description: document
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned),
        source: bucket.to_owned(),
        ..IndexEntry::default()
    })
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
