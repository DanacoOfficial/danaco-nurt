// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Process table of macOS, read through `libproc` and `sysctl`.
//!
//! `proc_pidinfo(PROC_PIDTASKALLINFO)` succeeds only for processes of the
//! same account unless the caller is privileged; for other processes the
//! reader falls back to `PROC_PIDTBSDINFO`, which omits memory, threads and
//! processor time. Processor times of `proc_taskinfo` are in units of the
//! Mach absolute time and are converted with the timebase of the host.

use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void};
use std::mem::{MaybeUninit, size_of};
use std::ptr::null_mut;
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::ProcessState;
use nurt_contracts::value::Instant;

use super::model::{ProcessInfo, ProcessReader, ProcessSource, ReadRequest};
use super::procargs;

/// `PROC_ALL_PIDS` of `libproc.h`.
const PROC_ALL_PIDS: u32 = 1;

/// `PROC_PIDPATHINFO_MAXSIZE` of `libproc.h`.
const PATH_MAX_SIZE: usize = 4 * 1024;

/// Spare entries requested beyond the reported number of processes, for
/// processes started between the two calls of `proc_listpids`.
const SPARE_PIDS: usize = 64;

/// Process states of `sys/proc.h`.
mod status {
    pub(super) const IDLE: u32 = 1;
    pub(super) const RUN: u32 = 2;
    pub(super) const SLEEP: u32 = 3;
    pub(super) const STOP: u32 = 4;
    pub(super) const ZOMBIE: u32 = 5;
}

const NANOS_PER_MICRO: u64 = 1_000;
const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// Process source of the running system.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LibprocSource;

impl ProcessSource for LibprocSource {
    fn open(&self, request: ReadRequest) -> Result<Box<dyn ProcessReader>> {
        Ok(Box::new(LibprocReader {
            request,
            timebase: timebase(),
            accounts: HashMap::new(),
        }))
    }
}

/// Lists the identifiers of all processes in ascending order.
///
/// # Errors
///
/// Fails with `E_IO` when the system refuses the list.
pub(crate) fn list_pids() -> Result<Vec<u64>> {
    // SAFETY: a null buffer of length zero asks for the required size only.
    let needed = unsafe { libc::proc_listpids(PROC_ALL_PIDS, 0, null_mut(), 0) };
    let count = usize::try_from(needed).map_err(|_| list_error())? / size_of::<c_int>();
    let mut buffer: Vec<c_int> = vec![0; count + SPARE_PIDS];
    let size = c_int::try_from(buffer.len() * size_of::<c_int>()).map_err(|_| list_error())?;
    // SAFETY: `buffer` holds `size` writable bytes.
    let filled = unsafe { libc::proc_listpids(PROC_ALL_PIDS, 0, buffer.as_mut_ptr().cast(), size) };
    let filled = usize::try_from(filled).map_err(|_| list_error())? / size_of::<c_int>();
    let mut pids: Vec<u64> = buffer
        .into_iter()
        .take(filled)
        .filter_map(|pid| u64::try_from(pid).ok())
        .filter(|&pid| pid != 0)
        .collect();
    pids.sort_unstable();
    pids.dedup();
    Ok(pids)
}

fn list_error() -> Error {
    Error::new(
        ErrorCode::Io,
        format!("cannot list processes: {}", std::io::Error::last_os_error()),
    )
}

/// Short name of a process (`proc_name`).
pub(crate) fn process_name(pid: u64) -> Option<String> {
    let pid = c_int::try_from(pid).ok()?;
    let mut buffer = [0_u8; 2 * libc::MAXCOMLEN + 1];
    let size = u32::try_from(buffer.len()).ok()?;
    // SAFETY: `buffer` holds `size` writable bytes.
    let length = unsafe { libc::proc_name(pid, buffer.as_mut_ptr().cast(), size) };
    let length = usize::try_from(length).ok().filter(|&length| length > 0)?;
    Some(String::from_utf8_lossy(buffer.get(..length)?).into_owned())
}

/// Reads one structure of `proc_pidinfo`.
fn pid_info<T>(pid: c_int, flavor: c_int) -> Option<T> {
    let mut value = MaybeUninit::<T>::zeroed();
    let size = c_int::try_from(size_of::<T>()).ok()?;
    // SAFETY: `value` provides `size` writable bytes for the structure of `flavor`.
    let written = unsafe { libc::proc_pidinfo(pid, flavor, 0, value.as_mut_ptr().cast(), size) };
    // SAFETY: the call filled the whole structure; it is plain data valid for any bytes.
    (written == size).then(|| unsafe { value.assume_init() })
}

/// Converts a C character array to text.
pub(crate) fn c_text(chars: &[c_char]) -> String {
    let bytes: Vec<u8> = chars
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c.to_ne_bytes()[0])
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[expect(
    deprecated,
    reason = "the timebase is available only through the Mach interface"
)]
fn timebase() -> (u64, u64) {
    let mut info = libc::mach_timebase_info { numer: 0, denom: 0 };
    // SAFETY: `info` is a writable structure of this frame.
    let status = unsafe { libc::mach_timebase_info(&mut info) };
    if status != 0 || info.denom == 0 {
        return (1, 1);
    }
    (u64::from(info.numer), u64::from(info.denom))
}

fn map_status(status: u32) -> (ProcessState, Option<String>) {
    match status {
        status::RUN => (ProcessState::Running, None),
        status::SLEEP => (ProcessState::Sleeping, None),
        status::STOP => (ProcessState::Stopped, None),
        status::ZOMBIE => (ProcessState::Zombie, None),
        status::IDLE => (ProcessState::Other, Some("idl".to_owned())),
        other => (ProcessState::Other, Some(other.to_string())),
    }
}

fn started(info: &libc::proc_bsdinfo) -> Option<Instant> {
    let nanos = info
        .pbi_start_tvsec
        .checked_mul(NANOS_PER_SECOND)?
        .checked_add(info.pbi_start_tvusec.checked_mul(NANOS_PER_MICRO)?)?;
    i64::try_from(nanos).ok().map(Instant::from_unix_nanos)
}

struct LibprocReader {
    request: ReadRequest,
    timebase: (u64, u64),
    accounts: HashMap<u32, Option<String>>,
}

impl LibprocReader {
    fn mach_to_duration(&self, ticks: u64) -> Duration {
        let (numer, denom) = self.timebase;
        let nanos = u128::from(ticks) * u128::from(numer) / u128::from(denom);
        Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
    }

    fn account_name(&mut self, uid: u32) -> Option<String> {
        self.accounts
            .entry(uid)
            .or_insert_with(|| {
                nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid))
                    .ok()
                    .flatten()
                    .map(|user| user.name)
            })
            .clone()
    }

    fn base(&mut self, pid: u64, bsd: &libc::proc_bsdinfo) -> ProcessInfo {
        let (state, state_raw) = map_status(bsd.pbi_status);
        let short = c_text(&bsd.pbi_name);
        ProcessInfo {
            pid,
            parent_pid: u64::from(bsd.pbi_ppid),
            name: if short.is_empty() {
                c_text(&bsd.pbi_comm)
            } else {
                short
            },
            user: self.account_name(bsd.pbi_uid),
            uid: Some(bsd.pbi_uid.to_string()),
            started: started(bsd),
            state,
            state_raw,
            ..ProcessInfo::default()
        }
    }
}

impl ProcessReader for LibprocReader {
    fn pids(&mut self) -> Result<Vec<u64>> {
        list_pids()
    }

    fn read(&mut self, pid: u64) -> Option<ProcessInfo> {
        let id = c_int::try_from(pid).ok()?;
        let mut info = match pid_info::<libc::proc_taskallinfo>(id, libc::PROC_PIDTASKALLINFO) {
            Some(all) => {
                let mut info = self.base(pid, &all.pbsd);
                info.memory = all.ptinfo.pti_resident_size;
                info.threads = u64::try_from(all.ptinfo.pti_threadnum).ok();
                info.cpu_time = Some(
                    self.mach_to_duration(
                        all.ptinfo
                            .pti_total_user
                            .saturating_add(all.ptinfo.pti_total_system),
                    ),
                );
                info
            }
            None => {
                let bsd = pid_info::<libc::proc_bsdinfo>(id, libc::PROC_PIDTBSDINFO)?;
                let mut info = self.base(pid, &bsd);
                info.restricted = true;
                info
            }
        };
        info.executable = executable(id);
        match arguments(id) {
            Some(command) => info.command = command,
            None => info.restricted = true,
        }
        if self.request.cwd {
            info.cwd = working_directory(id);
            info.restricted |= info.cwd.is_none();
        }
        Some(info)
    }

    fn cpu_time(&mut self, pid: u64) -> Option<Duration> {
        let id = c_int::try_from(pid).ok()?;
        let all = pid_info::<libc::proc_taskallinfo>(id, libc::PROC_PIDTASKALLINFO)?;
        Some(
            self.mach_to_duration(
                all.ptinfo
                    .pti_total_user
                    .saturating_add(all.ptinfo.pti_total_system),
            ),
        )
    }
}

fn executable(pid: c_int) -> Option<String> {
    let mut buffer = vec![0_u8; PATH_MAX_SIZE];
    let size = u32::try_from(buffer.len()).ok()?;
    // SAFETY: `buffer` holds `size` writable bytes.
    let length = unsafe { libc::proc_pidpath(pid, buffer.as_mut_ptr().cast(), size) };
    let length = usize::try_from(length).ok().filter(|&length| length > 0)?;
    Some(String::from_utf8_lossy(buffer.get(..length)?).into_owned())
}

fn sysctl_int(name: [c_int; 2]) -> Option<usize> {
    let mut mib = name;
    let mut value: c_int = 0;
    let mut size = size_of::<c_int>();
    // SAFETY: `mib` names an integer value; `value` provides `size` writable bytes.
    let status = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&raw mut value).cast::<c_void>(),
            &mut size,
            null_mut(),
            0,
        )
    };
    (status == 0).then(|| usize::try_from(value).ok()).flatten()
}

fn arguments(pid: c_int) -> Option<Vec<String>> {
    let capacity = sysctl_int([libc::CTL_KERN, libc::KERN_ARGMAX])?;
    let mut buffer = vec![0_u8; capacity];
    let mut size = buffer.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    // SAFETY: `buffer` provides `size` writable bytes; the call stores the used size.
    let status = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buffer.as_mut_ptr().cast::<c_void>(),
            &mut size,
            null_mut(),
            0,
        )
    };
    if status != 0 {
        return None;
    }
    procargs::decode(buffer.get(..size)?)
}

fn working_directory(pid: c_int) -> Option<String> {
    let info = pid_info::<libc::proc_vnodepathinfo>(pid, libc::PROC_PIDVNODEPATHINFO)?;
    let chars: Vec<c_char> = info.pvi_cdir.vip_path.iter().flatten().copied().collect();
    let path = c_text(&chars);
    (!path.is_empty()).then_some(path)
}
