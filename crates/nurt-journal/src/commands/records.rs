// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conversion of journal results into records of the language.

use nurt_contracts::fs::{AbsPath, FileKind, Follow, Fs};
use nurt_contracts::journal::{
    Conflict, ItemDetail, OpId, OperationDetails, SkippedEntry, TargetAction, TargetOutcome,
    TargetPath, UndoOutcome,
};
use nurt_contracts::record::{Change, ChangeAction, Effect, EffectStatus};
use nurt_contracts::report::{FLAG_DIRECTORY, Irreversible, IrreversibleEntry, tree_flags};
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
        TargetAction::Append => ChangeAction::Truncate,
        _ => ChangeAction::Modify,
    }
}

/// Most entries read below a restored directory to count them.
const MAX_COUNTED_ENTRIES: u64 = 200_000;

/// Content of a directory: files (links included), directories below it and
/// the size of the files in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TreeContent {
    files: u64,
    directories: u64,
    bytes: u64,
}

/// Counts the entries below `root`; `None` when a directory cannot be read
/// or the tree is larger than [`MAX_COUNTED_ENTRIES`].
fn count_tree(fs: &dyn Fs, root: &AbsPath) -> Option<TreeContent> {
    let mut content = TreeContent {
        files: 0,
        directories: 0,
        bytes: 0,
    };
    let mut pending = vec![root.clone()];
    while let Some(directory) = pending.pop() {
        for item in fs.read_dir(&directory).ok()? {
            let entry = item.ok()?;
            if content.files.saturating_add(content.directories) >= MAX_COUNTED_ENTRIES {
                return None;
            }
            let path = AbsPath::new(directory.as_path().join(&entry.name)).ok()?;
            let meta = fs.metadata(&path, Follow::NoFollow).ok()?;
            if meta.kind == FileKind::Directory {
                content.directories += 1;
                pending.push(path);
            } else {
                content.files += 1;
                content.bytes = content.bytes.saturating_add(meta.size);
            }
        }
    }
    Some(content)
}

/// Item restored by `undo` or `redo` as a `change` of the report.
///
/// A directory is marked as one. A directory that the item created, as the
/// restoring of a removed tree does, also reports the files and directories
/// below it and their size, read through `fs` when given.
pub(super) fn change(outcome: &TargetOutcome, fs: Option<&dyn Fs>) -> Change {
    let mut change = Change::new(target_text(&outcome.path), change_action(&outcome.action));
    if let TargetAction::Rename { to, .. } = &outcome.action {
        change.from = Some(change.path.clone());
        change.to = Some(Path::from(to.to_language_string()));
    }
    change.size_before = outcome.before.size.map(Size::from_bytes);
    change.hash_before = outcome.before.hash.as_ref().map(ToString::to_string);
    if let Some(after) = &outcome.after {
        change.size_after = after.size.map(Size::from_bytes);
        change.hash_after = after.hash.as_ref().map(ToString::to_string);
    }
    let kind = outcome
        .after
        .as_ref()
        .filter(|after| after.exists)
        .and_then(|after| after.kind)
        .or_else(|| outcome.before.exists.then_some(outcome.before.kind).flatten());
    if kind == Some(FileKind::Directory) {
        let content = match (&outcome.path, fs) {
            (TargetPath::Fs(path), Some(fs)) if change.action == ChangeAction::Create => {
                count_tree(fs, path)
            }
            _ => None,
        };
        change.flags = Some(match content {
            Some(content) => {
                change.size_after = Some(Size::from_bytes(content.bytes));
                tree_flags(content.files, content.directories)
            }
            None => vec![FLAG_DIRECTORY.to_owned()],
        });
    }
    change
}

/// Item of an entry as the `change` that undoing the entry makes: the state
/// recorded after the operation becomes the state before it.
fn reversal(item: &ItemDetail) -> Change {
    let mut change = match &item.action {
        TargetAction::Rename { to, .. } => {
            let mut change = Change::new(to.to_language_string(), ChangeAction::Rename);
            change.from = Some(change.path.clone());
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
pub(super) fn undo_outcome<'a>(
    kind: &str,
    result: UndoOutcome,
    fs: Option<&dyn Fs>,
) -> EffectOutcome<'a> {
    let changes: Vec<Change> = result
        .restored
        .iter()
        .map(|outcome| change(outcome, fs))
        .collect();
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

    fn restored(target: &str, action: TargetAction, kind: FileKind) -> TargetOutcome {
        TargetOutcome {
            index: nurt_contracts::journal::TargetIndex(0),
            path: path(target),
            action,
            mode: PreserveMode::Moved,
            bytes_held: 0,
            before: nurt_contracts::journal::BeforeState::default(),
            after: Some(nurt_contracts::journal::AfterState {
                exists: true,
                kind: Some(kind),
                size: Some(4096),
                ..nurt_contracts::journal::AfterState::default()
            }),
            irreversible: None,
            notable: Vec::new(),
        }
    }

    fn tree_fs() -> nurt_fs::mem::MemFs {
        let fs = nurt_fs::mem::MemFs::builder()
            .volume(nurt_fs::mem::VolumeSpec::posix(AbsPath::new("/").unwrap()))
            .build()
            .unwrap();
        for dir in ["/d/tree/sub/deep", "/d/tree/empty"] {
            fs.create_dir_all(&AbsPath::new(dir).unwrap()).unwrap();
        }
        fs.seed_file(&AbsPath::new("/d/tree/a.bin").unwrap(), b"1234")
            .unwrap();
        fs.seed_file(&AbsPath::new("/d/tree/sub/b.bin").unwrap(), b"56")
            .unwrap();
        fs.seed_file(&AbsPath::new("/d/tree/sub/deep/c.bin").unwrap(), b"7")
            .unwrap();
        fs
    }

    #[test]
    fn restored_tree_reports_its_kind_content_and_size() {
        let fs = tree_fs();
        let outcome = restored("/d/tree", TargetAction::Delete, FileKind::Directory);
        let item = change(&outcome, Some(&fs));
        assert_eq!(item.action, ChangeAction::Delete);
        assert_eq!(item.flags, Some(vec![FLAG_DIRECTORY.to_owned()]));
        let create = restored("/d/tree", TargetAction::Create, FileKind::Directory);
        let item = change(&create, Some(&fs));
        assert_eq!(item.action, ChangeAction::Create);
        let flags = item.flags.clone().unwrap();
        assert_eq!(nurt_contracts::report::tree_counts(&flags), Some((3, 3)));
        assert!(flags.iter().any(|flag| flag == FLAG_DIRECTORY));
        assert_eq!(item.size_after, Some(Size::from_bytes(7)));
    }

    #[test]
    fn restored_directory_without_a_file_system_is_still_marked_as_a_directory() {
        let create = restored("/d/tree", TargetAction::Create, FileKind::Directory);
        let item = change(&create, None);
        assert_eq!(item.flags, Some(vec![FLAG_DIRECTORY.to_owned()]));
        assert_eq!(item.size_after, Some(Size::from_bytes(4096)));
    }

    #[test]
    fn restored_file_has_no_directory_notes() {
        let fs = tree_fs();
        let create = restored("/d/tree/a.bin", TargetAction::Create, FileKind::File);
        assert_eq!(change(&create, Some(&fs)).flags, None);
    }

    #[test]
    fn tree_count_gives_up_when_a_directory_is_missing() {
        let fs = tree_fs();
        assert_eq!(count_tree(&fs, &AbsPath::new("/d/none").unwrap()), None);
        let content = count_tree(&fs, &AbsPath::new("/d/tree/empty").unwrap()).unwrap();
        assert_eq!((content.files, content.directories, content.bytes), (0, 0, 0));
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
        assert_eq!(changes[1].from, Some(Path::from("/d/e.txt")));
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
        let outcome = undo_outcome("undo", result, None);
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

    use nurt_contracts::journal::{AfterState, BeforeState, TargetIndex};

    fn outcome_of(target: &str, action: TargetAction) -> TargetOutcome {
        TargetOutcome {
            index: TargetIndex(0),
            path: path(target),
            action,
            mode: PreserveMode::Moved,
            bytes_held: 0,
            before: BeforeState {
                exists: true,
                size: Some(6),
                hash: Some(ContentHash([1; 32])),
                ..BeforeState::default()
            },
            after: Some(AfterState {
                exists: true,
                size: Some(5),
                hash: Some(ContentHash([2; 32])),
                ..AfterState::default()
            }),
            irreversible: None,
            notable: Vec::new(),
        }
    }

    #[test]
    fn numbers_and_counts_saturate_instead_of_wrapping() {
        assert_eq!(number(0), Value::Int(0));
        assert_eq!(number(u64::MAX), Value::Int(i64::MAX));
        assert_eq!(number(u64::try_from(i64::MAX).unwrap()), Value::Int(i64::MAX));
        assert_eq!(count(0), 0);
        assert_eq!(count(usize::MAX), u64::try_from(usize::MAX).unwrap_or(u64::MAX));
        assert_eq!(op_list(&[]), Value::List(Vec::new()));
        assert_eq!(op_list(&[OpId(u64::MAX)]), Value::List(vec![Value::Int(i64::MAX)]));
    }

    #[test]
    fn restored_items_are_reported_with_their_actions_sizes_and_hashes() {
        let modified = change(&outcome_of("/d/a.txt", TargetAction::Append));
        assert_eq!(modified.action, ChangeAction::Modify);
        assert_eq!(modified.size_before, Some(Size::from_bytes(6)));
        assert_eq!(modified.size_after, Some(Size::from_bytes(5)));
        assert_eq!(modified.hash_before, Some(ContentHash([1; 32]).to_string()));
        assert_eq!(modified.hash_after, Some(ContentHash([2; 32]).to_string()));
        assert_eq!(modified.to, None);

        let created = change(&outcome_of("/d/zażółć", TargetAction::CreateTree));
        assert_eq!(created.action, ChangeAction::Create);
        assert_eq!(created.path, Path::from("/d/zażółć"));

        let moved = change(&outcome_of(
            "/d/a.txt",
            TargetAction::Rename {
                to: AbsPath::new("/d/b.txt").unwrap(),
                replace: true,
            },
        ));
        assert_eq!(moved.action, ChangeAction::Rename);
        assert_eq!(moved.to, Some(Path::from("/d/b.txt")));
        assert_eq!(moved.from, Some(Path::from("/d/a.txt")));

        let mut unchanged = outcome_of("/d/a.txt", TargetAction::Delete);
        unchanged.after = None;
        let removed = change(&unchanged);
        assert_eq!(removed.action, ChangeAction::Delete);
        assert_eq!(removed.size_after, None);
        assert_eq!(removed.hash_after, None);
    }

    #[test]
    fn reversing_an_entry_reverses_each_action() {
        let expected = [
            (TargetAction::Create, ChangeAction::Delete),
            (TargetAction::CreateTree, ChangeAction::Delete),
            (TargetAction::Link, ChangeAction::Delete),
            (TargetAction::Delete, ChangeAction::Create),
            (TargetAction::Append, ChangeAction::Truncate),
            (TargetAction::Attributes, ChangeAction::Attributes),
            (TargetAction::Modify, ChangeAction::Modify),
            (TargetAction::Replace, ChangeAction::Modify),
        ];
        for (action, reversed) in expected {
            let details = OperationDetails {
                record: OperationRecord::default(),
                items: vec![detail("/d/a.txt", action.clone())],
            };
            let changes = reversals(&details, None);
            assert_eq!(changes.len(), 1, "{action:?}");
            assert_eq!(changes[0].action, reversed, "{action:?}");
            assert_eq!(changes[0].path, Path::from("/d/a.txt"));
        }
    }

    #[test]
    fn items_already_undone_or_not_selected_are_not_reversed() {
        let mut undone = detail("/d/a.txt", TargetAction::Modify);
        undone.undone = true;
        let details = OperationDetails {
            record: OperationRecord::default(),
            items: vec![undone, detail("/d/b.txt", TargetAction::Modify)],
        };
        assert_eq!(reversals(&details, None).len(), 1);
        let nothing: [TargetPath; 0] = [];
        assert!(reversals(&details, Some(&nothing)).is_empty());
        let other = [path("/d/c.txt")];
        assert!(reversals(&details, Some(&other)).is_empty());
        let empty = OperationDetails {
            record: OperationRecord::default(),
            items: Vec::new(),
        };
        assert!(reversals(&empty, None).is_empty());
    }

    #[test]
    fn effect_outcomes_carry_changes_and_markers_only_when_there_are_some() {
        let bare = outcome(effect("journal.pin", EffectStatus::Ok), Vec::new(), Vec::new());
        assert!(bare.changes.is_empty());
        assert!(bare.irreversible.is_empty());
        let marker = marker(Irreversible::Purged, &[OpId(1)]).unwrap();
        let full = outcome(
            effect("journal.purge", EffectStatus::Ok),
            vec![Change::new("/d/a.txt".to_owned(), ChangeAction::Delete)],
            vec![marker],
        );
        assert_eq!(full.changes.len(), 1);
        assert_eq!(full.irreversible.len(), 1);
    }

    #[test]
    fn every_kind_of_difference_is_named_for_the_language() {
        let named = |kind: ConflictKind| {
            let conflict = Conflict {
                op: OpId(3),
                path: path("/d/zażółć.txt"),
                kind,
                expected: Some(ContentHash([1; 32])),
                actual: None,
            };
            difference(&conflict)
        };
        for (kind, name) in [
            (ConflictKind::Modified, "modified"),
            (ConflictKind::Deleted, "deleted"),
            (ConflictKind::Created, "created"),
            (ConflictKind::TypeChanged, "type_changed"),
            (ConflictKind::BlobModified, "blob_modified"),
        ] {
            let value = named(kind);
            assert_eq!(value.get("difference"), Some(&Value::Enum(name.to_owned())));
            assert_eq!(value.get("later"), None);
            assert_eq!(value.get("op"), Some(&Value::Int(3)));
            assert_eq!(
                value.get("path"),
                Some(&Value::Path("/d/zażółć.txt".to_owned()))
            );
            assert_eq!(
                value.get("expected"),
                Some(&Value::Text(ContentHash([1; 32]).to_string()))
            );
            assert_eq!(value.get("actual"), None);
        }
    }

    #[test]
    fn undoing_an_append_truncates_and_undoing_a_truncate_modifies() {
        assert_eq!(reverse_action(&TargetAction::Append), ChangeAction::Truncate);
        assert_eq!(reverse_action(&TargetAction::Truncate), ChangeAction::Modify);
        assert_eq!(reverse_action(&TargetAction::Create), ChangeAction::Delete);
        assert_eq!(reverse_action(&TargetAction::Delete), ChangeAction::Create);
    }

    #[test]
    fn provider_targets_are_shown_as_keys() {
        use nurt_contracts::fs::{ProviderKey, ProviderScheme};

        let key = TargetPath::Provider(ProviderKey {
            scheme: ProviderScheme("env".to_owned()),
            segments: Vec::new(),
            value: Some("PATH".to_owned()),
        });
        assert_eq!(target_text(&key), "env:@PATH");
    }
}
