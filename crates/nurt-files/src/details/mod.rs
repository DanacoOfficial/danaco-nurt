// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Metadata of entries beyond the fields every listing shares.
//!
//! | Module | Content |
//! |---|---|
//! | [`accounts`] | names of accounts and groups by numeric identifier |
//! | [`acl`] | POSIX access control lists in their extended attribute form |
//! | [`mode`] | permission modes in octal and symbolic notation |
//! | [`content`] | media type, text properties, EXIF tags, PDF information and image dimensions |

pub(crate) mod accounts;
pub(crate) mod acl;
pub(crate) mod content;
pub(crate) mod mode;
