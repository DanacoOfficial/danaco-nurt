// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::host::windows
// Responsibility:   Host facts and service checks on Windows.
// Dependencies:     sysinfo, windows_sys
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Host facts and service checks on Windows.

use std::time::Duration;

use sysinfo::System;
use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
use windows_sys::Win32::System::Services::{
    CloseServiceHandle, OpenSCManagerW, OpenServiceW, SC_MANAGER_CONNECT, SERVICE_QUERY_STATUS,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::HostFacts;
use crate::accounts::windows as accounts;

/// Reads the facts of the running system for the account `user`.
pub(crate) fn read_facts(user: &str) -> HostFacts {
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
        virtualization: None,
        uid: accounts::current_sid(),
        groups: Some(accounts::user_local_groups(user)),
    }
}

fn memory() -> (Option<u64>, Option<u64>) {
    let Ok(mut system) = System::new() else {
        return (None, None);
    };
    system.refresh_memory();
    (Some(system.total_memory()), Some(system.available_memory()))
}

/// Returns `true` when the process `pid` exists and has not exited.
pub(crate) fn process_alive(pid: u32) -> bool {
    // SAFETY: plain call with a process identifier; a null handle signals
    // failure and is not used.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return false;
    }
    let mut code = 0u32;
    // SAFETY: `handle` is a valid process handle and `code` is writable.
    let read = unsafe { GetExitCodeProcess(handle, &mut code) };
    // SAFETY: `handle` was opened above and is closed once.
    unsafe { CloseHandle(handle) };
    read != 0 && i64::from(code) == i64::from(STILL_ACTIVE)
}

/// Returns `true` when the service control manager knows the service `name`.
pub(crate) fn service_exists(name: &str) -> bool {
    let wide = accounts::wide(name);
    // SAFETY: null machine and database names select the local manager.
    let manager = unsafe { OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT) };
    if manager.is_null() {
        return false;
    }
    // SAFETY: `manager` is open and `wide` is a null-terminated UTF-16 name.
    let service = unsafe { OpenServiceW(manager, wide.as_ptr(), SERVICE_QUERY_STATUS) };
    let exists = !service.is_null();
    if exists {
        // SAFETY: `service` was opened above and is closed once.
        unsafe { CloseServiceHandle(service) };
    }
    // SAFETY: `manager` was opened above and is closed once.
    unsafe { CloseServiceHandle(manager) };
    exists
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
