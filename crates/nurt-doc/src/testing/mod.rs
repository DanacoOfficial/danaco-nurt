// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Round-trip harness for document formats.
//!
//! Enabled by the `testing` feature. The harness opens every file of a
//! corpus, checks that the document keeps the file byte for byte, selects up
//! to a fixed number of nodes per file with a seeded generator and applies
//! the edit operations to each of them: setting the current value, replacing
//! a scalar, changing the type of a value, inserting a key, appending to a
//! list and removing a node. Every accepted change is checked again
//! independently of the guards built into the formats: the changed bytes
//! must lie in the lines of the node, and the changed document must read
//! back as the expected value. The [`Summary`] lists violations and counts
//! refusals by error code and by construct name.
//!
//! The module also provides a minimal `key = value` format
//! ([`KeyValueFormat`]) for tests of the shared layers and an in-memory
//! journal ([`MemoryJournal`]) for tests of commands that change documents.
//!
//! ```
//! use nurt_contracts::services::DocFormatId;
//! use nurt_doc::testing::{HarnessOptions, KeyValueFormat, run_document};
//!
//! let format = KeyValueFormat::new(DocFormatId::new("kv")?);
//! let summary = run_document("a.kv", b"# port\nport = 8080\nname = web\n", &format, &HarnessOptions::default());
//! assert_eq!(summary.files, 1);
//! assert!(summary.violations.is_empty(), "{:?}", summary.violations);
//! assert!(summary.accepted > 0);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod harness;
mod journal;
mod kv;

pub use harness::{
    FileFilter, Guard, HarnessOperation, HarnessOptions, Summary, Violation, run_corpus, run_document,
};
pub use journal::{MemoryJournal, MemoryTx};
pub use kv::{KeyValueFormat, KeyValueReference};
