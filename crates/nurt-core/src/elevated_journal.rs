// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The journal of the account with the references to elevated entries.
//!
//! An elevated operation is written to the system journal. The account keeps
//! a reference to it ([`crate::elevation::ReferenceLog`]) and this wrapper
//! shows the reference in `history` and `journal.status` with the number
//! `ELEVATED_OP_BASE` plus the number of the system entry, so that the
//! numbers of the two journals never collide and `undo` of such a number is
//! resolved without ambiguity.

use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{
    BeginRequest, HistoryQuery, JournalState, JournalStore, LimitsOutcome, LimitsRequest,
    MutationTx, OpId, OperationDetails, OperationRecord, PinOutcome, PinRequest, PurgeOutcome,
    PurgeRequest, RecoveryReport, RedoRequest, RollbackReason, SweepReport, UndoOutcome,
    UndoRequest, VolumeSpace,
};
use nurt_contracts::record::OperationState;
use nurt_contracts::value::{Instant, Size};

use crate::elevation::{ELEVATED_OP_BASE, ElevatedReference, ReferenceLog};

const ZERO_HASH: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

/// Journal of the account that also lists the elevated entries.
pub(crate) struct ElevatedJournal {
    inner: Arc<dyn JournalStore>,
    log: ReferenceLog,
}

impl ElevatedJournal {
    pub(crate) fn new(inner: Arc<dyn JournalStore>, log: ReferenceLog) -> Self {
        Self { inner, log }
    }

    fn record(reference: &ElevatedReference) -> OperationRecord {
        let backend = match &reference.as_user {
            Some(user) => format!("{} as {user}", reference.backend),
            None => reference.backend.clone(),
        };
        OperationRecord {
            id: ELEVATED_OP_BASE + reference.op,
            time: Instant::from_unix_nanos(reference.time_ms.saturating_mul(1_000_000)),
            kind: reference.kind.clone(),
            action: None,
            summary: format!(
                "elevated through {backend}; entry {} of the system journal",
                reference.op
            ),
            state: if reference.undone {
                OperationState::Undone
            } else {
                OperationState::Available
            },
            files: 0,
            bytes: Size::from_bytes(0),
            irreversible: Vec::new(),
            bulk: false,
            session: None,
            undo_of: Vec::new(),
            redo_of: None,
            undo_scope: None,
            ast_hash: ZERO_HASH.to_owned(),
            manifest_sha256: ZERO_HASH.to_owned(),
            chain: None,
            pinned: false,
            items: None,
        }
    }
}

impl JournalStore for ElevatedJournal {
    fn recover(&self) -> Result<RecoveryReport> {
        self.inner.recover()
    }

    fn begin(&self, request: BeginRequest) -> Box<dyn MutationTx + '_> {
        self.inner.begin(request)
    }

    fn rollback_committed(&self, op: OpId, reason: RollbackReason) -> Result<UndoOutcome> {
        self.inner.rollback_committed(op, reason)
    }

    fn history(&self, query: &HistoryQuery) -> Result<Vec<OperationRecord>> {
        if query.path.is_some() || query.deleted_only {
            return self.inner.history(query);
        }
        let elevated: Vec<OperationRecord> = self.log.latest().iter().map(Self::record).collect();
        if let Some(id) = query.id {
            if id.0 >= ELEVATED_OP_BASE {
                return Ok(elevated
                    .into_iter()
                    .filter(|record| record.id == id.0)
                    .filter(|record| query.state.is_none_or(|state| record.state == state))
                    .collect());
            }
            return self.inner.history(query);
        }
        // Pages are cut by the key (time, number), which is the order of the
        // list, so that a cursor given by an entry of either journal is
        // consistent.
        let key = |record: &OperationRecord| (record.time.as_unix_nanos(), record.id);
        let cursor = |id: Option<OpId>| -> Result<Option<(i64, u64)>> {
            let Some(id) = id else {
                return Ok(None);
            };
            if let Some(record) = elevated.iter().find(|record| record.id == id.0) {
                return Ok(Some(key(record)));
            }
            let found = self.inner.history(&HistoryQuery {
                id: Some(id),
                ..HistoryQuery::default()
            })?;
            Ok(found.first().map(key))
        };
        let before = cursor(query.before)?;
        let since = cursor(query.since)?;
        let passes = |record: &OperationRecord| {
            let at = key(record);
            before.is_none_or(|limit| at < limit)
                && since.is_none_or(|limit| at > limit)
                && query.state.is_none_or(|state| record.state == state)
        };
        let wanted = query.limit.map(|limit| limit as usize);
        let mut merged: Vec<OperationRecord> = elevated.into_iter().filter(passes).collect();
        // Entries of the account come newest first; read them in pages until
        // `limit` of them pass the filters or the journal ends.
        let page = wanted.map_or(500, |limit| limit.max(50));
        let mut collected: Vec<OperationRecord> = Vec::new();
        let mut cursor_id: Option<OpId> = None;
        loop {
            let chunk = self.inner.history(&HistoryQuery {
                id: None,
                limit: Some(u32::try_from(page).unwrap_or(u32::MAX)),
                state: query.state,
                before: cursor_id,
                since: None,
                path: None,
                deleted_only: false,
            })?;
            let Some(last) = chunk.last().map(|record| OpId(record.id)) else {
                break;
            };
            let full = chunk.len() >= page;
            let oldest_key = chunk.last().map(key);
            collected.extend(chunk.into_iter().filter(passes));
            let enough = wanted.is_some_and(|limit| collected.len() >= limit);
            let behind_since = since.is_some_and(|limit| oldest_key.is_some_and(|at| at <= limit));
            if enough || !full || behind_since {
                break;
            }
            cursor_id = Some(last);
        }
        merged.append(&mut collected);
        merged.sort_by_key(|record| std::cmp::Reverse(key(record)));
        if let Some(limit) = wanted {
            merged.truncate(limit);
        }
        Ok(merged)
    }

    fn details(&self, op: OpId) -> Result<OperationDetails> {
        if op.0 >= ELEVATED_OP_BASE {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "details of an elevated entry are kept in the system journal",
            )
            .with_next_step(format!(
                "run `history id: {} details: true privilege: elevated`",
                op.0 - ELEVATED_OP_BASE
            )));
        }
        self.inner.details(op)
    }

    fn undo(&self, request: &UndoRequest) -> Result<UndoOutcome> {
        self.inner.undo(request)
    }

    fn redo(&self, request: &RedoRequest) -> Result<UndoOutcome> {
        self.inner.redo(request)
    }

    fn pin(&self, request: &PinRequest) -> Result<PinOutcome> {
        self.inner.pin(request)
    }

    fn purge(&self, request: &PurgeRequest) -> Result<PurgeOutcome> {
        self.inner.purge(request)
    }

    fn status(&self, volume: Option<&AbsPath>) -> Result<Vec<JournalState>> {
        let mut states = self.inner.status(volume)?;
        let count = self
            .log
            .latest()
            .iter()
            .filter(|reference| !reference.undone)
            .count() as u64;
        if let Some(first) = states.first_mut() {
            first.entries_total += count;
            first.entries_available += count;
        }
        Ok(states)
    }

    fn limits(&self, request: &LimitsRequest) -> Result<LimitsOutcome> {
        self.inner.limits(request)
    }

    fn space(&self, volume: Option<&AbsPath>) -> Result<Vec<VolumeSpace>> {
        self.inner.space(volume)
    }

    fn sweep(&self) -> Result<SweepReport> {
        self.inner.sweep()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hint_for_details_is_a_call_of_history() {
        let reference = ElevatedReference {
            op: 3,
            backend: "sudo".to_owned(),
            as_user: None,
            kind: "write".to_owned(),
            time_ms: 1,
            journal: "/var/lib/nurt".to_owned(),
            undone: false,
        };
        let record = ElevatedJournal::record(&reference);
        assert_eq!(record.id, ELEVATED_OP_BASE + 3);
        assert_eq!(record.state, OperationState::Available);
    }
}
