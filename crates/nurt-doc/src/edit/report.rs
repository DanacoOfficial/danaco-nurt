// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
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
