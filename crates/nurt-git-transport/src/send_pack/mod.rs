// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-transport::send_pack
// Responsibility:   Client side of the send-pack protocol.
// Dependencies:     advertisement, client, command, file_plan, objects, pack, refs, report
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Client side of the send-pack protocol.
//!
//! A push reads the reference advertisement of `receive-pack`
//! ([`discover`]), sends update commands with the selected capabilities and
//! a pack of the missing objects ([`send`]) and reads the status report
//! ([`PushReport`]). Protocol version 0 is used because version 2 has no
//! push command. The module also plans journaled updates of reference files
//! ([`plan_ref_edits`], [`FilePlan`]), shared by the local tracking
//! references of the pushing repository and by the in-process
//! `receive-pack` service of [`crate::local::receive`].

mod advertisement;
mod client;
mod command;
mod error;
mod file_plan;
mod objects;
mod pack;
pub(crate) mod pkt;
mod refs;
mod report;
#[cfg(any(test, feature = "testutil"))]
pub mod testing;

pub use advertisement::{AdvertisedRef, Advertisement, discover};
pub use client::{SendOutcome, send};
pub use command::{Negotiated, RefCommand, SendOptions, negotiate, write_commands};
pub use file_plan::{FilePlan, LOCK_SUFFIX, LockedCheck, lock_path, preserve_and_apply};
pub use objects::{MISSING_OBJECT, objects_to_send};
pub use pack::{DELTA_NOTICE_BYTES, PackSummary, write_pack};
pub use refs::{
    Identity, LogPolicy, RefEdit, content_hash, plan_ref_edits, plan_ref_edits_in, read_ref,
    ref_path, validate_name,
};
pub use report::{PushReport, RefReport, RefStatus, RejectReason, parse_lines, parse_report};

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
