// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::edit::report
// Responsibility:   Outcome of a change of a document.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use nurt_contracts::report::{Notable, NotableEntry};

use crate::model::Span;

/// Outcome of a change of a document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EditReport {
    /// Changed spans of the new text, sorted.
    pub changed: Vec<Span>,
    /// Number of nodes changed, created or removed (`change.replacements`
    /// of the effect report).
    pub replacements: u64,
    /// Unusual effects of the change (`alias_shared`, `keys_created`,
    /// `duplicate_key`).
    pub notable: Vec<NotableEntry>,
    /// Markers of the change (`change.flags`), for example `keys_created`.
    pub flags: Vec<String>,
}

impl EditReport {
    /// Returns `true` when no byte changed.
    #[must_use]
    pub fn is_unchanged(&self) -> bool {
        self.changed.is_empty()
    }

    /// Adds a notable effect with its default severity, once per code.
    pub fn note(&mut self, code: Notable, detail: Option<String>) {
        if self.notable.iter().any(|entry| entry.code == code) {
            return;
        }
        let mut entry = NotableEntry::new(code);
        entry.detail = detail;
        self.notable.push(entry);
    }

    /// Adds a flag once.
    pub fn flag(&mut self, flag: &str) {
        if !self.flags.iter().any(|existing| existing == flag) {
            self.flags.push(flag.to_owned());
        }
    }

    /// Adds the replacements, notable effects and flags of `other`; changed
    /// spans are not merged because they refer to different texts.
    pub fn absorb(&mut self, other: Self) {
        self.replacements += other.replacements;
        for entry in other.notable {
            if !self
                .notable
                .iter()
                .any(|existing| existing.code == entry.code)
            {
                self.notable.push(entry);
            }
        }
        for flag in other.flags {
            self.flag(&flag);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_without_changed_spans_is_unchanged() {
        let mut report = EditReport::default();
        assert!(report.is_unchanged());
        report.changed.push(Span::new(1, 2));
        assert!(!report.is_unchanged());
    }

    #[test]
    fn notes_and_flags_are_kept_once() {
        let mut report = EditReport::default();
        report.note(Notable::KeysCreated, Some("a".to_owned()));
        report.note(Notable::KeysCreated, Some("b".to_owned()));
        report.note(Notable::DuplicateKey, None);
        report.flag("keys_created");
        report.flag("keys_created");
        assert_eq!(report.notable.len(), 2);
        assert_eq!(report.notable[0].detail.as_deref(), Some("a"));
        assert_eq!(report.flags, ["keys_created"]);
    }

    #[test]
    fn absorbing_a_report_adds_counts_notes_and_flags_without_spans() {
        let mut first = EditReport {
            replacements: 2,
            ..EditReport::default()
        };
        first.note(Notable::KeysCreated, None);
        first.flag("a");
        let mut second = EditReport {
            changed: vec![Span::new(0, 1)],
            replacements: 3,
            ..EditReport::default()
        };
        second.note(Notable::KeysCreated, Some("other".to_owned()));
        second.note(Notable::DuplicateKey, None);
        second.flag("a");
        second.flag("b");
        first.absorb(second);
        assert_eq!(first.replacements, 5);
        assert_eq!(first.notable.len(), 2);
        assert_eq!(first.notable[0].detail, None);
        assert_eq!(first.flags, ["a", "b"]);
        assert!(first.is_unchanged());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
