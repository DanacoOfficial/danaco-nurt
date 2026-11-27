// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::edit
// Responsibility:   Byte-range edits and the guards that bound every change to its target.
// Dependencies:     apply, diff, nurt_contracts, report
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Byte-range edits and the guards that bound every change to its target.
//!
//! A document is never written anew. A change is a list of [`ByteEdit`]
//! values applied to the original bytes; everything outside the edits stays
//! byte for byte. [`apply`] applies non-overlapping edits and returns the new
//! bytes with an [`OffsetMap`]. [`guard::commit`] runs the guards before a
//! change is accepted:
//!
//! - scope: every changed byte lies in a declared target span or in its
//!   permitted surroundings (blanks, line breaks, separators);
//! - meaning: the new bytes, read back by the reference parser of the format,
//!   give the value the change should produce;
//! - identity: opening a document and taking its bytes gives the original bytes,
//!   and setting a value equal to the current one changes nothing.
//!
//! A violation fails with `E_UNSUPPORTED_CONSTRUCT` naming the construct;
//! nothing is written.
//!
//! ```
//! use nurt_doc::edit::{ByteEdit, apply};
//! use nurt_doc::model::Span;
//!
//! let applied = apply(b"a = 1\nb = 2\n", &[ByteEdit::replace(Span::new(4, 5), "10")])?;
//! assert_eq!(applied.bytes, b"a = 10\nb = 2\n");
//! assert_eq!(applied.offsets.map(8), 9);
//! # Ok::<(), nurt_contracts::error::Error>(())
//! ```

mod apply;
mod diff;
pub mod guard;
mod report;

pub use apply::{Applied, ByteEdit, OffsetMap, apply, check_overlaps};
pub use diff::diff;
pub use report::EditReport;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
