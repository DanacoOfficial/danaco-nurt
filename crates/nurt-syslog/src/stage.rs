// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `logs` command.
//!
//! The stage reads the log of the operating system through a [`LogStore`]:
//! the systemd journal on Linux, the event log on Windows and the unified log
//! on macOS. It produces one `log_event` record per event, newest first.
//!
//! A [`LogStream`](crate::query::LogStream) borrows its query, so the stage
//! reads the store in pages. Each page is one query of the store that
//! continues after the cursor of the last event of the previous page, with
//! the same filters. The first page holds at most [`FIRST_PAGE`] events, and
//! no more than the `limit` or the number of records the rest of the
//! pipeline uses; every further page is twice as large, up to [`MAX_PAGE`].
//! Memory is therefore bounded by one page of records, independent of the
//! size of the log, and a query within [`FIRST_PAGE`] events reads the store
//! once.
//!
//! The notices of the store are reported when the output ends or is
//! dropped. `permission_denied_skipped` becomes an entry of `notable`; the
//! other codes are not defined by the report and are emitted as diagnostic
//! events of the execution, with the code at the start of the message.

#![forbid(unsafe_code)]

use std::collections::VecDeque;
use std::fmt::{self, Write as _};
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::report::Severity;
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, Event, ExecCtx, InputKinds, LogEvent as DiagnosticEvent,
    LogLevel as DiagnosticLevel, ReadStage, RecordStream, StageDescriptor, StageImpl, StageInput,
    StageOutput, StaticInfo,
};
use nurt_contracts::value::Record;

use crate::cursor::Cursor;
use crate::limits::LogLimits;
use crate::query::{LogNotice, LogQuery, LogStore, NoticeCode};

/// Name of the command in the catalog.
pub const COMMAND: &str = "logs";
/// Largest number of events read by the first query of the store.
pub const FIRST_PAGE: u32 = 1_000;
/// Largest number of events read by one query of the store.
pub const MAX_PAGE: u32 = 8_192;

/// Log store shared by all executions of a stage.
pub type SharedStore = Arc<dyn LogStore + Send + Sync>;

/// Implementation of `logs`.
///
/// ```
/// use std::sync::Arc;
///
/// use nurt_contracts::stage::{InputKinds, StageImpl};
/// use nurt_syslog::journald::JournaldStore;
/// use nurt_syslog::limits::LogLimits;
/// use nurt_syslog::stage::LogsStage;
///
/// let store = JournaldStore::from_files(Vec::new(), LogLimits::default());
/// let stage = LogsStage::with_store(Arc::new(store), LogLimits::default());
/// let descriptor = stage.descriptor();
/// assert_eq!(descriptor.command, "logs");
/// assert!(descriptor.source);
/// assert_eq!(descriptor.input, InputKinds::None);
/// ```
pub struct LogsStage {
    store: Option<SharedStore>,
    limits: LogLimits,
}

impl LogsStage {
    /// Stage over the log of the running system with the default limits.
    ///
    /// The store is the systemd journal on Linux, the event log on Windows
    /// and the unified log on macOS. On other systems the stage has no store
    /// and its execution fails with `E_OS_ONLY`.
    #[must_use]
    pub fn new() -> Self {
        let limits = LogLimits::default();
        Self {
            store: local_store(&limits),
            limits,
        }
    }

    /// Stage over `store`; `limits` bound the parameter `limit`.
    #[must_use]
    pub fn with_store(store: SharedStore, limits: LogLimits) -> Self {
        Self {
            store: Some(store),
            limits,
        }
    }
}

impl Default for LogsStage {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for LogsStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LogsStage")
            .field("store", &self.store.is_some())
            .field("limits", &self.limits)
            .finish()
    }
}

#[cfg(target_os = "linux")]
fn local_store(limits: &LogLimits) -> Option<SharedStore> {
    Some(Arc::new(crate::journald::JournaldStore::local(
        limits.clone(),
    )))
}

#[cfg(windows)]
fn local_store(limits: &LogLimits) -> Option<SharedStore> {
    Some(Arc::new(crate::eventlog::store::EventLogStore::new(
        crate::eventlog::WevtSource::new(),
        limits.clone(),
    )))
}

#[cfg(target_os = "macos")]
fn local_store(limits: &LogLimits) -> Option<SharedStore> {
    Some(Arc::new(crate::unified::store::UnifiedStore::new(
        crate::unified::OsLogBackend::local(),
        limits.clone(),
    )))
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
fn local_store(_limits: &LogLimits) -> Option<SharedStore> {
    None
}

fn unsupported_system() -> Error {
    Error::new(
        ErrorCode::OsOnly,
        "`logs` reads the system log on Linux, Windows and macOS only",
    )
}

impl StageImpl for LogsStage {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        LogQuery::from_args(args, &self.limits)?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for LogsStage {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let store = self.store.as_deref().ok_or_else(unsupported_system)?;
        let query = LogQuery::from_args(args, &self.limits)?;
        let mut pages = Pages::new(store, query, ctx);
        pages.fetch()?;
        if let Some(error) = pages.leading_error() {
            return Err(error);
        }
        Ok(StageOutput::records(RecordStream::new(pages).checked(ctx)))
    }
}

/// Records of one execution, read from the store page by page.
struct Pages<'a> {
    store: &'a (dyn LogStore + Send + Sync),
    ctx: &'a ExecCtx,
    query: LogQuery,
    remaining: u32,
    page: u32,
    resume: Option<Cursor>,
    buffer: VecDeque<Record>,
    pending: Option<Error>,
    notices: Vec<LogNotice>,
    pages_read: u32,
    more: Option<Cursor>,
    finished: bool,
    reported: bool,
}

impl<'a> Pages<'a> {
    fn new(store: &'a (dyn LogStore + Send + Sync), query: LogQuery, ctx: &'a ExecCtx) -> Self {
        let limit = query.limit;
        let used = ctx
            .demand()
            .limit
            .map_or(limit, |used| u32::try_from(used).unwrap_or(u32::MAX).max(1));
        Self {
            store,
            ctx,
            remaining: limit,
            page: limit.min(used).min(FIRST_PAGE),
            resume: query.cursor.clone(),
            query,
            buffer: VecDeque::new(),
            pending: None,
            notices: Vec::new(),
            pages_read: 0,
            more: None,
            finished: false,
            reported: false,
        }
    }

    /// Reads the next page into the buffer.
    ///
    /// An error of the store before its stream starts is returned; an error
    /// of the stream is kept and follows the events read before it.
    fn fetch(&mut self) -> Result<()> {
        let store = self.store;
        let mut query = self.query.clone();
        query.limit = self.page.min(self.remaining);
        query.cursor = self.resume.take();
        let mut stream = store.query(&query)?;
        let mut read: u32 = 0;
        let error = loop {
            if let Err(error) = self.ctx.checkpoint() {
                break Some(error);
            }
            match stream.next() {
                Some(Ok(event)) => {
                    read = read.saturating_add(1);
                    self.buffer.push_back(event.into_record());
                }
                Some(Err(error)) => break Some(error),
                None => break None,
            }
        };
        let next = stream.next_cursor().cloned();
        self.merge(stream.notices());
        self.pages_read = self.pages_read.saturating_add(1);
        self.remaining = self.remaining.saturating_sub(read);
        match (error, next) {
            (Some(error), _) => {
                self.pending = Some(error);
                self.finished = true;
            }
            (None, Some(cursor)) if self.remaining > 0 => {
                self.resume = Some(cursor);
                self.page = self.page.saturating_mul(2).min(MAX_PAGE);
            }
            (None, next) => {
                self.more = next;
                self.finished = true;
            }
        }
        Ok(())
    }

    /// The kept error when no event precedes it.
    fn leading_error(&mut self) -> Option<Error> {
        if self.buffer.is_empty() {
            self.pending.take()
        } else {
            None
        }
    }

    /// Adds the notices of one page.
    ///
    /// Every page queries the whole store again, so conditions of files and
    /// permissions repeat and are kept once; counts of unformatted messages
    /// concern the events of each page and are added. The limit of a page is
    /// not the limit of the execution, and names and the cursor given by the
    /// program are judged by the first page only.
    fn merge(&mut self, notices: Vec<LogNotice>) {
        let first = self.pages_read == 0;
        for notice in notices {
            let known = self
                .notices
                .iter_mut()
                .find(|known| same_subject(known, &notice));
            match (notice.code, known) {
                (NoticeCode::LimitReached, _) => {}
                (NoticeCode::UnknownName | NoticeCode::CursorApproximate, _) if !first => {}
                (NoticeCode::Unformatted, Some(known)) => {
                    known.count = match (known.count, notice.count) {
                        (Some(left), Some(right)) => Some(left.saturating_add(right)),
                        (left, right) => left.or(right),
                    };
                }
                (_, Some(_)) => {}
                (_, None) => self.notices.push(notice),
            }
        }
    }

    /// Reports the notices once.
    fn report(&mut self) {
        if self.reported {
            return;
        }
        self.reported = true;
        for notice in &self.notices {
            match notice.to_notable() {
                Some(entry) => self.ctx.note(entry),
                None => diagnostic(self.ctx, notice.code.severity(), describe(notice)),
            }
        }
        if let Some(cursor) = &self.more {
            diagnostic(
                self.ctx,
                NoticeCode::LimitReached.severity(),
                format!(
                    "{}: more than {} events match; older events follow cursor `{cursor}`",
                    NoticeCode::LimitReached.as_str(),
                    self.query.limit
                ),
            );
        }
    }
}

impl Iterator for Pages<'_> {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(record) = self.buffer.pop_front() {
                return Some(Ok(record));
            }
            if let Some(error) = self.pending.take() {
                self.report();
                return Some(Err(error));
            }
            if self.finished {
                self.report();
                return None;
            }
            if let Err(error) = self.fetch() {
                self.pending = Some(error);
                self.finished = true;
            }
        }
    }
}

impl Drop for Pages<'_> {
    fn drop(&mut self) {
        self.report();
    }
}

fn same_subject(left: &LogNotice, right: &LogNotice) -> bool {
    left.code == right.code && left.path == right.path && left.detail == right.detail
}

fn describe(notice: &LogNotice) -> String {
    let mut text = notice.code.as_str().to_owned();
    if let Some(path) = &notice.path {
        let _ = write!(text, " `{path}`");
    }
    if let Some(detail) = &notice.detail {
        let _ = write!(text, ": {detail}");
    }
    if let Some(count) = notice.count {
        let _ = write!(text, " ({count})");
    }
    text
}

fn diagnostic(ctx: &ExecCtx, severity: Severity, message: String) {
    let level = match severity {
        Severity::Warning | Severity::Alert => DiagnosticLevel::Warn,
        _ => DiagnosticLevel::Info,
    };
    ctx.emit(Event::Log(DiagnosticEvent {
        level,
        message,
        source: Some(COMMAND.to_owned()),
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(code: NoticeCode, path: Option<&str>, detail: Option<&str>, count: u64) -> LogNotice {
        let mut notice = LogNotice::new(code);
        notice.path = path.map(str::to_owned);
        notice.detail = detail.map(str::to_owned);
        notice.count = Some(count);
        notice
    }

    #[test]
    fn describes_notice_with_all_parts() {
        let text = describe(&notice(
            NoticeCode::FileSkipped,
            Some("/j/system.journal"),
            Some("corrupt"),
            1,
        ));
        assert_eq!(text, "log_file_skipped `/j/system.journal`: corrupt (1)");
        assert_eq!(
            describe(&LogNotice::new(NoticeCode::UnknownName)),
            "log_unknown_name"
        );
    }

    #[test]
    fn subjects_compare_code_path_and_detail() {
        let left = notice(NoticeCode::FileSkipped, Some("/a"), Some("io"), 1);
        let same = notice(NoticeCode::FileSkipped, Some("/a"), Some("io"), 5);
        let other = notice(NoticeCode::FileSkipped, Some("/b"), Some("io"), 1);
        assert!(same_subject(&left, &same));
        assert!(!same_subject(&left, &other));
    }

    #[test]
    fn stage_without_store_fails_with_e_os_only() {
        assert_eq!(unsupported_system().code(), ErrorCode::OsOnly);
        let stage = LogsStage {
            store: None,
            limits: LogLimits::default(),
        };
        assert!(format!("{stage:?}").contains("store: false"));
    }
}
