// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::host::linux
// Responsibility:   Host facts on Linux.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Host facts on Linux.

use super::{HostFacts, SysRoot, procfs};
use crate::accounts::unix;

/// Reads the facts of the running system from the files under `root`, the
/// kernel release from `uname` and the identity of the process.
pub(crate) fn read_facts(root: &SysRoot) -> HostFacts {
    let release = root
        .read_to_string("/etc/os-release")
        .or_else(|_| root.read_to_string("/usr/lib/os-release"))
        .map(|text| procfs::parse_os_release(&text))
        .unwrap_or_default();
    let (memory_total, memory_free) = root
        .read_to_string("/proc/meminfo")
        .map(|text| procfs::parse_meminfo(&text))
        .unwrap_or_default();
    let uname = rustix::system::uname();
    HostFacts {
        os_version: release.version_id.or(release.pretty_name),
        kernel: Some(uname.release().to_string_lossy().into_owned()),
        memory_total,
        memory_free,
        uptime: root
            .read_to_string("/proc/uptime")
            .ok()
            .and_then(|text| procfs::parse_uptime(&text)),
        boot_time: root
            .read_to_string("/proc/stat")
            .ok()
            .and_then(|text| procfs::parse_btime(&text)),
        virtualization: procfs::detect_virtualization(root),
        uid: Some(unix::current_uid().to_string()),
        groups: Some(unix::group_names(root, &unix::current_gids())),
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
