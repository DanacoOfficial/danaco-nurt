// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::windows::sys::security
// Responsibility:   Security descriptors and restoration of complete metadata.
// Dependencies:     nurt_contracts, windows_sys
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Security descriptors and restoration of complete metadata.
//!
//! Descriptors are read and written with `GetKernelObjectSecurity` and
//! `SetKernelObjectSecurity`. These calls transfer the descriptor exactly as
//! stored, including inherited entries and the protection bits, and do not
//! propagate inheritable entries to the children of a directory, which the
//! higher-level `SetSecurityInfo` does.

use std::fs::File;
use std::os::windows::io::AsRawHandle;

use nurt_contracts::fs::{AbsPath, ExtendedMeta, FsResult, MetaTransferReport};
use windows_sys::Win32::Security::{
    GetKernelObjectSecurity, GetSecurityDescriptorLength, SetKernelObjectSecurity,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_BASIC_INFO, FILE_READ_ATTRIBUTES, FILE_WRITE_ATTRIBUTES, OPEN_EXISTING, READ_CONTROL,
};

use super::handle::{
    ENTRY_FLAGS, WidePath, aligned_words, basic_info, create_file, last_error, list_streams,
    set_basic_info, words_to_bytes,
};
use crate::windows::error::codes::{
    ERROR_ACCESS_DENIED, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_PARAMETER,
    ERROR_PRIVILEGE_NOT_HELD,
};
use crate::windows::error::{is_unsupported_request, os_error};
use crate::windows::meta::{
    attributes_for_mode, basic_info_attributes, unix_ns_to_filetime, untransferable_attributes,
};
use crate::windows::security::{
    SecurityDescriptor, SecurityElement, access, information, mark_sacl_capture, restore_plan,
};

/// Size of a descriptor word.
const WORD: usize = 4;

/// Opens `path` for reading its descriptor, with the SACL when the process
/// holds an enabled security privilege.
fn open_for_reading(path: &WidePath, flags: u32) -> Result<(File, bool), u32> {
    match create_file(
        path,
        READ_CONTROL | access::ACCESS_SYSTEM_SECURITY,
        OPEN_EXISTING,
        flags,
    ) {
        Ok(file) => Ok((file, true)),
        Err(ERROR_PRIVILEGE_NOT_HELD | ERROR_ACCESS_DENIED) => {
            create_file(path, READ_CONTROL, OPEN_EXISTING, flags).map(|file| (file, false))
        }
        Err(code) => Err(code),
    }
}

/// Self-relative descriptor with account, group, DACL and, when readable,
/// SACL; the capture of the SACL is recorded as described in
/// [`crate::windows::security`]. `None` on volumes without security.
pub(crate) fn read_descriptor(path: &WidePath, flags: u32) -> Result<Option<Vec<u8>>, u32> {
    let (file, sacl) = open_for_reading(path, flags)?;
    let requested = information::OWNER
        | information::GROUP
        | information::DACL
        | if sacl { information::SACL } else { 0 };
    let mut buffer: Vec<u32> = Vec::new();
    loop {
        let capacity = u32::try_from(buffer.len() * WORD).map_err(|_| ERROR_INSUFFICIENT_BUFFER)?;
        let mut needed = 0_u32;
        // SAFETY: the handle is open with the rights for the requested parts;
        // `buffer` is writable for `capacity` bytes and aligned to four bytes as
        // descriptors require; `needed` is writable.
        let ok = unsafe {
            GetKernelObjectSecurity(
                file.as_raw_handle(),
                requested,
                buffer.as_mut_ptr().cast(),
                capacity,
                &raw mut needed,
            )
        };
        if ok != 0 {
            break;
        }
        let code = last_error();
        let needed = usize::try_from(needed).unwrap_or(0);
        if code == ERROR_INSUFFICIENT_BUFFER && needed > buffer.len() * WORD {
            buffer.resize(needed.div_ceil(WORD), 0);
            continue;
        }
        if is_unsupported_request(code) {
            return Ok(None);
        }
        return Err(code);
    }
    // SAFETY: the buffer holds the valid descriptor written by the successful
    // call above.
    let len = unsafe { GetSecurityDescriptorLength(buffer.as_mut_ptr().cast()) };
    let len = usize::try_from(len).unwrap_or(0).min(buffer.len() * WORD);
    let bytes = words_to_bytes(&buffer, len);
    mark_sacl_capture(&bytes, sacl)
        .map(Some)
        .map_err(|_| ERROR_INVALID_PARAMETER)
}

/// Writes one element of `descriptor` (aligned storage of a validated
/// self-relative descriptor).
fn write_element(
    path: &WidePath,
    flags: u32,
    element: SecurityElement,
    descriptor: &[u32],
) -> Result<(), u32> {
    let file = create_file(path, element.access(), OPEN_EXISTING, flags)?;
    // SAFETY: the handle is open with the right that `element` requires; the
    // descriptor is valid, aligned and outlives the call, which only reads it.
    let ok = unsafe {
        SetKernelObjectSecurity(
            file.as_raw_handle(),
            element.information(),
            descriptor.as_ptr().cast_mut().cast(),
        )
    };
    if ok == 0 { Err(last_error()) } else { Ok(()) }
}

/// Elements refused with their Win32 codes, and names of elements that were
/// never captured.
pub(crate) type DescriptorOutcome = (Vec<(SecurityElement, u32)>, Vec<&'static str>);

/// Writes the elements of `desired` that differ from the current descriptor.
pub(crate) fn apply_descriptor(
    path: &WidePath,
    flags: u32,
    desired: &[u8],
) -> Result<DescriptorOutcome, u32> {
    let parsed = SecurityDescriptor::parse(desired).map_err(|_| ERROR_INVALID_PARAMETER)?;
    let current_bytes = read_descriptor(path, flags).ok().flatten();
    let current = current_bytes
        .as_deref()
        .and_then(|bytes| SecurityDescriptor::parse(bytes).ok());
    let plan = restore_plan(&parsed, current.as_ref());
    let words = aligned_words(desired);
    let refused = plan
        .apply
        .into_iter()
        .filter_map(|element| {
            write_element(path, flags, element, &words)
                .err()
                .map(|code| (element, code))
        })
        .collect();
    Ok((refused, plan.not_captured))
}

/// Restores complete metadata of the entry at `path` itself (links are not
/// followed) and reports what could not be restored.
///
/// The order is security descriptor, then times and attributes, so that a
/// read-only attribute is set last. Stream content is written by the caller
/// before; streams of `meta` that are missing or have another size are
/// reported as `stream:<name>`. Extended attributes and file flags do not
/// exist on Windows and are reported when present.
pub(crate) fn restore_metadata(
    path: &AbsPath,
    meta: &ExtendedMeta,
) -> FsResult<MetaTransferReport> {
    let wide = WidePath::of(path)?;
    let fail = |code: u32| os_error(code, "restore_meta", Some(path.as_path()));
    let mut report = MetaTransferReport::default();
    let current = create_file(&wide, FILE_READ_ATTRIBUTES, OPEN_EXISTING, ENTRY_FLAGS)
        .and_then(|file| basic_info(&file))
        .map_err(fail)?;
    for xattr in &meta.xattrs {
        report
            .not_transferred
            .push(format!("xattr:{}", xattr.name.to_string_lossy()));
    }
    if meta.flags.is_some() {
        report.not_transferred.push("flags".to_owned());
    }
    if let Some(acl) = &meta.acl {
        match apply_descriptor(&wide, ENTRY_FLAGS, acl) {
            Ok((refused, not_captured)) => {
                report
                    .not_transferred
                    .extend(refused.iter().map(|(element, _)| element.name().to_owned()));
                report
                    .not_transferred
                    .extend(not_captured.iter().map(|name| (*name).to_owned()));
            }
            Err(_) => report.not_transferred.push("acl".to_owned()),
        }
    }
    if !meta.streams.is_empty() {
        let present = list_streams(&wide).unwrap_or_default();
        for stream in &meta.streams {
            if !present.contains(stream) {
                report
                    .not_transferred
                    .push(format!("stream:{}", stream.name));
            }
        }
    }
    let desired_attributes = if meta.basic.windows_attributes == 0 {
        attributes_for_mode(current.FileAttributes, meta.basic.mode)
    } else {
        meta.basic.windows_attributes
    };
    let info = FILE_BASIC_INFO {
        CreationTime: meta.basic.created_ns.map_or(0, unix_ns_to_filetime),
        LastAccessTime: meta.basic.accessed_ns.map_or(0, unix_ns_to_filetime),
        LastWriteTime: unix_ns_to_filetime(meta.basic.modified_ns),
        ChangeTime: 0,
        FileAttributes: basic_info_attributes(desired_attributes),
    };
    let applied = create_file(&wide, FILE_WRITE_ATTRIBUTES, OPEN_EXISTING, ENTRY_FLAGS)
        .and_then(|file| set_basic_info(&file, &info).and_then(|()| basic_info(&file)));
    match applied {
        Ok(after) => report.not_transferred.extend(untransferable_attributes(
            desired_attributes,
            after.FileAttributes,
        )),
        Err(_) => report.not_transferred.push("attributes".to_owned()),
    }
    Ok(report)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
