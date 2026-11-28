// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::admin::effects
// Responsibility:   Shared parts of the effect stages of this module: markers, changes, notes and effect records.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Shared parts of the effect stages of this module: markers, changes,
//! notes and effect records.

use nurt_contracts::record::{Change, ChangeAction, Effect, EffectStatus};
use nurt_contracts::report::{Irreversible, IrreversibleEntry, Notable, NotableEntry};
use nurt_contracts::value::{Record, Value};

/// Prefix of the `detail` of the marker of an administration change.
pub(crate) const MARKER_PREFIX: &str = "admin:";

/// Marker that opens the journal entry of a change outside the file system.
///
/// The engine removes the marker when it derives the inverse of the
/// operation from the recorded states.
#[must_use]
pub(crate) fn marker(kind: &str, target: &str) -> IrreversibleEntry {
    let mut entry = IrreversibleEntry::new(Irreversible::ExternalEffect);
    entry.path = Some(target.to_owned());
    entry.detail = Some(format!("{MARKER_PREFIX}{kind}"));
    entry
}

/// `state` change of `target` with the values before and after.
#[must_use]
pub(crate) fn state_change(target: &str, before: Option<&str>, after: Option<&str>) -> Change {
    let mut change = Change::new(target.to_owned(), ChangeAction::State);
    change.before = before.map(str::to_owned);
    change.after = after.map(str::to_owned);
    change
}

/// Note that the system already held the requested state.
#[must_use]
pub(crate) fn unchanged(target: &str, detail: &str) -> NotableEntry {
    let mut note = NotableEntry::new(Notable::Unchanged);
    note.path = Some(target.to_owned());
    note.detail = Some(detail.to_owned());
    note
}

/// `effect` record of one change.
#[must_use]
pub(crate) fn effect_record(
    kind: &str,
    target: &str,
    status: EffectStatus,
    before: Option<&str>,
    after: Option<&str>,
) -> Record {
    Record::from(Effect {
        status,
        kind: kind.to_owned(),
        target: Some(target.to_owned()),
        action: Some(kind.to_owned()),
        before: before.map(|text| Value::Text(text.to_owned())),
        after: after.map(|text| Value::Text(text.to_owned())),
        ..Effect::default()
    })
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
