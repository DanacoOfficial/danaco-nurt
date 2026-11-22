// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Facilities shared by the readers of system sources: the root of system
//! files, errors of unreadable sources and canonical record paths.

pub(crate) mod sysroot;

pub(crate) use self::sysroot::{REQUIRED_PRIVILEGE, SysRoot, canonical_path, source_error};
