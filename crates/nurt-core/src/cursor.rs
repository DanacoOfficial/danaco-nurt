// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Pages of results and their cursors.
//!
//! A response carries at most `limit` records or changes. When a result is
//! longer, the engine keeps the complete result and the report gives
//! `next_cursor`. Reading a further page returns a part of the kept result
//! and never runs the program again, so paging through the result of a
//! changing program changes nothing. Kept results expire after
//! `output.spool_ttl`.
//!
//! A cursor has the form `c_<program>-<entry>-<offset>`: eight hexadecimal
//! digits of the program hash, sixteen hexadecimal digits naming the kept
//! result and the index of the first item of the page. A cursor of one
//! program is therefore never read as a page of another.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nurt_contracts::config::DurationSetting;
use nurt_contracts::env::WallTime;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::report::Report;
use serde::{Deserialize, Serialize};

use crate::state::{read_optional, sha256_hex, write_atomic};

/// Prefix of cursors.
pub const CURSOR_PREFIX: &str = "c_";

const PROGRAM_DIGITS: usize = 8;
const ENTRY_DIGITS: usize = 16;
const CURSOR_DIR: &str = "cursors";
const ENTRY_EXTENSION: &str = "json";

/// Program whose result is paged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramRef {
    /// Hexadecimal hash of the bound program.
    pub hash: String,
    /// Program text, offered as the fix when a cursor of a reading program
    /// has expired.
    pub text: Option<String>,
}

impl ProgramRef {
    /// Reference with the hash of `program`, any serializable form of the
    /// bound program.
    #[must_use]
    pub fn of<T: Serialize>(program: &T, text: Option<String>) -> Self {
        let bytes = serde_json::to_vec(program).unwrap_or_default();
        Self {
            hash: sha256_hex(&bytes),
            text,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StoredResult {
    program: String,
    text: Option<String>,
    expires: WallTime,
    report: Report,
}

struct Cursor<'a> {
    program: &'a str,
    entry: &'a str,
    offset: usize,
}

/// Kept results of one engine.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use nurt_contracts::env::WallTime;
/// use nurt_contracts::report::Report;
/// use nurt_core::cursor::{CursorStore, ProgramRef};
///
/// let report: Report = serde_json::from_value(serde_json::json!({
///     "nurt": "1.0", "status": "ok", "kind": "remove",
///     "changes": [
///         {"path": "/srv/a.log", "action": "delete"},
///         {"path": "/srv/b.log", "action": "delete"},
///         {"path": "/srv/c.log", "action": "delete"}
///     ]
/// }))?;
/// let store = CursorStore::new(None, Duration::from_secs(3600));
/// let now = WallTime::from_unix_nanos(0);
/// let first = store.first_page(report, 2, &ProgramRef::of(&"remove", None), now);
/// assert_eq!(first.changes.len(), 2);
/// let cursor = first.next_cursor.unwrap();
/// let second = store.page(&cursor, 2, now).unwrap();
/// assert_eq!(second.changes.len(), 1);
/// assert!(second.next_cursor.is_none());
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug)]
pub struct CursorStore {
    dir: Option<PathBuf>,
    ttl: Duration,
    counter: AtomicU64,
    memory: Mutex<HashMap<String, Arc<StoredResult>>>,
}

impl CursorStore {
    /// Store keeping results for `ttl`, also on disk below `dir` when given
    /// (the `results` area of the state directory).
    #[must_use]
    pub fn new(dir: Option<PathBuf>, ttl: Duration) -> Self {
        Self {
            dir: dir.map(|dir| dir.join(CURSOR_DIR)),
            ttl,
            counter: AtomicU64::new(0),
            memory: Mutex::new(HashMap::new()),
        }
    }

    /// First page of `report`: at most `limit` records or changes, with
    /// `next_cursor` when the result is longer.
    ///
    /// A result that cannot be written to disk stays available to this
    /// process.
    #[must_use]
    pub fn first_page(
        &self,
        report: Report,
        limit: u32,
        program: &ProgramRef,
        now: WallTime,
    ) -> Report {
        let limit = page_size(limit);
        if item_count(&report) <= limit {
            return report;
        }
        let entry = self.entry_id(&program.hash, now);
        let stored = StoredResult {
            program: program.hash.clone(),
            text: program.text.clone(),
            expires: now.saturating_add(self.ttl),
            report,
        };
        let (mut page, next) = slice(&stored.report, 0, limit);
        page.next_cursor = next.map(|offset| cursor_text(&stored.program, &entry, offset));
        if let Some(dir) = &self.dir
            && let Ok(bytes) = serde_json::to_vec(&stored)
        {
            let _ = write_atomic(&dir.join(format!("{entry}.{ENTRY_EXTENSION}")), &bytes);
        }
        self.lock().insert(entry, Arc::new(stored));
        page
    }

    /// The page that starts at the position named by `cursor`, of at most
    /// `limit` items.
    ///
    /// # Errors
    ///
    /// Returns `E_ARG` when the cursor is malformed, unknown, belongs to
    /// another program or has expired; for an expired cursor of a reading
    /// program the fix is the program text.
    pub fn page(&self, cursor: &str, limit: u32, now: WallTime) -> Result<Report, Error> {
        let parsed = parse_cursor(cursor).ok_or_else(|| malformed(cursor))?;
        let stored = self
            .load(parsed.entry)
            .filter(|stored| stored.program.starts_with(parsed.program))
            .ok_or_else(|| self.unknown(cursor, None))?;
        if stored.expires <= now {
            let fix = stored.text.clone().filter(|_| stored.report.op.is_none());
            return Err(self.unknown(cursor, fix));
        }
        if parsed.offset > item_count(&stored.report) {
            return Err(malformed(cursor));
        }
        let (mut page, next) = slice(&stored.report, parsed.offset, page_size(limit));
        page.next_cursor = next.map(|offset| cursor_text(&stored.program, parsed.entry, offset));
        Ok(page)
    }

    /// Removes results expired at `now` from memory and disk and returns
    /// their number.
    pub fn sweep(&self, now: WallTime) -> usize {
        let mut removed = 0;
        {
            let mut memory = self.lock();
            let before = memory.len();
            memory.retain(|_, stored| stored.expires > now);
            removed += before - memory.len();
        }
        let Some(dir) = &self.dir else {
            return removed;
        };
        let Ok(entries) = fs::read_dir(dir) else {
            return removed;
        };
        for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
            let expired = read_optional(&path)
                .ok()
                .flatten()
                .and_then(|bytes| serde_json::from_slice::<StoredResult>(&bytes).ok())
                .is_none_or(|stored| stored.expires <= now);
            if expired && fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
        removed
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Arc<StoredResult>>> {
        self.memory.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn load(&self, entry: &str) -> Option<Arc<StoredResult>> {
        if let Some(stored) = self.lock().get(entry) {
            return Some(Arc::clone(stored));
        }
        let path = self
            .dir
            .as_ref()?
            .join(format!("{entry}.{ENTRY_EXTENSION}"));
        let bytes = read_optional(&path).ok()??;
        let stored = Arc::new(serde_json::from_slice::<StoredResult>(&bytes).ok()?);
        self.lock().insert(entry.to_owned(), Arc::clone(&stored));
        Some(stored)
    }

    fn entry_id(&self, program: &str, now: WallTime) -> String {
        let seed = format!(
            "{program}:{}:{}:{}",
            now.unix_nanos(),
            std::process::id(),
            self.counter.fetch_add(1, Ordering::Relaxed)
        );
        sha256_hex(seed.as_bytes())[..ENTRY_DIGITS].to_owned()
    }

    fn unknown(&self, cursor: &str, fix: Option<String>) -> Error {
        let ttl =
            DurationSetting::from_millis(u64::try_from(self.ttl.as_millis()).unwrap_or(u64::MAX));
        let error = Error::new(
            ErrorCode::Arg,
            format!("cursor `{cursor}` is unknown or expired"),
        )
        .with_problem(format!(
            "pages of a result are kept for {ttl}; run the program again to read the result"
        ));
        match fix {
            Some(program) => error.with_fix(program),
            None => error,
        }
    }
}

fn malformed(cursor: &str) -> Error {
    Error::new(ErrorCode::Arg, format!("malformed cursor `{cursor}`"))
        .with_found(cursor)
        .with_problem("copy the cursor from `next_cursor` of an earlier response")
}

fn page_size(limit: u32) -> usize {
    usize::try_from(limit).unwrap_or(usize::MAX).max(1)
}

fn item_count(report: &Report) -> usize {
    let rows = report
        .records
        .as_ref()
        .map_or(0, |records| records.rows.len());
    rows.max(report.changes.len())
}

fn cursor_text(program: &str, entry: &str, offset: usize) -> String {
    let prefix = program.get(..PROGRAM_DIGITS).unwrap_or(program);
    format!("{CURSOR_PREFIX}{prefix}-{entry}-{offset}")
}

fn parse_cursor(cursor: &str) -> Option<Cursor<'_>> {
    let mut parts = cursor.strip_prefix(CURSOR_PREFIX)?.split('-');
    let program = parts.next()?;
    let entry = parts.next()?;
    let offset = parts.next()?.parse().ok()?;
    let hex = |text: &str, length: usize| {
        text.len() == length && text.bytes().all(|b| b.is_ascii_hexdigit())
    };
    (parts.next().is_none() && hex(program, PROGRAM_DIGITS) && hex(entry, ENTRY_DIGITS)).then_some(
        Cursor {
            program,
            entry,
            offset,
        },
    )
}

/// Page of `full` with the items `offset..offset + limit` and the offset of
/// the next page, when there is one.
fn slice(full: &Report, offset: usize, limit: usize) -> (Report, Option<usize>) {
    let total = item_count(full);
    let end = offset.saturating_add(limit).min(total);
    let mut page = full.clone();
    if let Some(records) = page.records.as_mut() {
        records.rows = window(&records.rows, offset, end);
    }
    page.changes = window(&full.changes, offset, end);
    if offset > 0 || end < total {
        let paths: Vec<String> = page
            .changes
            .iter()
            .map(|change| change.path.to_string())
            .collect();
        page.diff
            .retain(|hunk| paths.contains(&hunk.path.to_string()));
    }
    if let Some(targets) = page.targets.as_mut()
        && targets.listed.is_some()
    {
        targets.listed = Some(u64::try_from(page.changes.len()).unwrap_or(u64::MAX));
    }
    page.next_cursor = None;
    (page, (end < total).then_some(end))
}

fn window<T: Clone>(items: &[T], start: usize, end: usize) -> Vec<T> {
    let end = end.min(items.len());
    items
        .get(start.min(end)..end)
        .map(<[T]>::to_vec)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const TTL: Duration = Duration::from_secs(3600);

    fn at(secs: u64) -> WallTime {
        WallTime::from_unix_nanos(0).saturating_add(Duration::from_secs(secs))
    }

    fn records(count: usize) -> Report {
        let rows: Vec<_> = (0..count)
            .map(|index| json!({"kind": "entry", "fields": {"name": {"t": "text", "v": format!("f{index}")}}}))
            .collect();
        serde_json::from_value(json!({
            "nurt": "1.0", "status": "ok", "kind": "find",
            "records": {"type": "entry", "matched": count, "rows": rows}
        }))
        .unwrap()
    }

    fn changes(count: usize) -> Report {
        let changes: Vec<_> = (0..count)
            .map(|index| json!({"path": format!("/srv/f{index}.log"), "action": "delete"}))
            .collect();
        serde_json::from_value(json!({
            "nurt": "1.0", "status": "ok", "kind": "remove", "op": 7,
            "targets": {"count": count, "listed": count},
            "changes": changes
        }))
        .unwrap()
    }

    fn program() -> ProgramRef {
        ProgramRef::of(&json!({"statements": []}), Some("find \"/srv\"".to_owned()))
    }

    #[test]
    fn short_results_have_no_cursor() {
        let store = CursorStore::new(None, TTL);
        let report = store.first_page(records(3), 50, &program(), at(0));
        assert_eq!(report.records.unwrap().rows.len(), 3);
        assert!(report.next_cursor.is_none());
    }

    #[test]
    fn records_are_paged_in_order() {
        let store = CursorStore::new(None, TTL);
        let first = store.first_page(records(5), 2, &program(), at(0));
        let cursor = first.next_cursor.clone().unwrap();
        assert!(cursor.starts_with(CURSOR_PREFIX));
        assert!(cursor.ends_with("-2"));
        let second = store.page(&cursor, 2, at(1)).unwrap();
        let third = store
            .page(second.next_cursor.as_deref().unwrap(), 2, at(1))
            .unwrap();
        let names: Vec<String> = [first, second, third]
            .iter()
            .flat_map(|page| page.records.clone().unwrap().rows)
            .map(|row| serde_json::to_value(row).unwrap()["fields"]["name"]["v"].to_string())
            .collect();
        assert_eq!(names.len(), 5);
        assert_eq!(names[4], "\"f4\"");
    }

    #[test]
    fn listed_targets_follow_the_page() {
        let store = CursorStore::new(None, TTL);
        let first = store.first_page(changes(3), 2, &program(), at(0));
        assert_eq!(first.targets.unwrap().listed, Some(2));
        first.validate().unwrap();
        let second = store
            .page(first.next_cursor.as_deref().unwrap(), 10, at(0))
            .unwrap();
        assert_eq!(second.changes.len(), 1);
        second.validate().unwrap();
    }

    #[test]
    fn a_page_can_be_read_again() {
        let store = CursorStore::new(None, TTL);
        let first = store.first_page(records(4), 2, &program(), at(0));
        let cursor = first.next_cursor.unwrap();
        assert_eq!(
            store.page(&cursor, 2, at(1)).unwrap(),
            store.page(&cursor, 2, at(2)).unwrap()
        );
    }

    #[test]
    fn expired_cursor_offers_the_reading_program() {
        let store = CursorStore::new(None, TTL);
        let cursor = store
            .first_page(records(4), 2, &program(), at(0))
            .next_cursor
            .unwrap();
        let error = store.page(&cursor, 2, at(3600)).unwrap_err();
        assert_eq!(error.code, ErrorCode::Arg);
        assert_eq!(error.fix, vec!["find \"/srv\"".to_owned()]);
    }

    #[test]
    fn expired_cursor_of_a_changing_program_has_no_fix() {
        let store = CursorStore::new(None, TTL);
        let cursor = store
            .first_page(changes(4), 2, &program(), at(0))
            .next_cursor
            .unwrap();
        assert!(store.page(&cursor, 2, at(3600)).unwrap_err().fix.is_empty());
    }

    #[test]
    fn malformed_and_foreign_cursors_are_rejected() {
        let store = CursorStore::new(None, TTL);
        let cursor = store
            .first_page(records(4), 2, &program(), at(0))
            .next_cursor
            .unwrap();
        for bad in [
            "c_91fe",
            "x_00000000-0000000000000000-1",
            "c_0000000g-0000000000000000-1",
        ] {
            let error = store.page(bad, 2, at(0)).unwrap_err();
            assert!(error.message.starts_with("malformed"), "{bad}");
        }
        let foreign = cursor.replacen(&cursor[2..10], "ffffffff", 1);
        let error = store.page(&foreign, 2, at(0)).unwrap_err();
        assert!(error.message.contains("unknown or expired"));
        let beyond = format!("{}-99", cursor.rsplit_once('-').unwrap().0);
        assert!(store.page(&beyond, 2, at(0)).is_err());
    }

    #[test]
    fn pages_survive_a_new_process_and_expire() {
        let temp = tempfile::tempdir().unwrap();
        let first = CursorStore::new(Some(temp.path().to_path_buf()), TTL);
        let cursor = first
            .first_page(records(3), 1, &program(), at(0))
            .next_cursor
            .unwrap();
        let second = CursorStore::new(Some(temp.path().to_path_buf()), TTL);
        assert_eq!(
            second
                .page(&cursor, 1, at(5))
                .unwrap()
                .records
                .unwrap()
                .rows
                .len(),
            1
        );
        assert_eq!(second.sweep(at(10)), 0);
        assert_eq!(second.sweep(at(3600)), 2);
        assert!(
            CursorStore::new(Some(temp.path().to_path_buf()), TTL)
                .page(&cursor, 1, at(5))
                .is_err()
        );
    }

    #[test]
    fn diff_follows_the_changes_of_the_page() {
        let mut report = changes(2);
        report.diff = serde_json::from_value(json!([
            {"path": "/srv/f0.log", "lines": []},
            {"path": "/srv/f1.log", "lines": []}
        ]))
        .unwrap_or_default();
        let store = CursorStore::new(None, TTL);
        let first = store.first_page(report.clone(), 1, &program(), at(0));
        assert!(first.diff.len() <= 1);
        assert!(
            first
                .diff
                .iter()
                .all(|hunk| hunk.path.to_string() == "/srv/f0.log")
        );
    }
}
