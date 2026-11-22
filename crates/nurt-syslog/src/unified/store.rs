// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Log store over the macOS unified log.
//!
//! [`UnifiedStore`] asks its backend for the entries before `until`, newest
//! first, with a predicate on subsystem, process and message type. It stops
//! at the first entry older than `since` and applies the final filter and
//! the size limits of the crate.
//!
//! A cursor continues from the time of its entry. Entries newer than the
//! cursor are skipped, and so are entries of the same time up to and
//! including the entry of the cursor. When no entry of that time matches the
//! cursor, reading continues with the first older entry.

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::value::Instant;
use serde_json::json;

use crate::cursor::Cursor;
use crate::event::LogEvent;
use crate::filter::EventFilter;
use crate::limits::LogLimits;
use crate::query::{LogNotice, LogQuery, LogStore, LogStream, NameMatch, NoticeCode, Notices};

use super::backend::{BackendError, BackendItem, EntryRequest, Requirement, UnifiedBackend};
use super::entry::UnifiedEntry;
use super::predicate::Predicate;

/// Log store of the unified log.
#[derive(Debug)]
pub struct UnifiedStore<B> {
    backend: B,
    limits: LogLimits,
}

impl<B: UnifiedBackend> UnifiedStore<B> {
    /// Store over a backend with the given size limits.
    #[must_use]
    pub const fn new(backend: B, limits: LogLimits) -> Self {
        Self { backend, limits }
    }

    /// Backend of the store.
    #[must_use]
    pub const fn backend(&self) -> &B {
        &self.backend
    }
}

impl<B: UnifiedBackend> LogStore for UnifiedStore<B> {
    /// Reads the entries of the query, newest first.
    ///
    /// # Errors
    ///
    /// - `E_PRIVILEGE` with the detail `required` (`root` or
    ///   `full_disk_access`) when the account may not read the log store.
    /// - `E_NOT_FOUND` when the log store cannot be opened.
    /// - `E_STAGE_FAILED` when reading the log store fails.
    /// - `E_INVALID_VALUE` for a cursor of another log store.
    fn query<'a>(&'a self, query: &'a LogQuery) -> Result<LogStream<'a>, Error> {
        let resume = match &query.cursor {
            None => None,
            Some(cursor @ Cursor::Unified { nanos, .. }) => {
                Some((cursor.clone(), Instant::from_unix_nanos(*nanos)))
            }
            Some(other) => return Err(foreign_cursor(other)),
        };
        let start = resume
            .as_ref()
            .and_then(|(_, time)| time.as_unix_nanos().checked_add(1))
            .map(Instant::from_unix_nanos);
        let until = match (query.until, start) {
            (Some(until), Some(start)) => Some(until.min(start)),
            (until, start) => until.or(start),
        };
        let request = EntryRequest {
            predicate: Predicate::new(
                query.unit.as_ref().and_then(NameMatch::literal),
                query.source.as_ref().and_then(NameMatch::literal),
                query.level,
            ),
            until,
        };
        let items = self
            .backend
            .items(&request)
            .map_err(|error| backend_error(&error))?;
        let notices = Notices::new();
        let entries = Entries {
            items,
            since: query.since,
            until,
            resume,
            notices: notices.clone(),
            done: false,
        };
        let filter = EventFilter::new(query);
        let limits = &self.limits;
        let events = entries.filter_map(move |item| match item {
            Ok(mut event) if filter.matches(&event) => {
                limits.apply(&mut event);
                Some(Ok(event))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        });
        Ok(LogStream::new(events, notices, query.limit))
    }
}

/// Log events of the backend items, newest first.
struct Entries<I> {
    items: I,
    since: Option<Instant>,
    until: Option<Instant>,
    resume: Option<(Cursor, Instant)>,
    notices: Notices,
    done: bool,
}

impl<I> Entries<I> {
    /// Whether an entry is returned. Clears the cursor once the entry of the
    /// cursor or an older entry is reached.
    fn admits(&mut self, entry: &UnifiedEntry) -> bool {
        if self.until.is_some_and(|until| entry.time >= until) {
            return false;
        }
        let Some((cursor, time)) = &self.resume else {
            return true;
        };
        if entry.time > *time {
            return false;
        }
        if entry.time == *time {
            if entry.is_at(cursor) {
                self.resume = None;
            }
            return false;
        }
        self.resume = None;
        true
    }
}

impl<I> Iterator for Entries<I>
where
    I: Iterator<Item = Result<BackendItem, BackendError>>,
{
    type Item = Result<LogEvent, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        while !self.done {
            match self.items.next() {
                None => self.done = true,
                Some(Err(error)) => {
                    self.done = true;
                    return Some(Err(backend_error(&error)));
                }
                Some(Ok(BackendItem::Lost(count))) => {
                    let mut notice = LogNotice::new(NoticeCode::EventsLost);
                    notice.count = Some(count);
                    self.notices.add(notice);
                }
                Some(Ok(BackendItem::Other)) => {}
                Some(Ok(BackendItem::Log(entry))) => {
                    if self.since.is_some_and(|since| entry.time < since) {
                        self.done = true;
                    } else if self.admits(&entry) {
                        return Some(Ok(entry.to_event()));
                    }
                }
            }
        }
        None
    }
}

fn foreign_cursor(cursor: &Cursor) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!(
            "cursor `{cursor}` belongs to another log store; pass a cursor returned by `logs` on this system"
        ),
    )
    .with_found(cursor.to_string())
}

fn backend_error(error: &BackendError) -> Error {
    match error {
        BackendError::AccessDenied(requirement) => privilege_error(*requirement),
        BackendError::Unavailable(reason) => Error::new(
            ErrorCode::NotFound,
            format!("the unified log store is not available: {reason}"),
        ),
        BackendError::Failed(reason) => Error::new(
            ErrorCode::StageFailed,
            format!("reading the unified log failed: {reason}"),
        ),
    }
}

fn privilege_error(requirement: Requirement) -> Error {
    let message = match requirement {
        Requirement::Root => "reading the unified log requires root; run the command with sudo",
        Requirement::FullDiskAccess => {
            "reading the unified log requires Full Disk Access; grant it to the terminal application in System Settings, Privacy & Security"
        }
    };
    Error::new(ErrorCode::Privilege, message).with_detail("required", json!(requirement.as_str()))
}
