// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Archives and compression for Danaco Nurt.
//!
//! The crate lists, extracts and creates tar, zip and 7z archives and
//! compresses and decompresses gzip, zstd, xz, bzip2 and lz4 streams, with
//! protection against entries that escape the destination directory.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod codec;
