// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Process table of Windows.
//!
//! The table comes from a Toolhelp snapshot. Details of each process are read
//! through a handle opened with `PROCESS_QUERY_LIMITED_INFORMATION`, which
//! the system usually grants for processes of other accounts; the arguments and
//! the working directory are read from the process environment block through
//! `sysinfo` and stay empty when the process denies access.

use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::size_of;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::{null, null_mut};
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::ProcessState;
use nurt_contracts::value::Instant;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use windows_sys::Win32::Foundation::{FILETIME, INVALID_HANDLE_VALUE, LocalFree};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{
    GetTokenInformation, LookupAccountSidW, PSID, SID_NAME_USE, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

use super::model::{ProcessInfo, ProcessReader, ProcessSource, ReadRequest};
use super::winpath::{canonical, from_wide};

/// Identifiers of the idle and system processes, which have no handle and
/// no user-mode details.
const KERNEL_PIDS: [u32; 2] = [0, 4];

/// Length of the buffers for image paths, in UTF-16 units.
const PATH_UNITS: usize = 32_768;

/// Length of the buffers for account and domain names, in UTF-16 units.
const NAME_UNITS: usize = 256;

/// Difference between the Windows epoch (1601) and the Unix epoch, in units
/// of 100 nanoseconds.
const EPOCH_DIFFERENCE: u64 = 116_444_736_000_000_000;

const FILETIME_UNIT_NANOSECONDS: u64 = 100;

/// Process source of the running system.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Win32Source;

impl ProcessSource for Win32Source {
    fn open(&self, request: ReadRequest) -> Result<Box<dyn ProcessReader>> {
        Ok(Box::new(Win32Reader {
            request,
            entries: None,
            details: None,
            accounts: HashMap::new(),
        }))
    }
}

/// One process of a Toolhelp snapshot.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub(crate) pid: u32,
    pub(crate) parent: u32,
    pub(crate) threads: u32,
    pub(crate) name: String,
}

/// Takes a Toolhelp snapshot of the processes of the system.
///
/// # Errors
///
/// Fails with `E_IO` when the snapshot cannot be taken.
pub(crate) fn snapshot() -> Result<Vec<Entry>> {
    // SAFETY: the call takes no pointers; failure is reported by the return value.
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if raw == INVALID_HANDLE_VALUE {
        return Err(Error::new(
            ErrorCode::Io,
            format!("cannot list processes: {}", std::io::Error::last_os_error()),
        ));
    }
    // SAFETY: `raw` is a valid handle owned by this function and closed once by `OwnedHandle`.
    let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut entry = PROCESSENTRY32W {
        dwSize: u32::try_from(size_of::<PROCESSENTRY32W>()).unwrap_or(u32::MAX),
        ..PROCESSENTRY32W::default()
    };
    let mut entries = Vec::new();
    // SAFETY: `entry` is a writable structure whose `dwSize` holds its size.
    let mut more = unsafe { Process32FirstW(handle.as_raw_handle(), &mut entry) } != 0;
    while more {
        entries.push(Entry {
            pid: entry.th32ProcessID,
            parent: entry.th32ParentProcessID,
            threads: entry.cntThreads,
            name: from_wide(&entry.szExeFile),
        });
        // SAFETY: as for `Process32FirstW`.
        more = unsafe { Process32NextW(handle.as_raw_handle(), &mut entry) } != 0;
    }
    Ok(entries)
}

struct Win32Reader {
    request: ReadRequest,
    entries: Option<HashMap<u32, Entry>>,
    details: Option<Option<System>>,
    accounts: HashMap<String, Option<String>>,
}

impl Win32Reader {
    fn entries(&mut self) -> Result<&HashMap<u32, Entry>> {
        if self.entries.is_none() {
            let entries = snapshot()?.into_iter().map(|e| (e.pid, e)).collect();
            self.entries = Some(entries);
        }
        Ok(self.entries.get_or_insert_with(HashMap::new))
    }

    /// Arguments and working directories of all processes, read once;
    /// `None` when the system information cannot be opened.
    fn details(&mut self) -> Option<&System> {
        let cwd = if self.request.cwd {
            UpdateKind::Always
        } else {
            UpdateKind::Never
        };
        self.details
            .get_or_insert_with(|| {
                let mut system = System::new().ok()?;
                system.refresh_processes_specifics(
                    ProcessesToUpdate::All,
                    true,
                    ProcessRefreshKind::nothing()
                        .with_cmd(UpdateKind::Always)
                        .with_cwd(cwd),
                );
                Some(system)
            })
            .as_ref()
    }

    fn account_name(&mut self, sid: PSID, sid_text: &str) -> Option<String> {
        self.accounts
            .entry(sid_text.to_owned())
            .or_insert_with(|| lookup_account(sid))
            .clone()
    }
}

impl ProcessReader for Win32Reader {
    fn pids(&mut self) -> Result<Vec<u64>> {
        let mut pids: Vec<u64> = self.entries()?.keys().map(|&pid| u64::from(pid)).collect();
        pids.sort_unstable();
        Ok(pids)
    }

    fn read(&mut self, pid: u64) -> Option<ProcessInfo> {
        let id = u32::try_from(pid).ok()?;
        let entry = self.entries().ok()?.get(&id)?.clone();
        let mut info = ProcessInfo {
            pid,
            parent_pid: u64::from(entry.parent),
            name: entry.name,
            threads: Some(u64::from(entry.threads)),
            state: ProcessState::Running,
            ..ProcessInfo::default()
        };
        if KERNEL_PIDS.contains(&id) {
            return Some(info);
        }
        match open_process(id) {
            Some(handle) => self.read_handle(&handle, &mut info),
            None => info.restricted = true,
        }
        self.read_environment_block(id, &mut info);
        Some(info)
    }

    fn cpu_time(&mut self, pid: u64) -> Option<Duration> {
        let handle = open_process(u32::try_from(pid).ok()?)?;
        Some(process_times(&handle)?.1)
    }
}

impl Win32Reader {
    fn read_handle(&mut self, handle: &OwnedHandle, info: &mut ProcessInfo) {
        info.executable = image_path(handle);
        if let Some((started, cpu_time)) = process_times(handle) {
            info.started = Some(started);
            info.cpu_time = Some(cpu_time);
        }
        info.memory = working_set(handle).unwrap_or(0);
        match token_user(handle) {
            Some(token) => {
                let sid = token.sid();
                if let Some(text) = sid_string(sid) {
                    info.user = self.account_name(sid, &text);
                    info.uid = Some(text);
                }
            }
            None => info.restricted = true,
        }
    }

    fn read_environment_block(&mut self, id: u32, info: &mut ProcessInfo) {
        let want_cwd = self.request.cwd;
        let Some(process) = self
            .details()
            .and_then(|system| system.process(Pid::from_u32(id)))
        else {
            return;
        };
        info.command = process
            .cmd()
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        if want_cwd {
            info.cwd = process.cwd().map(|path| canonical(&path.to_string_lossy()));
        }
        if info.command.is_empty() || (want_cwd && info.cwd.is_none()) {
            info.restricted = true;
        }
    }
}

fn open_process(pid: u32) -> Option<OwnedHandle> {
    // SAFETY: the call takes no pointers; failure is reported by a null handle.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if raw.is_null() {
        return None;
    }
    // SAFETY: `raw` is a valid process handle owned by the returned value.
    Some(unsafe { OwnedHandle::from_raw_handle(raw) })
}

fn image_path(handle: &OwnedHandle) -> Option<String> {
    let mut buffer = vec![0_u16; PATH_UNITS];
    let mut length = u32::try_from(buffer.len()).ok()?;
    // SAFETY: `buffer` holds `length` writable units; the call stores the used length.
    let ok = unsafe {
        QueryFullProcessImageNameW(
            handle.as_raw_handle(),
            PROCESS_NAME_WIN32,
            buffer.as_mut_ptr(),
            &mut length,
        )
    };
    if ok == 0 {
        return None;
    }
    let used = usize::try_from(length).ok()?;
    Some(canonical(&from_wide(buffer.get(..used)?)))
}

fn filetime(time: &FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

/// Start time and processor time of a process.
fn process_times(handle: &OwnedHandle) -> Option<(Instant, Duration)> {
    let zero = || FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero(), zero(), zero(), zero());
    // SAFETY: the four pointers refer to writable `FILETIME` values of this frame.
    let ok = unsafe {
        GetProcessTimes(
            handle.as_raw_handle(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    if ok == 0 {
        return None;
    }
    let since_epoch = filetime(&created).checked_sub(EPOCH_DIFFERENCE)?;
    let unix_time = i64::try_from(since_epoch.checked_mul(FILETIME_UNIT_NANOSECONDS)?).ok()?;
    let busy = filetime(&kernel).saturating_add(filetime(&user));
    Some((
        Instant::from_unix_nanos(unix_time),
        Duration::from_nanos(busy.saturating_mul(FILETIME_UNIT_NANOSECONDS)),
    ))
}

fn working_set(handle: &OwnedHandle) -> Option<u64> {
    let size = u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?;
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: size,
        PageFaultCount: 0,
        PeakWorkingSetSize: 0,
        WorkingSetSize: 0,
        QuotaPeakPagedPoolUsage: 0,
        QuotaPagedPoolUsage: 0,
        QuotaPeakNonPagedPoolUsage: 0,
        QuotaNonPagedPoolUsage: 0,
        PagefileUsage: 0,
        PeakPagefileUsage: 0,
    };
    // SAFETY: `counters` is a writable structure of `size` bytes.
    let ok = unsafe { K32GetProcessMemoryInfo(handle.as_raw_handle(), &mut counters, size) };
    (ok != 0).then(|| u64::try_from(counters.WorkingSetSize).unwrap_or(u64::MAX))
}

/// `TOKEN_USER` structure of a process token, in a buffer aligned for it.
struct TokenUserBuffer {
    buffer: Vec<u64>,
}

impl TokenUserBuffer {
    fn sid(&self) -> PSID {
        // SAFETY: the buffer was filled by `GetTokenInformation(TokenUser)`, is aligned to
        // 8 bytes and starts with a `TOKEN_USER` whose SID points into the same buffer,
        // which lives as long as `self`.
        unsafe { (*self.buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
}

fn token_user(handle: &OwnedHandle) -> Option<TokenUserBuffer> {
    let mut raw = null_mut();
    // SAFETY: `raw` is a writable handle slot of this frame.
    if unsafe { OpenProcessToken(handle.as_raw_handle(), TOKEN_QUERY, &mut raw) } == 0 {
        return None;
    }
    // SAFETY: `raw` is a valid token handle owned by `token`.
    let token = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut needed = 0_u32;
    // SAFETY: a null buffer of length zero asks for the required length only.
    unsafe { GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut needed) };
    let words = usize::try_from(needed).ok()?.div_ceil(size_of::<u64>());
    let mut buffer = vec![0_u64; words.max(1)];
    let length = u32::try_from(buffer.len() * size_of::<u64>()).ok()?;
    // SAFETY: `buffer` holds `length` writable bytes aligned to 8 bytes.
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast::<c_void>(),
            length,
            &mut needed,
        )
    };
    (ok != 0).then_some(TokenUserBuffer { buffer })
}

fn sid_string(sid: PSID) -> Option<String> {
    let mut text = null_mut();
    // SAFETY: `sid` is a valid SID; the system allocates the string and stores it in `text`.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 || text.is_null() {
        return None;
    }
    let mut length = 0;
    // SAFETY: `text` is a zero-terminated wide string allocated by the system.
    while unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    // SAFETY: `text` holds `length` initialized units before the terminating zero.
    let value = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, length) });
    // SAFETY: `text` was allocated with `LocalAlloc` by `ConvertSidToStringSidW`.
    unsafe { LocalFree(text.cast::<c_void>()) };
    Some(value)
}

fn lookup_account(sid: PSID) -> Option<String> {
    let mut name = vec![0_u16; NAME_UNITS];
    let mut domain = vec![0_u16; NAME_UNITS];
    let mut name_length = u32::try_from(name.len()).ok()?;
    let mut domain_length = u32::try_from(domain.len()).ok()?;
    let mut kind: SID_NAME_USE = 0;
    // SAFETY: both buffers hold the stated number of writable units; `sid` is a valid SID.
    let ok = unsafe {
        LookupAccountSidW(
            null(),
            sid,
            name.as_mut_ptr(),
            &mut name_length,
            domain.as_mut_ptr(),
            &mut domain_length,
            &mut kind,
        )
    };
    (ok != 0).then(|| from_wide(&name))
}
