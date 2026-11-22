// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interfaces shared by the test and analysis commands.

use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::stage::ExecCtx;
use nurt_contracts::value::Record;

/// Kind of a stored entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum EntryKind {
    /// Test run with its test results.
    TestRun,
    /// Analysis with its findings.
    Analysis,
}

/// Stored test run or analysis.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredEntry {
    /// Identifier.
    pub(crate) id: String,
    /// Header record (`test_run` or `analysis`).
    pub(crate) header: Record,
    /// Item records (`test_result` or `finding`), in output order.
    pub(crate) items: Vec<Record>,
}

/// History of test runs and analyses in the state directory.
pub(crate) trait VerifyStore: Send + Sync {
    /// Reserves the next identifier of the kind.
    fn next_id(&self, ctx: &ExecCtx, kind: EntryKind) -> Result<String>;

    /// Saves an entry, replacing an entry with the same identifier, and
    /// removes the oldest entries beyond the configured history size.
    fn save(&self, ctx: &ExecCtx, kind: EntryKind, entry: &StoredEntry) -> Result<()>;

    /// Loads an entry; `None` when it does not exist.
    fn load(&self, ctx: &ExecCtx, kind: EntryKind, id: &str) -> Result<Option<StoredEntry>>;

    /// Header records of the newest entries, newest first.
    fn list(&self, ctx: &ExecCtx, kind: EntryKind, limit: u32) -> Result<Vec<Record>>;

    /// Identifiers of all stored entries of the kind, for candidates in
    /// error messages.
    fn ids(&self, ctx: &ExecCtx, kind: EntryKind) -> Result<Vec<String>>;
}

/// Store used by the commands.
pub(crate) fn verify_store() -> Arc<dyn VerifyStore> {
    crate::store::verify_store()
}
