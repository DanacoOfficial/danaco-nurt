// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Holders of files on Windows, from the Restart Manager.
//!
//! One session registers the target file, or the files below a target
//! directory, and `RmGetList` names the processes and services that hold any
//! of them. The session always ends with `RmEndSession`.

use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use nurt_contracts::error::{Error, ErrorCode, Result};
use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_MORE_DATA, ERROR_SUCCESS};
use windows_sys::Win32::System::RestartManager::{
    CCH_RM_SESSION_KEY, RM_APP_TYPE, RM_PROCESS_INFO, RmConsole, RmCritical, RmEndSession,
    RmExplorer, RmGetList, RmMainWindow, RmOtherWindow, RmRegisterResources, RmService,
    RmStartSession,
};

pub(crate) use super::process::processes;
use super::{Holder, Scan, Target};
use crate::accounts::windows::wide;
use crate::host::{REQUIRED_PRIVILEGE, SysRoot};

/// Largest number of files below a directory registered with one session.
const MAX_RESOURCES: usize = 10_000;
/// Calls of `RmGetList` while the list of holders keeps growing.
const LIST_ATTEMPTS: usize = 5;

/// Asks the Restart Manager for the holders of `target`.
///
/// # Errors
///
/// Fails with `E_PRIVILEGE` when the Restart Manager refuses access and with
/// `E_IO` for other failures.
pub(crate) fn scan(_root: &SysRoot, target: &Target) -> Result<Scan> {
    let (files, truncated) = resources(target);
    let mut scan = Scan {
        truncated: truncated.then(|| u64::try_from(files.len()).unwrap_or(u64::MAX)),
        ..Scan::default()
    };
    if files.is_empty() {
        return Ok(scan);
    }
    let session = Session::start()?;
    session.register(&files)?;
    for info in session.list()? {
        let pid = info.Process.dwProcessId;
        if scan.holders.iter().any(|holder| holder.pid == pid) {
            continue;
        }
        let name = array_text(&info.strAppName);
        scan.holders.push(Holder {
            name: (!name.is_empty()).then_some(name),
            kind: Some(app_kind(info.ApplicationType).to_owned()),
            ..Holder::new(pid)
        });
    }
    scan.holders.sort_by_key(|holder| holder.pid);
    Ok(scan)
}

/// Files to register: the target file, or the regular files below a target
/// directory up to [`MAX_RESOURCES`]; the flag is set when files were left
/// out.
fn resources(target: &Target) -> (Vec<Vec<u16>>, bool) {
    if !target.directory {
        return (vec![wide(&plain_path(&target.path))], false);
    }
    let mut files = Vec::new();
    let mut pending: Vec<PathBuf> = vec![target.path.clone()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                if files.len() == MAX_RESOURCES {
                    return (files, true);
                }
                files.push(wide(&plain_path(&entry.path())));
            }
        }
    }
    (files, false)
}

/// Path without the verbatim prefix `\\?\` that canonical paths carry.
fn plain_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
}

/// Text of a null-terminated UTF-16 array.
fn array_text(units: &[u16]) -> String {
    let end = units
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(units.len());
    String::from_utf16_lossy(units.get(..end).unwrap_or_default())
}

fn app_kind(kind: RM_APP_TYPE) -> &'static str {
    match kind {
        RmMainWindow | RmOtherWindow => "application",
        RmService => "service",
        RmExplorer => "explorer",
        RmConsole => "console",
        RmCritical => "critical",
        _ => "unknown",
    }
}

fn rm_error(status: u32, action: &str) -> Error {
    if status == ERROR_ACCESS_DENIED {
        Error::new(
            ErrorCode::Privilege,
            format!("the restart manager refused to {action}; run as {REQUIRED_PRIVILEGE}"),
        )
        .with_detail("required", serde_json::json!(REQUIRED_PRIVILEGE))
    } else {
        Error::new(
            ErrorCode::Io,
            format!("the restart manager could not {action}: system error {status}"),
        )
    }
}

/// Restart Manager session, ended when dropped.
struct Session(u32);

impl Session {
    fn start() -> Result<Self> {
        let mut handle = 0_u32;
        let mut key = [0_u16; CCH_RM_SESSION_KEY as usize + 1];
        // SAFETY: `handle` is writable and `key` holds the session key and its
        // terminator.
        let status = unsafe { RmStartSession(&mut handle, 0, key.as_mut_ptr()) };
        if status != ERROR_SUCCESS {
            return Err(rm_error(status, "start a session"));
        }
        Ok(Self(handle))
    }

    fn register(&self, files: &[Vec<u16>]) -> Result<()> {
        let names: Vec<*const u16> = files.iter().map(|file| file.as_ptr()).collect();
        let count = u32::try_from(names.len()).unwrap_or(u32::MAX);
        // SAFETY: `names` holds `count` pointers to null-terminated strings
        // that outlive the call; no applications or services are passed.
        let status =
            unsafe { RmRegisterResources(self.0, count, names.as_ptr(), 0, null(), 0, null()) };
        if status != ERROR_SUCCESS {
            return Err(rm_error(status, "register the files"));
        }
        Ok(())
    }

    fn list(&self) -> Result<Vec<RM_PROCESS_INFO>> {
        let mut infos: Vec<RM_PROCESS_INFO> = Vec::new();
        for _ in 0..LIST_ATTEMPTS {
            let mut needed = 0_u32;
            let mut count = u32::try_from(infos.len()).unwrap_or(u32::MAX);
            let mut reasons = 0_u32;
            let buffer = if infos.is_empty() {
                null_mut()
            } else {
                infos.as_mut_ptr()
            };
            // SAFETY: `buffer` is null with a count of zero or holds `count`
            // writable entries; the other pointers are writable.
            let status =
                unsafe { RmGetList(self.0, &mut needed, &mut count, buffer, &mut reasons) };
            match status {
                ERROR_SUCCESS => {
                    infos.truncate(usize::try_from(count).unwrap_or_default());
                    return Ok(infos);
                }
                ERROR_MORE_DATA => {
                    // SAFETY: the structure is plain data for which all zero
                    // bytes are a valid value.
                    let empty: RM_PROCESS_INFO = unsafe { std::mem::zeroed() };
                    infos = vec![empty; usize::try_from(needed).unwrap_or_default()];
                }
                other => return Err(rm_error(other, "list the holders")),
            }
        }
        Err(rm_error(ERROR_MORE_DATA, "list the holders"))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: the session was started by `Session::start` and is ended
        // once.
        unsafe { RmEndSession(self.0) };
    }
}
