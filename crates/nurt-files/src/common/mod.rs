// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::common
// Responsibility:   Building blocks shared by the stages of the file, content, archive and.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Building blocks shared by the stages of the file, content, archive and
//! notebook commands.
//!
//! | Module | Content |
//! |---|---|
//! | [`path`] | paths of arguments and input records, resolved against the base directory |
//! | [`params`] | parameters shared by several commands: `encoding`, `line_ending`, `on_missing`, `if_match` and the walk filters |
//! | [`etag`] | text form of entity tags, observed state of targets and `E_STALE` |
//! | [`entry`] | `entry` records from file metadata |
//! | [`text`] | text files read for a change and written back in their original form |
//! | [`notable`] | notable entries of the effect report |
//! | [`mutation`] | target declarations, file operations and `change` records |
//!
//! Stages read through the file system of the execution context and change
//! files only through the transaction passed to
//! [`EffectStage::apply`](nurt_contracts::stage::EffectStage::apply). An
//! effect stage observes every target in the plan step
//! ([`etag::observe`]), declares it with the observed state as its
//! expectation ([`etag::Observed::expect`]) and applies its change
//! ([`mutation::apply_changes`], [`text::write_text_preserving`]); the journal
//! checks the expectation atomically with the change.

pub mod continuation;
pub mod entry;
pub mod etag;
pub mod mutation;
pub mod notable;
pub mod params;
pub mod path;
pub mod text;

#[cfg(test)]
mod fixture;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
