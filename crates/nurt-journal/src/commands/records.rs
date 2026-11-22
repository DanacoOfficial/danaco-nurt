// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conversion of journal results into records of the language.

use nurt_contracts::journal::{
    Conflict, ItemDetail, OpId, OperationDetails, SkippedEntry, TargetAction, TargetOutcome,
    TargetPath, UndoOutcome,
};
use nurt_contracts::record::{Change, ChangeAction, Effect, EffectStatus};
use nurt_contracts::report::{Irreversible, IrreversibleEntry};
use nurt_contracts::stage::{EffectOutcome, RecordStream, StageOutput};
use nurt_contracts::value::{Fields, Path, Record, Size, Value};

/// A count or number as a value of the language.
pub(super) fn number(value: u64) -> Value {
    Value::Int(i64::try_from(value).unwrap_or(i64::MAX))
}

/// Length of a list as a count of a record.
pub(super) fn count(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// Operation numbers as a list value.
pub(super) fn op_list(ops: &[OpId]) -> Value {
    Value::List(ops.iter().map(|op| number(op.0)).collect())
}

/// Address of a target as shown to the language.
pub(super) fn target_text(path: &TargetPath) -> String {
    crate::conflict::display_path(path)
}

fn change_action(action: &TargetAction) -> ChangeAction {
    match action {
        TargetAction::Create | TargetAction::CreateTree => ChangeAction::Create,
        TargetAction::Delete => ChangeAction::Delete,
        TargetAction::Rename { .. } => ChangeAction::Rename,
        TargetAction::Attributes => ChangeAction::Attributes,
        TargetAction::Link => ChangeAction::Link,
        TargetAction::Truncate => ChangeAction::Truncate,
        _ => ChangeAction::Modify,
    }
}

/// Action that reverses `action` when an entry is undone.
fn reverse_action(action: &TargetAction) -> ChangeAction {
    match action {
        TargetAction::Create | TargetAction::CreateTree | TargetAction::Link => {
            ChangeAction::Delete
        }
        TargetAction::Delete => ChangeAction::Create,
        TargetAction::Rename { .. } => ChangeAction::Rename,
        TargetAction::Attributes => ChangeAction::Attributes,
        TargetAction::Append | TargetAction::Truncate => ChangeAction::Truncate,
        _ => ChangeAction::Modify,
    }
}

/// Item restored by `undo` or `redo` as a `change` of the report.
pub(super) fn change(outcome: &TargetOutcome) -> Change {
    let mut change = Change::new(target_text(&outcome.path), change_action(&outcome.action));
    if let TargetAction::Rename { to, .. } = &outcome.action {
        change.to = Some(Path::from(to.to_language_string()));
    }
    change.size_before = outcome.before.size.map(Size::from_bytes);
    change.hash_before = outcome.before.hash.as_ref().map(ToString::to_string);
    if let Some(after) = &outcome.after {
        change.size_after = after.size.map(Size::from_bytes);
        change.hash_after = after.hash.as_ref().map(ToString::to_string);
    }
    change
}

/// Item of an entry as the `change` that undoing the entry makes: the state
/// recorded after the operation becomes the state before it.
fn reversal(item: &ItemDetail) -> Change {
    let mut change = match &item.action {
        TargetAction::Rename { to, .. } => {
            let mut change = Change::new(to.to_language_string(), ChangeAction::Rename);
            change.to = Some(Path::from(target_text(&item.path)));
            change
        }
        action => Change::new(target_text(&item.path), reverse_action(action)),
    };
    change.hash_before = item.after_hash.as_ref().map(ToString::to_string);
    change.hash_after = item.before_hash.as_ref().map(ToString::to_string);
    change
}

/// Planned changes of undoing the items of an entry that are still kept,
/// limited to `only` when given.
pub(super) fn reversals(details: &OperationDetails, only: Option<&[TargetPath]>) -> Vec<Change> {
    details
        .items
        .iter()
        .filter(|item| !item.undone && !item.released)
        .filter(|item| only.is_none_or(|paths| paths.contains(&item.path)))
        .map(reversal)
        .collect()
}

/// `effect` record of a journal command.
pub(super) fn effect(kind: &str, status: EffectStatus) -> Effect {
    Effect {
        status,
        kind: kind.to_owned(),
        ..Effect::default()
    }
}

/// `effect` record shown by a preview that has no file targets.
pub(super) fn planned(kind: &str) -> Record {
    Record::from(effect(kind, EffectStatus::Planned))
}

/// Irreversible marker for the given entries; `None` when there are none.
pub(super) fn marker(reason: Irreversible, ops: &[OpId]) -> Option<IrreversibleEntry> {
    (!ops.is_empty()).then(|| IrreversibleEntry {
        reason,
        path: None,
        count: Some(count(ops.len())),
        detail: None,
    })
}

/// Outcome of an effect stage whose output is one `effect` record.
pub(super) fn outcome<'a>(
    mut effect: Effect,
    changes: Vec<Change>,
    irreversible: Vec<IrreversibleEntry>,
) -> EffectOutcome<'a> {
    if !changes.is_empty() {
        effect.changes = Some(changes.clone());
    }
    if !irreversible.is_empty() {
        effect.irreversible = Some(irreversible.clone());
    }
    let mut result = EffectOutcome::new(StageOutput::records(RecordStream::once(Record::from(
        effect,
    ))));
    result.changes = changes;
    result.irreversible = irreversible;
    result
}

fn skipped(entry: &SkippedEntry) -> Value {
    let mut fields = Fields::new();
    fields.insert("op".to_owned(), number(entry.op.0));
    if let Some(session) = &entry.session {
        fields.insert("session".to_owned(), Value::Text(session.0.clone()));
    }
    Value::Map(fields)
}

/// Difference found by the undo check and overridden by `force`.
fn difference(conflict: &Conflict) -> Value {
    let kind = serde_json::to_value(&conflict.kind).unwrap_or_default();
    let mut fields = Fields::new();
    fields.insert("op".to_owned(), number(conflict.op.0));
    fields.insert("path".to_owned(), Value::Path(target_text(&conflict.path)));
    let name = kind
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    fields.insert("difference".to_owned(), Value::Enum(name.to_owned()));
    if let Some(later) = kind.get("op").and_then(serde_json::Value::as_u64) {
        fields.insert("later".to_owned(), number(later));
    }
    if let Some(hash) = &conflict.expected {
        fields.insert("expected".to_owned(), Value::Text(hash.to_string()));
    }
    if let Some(hash) = &conflict.actual {
        fields.insert("actual".to_owned(), Value::Text(hash.to_string()));
    }
    Value::Map(fields)
}

/// Outcome of `undo` or `redo`: the `effect` record of the new entry and the
/// restored items as changes of the report.
pub(super) fn undo_outcome<'a>(kind: &str, result: UndoOutcome) -> EffectOutcome<'a> {
    let changes: Vec<Change> = result.restored.iter().map(change).collect();
    let mut after = Fields::new();
    after.insert(
        "scope".to_owned(),
        Value::Enum(result.scope.as_str().to_owned()),
    );
    after.insert("sources".to_owned(), op_list(&result.sources));
    after.insert(
        "unavailable".to_owned(),
        Value::List(
            result
                .unavailable
                .iter()
                .map(|path| Value::Path(target_text(path)))
                .collect(),
        ),
    );
    after.insert(
        "skipped".to_owned(),
        Value::List(result.skipped.iter().map(skipped).collect()),
    );
    after.insert(
        "forced".to_owned(),
        Value::List(result.forced.iter().map(difference).collect()),
    );
    let summary = [
        ("operations", result.sources.len()),
        ("items_restored", result.restored.len()),
        ("items_unavailable", result.unavailable.len()),
    ]
    .into_iter()
    .map(|(name, len)| (name.to_owned(), count(len)))
    .collect();
    let record = Effect {
        op: Some(result.entry.0),
        targets: Some(count(result.restored.len())),
        summary: Some(summary),
        after: Some(Value::Map(after)),
        ..effect(kind, EffectStatus::Ok)
    };
    outcome(record, changes, result.irreversible)
}

#[cfg(test)]
mod tests {
    use nurt_contracts::fs::{AbsPath, ContentHash};
    use nurt_contracts::journal::{
        ConflictKind, OperationRecord, PreserveMode, SessionId, UndoScope,
    };

    use super::*;

    fn path(text: &str) -> TargetPath {
        TargetPath::Fs(AbsPath::new(text).unwrap())
    }

    fn detail(target: &str, action: TargetAction) -> ItemDetail {
        ItemDetail {
            path: path(target),
            action,
            mode: PreserveMode::Moved,
            before_hash: Some(ContentHash([1; 32])),
            after_hash: Some(ContentHash([2; 32])),
            undone: false,
            released: false,
        }
    }

    #[test]
    fn reversals_swap_the_states_and_reverse_the_actions() {
        let mut released = detail("/d/c.txt", TargetAction::Modify);
        released.released = true;
        let details = OperationDetails {
            record: OperationRecord::default(),
            items: vec![
                detail("/d/a.txt", TargetAction::Delete),
                detail(
                    "/d/b.txt",
                    TargetAction::Rename {
                        to: AbsPath::new("/d/e.txt").unwrap(),
                        replace: false,
                    },
                ),
                released,
            ],
        };
        let changes = reversals(&details, None);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].action, ChangeAction::Create);
        assert_eq!(
            changes[0].hash_after,
            Some(ContentHash([1; 32]).to_string())
        );
        assert_eq!(changes[1].path, Path::from("/d/e.txt"));
        assert_eq!(changes[1].to, Some(Path::from("/d/b.txt")));
        let only = [path("/d/b.txt")];
        assert_eq!(reversals(&details, Some(&only)).len(), 1);
    }

    #[test]
    fn undo_outcome_lists_sources_skipped_and_forced_entries() {
        let result = UndoOutcome {
            entry: OpId(9),
            scope: UndoScope::To,
            sources: vec![OpId(8), OpId(7)],
            restored: Vec::new(),
            unavailable: vec![path("/d/a.txt")],
            skipped: vec![SkippedEntry {
                op: OpId(6),
                session: Some(SessionId("other".to_owned())),
            }],
            forced: vec![Conflict {
                op: OpId(7),
                path: path("/d/b.txt"),
                kind: ConflictKind::LaterEntry { op: OpId(8) },
                expected: None,
                actual: None,
            }],
            irreversible: Vec::new(),
        };
        let outcome = undo_outcome("undo", result);
        assert!(outcome.changes.is_empty());
        let nurt_contracts::stage::StageData::Records(stream) = outcome.output.data else {
            panic!("the outcome has no records");
        };
        let records: Vec<Record> = stream.map(Result::unwrap).collect();
        let after = records[0].get("after").unwrap();
        assert_eq!(after.get("scope"), Some(&Value::Enum("to".to_owned())));
        assert_eq!(after.get("sources"), Some(&op_list(&[OpId(8), OpId(7)])));
        let forced = after.get("forced").and_then(Value::as_list).unwrap();
        assert_eq!(
            forced[0].get("difference"),
            Some(&Value::Enum("later_entry".to_owned()))
        );
        assert_eq!(forced[0].get("later"), Some(&Value::Int(8)));
        let skipped = after.get("skipped").and_then(Value::as_list).unwrap();
        assert_eq!(skipped[0].get("op"), Some(&Value::Int(6)));
        assert_eq!(records[0].get("op"), Some(&Value::Int(9)));
    }

    #[test]
    fn markers_count_entries_and_are_absent_for_none() {
        assert_eq!(marker(Irreversible::Purged, &[]), None);
        let entry = marker(Irreversible::Purged, &[OpId(1), OpId(2)]).unwrap();
        assert_eq!(entry.count, Some(2));
    }
}
