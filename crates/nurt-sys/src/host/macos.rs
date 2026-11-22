// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Host facts on macOS.

use std::ffi::CStr;
use std::time::Duration;

use sysinfo::System;

use super::{HostFacts, SysRoot};
use crate::accounts::unix;

/// Name of the system control that reports a guest of a hypervisor.
const VMM_PRESENT: &CStr = c"kern.hv_vmm_present";
/// Virtualisation reported for a guest of any hypervisor.
const VIRTUAL_MACHINE: &str = "vm";

/// Reads the facts of the running system.
pub(crate) fn read_facts(root: &SysRoot) -> HostFacts {
    let (memory_total, memory_free) = memory();
    HostFacts {
        os_version: System::os_version().ok(),
        kernel: System::kernel_version().ok(),
        memory_total,
        memory_free,
        uptime: System::uptime().ok().map(Duration::from_secs),
        boot_time: System::boot_time()
            .ok()
            .and_then(|seconds| i64::try_from(seconds).ok()),
        virtualization: hypervisor_guest().then(|| VIRTUAL_MACHINE.to_owned()),
        uid: Some(unix::current_uid().to_string()),
        groups: Some(unix::group_names(root, &unix::current_gids())),
    }
}

fn memory() -> (Option<u64>, Option<u64>) {
    let Ok(mut system) = System::new() else {
        return (None, None);
    };
    system.refresh_memory();
    (Some(system.total_memory()), Some(system.available_memory()))
}

fn hypervisor_guest() -> bool {
    let mut value: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    // SAFETY: the name is a null-terminated string, `value` and `size`
    // describe a writable buffer of the size of the integer the control
    // returns, and no new value is set.
    let status = unsafe {
        libc::sysctlbyname(
            VMM_PRESENT.as_ptr(),
            std::ptr::from_mut(&mut value).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    status == 0 && value == 1
}
