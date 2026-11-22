// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Holders of files on macOS, from the process information interface of
//! the kernel.
//!
//! Open descriptors come from `PROC_PIDLISTFDS` and, for each vnode
//! descriptor, `proc_pidfdinfo` with `PROC_PIDFDVNODEPATHINFO`; the working
//! and root directories from `PROC_PIDVNODEPATHINFO`; the executable from
//! `proc_pidpath`; mapped files from `PROC_PIDREGIONPATHINFO`, region by
//! region.

use std::collections::BTreeSet;
use std::ffi::{c_char, c_int, c_void};
use std::io;
use std::mem::MaybeUninit;
use std::ptr::null_mut;

use nurt_contracts::error::Result;

pub(crate) use super::process::processes;
use super::{Access, Holder, Scan, Target};
use crate::common::SysRoot;
use crate::processes::macos::{c_text, list_pids, process_name};

/// Flavor of `proc_pidinfo` listing the open descriptors.
const PROC_PIDLISTFDS: c_int = 1;
/// Flavor of `proc_pidfdinfo` with the path of a vnode descriptor.
const PROC_PIDFDVNODEPATHINFO: c_int = 2;
/// Flavor of `proc_pidinfo` with one memory region and its file.
const PROC_PIDREGIONPATHINFO: c_int = 8;
/// Descriptor type of files and directories.
const PROX_FDTYPE_VNODE: u32 = 1;
/// Size of `struct proc_fileinfo`.
const PROC_FILEINFO_SIZE: usize = 24;
/// Size of `struct proc_regioninfo`.
const REGION_INFO_SIZE: usize = 96;
/// Offset of `pri_address` in `struct proc_regioninfo`.
const REGION_ADDRESS_OFFSET: usize = 80;
/// Offset of `pri_size` in `struct proc_regioninfo`.
const REGION_LENGTH_OFFSET: usize = 88;
/// Length of `vip_path` (`MAXPATHLEN`).
const MAX_PATH: usize = 1024;
/// Buffer length of `proc_pidpath` (`PROC_PIDPATHINFO_MAXSIZE`).
const PID_PATH_SIZE: usize = 4 * MAX_PATH;
/// Size of `struct vnode_info_path`.
const VNODE_PATH_INFO_SIZE: usize = size_of::<libc::vnode_info_path>();
/// Offset of `vip_path` in `struct vnode_info_path`.
const VNODE_PATH_OFFSET: usize = VNODE_PATH_INFO_SIZE - MAX_PATH;
/// Size of `struct vnode_fdinfowithpath`.
const FD_PATH_INFO_SIZE: usize = PROC_FILEINFO_SIZE + VNODE_PATH_INFO_SIZE;
/// Size of `struct proc_regionwithpathinfo`.
const REGION_PATH_INFO_SIZE: usize = REGION_INFO_SIZE + VNODE_PATH_INFO_SIZE;
/// Descriptors allowed to appear between the two listing calls.
const SPARE_DESCRIPTORS: usize = 16;
/// Upper bound of regions read per process.
const MAX_REGIONS: usize = 65_536;

/// `struct proc_fdinfo`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct ProcFdInfo {
    proc_fd: i32,
    proc_fdtype: u32,
}

unsafe extern "C" {
    /// Information about one descriptor of a process.
    fn proc_pidfdinfo(
        pid: c_int,
        fd: c_int,
        flavor: c_int,
        buffer: *mut c_void,
        size: c_int,
    ) -> c_int;
}

/// Searches every process for holders of `target`.
///
/// # Errors
///
/// Fails with `E_IO` when the processes cannot be listed.
pub(crate) fn scan(_root: &SysRoot, target: &Target) -> Result<Scan> {
    let mut scan = Scan::default();
    for pid in list_pids()? {
        let (Ok(id), Ok(pid)) = (c_int::try_from(pid), u32::try_from(pid)) else {
            continue;
        };
        let mut holder = Holder::new(pid);
        scan.denied += u64::from(inspect(id, &mut holder, target));
        if holder.is_holding() {
            holder.name = process_name(u64::from(pid));
            scan.holders.push(holder);
        }
    }
    scan.holders.sort_by_key(|holder| holder.pid);
    Ok(scan)
}

/// Records the matching paths of process `pid`; returns `true` when its
/// descriptors could not be read for lack of permission.
fn inspect(pid: c_int, holder: &mut Holder, target: &Target) -> bool {
    let descriptors = match descriptors(pid) {
        Ok(descriptors) => descriptors,
        Err(error) => return error.kind() == io::ErrorKind::PermissionDenied,
    };
    for descriptor in descriptors
        .iter()
        .filter(|descriptor| descriptor.proc_fdtype == PROX_FDTYPE_VNODE)
    {
        if let Some(path) = descriptor_path(pid, descriptor.proc_fd) {
            holder.check(target, Access::Fd, &path);
        }
    }
    if let Some(info) = pid_info::<libc::proc_vnodepathinfo>(pid, libc::PROC_PIDVNODEPATHINFO) {
        holder.check(target, Access::Cwd, &vnode_path(&info.pvi_cdir));
        holder.check(target, Access::Root, &vnode_path(&info.pvi_rdir));
    }
    if let Some(path) = executable(pid) {
        holder.check(target, Access::Exe, &path);
    }
    for path in region_paths(pid) {
        holder.check(target, Access::Mmap, &path);
    }
    false
}

fn descriptors(pid: c_int) -> io::Result<Vec<ProcFdInfo>> {
    // SAFETY: a null buffer of length zero asks for the required size only.
    let needed = unsafe { libc::proc_pidinfo(pid, PROC_PIDLISTFDS, 0, null_mut(), 0) };
    if needed <= 0 {
        return Err(io::Error::last_os_error());
    }
    let count =
        usize::try_from(needed).unwrap_or_default() / size_of::<ProcFdInfo>() + SPARE_DESCRIPTORS;
    let mut buffer = vec![ProcFdInfo::default(); count];
    let size = c_int::try_from(count * size_of::<ProcFdInfo>())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    // SAFETY: `buffer` holds `size` writable bytes.
    let written =
        unsafe { libc::proc_pidinfo(pid, PROC_PIDLISTFDS, 0, buffer.as_mut_ptr().cast(), size) };
    if written <= 0 {
        return Err(io::Error::last_os_error());
    }
    buffer.truncate(usize::try_from(written).unwrap_or_default() / size_of::<ProcFdInfo>());
    Ok(buffer)
}

fn descriptor_path(pid: c_int, fd: i32) -> Option<String> {
    let mut buffer = [0_u8; FD_PATH_INFO_SIZE];
    let size = c_int::try_from(FD_PATH_INFO_SIZE).ok()?;
    // SAFETY: `buffer` holds `size` writable bytes for the structure of the
    // flavor.
    let written = unsafe {
        proc_pidfdinfo(
            pid,
            fd,
            PROC_PIDFDVNODEPATHINFO,
            buffer.as_mut_ptr().cast(),
            size,
        )
    };
    if written != size {
        return None;
    }
    path_at(&buffer, PROC_FILEINFO_SIZE + VNODE_PATH_OFFSET)
}

/// Reads one structure of `proc_pidinfo`.
fn pid_info<T>(pid: c_int, flavor: c_int) -> Option<T> {
    let mut value = MaybeUninit::<T>::zeroed();
    let size = c_int::try_from(size_of::<T>()).ok()?;
    // SAFETY: `value` provides `size` writable bytes for the structure of
    // `flavor`.
    let written = unsafe { libc::proc_pidinfo(pid, flavor, 0, value.as_mut_ptr().cast(), size) };
    // SAFETY: the call filled the whole structure, which is plain data valid
    // for any bytes.
    (written == size).then(|| unsafe { value.assume_init() })
}

fn vnode_path(info: &libc::vnode_info_path) -> String {
    let chars: Vec<c_char> = info.vip_path.iter().flatten().copied().collect();
    c_text(&chars)
}

fn executable(pid: c_int) -> Option<String> {
    let mut buffer = [0_u8; PID_PATH_SIZE];
    let size = u32::try_from(PID_PATH_SIZE).ok()?;
    // SAFETY: `buffer` holds `size` writable bytes.
    let length = unsafe { libc::proc_pidpath(pid, buffer.as_mut_ptr().cast(), size) };
    let length = usize::try_from(length).ok().filter(|&length| length > 0)?;
    path_at(buffer.get(..length)?, 0)
}

/// Paths of the files mapped into the memory of process `pid`.
fn region_paths(pid: c_int) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    let Ok(size) = c_int::try_from(REGION_PATH_INFO_SIZE) else {
        return paths;
    };
    let mut address = 0_u64;
    for _ in 0..MAX_REGIONS {
        let mut buffer = [0_u8; REGION_PATH_INFO_SIZE];
        // SAFETY: `buffer` holds `size` writable bytes for the structure of
        // the flavor.
        let written = unsafe {
            libc::proc_pidinfo(
                pid,
                PROC_PIDREGIONPATHINFO,
                address,
                buffer.as_mut_ptr().cast(),
                size,
            )
        };
        if written != size {
            break;
        }
        let (Some(start), Some(length)) = (
            read_u64(&buffer, REGION_ADDRESS_OFFSET),
            read_u64(&buffer, REGION_LENGTH_OFFSET),
        ) else {
            break;
        };
        if let Some(path) = path_at(&buffer, REGION_INFO_SIZE + VNODE_PATH_OFFSET) {
            paths.insert(path);
        }
        let next = start.saturating_add(length);
        if next <= address {
            break;
        }
        address = next;
    }
    paths
}

fn read_u64(buffer: &[u8], offset: usize) -> Option<u64> {
    let bytes: [u8; 8] = buffer
        .get(offset..offset + size_of::<u64>())?
        .try_into()
        .ok()?;
    Some(u64::from_ne_bytes(bytes))
}

/// Null-terminated path at `offset` of `buffer`; `None` when empty.
fn path_at(buffer: &[u8], offset: usize) -> Option<String> {
    let bytes = buffer.get(offset..)?;
    let end = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len())
        .min(PID_PATH_SIZE);
    let path = String::from_utf8_lossy(bytes.get(..end)?).into_owned();
    (!path.is_empty()).then_some(path)
}
