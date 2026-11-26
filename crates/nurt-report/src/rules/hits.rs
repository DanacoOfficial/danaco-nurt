// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Collection of the targets that meet one rule, with a bounded number of
//! entries per rule.

use std::collections::HashMap;

use nurt_contracts::value::Bytes;

use super::paths::{reportable_ancestor, shrink_to_common};
use crate::assemble::TargetChange;

/// Native bytes of the targets that are not valid Unicode, by the display form
/// of their path, so that an entry about such a target can carry them.
pub(crate) fn raw_paths(changes: &[TargetChange]) -> HashMap<&str, &Bytes> {
    changes
        .iter()
        .filter_map(|target| {
            Some((
                target.change.path.as_str(),
                target.change.raw_path.as_ref()?,
            ))
        })
        .collect()
}

/// Largest number of targets reported one by one for a single code or
/// reason; beyond it the targets are reported as one entry with their number
/// and their common directory. The bound matches the ten largest groups shown
/// by a bulk summary.
pub const ENTRIES_PER_CODE: usize = 10;

/// Count of an entry merged from two entries: the sum of the counts that are
/// present, or none when neither entry has one.
pub(crate) fn combine_counts(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        (count, None) | (None, count) => count,
    }
}

/// One target that meets a rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hit {
    /// Target concerned.
    pub(crate) path: String,
    /// Quantity concerned, for example the number of replacements.
    pub(crate) count: Option<u64>,
    /// Short description.
    pub(crate) detail: Option<String>,
}

/// Targets that meet one rule, in the order they were observed.
#[derive(Debug, Default)]
pub(crate) struct Hits {
    entries: Vec<Hit>,
    total: u64,
    ancestor: Option<String>,
}

/// Entries produced from the collected targets.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Collected {
    /// Every target, one entry each.
    Each(Vec<Hit>),
    /// Too many targets to list: their number and common directory.
    Many {
        /// Number of targets.
        total: u64,
        /// Common directory of the targets, when there is one.
        ancestor: Option<String>,
    },
}

impl Hits {
    /// Records a target. The detail is computed only while targets are still
    /// listed one by one.
    pub(crate) fn push(
        &mut self,
        path: &str,
        count: Option<u64>,
        detail: impl FnOnce() -> Option<String>,
    ) {
        self.total += 1;
        match &mut self.ancestor {
            Some(prefix) => shrink_to_common(prefix, path),
            None => self.ancestor = Some(path.to_owned()),
        }
        if self.entries.len() < ENTRIES_PER_CODE {
            self.entries.push(Hit {
                path: path.to_owned(),
                count,
                detail: detail(),
            });
        }
    }

    /// Whether no target was recorded.
    pub(crate) fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// Finishes the collection.
    pub(crate) fn collect(self) -> Collected {
        if self.total <= ENTRIES_PER_CODE as u64 {
            return Collected::Each(self.entries);
        }
        Collected::Many {
            total: self.total,
            ancestor: self.ancestor.and_then(reportable_ancestor),
        }
    }
}

/// Description of an entry that stands for `total` targets.
pub(crate) fn many_targets(total: u64) -> String {
    format!("{total} targets")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_targets_up_to_the_bound() {
        let mut hits = Hits::default();
        for index in 0..ENTRIES_PER_CODE {
            hits.push(&format!("/srv/app/{index}"), Some(1), || None);
        }
        match hits.collect() {
            Collected::Each(entries) => assert_eq!(entries.len(), ENTRIES_PER_CODE),
            Collected::Many { .. } => panic!("expected separate entries"),
        }
    }

    #[test]
    fn aggregates_targets_beyond_the_bound() {
        let mut hits = Hits::default();
        let mut computed = 0;
        for index in 0..=ENTRIES_PER_CODE {
            hits.push(&format!("/srv/app/{index}"), None, || {
                computed += 1;
                None
            });
        }
        assert_eq!(computed, ENTRIES_PER_CODE);
        assert_eq!(
            hits.collect(),
            Collected::Many {
                total: ENTRIES_PER_CODE as u64 + 1,
                ancestor: Some("/srv/app".to_owned()),
            }
        );
    }

    #[test]
    fn empty_collection_reports_nothing() {
        let hits = Hits::default();
        assert!(hits.is_empty());
        assert_eq!(hits.collect(), Collected::Each(Vec::new()));
    }
}
