// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::linux::holders
// Responsibility:   Processes holding a file open, found through `/proc/<pid>/fd`.
// Dependencies:     nurt_contracts, rustix
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Processes holding a file open, found through `/proc/<pid>/fd`.
//!
//! Descriptors of processes of other accounts are not readable without
//! privileges; such processes are omitted, so the list may be incomplete.
//! Memory mappings without an open descriptor are not reported.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use nurt_contracts::fs::{Follow, FsResult, Holder};
use rustix::fs::CWD;

use super::meta::stat_at;

/// Process directory of procfs.
const PROC: &str = "/proc";

pub(super) fn holders(path: &Path) -> FsResult<Vec<Holder>> {
    let target = stat_at(CWD, path, Follow::Follow, "holders", path)?.file_id;
    let Ok(processes) = fs::read_dir(PROC) else {
        return Ok(Vec::new());
    };
    let mut holders = Vec::new();
    for process in processes.flatten() {
        let Some(pid) = process
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(descriptors) = fs::read_dir(process.path().join("fd")) else {
            continue;
        };
        let holds = descriptors.flatten().any(|descriptor| {
            fs::metadata(descriptor.path())
                .is_ok_and(|meta| meta.dev() == target.volume && meta.ino() == target.low)
        });
        if holds {
            let name = fs::read_to_string(process.path().join("comm"))
                .map(|name| name.trim_end().to_owned())
                .unwrap_or_default();
            holders.push(Holder { pid, name });
        }
    }
    holders.sort_by_key(|holder| holder.pid);
    Ok(holders)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
