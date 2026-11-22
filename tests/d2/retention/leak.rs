// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Results of operations carry no expiry time and nothing from which one can
//! be derived; retention state is reported only by `journal.status`.
//!
//! Every channel is checked twice: no key of its JSON form names an expiry
//! (`expires`, `deadline`, `ttl`, `remaining`, `until` and similar words),
//! and no number in it equals the expiry instant of an entry in nanoseconds,
//! milliseconds or seconds.

use std::time::Duration;

use nurt_contracts::env::WallTime;
use nurt_contracts::fs::{AbsPath, ContentHash};
use nurt_contracts::journal::{
    AfterState, BeforeState, ItemDetail, OpHandle, OpId, OpKind, OpState, OperationDetails,
    OperationRecord, PinOutcome, PreserveMode, PurgeOutcome, RollbackReport, SessionId,
    SkippedEntry, TargetAction, TargetIndex, TargetOutcome, TargetPath, UndoOutcome, UndoScope,
};
use nurt_contracts::report::{Irreversible, IrreversibleEntry};
use serde::Serialize;
use serde_json::Value;

use crate::harness::clock::START;
use crate::harness::fixture::{Channel, Fixture};
use crate::support::{self, AGE};

/// Words that name an expiry when they appear as a part of a key.
const FORBIDDEN_WORDS: &[&str] = &[
    "expires",
    "expire",
    "expiry",
    "expiration",
    "deadline",
    "ttl",
    "remaining",
    "until",
    "retention",
    "lifetime",
    "age",
];

const NANOS_PER_MILLI: i128 = 1_000_000;
const NANOS_PER_SECOND: i128 = 1_000_000_000;

fn key_names_expiry(key: &str) -> bool {
    key.split(['_', '-', '.'])
        .any(|word| FORBIDDEN_WORDS.contains(&word.to_ascii_lowercase().as_str()))
}

fn forbidden_keys(value: &Value, path: &str, found: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                let here = format!("{path}.{key}");
                if key_names_expiry(key) {
                    found.push(here.clone());
                }
                forbidden_keys(inner, &here, found);
            }
        }
        Value::Array(items) => {
            for (index, inner) in items.iter().enumerate() {
                forbidden_keys(inner, &format!("{path}[{index}]"), found);
            }
        }
        _ => {}
    }
}

fn numbers(value: &Value, out: &mut Vec<i128>) {
    match value {
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                out.push(i128::from(integer));
            } else if let Some(integer) = number.as_u64() {
                out.push(i128::from(integer));
            }
        }
        Value::String(text) => {
            if let Ok(integer) = text.parse::<i128>() {
                out.push(integer);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| numbers(item, out)),
        Value::Object(map) => map.values().for_each(|item| numbers(item, out)),
        _ => {}
    }
}

/// Forms of an expiry instant a channel must not contain.
fn deadline_forms(deadline: WallTime) -> [i128; 3] {
    let nanos = deadline.unix_nanos();
    [nanos, nanos / NANOS_PER_MILLI, nanos / NANOS_PER_SECOND]
}

/// Asserts that the JSON form of a result names no expiry and contains none
/// of the given expiry instants.
pub(crate) fn assert_no_expiry(channel: &str, value: &Value, deadlines: &[WallTime]) {
    let mut keys = Vec::new();
    forbidden_keys(value, channel, &mut keys);
    assert!(keys.is_empty(), "{channel} names an expiry: {keys:?}");
    let mut found = Vec::new();
    numbers(value, &mut found);
    for deadline in deadlines {
        for form in deadline_forms(*deadline) {
            assert!(
                !found.contains(&form),
                "{channel} contains the expiry instant {form}"
            );
        }
    }
}

fn json(value: &impl Serialize) -> Value {
    serde_json::to_value(value).expect("result serializes")
}

/// Expiry instants of the entries listed by `history` under the age limit of
/// the scenarios.
fn deadlines(fixture: &Fixture) -> Vec<WallTime> {
    fixture
        .history()
        .iter()
        .map(|record| record.time.saturating_add(AGE))
        .collect()
}

/// Checks `history` and `history details` of every entry.
pub(crate) fn history_carries_no_expiry(fixture: &Fixture) {
    let records = fixture.history();
    let deadlines = deadlines(fixture);
    assert_no_expiry("history", &json(&records), &deadlines);
    for record in &records {
        if let Ok(details) = fixture.journal.details(record.id) {
            assert_no_expiry("history details", &json(&details), &deadlines);
        }
    }
}

fn hash(byte: u8) -> ContentHash {
    ContentHash([byte; 32])
}

fn sample_path() -> AbsPath {
    AbsPath::new(if cfg!(windows) {
        "C:/Users/user/notes.txt"
    } else {
        "/home/user/notes.txt"
    })
    .expect("sample path is absolute")
}

fn sample_outcome() -> TargetOutcome {
    TargetOutcome {
        index: TargetIndex(0),
        path: TargetPath::Fs(sample_path()),
        action: TargetAction::Modify,
        mode: PreserveMode::Exchange,
        bytes_held: 4096,
        before: BeforeState {
            exists: true,
            size: Some(4096),
            hash: Some(hash(1)),
            mode: Some(0o644),
            ..BeforeState::default()
        },
        after: Some(AfterState {
            exists: true,
            size: Some(10),
            hash: Some(hash(2)),
            mode: Some(0o644),
            ..AfterState::default()
        }),
        irreversible: Some(Irreversible::NoSpaceForCopy),
        notable: Vec::new(),
    }
}

fn sample_record() -> OperationRecord {
    OperationRecord {
        id: OpId(7),
        time: START,
        kind: OpKind::with_action("write", "overwrite"),
        summary: "overwrite /home/user/notes.txt".to_owned(),
        state: OpState::Available,
        files: 1,
        bytes: 10,
        irreversible: vec![IrreversibleEntry::new(Irreversible::NoSpaceForCopy)],
        bulk: false,
        session: Some(SessionId("session-a".to_owned())),
        undo_of: vec![OpId(6)],
        redo_of: Some(OpId(5)),
        undo_scope: Some(UndoScope::Op),
        ast_hash: hash(3),
        manifest_hash: hash(4),
        chain: Some(hash(5)),
        pinned: true,
    }
}

#[test]
fn result_types_carry_no_expiry_fields() {
    let handle = OpHandle {
        op: Some(OpId(7)),
        kind: OpKind::with_action("write", "overwrite"),
        bulk: false,
        targets: vec![sample_outcome()],
        irreversible: vec![IrreversibleEntry::new(Irreversible::NoSpaceForCopy)],
        notable: Vec::new(),
        exempt: Vec::new(),
    };
    let details = OperationDetails {
        record: sample_record(),
        items: vec![ItemDetail {
            path: TargetPath::Fs(sample_path()),
            action: TargetAction::Modify,
            mode: PreserveMode::Exchange,
            before_hash: Some(hash(1)),
            after_hash: Some(hash(2)),
            undone: false,
            released: false,
        }],
    };
    let undo = UndoOutcome {
        entry: OpId(8),
        scope: UndoScope::To,
        sources: vec![OpId(7)],
        restored: vec![sample_outcome()],
        unavailable: vec![TargetPath::Fs(sample_path())],
        skipped: vec![SkippedEntry {
            op: OpId(6),
            session: Some(SessionId("session-b".to_owned())),
        }],
        forced: Vec::new(),
        irreversible: Vec::new(),
    };
    let pin = PinOutcome {
        changed: vec![OpId(7)],
        not_available: vec![OpId(1)],
    };
    let purge = PurgeOutcome {
        released: vec![OpId(1), OpId(2)],
        bytes: 4096,
    };
    let rollback = RollbackReport {
        op: Some(OpId(9)),
        restored: vec![TargetPath::Fs(sample_path())],
        failed: Vec::new(),
    };
    let deadlines = [START.saturating_add(AGE)];
    assert_no_expiry("operation handle", &json(&handle), &deadlines);
    assert_no_expiry("history", &json(&vec![sample_record()]), &deadlines);
    assert_no_expiry("history details", &json(&details), &deadlines);
    assert_no_expiry("undo outcome", &json(&undo), &deadlines);
    assert_no_expiry("pin outcome", &json(&pin), &deadlines);
    assert_no_expiry("purge outcome", &json(&purge), &deadlines);
    assert_no_expiry("rollback report", &json(&rollback), &deadlines);
}

#[test]
fn expiry_check_detects_named_fields_and_instants() {
    let deadline = START.saturating_add(Duration::from_secs(1800));
    let named = serde_json::json!({ "op": 1, "undo_until": "later" });
    let leaked = serde_json::json!({ "op": 1, "at": deadline.unix_nanos() / NANOS_PER_SECOND });
    for value in [named, leaked] {
        let result = std::panic::catch_unwind(|| assert_no_expiry("sample", &value, &[deadline]));
        assert!(result.is_err(), "the check rejects {value}");
    }
}

/// Runs operations, undo, pin and purge, and checks every result.
#[test]
#[ignore = "requires the journal store implementation"]
fn operation_results_carry_no_expiry_time() {
    let fixture = Fixture::open(support::options());
    let first = support::large_write(&fixture, "a.bin", 4096);
    let second = support::small_write(&fixture, 1);
    let deadlines = deadlines(&fixture);
    assert_no_expiry("operation handle", &json(&first), &deadlines);
    let undone = fixture
        .journal
        .undo(&crate::harness::request::undo_op(second))
        .expect("entry is undone");
    assert_no_expiry("undo outcome", &json(&undone), &deadlines);
    let redone = fixture
        .journal
        .redo(&crate::harness::request::redo(Some(undone.entry)))
        .expect("undo is redone");
    assert_no_expiry("redo outcome", &json(&redone), &deadlines);
    let pinned = fixture
        .journal
        .pin(&nurt_contracts::journal::PinRequest {
            ids: vec![second],
            release: false,
            until: None,
        })
        .expect("entry is pinned");
    assert_no_expiry("pin outcome", &json(&pinned), &deadlines);
    let purged = fixture
        .journal
        .purge(&nurt_contracts::journal::PurgeRequest {
            volume: None,
            selector: nurt_contracts::journal::PurgeSelector::Ids(vec![second]),
        })
        .expect("entry is purged");
    assert_no_expiry("purge outcome", &json(&purged), &deadlines);
    support::finish(&fixture);
}

fn channel_carries_no_expiry(channel: Channel) {
    let fixture = Fixture::open(support::options());
    for index in 0..3 {
        support::small_write(&fixture, index);
    }
    fixture.clock.advance(AGE + support::MINUTE);
    fixture.journal.sweep().expect("expired entries are swept");
    let deadlines = deadlines(&fixture);
    assert_no_expiry(channel.name(), &fixture.channel(channel), &deadlines);
}

#[test]
#[ignore = "requires the event stream of the space service"]
fn expiry_events_carry_no_expiry_time() {
    channel_carries_no_expiry(Channel::Events);
}

#[test]
#[ignore = "requires the audit export of the journal"]
fn audit_export_carries_no_expiry_time() {
    channel_carries_no_expiry(Channel::AuditExport);
}

#[test]
#[ignore = "requires the history call of the C interface"]
fn c_interface_history_carries_no_expiry_time() {
    channel_carries_no_expiry(Channel::CInterfaceHistory);
}
