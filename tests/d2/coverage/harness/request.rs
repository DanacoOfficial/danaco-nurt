// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Builders of journal requests with the values the suites share.

use nurt_contracts::ast::AST_VERSION;
use nurt_contracts::bound::{BOUND_VERSION, BoundDocument};
use nurt_contracts::journal::{
    BeginRequest, OnConflict, OpId, OpKind, RedoRequest, SessionId, UndoRequest, UndoSelector,
};
use serde_json::json;

/// Session of every request unless a test names another one.
pub(crate) const SESSION: &str = "session-a";

/// Second session, for tests of entries of other sessions.
pub(crate) const OTHER_SESSION: &str = "session-b";

/// Bound program with one stage named after the command; the journal stores
/// it verbatim and hashes it.
pub(crate) fn program(command: &str) -> BoundDocument {
    let document = json!({
        "nurt": AST_VERSION,
        "bound": BOUND_VERSION,
        "catalog_rev": "test",
        "corr_rev": "test",
        "program": { "statements": [{
            "t": "pipeline",
            "head": { "t": "stage", "name": command, "effect": "local", "phase": "mutate", "args": [] },
            "stages": []
        }]}
    });
    serde_json::from_value(document).expect("the bound program of a test stage is valid")
}

/// Request of a user operation that counts as a step.
pub(crate) fn begin(command: &str, action: Option<&str>) -> BeginRequest {
    begin_in(command, action, SESSION)
}

/// Request of a user operation of the given session.
pub(crate) fn begin_in(command: &str, action: Option<&str>, session: &str) -> BeginRequest {
    let kind = match action {
        Some(action) => OpKind::with_action(command, action),
        None => OpKind::command(command),
    };
    BeginRequest {
        kind,
        session: Some(SessionId(session.to_owned())),
        program: program(command),
        corrections: Vec::new(),
        bulk: false,
        counts_as_step: true,
        preserve_budget: None,
    }
}

/// `undo` request of the default session.
pub(crate) fn undo(selector: UndoSelector, on_conflict: OnConflict) -> UndoRequest {
    UndoRequest {
        selector,
        on_conflict,
        session: Some(SessionId(SESSION.to_owned())),
        program: program("undo"),
    }
}

/// `undo N` with the default conflict behaviour.
pub(crate) fn undo_op(op: OpId) -> UndoRequest {
    undo(UndoSelector::Op(op), OnConflict::Report)
}

/// `undo` of the newest entry of the default session.
pub(crate) fn undo_last() -> UndoRequest {
    undo(UndoSelector::Last, OnConflict::Report)
}

/// `redo` of the given undo entry, or of the latest one.
pub(crate) fn redo(undo_entry: Option<OpId>) -> RedoRequest {
    RedoRequest {
        undo_entry,
        session: Some(SessionId(SESSION.to_owned())),
        program: program("redo"),
    }
}
