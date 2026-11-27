// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::macos
// Responsibility:   MacOS back end.
// Dependencies:     acl, attrlist, errno, meta, names, space, sys, volume
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! macOS back end.
//!
//! `MacosFs` implements the file system interface with the calls of macOS:
//! `renamex_np` with `RENAME_SWAP` and `RENAME_EXCL` for exchange and rename
//! without replacement, `clonefile` for block-sharing clones on APFS,
//! `F_FULLFSYNC` for durable flushes, extended attributes including the
//! `com.apple.*` namespace, ACLs in the text form of `acl_t`, BSD file flags
//! and the creation time through `setattrlist`. Volume space is read through
//! the capacity keys of Core Foundation, which report the space ordinary
//! programs can use.
//!
//! The functions and types exported next to `MacosFs` hold the logic that
//! needs no system call (classification of `errno` values, volume types,
//! attribute buffers, ACL text, space figures and flag arithmetic). They
//! are compiled on every system so that they are tested independently of
//! the host.

mod acl;
mod attrlist;
mod errno;
mod meta;
mod names;
mod space;
#[cfg(target_os = "macos")]
mod sys;
mod volume;

#[cfg(target_os = "macos")]
pub use sys::MacosFs;

pub use acl::{ACL_TEXT_HEADER, AclTextError, acl_from_system_text, acl_text_for_system};
pub use attrlist::{
    ATTR_BIT_MAP_COUNT, ATTR_CMN_CRTIME, ATTR_VOL_CAPABILITIES, ATTR_VOL_INFO, ATTR_VOL_UUID,
    AttrList, TIMESPEC_SIZE, VOLUME_INFO_REPLY_SIZE, VolumeInfoReply, creation_time_buffer,
    parse_volume_info, split_nanos, timespec_nanos, uuid_text,
};
pub use errno::{ErrnoContext, errno_error, error_kind, full_sync_unsupported};
pub use meta::{
    MODE_PERMISSION_BITS, SF_APPEND, SF_IMMUTABLE, SF_SETTABLE, UF_APPEND, UF_COMPRESSED,
    UF_HIDDEN, UF_IMMUTABLE, UF_SETTABLE, WRITE_BLOCKING_FLAGS, file_kind, flags_to_restore,
    hidden_flags, unblocked_flags,
};
pub use names::{NAME_MAX_BYTES, NOINDEX_SUFFIX, temp_name};
pub use space::{Q_GETQUOTA, QuotaFigures, SpaceFigures, USRQUOTA, quota_command, space_info};
pub use volume::{
    MNT_LOCAL, MNT_QUOTA, MNT_RDONLY, VOL_CAP_FMT_CASE_SENSITIVE, VOL_CAP_INT_CLONE,
    VOL_CAP_INT_EXTENDED_ATTR, VOL_CAP_INT_EXTENDED_SECURITY, VOL_CAP_INT_RENAME_EXCL,
    VOL_CAP_INT_RENAME_SWAP, VolumeCapabilities, device_number, statfs_name, volume_features,
    volume_kind,
};

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
