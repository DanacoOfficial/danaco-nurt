// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Holders of files on Linux, from the process directories of `/proc`.
//!
//! For each process the links `fd/*`, `cwd`, `root` and `exe` are read with
//! `readlink`, and the path column of `maps` lists the files mapped into
//! memory.

use std::io;
use std::path::Path;

use nurt_contracts::error::Result;

use super::{Access, Holder, Scan, Target};
use crate::common::{SysRoot, source_error};

/// Links of a process directory that name one path each.
const LINKS: [(Access, &str); 3] = [
    (Access::Cwd, "cwd"),
    (Access::Root, "root"),
    (Access::Exe, "exe"),
];
/// Columns of a `maps` line before the path: address, permissions, offset,
/// device and inode.
const MAPS_FIXED_FIELDS: usize = 5;

/// Searches the process directories under `root` for holders of `target`.
///
/// # Errors
///
/// Fails with `E_PRIVILEGE` or `E_IO` when `/proc` cannot be listed.
pub(crate) fn scan(root: &SysRoot, target: &Target) -> Result<Scan> {
    let entries =
        std::fs::read_dir(root.path("/proc")).map_err(|error| source_error(&error, "/proc"))?;
    let mut scan = Scan::default();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let (holder, denied) = inspect(&entry.path(), pid, target);
        scan.denied += u64::from(denied);
        if holder.is_holding() {
            scan.holders.push(holder);
        }
    }
    scan.holders.sort_by_key(|holder| holder.pid);
    Ok(scan)
}

/// Matching paths of the process directory `dir` and whether any part of it
/// was not readable.
fn inspect(dir: &Path, pid: u32, target: &Target) -> (Holder, bool) {
    let mut holder = Holder::new(pid);
    let mut denied = false;
    for (access, name) in LINKS {
        match std::fs::read_link(dir.join(name)) {
            Ok(link) => holder.check(target, access, &link.to_string_lossy()),
            Err(error) => denied |= is_denied(&error),
        }
    }
    match std::fs::read_dir(dir.join("fd")) {
        Ok(descriptors) => {
            for descriptor in descriptors.flatten() {
                if let Ok(link) = std::fs::read_link(descriptor.path()) {
                    holder.check(target, Access::Fd, &link.to_string_lossy());
                }
            }
        }
        Err(error) => denied |= is_denied(&error),
    }
    match std::fs::read_to_string(dir.join("maps")) {
        Ok(maps) => {
            for path in maps.lines().filter_map(mapped_path) {
                holder.check(target, Access::Mmap, path);
            }
        }
        Err(error) => denied |= is_denied(&error),
    }
    if holder.is_holding() {
        holder.name = std::fs::read_to_string(dir.join("comm"))
            .ok()
            .map(|name| name.trim_end().to_owned());
    }
    (holder, denied)
}

fn is_denied(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::PermissionDenied
}

/// Path column of one line of `maps`; `None` for anonymous mappings and
/// pseudo paths such as `[heap]`.
pub(crate) fn mapped_path(line: &str) -> Option<&str> {
    let mut rest = line;
    for _ in 0..MAPS_FIXED_FIELDS {
        rest = rest.trim_start();
        let end = rest.find(char::is_whitespace)?;
        rest = rest.get(end..)?;
    }
    let path = rest.trim();
    path.starts_with('/').then_some(path)
}
