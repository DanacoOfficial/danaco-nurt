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
