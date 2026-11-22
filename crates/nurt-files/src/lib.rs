// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! File and content commands of Danaco Nurt.
//!
//! The crate implements the commands that list, find, read, search, write,
//! replace, insert, edit, copy, move, remove, rename and create files, inspect
//! and change attributes, compare and patch files, compute hashes and convert
//! encodings, and the commands of archives, compression and notebooks.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

use std::sync::Arc;

use nurt_contracts::stage::StageImpl;

pub mod common;
pub mod fuzzy;
pub mod read;
pub mod walk;

/// Stage implementations of the commands of this crate, for the registry of
/// the engine.
///
/// The registry selects an implementation by the command name and the input
/// record kinds of its
/// [`StageDescriptor`](nurt_contracts::stage::StageDescriptor); every
/// implementation listed here has a combination of both no other
/// implementation of this crate has.
#[must_use]
pub fn stages() -> Vec<Arc<dyn StageImpl>> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::stages;

    #[test]
    fn stage_descriptors_are_unique_and_usable() {
        let mut seen = HashSet::new();
        for stage in stages() {
            let descriptor = stage.descriptor();
            let key = (
                descriptor.command.clone(),
                format!("{:?}", descriptor.input),
            );
            assert!(
                seen.insert(key),
                "{} is registered twice",
                descriptor.command
            );
            assert!(
                stage.as_read().is_some() || stage.as_effect().is_some(),
                "{} has no interface",
                descriptor.command
            );
        }
    }
}
