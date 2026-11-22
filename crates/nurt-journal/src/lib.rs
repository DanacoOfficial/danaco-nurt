// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Operation journal of Danaco Nurt.
//!
//! Every changing operation is recorded in the journal before it takes effect,
//! so that it can be undone individually. The crate implements the journal
//! interface defined in `nurt_contracts`: entry manifests, the recording
//! protocol, preservation of previous content, crash recovery, integrity
//! checks, undo and redo, conflict detection and retention by operation count,
//! age and size.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub(crate) mod blob;
pub(crate) mod catalog;
mod commands;
pub(crate) mod conflict;
pub(crate) mod history;
pub(crate) mod integrity;
pub mod layout;
pub mod manifest;
pub(crate) mod overwrite;
pub(crate) mod pin;
pub(crate) mod recovery;
pub(crate) mod retention;
pub mod salvage;
pub(crate) mod seq;
pub mod store;
pub(crate) mod tx;
pub(crate) mod undo;
pub mod volumes;

pub use commands::register;
pub use retention::resolve::ConfiguredLimits;
pub use store::{Journal, JournalBuilder};
