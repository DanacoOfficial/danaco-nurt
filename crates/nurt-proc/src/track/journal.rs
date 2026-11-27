// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::track::journal
// Responsibility:   Recording of detected changes in the operation journal and their change.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Recording of detected changes in the operation journal and their change
//! records.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use nurt_contracts::error::Result;
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{ExternalChange, ExternalChangeKind, MutationTx, TargetIndex};
use nurt_contracts::record::{Change, ChangeAction};
use nurt_contracts::report::Irreversible;
use nurt_contracts::value::{Path as ValuePath, Size};

use super::diff::{Action, Detected};
use super::preimage::{Kept, Preimage};
use super::scan::{EntryState, Kind};

/// Store name of a created entry.
const STORE_CREATED: &str = "created";

/// Store name of a change without kept content.
const STORE_NONE: &str = "none";

/// Context of recording.
pub(super) struct Recorder<'a> {
    /// Kept previous content.
    pub(super) preimage: &'a Preimage,
    /// Declared indexes belong to the transaction being finished.
    pub(super) declared: bool,
    /// Flag of a change record of a file without kept content.
    pub(super) not_kept: &'a dyn Fn(&Path) -> Option<&'static str>,
}

impl Recorder<'_> {
    fn index(&self, path: &Path) -> Option<TargetIndex> {
        self.declared
            .then(|| self.preimage.kept(path).map(|kept| TargetIndex(kept.index)))
            .flatten()
    }

    fn external(&self, path: &Path, kind: ExternalChangeKind) -> Option<ExternalChange> {
        Some(ExternalChange {
            path: AbsPath::new(path).ok()?,
            kind,
            declared: self.index(path),
        })
    }

    /// Journal changes of one detected change. A file renamed without kept
    /// content of its source reports only the deletion: undo must not remove
    /// the new path when it cannot bring back the old one. A renamed
    /// directory is always reported as created; its files are reported
    /// individually.
    fn externals(&self, change: &Detected) -> Vec<ExternalChange> {
        let path = change.path.as_path();
        let entries = match &change.action {
            Action::Create => vec![self.external(path, ExternalChangeKind::Created)],
            Action::Delete => vec![self.external(path, ExternalChangeKind::Deleted)],
            Action::Rename { from, .. } => {
                let mut entries = vec![self.external(from, ExternalChangeKind::Deleted)];
                let directory = change
                    .after
                    .as_ref()
                    .is_some_and(|state| state.kind == Kind::Directory);
                if directory || self.preimage.kept(from).is_some() {
                    entries.push(self.external(path, ExternalChangeKind::Created));
                }
                entries
            }
            Action::Modify | Action::Attributes | Action::Link | Action::Replace => {
                vec![self.external(path, ExternalChangeKind::Modified)]
            }
        };
        entries.into_iter().flatten().collect()
    }

    /// Reports the changes to `tx` and returns the change records, without
    /// renames implied by the rename of their directory.
    ///
    /// # Errors
    ///
    /// `E_IO` from `MutationTx::observe_external` when the state of a changed
    /// entry cannot be read; the transaction is then rolled back.
    pub(super) fn record(
        &self,
        detected: &[Detected],
        tx: &mut dyn MutationTx,
    ) -> Result<Vec<Change>> {
        let externals: Vec<ExternalChange> = detected
            .iter()
            .flat_map(|change| self.externals(change))
            .collect();
        if externals.is_empty() {
            return Ok(Vec::new());
        }
        let applied = tx.observe_external(&externals)?;
        let hashes: HashMap<PathBuf, String> = if applied.len() == externals.len() {
            externals
                .iter()
                .zip(applied)
                .filter_map(|(external, applied)| {
                    let hash = applied.after.hash?.to_string();
                    Some((external.path.as_path().to_path_buf(), hash))
                })
                .collect()
        } else {
            HashMap::new()
        };
        Ok(detected
            .iter()
            .filter(|change| !matches!(change.action, Action::Rename { implied: true, .. }))
            .map(|change| self.change_record(change, &hashes))
            .collect())
    }

    fn change_record(&self, change: &Detected, hashes: &HashMap<PathBuf, String>) -> Change {
        let (action, from) = match &change.action {
            Action::Create => (ChangeAction::Create, None),
            Action::Modify | Action::Replace => (ChangeAction::Modify, None),
            Action::Delete => (ChangeAction::Delete, None),
            Action::Rename { from, .. } => (ChangeAction::Rename, Some(from.as_path())),
            Action::Attributes => (ChangeAction::Attributes, None),
            Action::Link => (ChangeAction::Link, None),
        };
        let source = from.unwrap_or(change.path.as_path());
        let kept = self.preimage.kept(source);
        let mut record = Change {
            path: value_path(&change.path),
            action,
            from: from.map(value_path),
            size_before: file_size(change.before.as_ref()),
            size_after: file_size(change.after.as_ref()),
            hash_before: kept
                .and_then(|kept| kept.hash_before.clone())
                .or_else(|| change.before.as_ref().and_then(|state| state.hash.clone())),
            hash_after: hashes
                .get(&change.path)
                .cloned()
                .or_else(|| change.after.as_ref().and_then(|state| state.hash.clone())),
            link_target: change
                .after
                .as_ref()
                .and_then(|state| state.link.as_deref())
                .map(value_path),
            ..Change::default()
        };
        self.store(&mut record, change, source, kept);
        record
    }

    fn store(&self, record: &mut Change, change: &Detected, source: &Path, kept: Option<&Kept>) {
        if matches!(change.action, Action::Create) {
            record.store = Some(STORE_CREATED.to_owned());
            return;
        }
        match kept {
            Some(kept) => {
                record.store = Some(kept.mode.as_str().to_owned());
                record.irreversible = kept.irreversible;
            }
            None => {
                record.store = Some(STORE_NONE.to_owned());
                record.irreversible = Some(Irreversible::PreviousContentNotKept);
                record.flags = (self.not_kept)(source).map(|flag| vec![flag.to_owned()]);
            }
        }
    }
}

fn value_path(path: &Path) -> ValuePath {
    match AbsPath::new(path) {
        Ok(abs) => ValuePath::from(abs.to_language_string()),
        Err(_) => ValuePath::from(path.to_string_lossy().into_owned()),
    }
}

fn file_size(state: Option<&EntryState>) -> Option<Size> {
    state
        .filter(|state| state.kind == Kind::File)
        .map(|state| Size::from_bytes(state.size))
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
