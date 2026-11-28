// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-report::rules::irreversible
// Responsibility:   Collection of the reasons that make an effect irreversible.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Collection of the reasons that make an effect irreversible.
//!
//! Reasons come from three places: the `irreversible` field of each change,
//! the entries reported by commands and the entries recorded by the journal
//! for the operation. The report lists each reason once per target; the
//! field is absent when no reason was collected.

use nurt_contracts::report::{Irreversible, IrreversibleEntry};

use super::hits::{Collected, Hits, combine_counts, many_targets, raw_paths};
use crate::assemble::TargetChange;

/// Collects the irreversible effects of an operation.
///
/// Changes that carry a reason produce one entry per target; a reason shared
/// by more than [`ENTRIES_PER_CODE`](super::ENTRIES_PER_CODE) targets
/// produces one entry with the number of targets and their common directory.
/// Entries with the same reason, path and detail are merged into one with
/// the sum of their counts. The result follows the declaration order of the
/// reasons, then the path, then the detail; it is empty when the effect is
/// fully reversible.
#[must_use]
pub fn collect_irreversible(
    changes: &[TargetChange],
    reported: impl IntoIterator<Item = IrreversibleEntry>,
) -> Vec<IrreversibleEntry> {
    let mut by_reason: Vec<(Irreversible, Hits)> = Vec::new();
    for target in changes {
        let Some(reason) = target.change.irreversible else {
            continue;
        };
        let index = match by_reason.iter().position(|(known, _)| *known == reason) {
            Some(index) => index,
            None => {
                by_reason.push((reason, Hits::default()));
                by_reason.len() - 1
            }
        };
        by_reason[index]
            .1
            .push(target.change.path.as_str(), None, || None);
    }
    let mut entries: Vec<IrreversibleEntry> = by_reason
        .into_iter()
        .flat_map(|(reason, hits)| entries_for(reason, hits))
        .collect();
    let raw = raw_paths(changes);
    if !raw.is_empty() {
        for entry in &mut entries {
            entry.raw_path = entry
                .path
                .as_deref()
                .and_then(|path| raw.get(path))
                .map(|bytes| (*bytes).clone());
        }
    }
    entries.extend(reported);
    entries.sort_by(|left, right| merge_key(left).cmp(&merge_key(right)));
    entries.dedup_by(|later, earlier| {
        if merge_key(later) != merge_key(earlier) {
            return false;
        }
        earlier.count = combine_counts(earlier.count, later.count);
        true
    });
    entries
}

fn entries_for(reason: Irreversible, hits: Hits) -> Vec<IrreversibleEntry> {
    match hits.collect() {
        Collected::Each(list) => list
            .into_iter()
            .map(|hit| IrreversibleEntry {
                path: Some(hit.path),
                ..IrreversibleEntry::new(reason)
            })
            .collect(),
        Collected::Many { total, ancestor } => vec![IrreversibleEntry {
            path: ancestor,
            count: Some(total),
            detail: Some(many_targets(total)),
            ..IrreversibleEntry::new(reason)
        }],
    }
}

fn merge_key(entry: &IrreversibleEntry) -> (usize, Option<&str>, Option<&str>) {
    (
        reason_rank(entry.reason),
        entry.path.as_deref(),
        entry.detail.as_deref(),
    )
}

fn reason_rank(reason: Irreversible) -> usize {
    Irreversible::ALL
        .iter()
        .position(|candidate| *candidate == reason)
        .unwrap_or(Irreversible::ALL.len())
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
