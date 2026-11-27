// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::windows::sys::restart
// Responsibility:   Processes holding files, from the Restart Manager.
// Dependencies:     nurt_contracts, windows_sys
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Processes holding files, from the Restart Manager.

use std::ptr;

use nurt_contracts::fs::Holder;
use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
use windows_sys::Win32::System::RestartManager::{
    CCH_RM_SESSION_KEY, RM_PROCESS_INFO, RmEndSession, RmGetList, RmRegisterResources,
    RmStartSession,
};

use super::handle::WidePath;
use crate::windows::path::{strip_verbatim, until_nul};

/// Attempts to read a list that keeps growing between calls.
const LIST_ATTEMPTS: usize = 4;

/// Restart Manager session, ended on drop.
struct Session(u32);

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: the session was started by `RmStartSession` and is ended
        // exactly once here; a failure leaves nothing to release.
        unsafe { RmEndSession(self.0) };
    }
}

fn start_session() -> Option<Session> {
    let mut handle = 0_u32;
    let mut key = [0_u16; CCH_RM_SESSION_KEY as usize + 1];
    // SAFETY: `handle` is writable and `key` holds `CCH_RM_SESSION_KEY + 1`
    // characters, the size the function requires.
    let status = unsafe { RmStartSession(&raw mut handle, 0, key.as_mut_ptr()) };
    (status == ERROR_SUCCESS).then_some(Session(handle))
}

/// Processes that hold any of `paths` open; empty when the Restart Manager is
/// unavailable or fails.
///
/// The Restart Manager matches the names under which files were opened, so
/// paths are registered without the verbatim prefix.
pub(crate) fn holders(paths: &[&WidePath]) -> Vec<Holder> {
    let plain: Vec<WidePath> = paths
        .iter()
        .map(|path| WidePath::from_units(&strip_verbatim(path.units())))
        .collect();
    let names: Vec<*const u16> = plain.iter().map(WidePath::as_ptr).collect();
    let Ok(count) = u32::try_from(names.len()) else {
        return Vec::new();
    };
    let Some(session) = start_session() else {
        return Vec::new();
    };
    // SAFETY: `names` holds `count` pointers to zero-terminated paths that
    // outlive the call; no applications or services are registered.
    let status = unsafe {
        RmRegisterResources(
            session.0,
            count,
            names.as_ptr(),
            0,
            ptr::null(),
            0,
            ptr::null(),
        )
    };
    if status != ERROR_SUCCESS {
        return Vec::new();
    }
    let mut infos: Vec<RM_PROCESS_INFO> = Vec::new();
    for _ in 0..LIST_ATTEMPTS {
        let mut needed = 0_u32;
        let mut reasons = 0_u32;
        let Ok(mut returned) = u32::try_from(infos.len()) else {
            return Vec::new();
        };
        let buffer = if infos.is_empty() {
            ptr::null_mut()
        } else {
            infos.as_mut_ptr()
        };
        // SAFETY: the session is open; `buffer` is null with a zero count or
        // points to `returned` writable entries; the counters are writable.
        let status = unsafe {
            RmGetList(
                session.0,
                &raw mut needed,
                &raw mut returned,
                buffer,
                &raw mut reasons,
            )
        };
        match status {
            ERROR_SUCCESS => {
                let returned = usize::try_from(returned).unwrap_or(0).min(infos.len());
                return infos[..returned]
                    .iter()
                    .map(|info| Holder {
                        pid: info.Process.dwProcessId,
                        name: String::from_utf16_lossy(until_nul(&info.strAppName)),
                    })
                    .collect();
            }
            ERROR_MORE_DATA => {
                let needed = usize::try_from(needed).unwrap_or(0);
                infos.resize(needed, RM_PROCESS_INFO::default());
            }
            _ => return Vec::new(),
        }
    }
    Vec::new()
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
