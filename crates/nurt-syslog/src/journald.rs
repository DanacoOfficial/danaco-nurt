// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reader of systemd journal files.
//!
//! The reader parses the journal file format directly, without the systemd
//! library and without running any program. Files are read with positioned
//! reads (`pread` on Unix) through a small block cache, never memory mapped,
//! so a file that is truncated or removed while it is read cannot fault the
//! process. Every offset is checked against the read boundary before use;
//! a structural defect ends the reading of that file and is reported as the
//! notice `log_file_skipped`, while the entries read before the defect are
//! kept.
//!
//! The parser compiles on every system, so that journal files copied from a
//! Linux host can be read anywhere; [`JournaldStore::local`] selects the
//! journal of the running system and exists on Linux only.

#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing, clippy::cast_possible_truncation)]

mod discover;
mod file;
pub mod hash;
mod header;
mod index;
mod mapping;
mod object;
mod reader;
mod store;

pub use store::{JournaldStore, query_bytes};

/// Defect of a journal file.
#[derive(Debug)]
enum Fault {
    /// The file ends before its declared size.
    Truncated,
    /// An object or a link violates the structure of the format.
    Corrupt,
    /// The signature, the header size or an incompatible flag is unknown.
    Unsupported,
    /// A position lies beyond the read boundary, which happens for objects
    /// written after the file was opened.
    Beyond,
    /// Reading failed.
    Io(std::io::Error),
}

impl Fault {
    /// Class reported in the `detail` of `log_file_skipped`.
    const fn class(&self) -> &'static str {
        match self {
            Self::Truncated | Self::Beyond => "truncated",
            Self::Corrupt => "corrupt",
            Self::Unsupported => "unsupported_format",
            Self::Io(_) => "io",
        }
    }
}

impl From<std::io::Error> for Fault {
    fn from(error: std::io::Error) -> Self {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            Self::Truncated
        } else {
            Self::Io(error)
        }
    }
}
