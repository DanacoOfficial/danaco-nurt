// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syslog::query
// Responsibility:   Log queries.
// Dependencies:     globset, nurt_contracts, regex
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Log queries.
//!
//! A [`LogQuery`] holds the validated parameters of `logs`. A [`LogStore`]
//! answers it with a [`LogStream`]: events one at a time, newest first,
//! together with the unusual conditions met while reading ([`LogNotice`]).
//! Every parameter has the meaning of a later `filter` on the field of the
//! same name; stores use their indexes only to find candidates faster.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use globset::{GlobBuilder, GlobMatcher};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::record::LogLevel;
use nurt_contracts::report::{Notable, NotableEntry, Severity};
use nurt_contracts::stage::{BoundArgs, CancelToken};
use nurt_contracts::value::{Instant, Regex as RegexValue, Value};
use regex::{Regex, RegexBuilder};

use crate::cursor::Cursor;
use crate::event::{LogEvent, max_priority};
use crate::limits::{DEFAULT_LIMIT, LogLimits};

const GLOB_CHARACTERS: [char; 3] = ['*', '?', '['];

/// Name given to `source` or `unit`: a literal name or a glob pattern.
///
/// A name with `*`, `?` or `[` is a glob; any other name matches itself
/// only. Matching is case-sensitive unless the name was parsed with
/// [`NameMatch::parse_ignore_case`].
///
/// ```
/// use nurt_syslog::query::NameMatch;
///
/// let exact = NameMatch::parse("sshd").unwrap();
/// assert!(exact.matches("sshd"));
/// assert!(!exact.matches("sshd2"));
///
/// let glob = NameMatch::parse("ssh*").unwrap();
/// assert!(glob.matches("sshd"));
/// assert_eq!(glob.literal(), None);
/// ```
#[derive(Debug, Clone)]
pub struct NameMatch {
    text: String,
    ignore_case: bool,
    glob: Option<GlobMatcher>,
}

impl NameMatch {
    /// Parses a case-sensitive name or pattern.
    ///
    /// # Errors
    ///
    /// Returns `E_PATTERN_SYNTAX` when a glob pattern is malformed and
    /// `E_INVALID_VALUE` when the name is empty.
    pub fn parse(text: &str) -> Result<Self, Error> {
        Self::build(text, false)
    }

    /// Parses a name or pattern that ignores the case of letters.
    ///
    /// # Errors
    ///
    /// Same as [`NameMatch::parse`].
    pub fn parse_ignore_case(text: &str) -> Result<Self, Error> {
        Self::build(text, true)
    }

    fn build(text: &str, ignore_case: bool) -> Result<Self, Error> {
        if text.is_empty() {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                "empty name; give a name or a pattern such as `ssh*`",
            ));
        }
        let glob = if text.contains(GLOB_CHARACTERS) {
            let glob = GlobBuilder::new(text)
                .case_insensitive(ignore_case)
                .literal_separator(false)
                .backslash_escape(true)
                .build()
                .map_err(|error| {
                    Error::new(
                        ErrorCode::PatternSyntax,
                        format!("invalid name pattern `{text}`: {}", error.kind()),
                    )
                    .with_found(text)
                })?;
            Some(glob.compile_matcher())
        } else {
            None
        };
        Ok(Self {
            text: text.to_owned(),
            ignore_case,
            glob,
        })
    }

    /// Text as given.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The name when it is not a pattern.
    #[must_use]
    pub fn literal(&self) -> Option<&str> {
        self.glob.is_none().then_some(self.text.as_str())
    }

    /// Whether `name` matches.
    #[must_use]
    pub fn matches(&self, name: &str) -> bool {
        match &self.glob {
            Some(glob) => glob.is_match(name),
            None if self.ignore_case => self.text.eq_ignore_ascii_case(name),
            None => self.text == name,
        }
    }
}

impl fmt::Display for NameMatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// Validated parameters of `logs`.
#[derive(Debug, Clone)]
pub struct LogQuery {
    /// Name or pattern of the source.
    pub source: Option<NameMatch>,
    /// Name or pattern of the unit, channel or subsystem.
    pub unit: Option<NameMatch>,
    /// Lowest level returned.
    pub level: LogLevel,
    /// Earliest time, inclusive.
    pub since: Option<Instant>,
    /// Latest time, exclusive.
    pub until: Option<Instant>,
    /// Pattern that the message must contain.
    pub message: Option<Regex>,
    /// Number of events to return, newest first.
    pub limit: u32,
    /// Position after which to continue: only older events are returned.
    pub cursor: Option<Cursor>,
}

impl Default for LogQuery {
    fn default() -> Self {
        Self {
            source: None,
            unit: None,
            level: LogLevel::Info,
            since: None,
            until: None,
            message: None,
            limit: DEFAULT_LIMIT,
            cursor: None,
        }
    }
}

impl LogQuery {
    /// Builds a query from the bound arguments of `logs`.
    ///
    /// ```
    /// use nurt_contracts::stage::{ArgValue, BoundArgs};
    /// use nurt_contracts::value::Value;
    /// use nurt_syslog::limits::LogLimits;
    /// use nurt_syslog::query::LogQuery;
    ///
    /// let args = BoundArgs::new("logs")
    ///     .with("source", ArgValue::Value(Value::Text("sshd".into())), false)
    ///     .with("limit", ArgValue::Value(Value::Int(10)), false);
    /// let query = LogQuery::from_args(&args, &LogLimits::default()).unwrap();
    /// assert_eq!(query.limit, 10);
    /// assert_eq!(query.source.unwrap().as_str(), "sshd");
    /// ```
    ///
    /// # Errors
    ///
    /// Returns `E_TYPE` for a value of the wrong type, `E_PATTERN_SYNTAX`
    /// for a malformed name pattern, `E_REGEX` for a malformed `message`,
    /// `E_LIMIT` for a `limit` out of range and `E_INVALID_VALUE` for an
    /// unknown level, a malformed cursor or `since` not before `until`.
    pub fn from_args(args: &BoundArgs, limits: &LogLimits) -> Result<Self, Error> {
        let query = Self {
            source: args.text("source")?.map(NameMatch::parse).transpose()?,
            unit: args.text("unit")?.map(NameMatch::parse).transpose()?,
            level: level_arg(args)?,
            since: instant_arg(args, "since")?,
            until: instant_arg(args, "until")?,
            message: regex_arg(args)?,
            limit: match args.int("limit")? {
                Some(limit) => limits.check_limit(limit)?,
                None => DEFAULT_LIMIT,
            },
            cursor: args.text("cursor")?.map(Cursor::parse_param).transpose()?,
        };
        query.validate()?;
        Ok(query)
    }

    /// Checks the relation between the parameters.
    ///
    /// # Errors
    ///
    /// Returns `E_INVALID_VALUE` when `since` is not before `until`.
    pub fn validate(&self) -> Result<(), Error> {
        if let (Some(since), Some(until)) = (self.since, self.until)
            && since >= until
        {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                "`since` is not before `until`; give an earlier `since` or a later `until`",
            ));
        }
        Ok(())
    }

    /// Highest priority value admitted by [`LogQuery::level`].
    #[must_use]
    pub const fn max_priority(&self) -> u8 {
        max_priority(self.level)
    }
}

fn level_arg(args: &BoundArgs) -> Result<LogLevel, Error> {
    match args.value("level") {
        None => Ok(LogLevel::Info),
        Some(Value::Enum(text) | Value::Text(text)) => text.parse().map_err(|_| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("unknown level `{text}`; use critical, error, warning, info or debug"),
            )
            .with_found(text.as_str())
        }),
        Some(_) => Err(type_error("level", "enum")),
    }
}

fn instant_arg(args: &BoundArgs, name: &str) -> Result<Option<Instant>, Error> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Instant(instant)) => Ok(Some(*instant)),
        Some(_) => Err(type_error(name, "instant")),
    }
}

fn regex_arg(args: &BoundArgs) -> Result<Option<Regex>, Error> {
    match args.value("message") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Regex(regex)) => compile_regex(regex).map(Some),
        Some(Value::Text(text)) => compile_regex(&RegexValue {
            pattern: text.clone(),
            flags: Default::default(),
        })
        .map(Some),
        Some(_) => Err(type_error("message", "regex")),
    }
}

/// Compiles a regular expression value of the language.
///
/// # Errors
///
/// Returns `E_REGEX` when the pattern is malformed or too large.
pub fn compile_regex(regex: &RegexValue) -> Result<Regex, Error> {
    RegexBuilder::new(&regex.pattern)
        .case_insensitive(regex.flags.case_insensitive)
        .multi_line(regex.flags.multi_line)
        .dot_matches_new_line(regex.flags.dot_matches_new_line)
        .ignore_whitespace(regex.flags.extended)
        .build()
        .map_err(|error| {
            Error::new(
                ErrorCode::Regex,
                format!("invalid pattern for `message`: {error}"),
            )
            .with_found(regex.pattern.as_str())
        })
}

fn type_error(name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`logs` expects a value of type {expected} for `{name}`"),
    )
}

/// Code of an unusual condition met while reading a log store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoticeCode {
    /// A log file was skipped or read only in part; `detail` is one of
    /// `corrupt`, `truncated`, `unsupported_format` or `io`.
    FileSkipped,
    /// Files or channels could not be read for lack of permission.
    PermissionDeniedSkipped,
    /// Messages could not be formatted.
    Unformatted,
    /// The limit was reached and older events exist.
    LimitReached,
    /// A name given to `source` or `unit` does not occur in the store;
    /// `detail` lists the closest existing names.
    UnknownName,
    /// The system reported lost events.
    EventsLost,
    /// The cursor no longer points into the store; paging continued from
    /// its time.
    CursorApproximate,
}

impl NoticeCode {
    /// Code as written in the report.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FileSkipped => "log_file_skipped",
            Self::PermissionDeniedSkipped => "permission_denied_skipped",
            Self::Unformatted => "log_unformatted",
            Self::LimitReached => "log_limit_reached",
            Self::UnknownName => "log_unknown_name",
            Self::EventsLost => "log_events_lost",
            Self::CursorApproximate => "log_cursor_approximate",
        }
    }

    /// Importance of the condition.
    #[must_use]
    pub const fn severity(self) -> Severity {
        match self {
            Self::FileSkipped | Self::PermissionDeniedSkipped => Severity::Warning,
            Self::LimitReached => Severity::Info,
            Self::Unformatted | Self::UnknownName | Self::EventsLost | Self::CursorApproximate => {
                Severity::Notice
            }
        }
    }
}

/// Unusual condition met while reading a log store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogNotice {
    /// Code of the condition.
    pub code: NoticeCode,
    /// File or channel concerned.
    pub path: Option<String>,
    /// Number of files, channels or events concerned.
    pub count: Option<u64>,
    /// Class of the condition or candidate names.
    pub detail: Option<String>,
}

impl LogNotice {
    /// Notice with a code only.
    #[must_use]
    pub const fn new(code: NoticeCode) -> Self {
        Self {
            code,
            path: None,
            count: None,
            detail: None,
        }
    }

    /// Entry of the effect report, for codes that the report defines.
    #[must_use]
    pub fn to_notable(&self) -> Option<NotableEntry> {
        let code = match self.code {
            NoticeCode::PermissionDeniedSkipped => Notable::PermissionDeniedSkipped,
            _ => return None,
        };
        let mut entry = NotableEntry::new(code);
        entry.path.clone_from(&self.path);
        entry.count = self.count;
        entry.detail.clone_from(&self.detail);
        Some(entry)
    }
}

/// Shared list of notices filled while a stream is read.
///
/// Notices with the same code, path and detail are merged and their counts
/// added.
#[derive(Debug, Clone, Default)]
pub struct Notices {
    entries: Arc<Mutex<Vec<LogNotice>>>,
}

impl Notices {
    /// Empty list.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a notice or increases the count of an equal one.
    pub fn add(&self, notice: LogNotice) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let existing = entries.iter_mut().find(|entry| {
            entry.code == notice.code && entry.path == notice.path && entry.detail == notice.detail
        });
        match existing {
            Some(entry) => {
                entry.count = match (entry.count, notice.count) {
                    (Some(left), Some(right)) => Some(left.saturating_add(right)),
                    (left, right) => left.or(right),
                };
            }
            None => entries.push(notice),
        }
    }

    /// Copy of the notices collected so far.
    #[must_use]
    pub fn snapshot(&self) -> Vec<LogNotice> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Store of system log events.
pub trait LogStore {
    /// Starts a query.
    ///
    /// The stream returns the newest events first and stops after
    /// [`LogQuery::limit`] events.
    ///
    /// # Errors
    ///
    /// Returns `E_NOT_FOUND` when the store does not exist on this system,
    /// `E_PERMISSION` or `E_PRIVILEGE` when nothing of it can be read and
    /// `E_FORMAT` when none of its files has a readable format.
    fn query<'a>(&'a self, query: &'a LogQuery) -> Result<LogStream<'a>, Error>;
}

type EventIter<'a> = Box<dyn Iterator<Item = Result<LogEvent, Error>> + Send + 'a>;

/// Events of one query, newest first, with the notices of the query.
///
/// The stream stops after the limit. When older events remain it adds the
/// notice `log_limit_reached` and [`LogStream::next_cursor`] gives the
/// position from which the next page continues. Cancellation is checked
/// before every event.
pub struct LogStream<'a> {
    events: EventIter<'a>,
    notices: Notices,
    limit: u32,
    emitted: u32,
    cancel: Option<CancelToken>,
    last_cursor: Option<Cursor>,
    next_cursor: Option<Cursor>,
    finished: bool,
}

impl<'a> LogStream<'a> {
    /// Stream over `events`, stopped after `limit` events.
    ///
    /// ```
    /// use nurt_contracts::value::Instant;
    /// use nurt_syslog::cursor::Cursor;
    /// use nurt_syslog::event::{LogEvent, StoreKind};
    /// use nurt_syslog::query::{LogStream, Notices};
    ///
    /// let events = (0..5_u64).rev().map(|n| {
    ///     let cursor = Cursor::EventLog { channel: "System".to_owned(), record_id: n };
    ///     Ok(LogEvent::new(Instant::from_unix_nanos(0), "app", 6, "m", StoreKind::EventLog, cursor))
    /// });
    /// let mut stream = LogStream::new(events, Notices::new(), 2);
    /// assert_eq!(stream.by_ref().count(), 2);
    /// assert_eq!(stream.next_cursor().unwrap().to_string(), "v1:w:System:3");
    /// ```
    pub fn new<I>(events: I, notices: Notices, limit: u32) -> Self
    where
        I: Iterator<Item = Result<LogEvent, Error>> + Send + 'a,
    {
        Self {
            events: Box::new(events),
            notices,
            limit,
            emitted: 0,
            cancel: None,
            last_cursor: None,
            next_cursor: None,
            finished: false,
        }
    }

    /// Checks `token` before every event.
    #[must_use]
    pub fn with_cancel(mut self, token: CancelToken) -> Self {
        self.cancel = Some(token);
        self
    }

    /// Notices collected so far.
    #[must_use]
    pub fn notices(&self) -> Vec<LogNotice> {
        self.notices.snapshot()
    }

    /// Cursor of the last returned event when the limit stopped the stream.
    #[must_use]
    pub fn next_cursor(&self) -> Option<&Cursor> {
        self.next_cursor.as_ref()
    }

    fn finish_at_limit(&mut self) {
        self.finished = true;
        if let Some(Ok(_)) = self.events.next() {
            self.next_cursor = self.last_cursor.take();
            let mut notice = LogNotice::new(NoticeCode::LimitReached);
            notice.count = Some(u64::from(self.limit));
            self.notices.add(notice);
        }
    }
}

impl Iterator for LogStream<'_> {
    type Item = Result<LogEvent, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        if let Some(token) = &self.cancel
            && let Err(error) = token.check()
        {
            self.finished = true;
            return Some(Err(error));
        }
        if self.emitted >= self.limit {
            self.finish_at_limit();
            return None;
        }
        match self.events.next() {
            Some(Ok(event)) => {
                self.emitted += 1;
                self.last_cursor = Some(event.cursor.clone());
                if event.flags.unformatted {
                    // Counted here, not when the store reads the event: the
                    // event peeked at to learn whether older ones remain is
                    // not returned and would be counted again on the next page.
                    let mut notice = LogNotice::new(NoticeCode::Unformatted);
                    notice.count = Some(1);
                    self.notices.add(notice);
                }
                Some(Ok(event))
            }
            Some(Err(error)) => {
                self.finished = true;
                Some(Err(error))
            }
            None => {
                self.finished = true;
                None
            }
        }
    }
}

impl fmt::Debug for LogStream<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LogStream")
            .field("limit", &self.limit)
            .field("emitted", &self.emitted)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::StoreKind;
    use nurt_contracts::stage::{ArgValue, CancelReason};
    use nurt_contracts::value::RegexFlags;

    fn text(value: &str) -> ArgValue {
        ArgValue::Value(Value::Text(value.to_owned()))
    }

    fn event(record_id: u64) -> LogEvent {
        LogEvent::new(
            Instant::from_unix_nanos(i64::try_from(record_id).unwrap()),
            "app",
            6,
            "message",
            StoreKind::EventLog,
            Cursor::EventLog {
                channel: "System".to_owned(),
                record_id,
            },
        )
    }

    #[test]
    fn literal_name_matches_only_itself() {
        let name = NameMatch::parse("cron.service").unwrap();
        assert_eq!(name.literal(), Some("cron.service"));
        assert!(name.matches("cron.service"));
        assert!(!name.matches("Cron.service"));
        assert_eq!(name.to_string(), "cron.service");
    }

    #[test]
    fn glob_name_matches_pattern_case_sensitively() {
        let name = NameMatch::parse("systemd-*.service").unwrap();
        assert!(name.matches("systemd-journald.service"));
        assert!(!name.matches("Systemd-journald.service"));
        let class = NameMatch::parse("ssh[d]").unwrap();
        assert!(class.matches("sshd"));
        let any = NameMatch::parse("a?c").unwrap();
        assert!(any.matches("abc"));
        assert!(!any.matches("ac"));
    }

    #[test]
    fn ignore_case_applies_to_literals_and_globs() {
        let literal = NameMatch::parse_ignore_case("System").unwrap();
        assert!(literal.matches("SYSTEM"));
        let glob = NameMatch::parse_ignore_case("micro*").unwrap();
        assert!(glob.matches("Microsoft-Windows-Test/Operational"));
    }

    #[test]
    fn glob_star_crosses_slashes() {
        let name = NameMatch::parse("Microsoft-*").unwrap();
        assert!(name.matches("Microsoft-Windows-Test/Operational"));
    }

    #[test]
    fn escaped_glob_character_matches_itself() {
        let name = NameMatch::parse(r"a\*").unwrap();
        assert_eq!(name.literal(), None);
        assert!(name.matches("a*"));
        assert!(!name.matches("ab"));
    }

    #[test]
    fn stream_with_limit_zero_returns_nothing_and_reports_the_limit() {
        let mut stream = LogStream::new((0..3).map(|n| Ok(event(n))), Notices::new(), 0);
        assert!(stream.next().is_none());
        assert!(stream.next_cursor().is_none());
        assert_eq!(stream.notices()[0].code, NoticeCode::LimitReached);
        assert_eq!(stream.notices()[0].count, Some(0));
    }

    #[test]
    fn unformatted_events_are_counted_when_returned_and_not_when_peeked_at() {
        let events = (0..5).map(|n| {
            let mut item = event(n);
            item.flags.unformatted = true;
            Ok(item)
        });
        let notices = Notices::new();
        let mut stream = LogStream::new(events, notices, 3);
        assert_eq!(stream.by_ref().count(), 3);
        let counted = stream
            .notices()
            .into_iter()
            .find(|notice| notice.code == NoticeCode::Unformatted)
            .unwrap();
        assert_eq!(counted.count, Some(3));
    }

    #[test]
    fn cancelled_stream_returns_the_error_before_any_event() {
        let token = CancelToken::new();
        token.cancel(CancelReason::Requested);
        let mut stream =
            LogStream::new((0..3).map(|n| Ok(event(n))), Notices::new(), 5).with_cancel(token);
        assert_eq!(
            stream.next().unwrap().unwrap_err().code(),
            ErrorCode::Cancelled
        );
        assert!(stream.next().is_none());
    }

    #[test]
    fn rejects_malformed_glob_and_empty_name() {
        let error = NameMatch::parse("ssh[").unwrap_err();
        assert_eq!(error.code(), ErrorCode::PatternSyntax);
        let error = NameMatch::parse("").unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn builds_query_from_all_arguments() {
        let regex = RegexValue::new("fail(ed)?", "i").unwrap();
        let args = BoundArgs::new("logs")
            .with("source", text("ssh*"), false)
            .with("unit", text("ssh"), false)
            .with(
                "level",
                ArgValue::Value(Value::Enum("warning".into())),
                false,
            )
            .with(
                "since",
                ArgValue::Value(Value::Instant(Instant::from_unix_nanos(1))),
                false,
            )
            .with(
                "until",
                ArgValue::Value(Value::Instant(Instant::from_unix_nanos(9))),
                false,
            )
            .with("message", ArgValue::Value(Value::Regex(regex)), false)
            .with("limit", ArgValue::Value(Value::Int(5)), false)
            .with("cursor", text("v1:w:System:4"), false);
        let query = LogQuery::from_args(&args, &LogLimits::default()).unwrap();
        assert_eq!(query.source.as_ref().unwrap().literal(), None);
        assert_eq!(query.unit.as_ref().unwrap().literal(), Some("ssh"));
        assert_eq!(query.level, LogLevel::Warning);
        assert_eq!(query.max_priority(), 4);
        assert!(query.message.unwrap().is_match("FAILED"));
        assert_eq!(query.limit, 5);
        assert_eq!(query.since, Some(Instant::from_unix_nanos(1)));
        assert_eq!(query.until, Some(Instant::from_unix_nanos(9)));
        assert!(matches!(
            query.cursor,
            Some(Cursor::EventLog { record_id: 4, .. })
        ));
    }

    #[test]
    fn defaults_apply_without_arguments() {
        let args = BoundArgs::new("logs").with("limit", ArgValue::Absent, false);
        let query = LogQuery::from_args(&args, &LogLimits::default()).unwrap();
        assert_eq!(query.level, LogLevel::Info);
        assert_eq!(query.limit, DEFAULT_LIMIT);
        assert!(query.source.is_none() && query.message.is_none() && query.cursor.is_none());
    }

    #[test]
    fn message_as_text_is_a_pattern() {
        let args = BoundArgs::new("logs").with("message", text("^a.c$"), false);
        let query = LogQuery::from_args(&args, &LogLimits::default()).unwrap();
        assert!(query.message.unwrap().is_match("abc"));
    }

    #[test]
    fn regex_flags_are_applied() {
        let flags = RegexFlags {
            multi_line: true,
            dot_matches_new_line: true,
            extended: true,
            ..RegexFlags::default()
        };
        let regex = compile_regex(&RegexValue {
            pattern: "^b . c $".to_owned(),
            flags,
        })
        .unwrap();
        assert!(regex.is_match("a\nb\nc"));
    }

    #[test]
    fn rejects_invalid_arguments_with_codes() {
        let cases = [
            ("limit", ArgValue::Value(Value::Int(0)), ErrorCode::Limit),
            ("limit", text("5"), ErrorCode::Type),
            ("message", text("("), ErrorCode::Regex),
            ("message", ArgValue::Value(Value::Int(1)), ErrorCode::Type),
            ("level", text("verbose"), ErrorCode::InvalidValue),
            ("level", ArgValue::Value(Value::Int(1)), ErrorCode::Type),
            ("since", text("yesterday"), ErrorCode::Type),
            ("cursor", text("v9"), ErrorCode::InvalidValue),
            ("source", text("[a"), ErrorCode::PatternSyntax),
        ];
        for (name, value, code) in cases {
            let args = BoundArgs::new("logs").with(name, value, false);
            let error = LogQuery::from_args(&args, &LogLimits::default()).unwrap_err();
            assert_eq!(error.code(), code, "{name}");
        }
    }

    #[test]
    fn rejects_since_not_before_until() {
        let at = ArgValue::Value(Value::Instant(Instant::from_unix_nanos(5)));
        let args = BoundArgs::new("logs")
            .with("since", at.clone(), false)
            .with("until", at, false);
        let error = LogQuery::from_args(&args, &LogLimits::default()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn stream_stops_at_limit_and_reports_more() {
        let notices = Notices::new();
        let mut stream = LogStream::new((0..10).rev().map(|n| Ok(event(n))), notices.clone(), 3);
        let ids: Vec<_> = stream.by_ref().map(|event| event.unwrap().cursor).collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(stream.next(), None);
        assert_eq!(stream.next_cursor(), ids.last());
        let reported = stream.notices();
        assert_eq!(reported.len(), 1);
        assert_eq!(reported[0].code, NoticeCode::LimitReached);
        assert_eq!(reported[0].count, Some(3));
        assert!(format!("{stream:?}").contains("LogStream"));
    }

    #[test]
    fn stream_without_more_events_has_no_cursor() {
        let mut stream = LogStream::new((0..3).map(|n| Ok(event(n))), Notices::new(), 3);
        assert_eq!(stream.by_ref().count(), 3);
        assert!(stream.next_cursor().is_none());
        assert!(stream.notices().is_empty());
    }

    #[test]
    fn stream_ends_after_error() {
        let items = vec![
            Ok(event(1)),
            Err(Error::new(ErrorCode::Io, "read failed")),
            Ok(event(2)),
        ];
        let mut stream = LogStream::new(items.into_iter(), Notices::new(), 10);
        assert!(stream.next().unwrap().is_ok());
        assert!(stream.next().unwrap().is_err());
        assert!(stream.next().is_none());
    }

    #[test]
    fn logs_cancel_between_events() {
        let token = CancelToken::new();
        let mut stream = LogStream::new((0..10).map(|n| Ok(event(n))), Notices::new(), 10)
            .with_cancel(token.clone());
        assert!(stream.next().unwrap().is_ok());
        token.cancel(CancelReason::Requested);
        let error = stream.next().unwrap().unwrap_err();
        assert_eq!(error.code(), ErrorCode::Cancelled);
        assert!(stream.next().is_none());
    }

    #[test]
    fn notices_merge_counts_of_equal_entries() {
        let notices = Notices::new();
        let mut first = LogNotice::new(NoticeCode::PermissionDeniedSkipped);
        first.count = Some(1);
        notices.add(first.clone());
        notices.add(first);
        let mut other = LogNotice::new(NoticeCode::FileSkipped);
        other.path = Some("/a".to_owned());
        notices.add(other.clone());
        notices.add(LogNotice::new(NoticeCode::FileSkipped));
        let mut counted = LogNotice::new(NoticeCode::FileSkipped);
        counted.count = Some(2);
        notices.add(counted);
        let all = notices.snapshot();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].count, Some(2));
        assert_eq!(all[1], other);
        assert_eq!(all[2].count, Some(2));
    }

    #[test]
    fn notice_codes_have_report_names_and_severities() {
        let all = [
            (
                NoticeCode::FileSkipped,
                "log_file_skipped",
                Severity::Warning,
            ),
            (
                NoticeCode::PermissionDeniedSkipped,
                "permission_denied_skipped",
                Severity::Warning,
            ),
            (NoticeCode::Unformatted, "log_unformatted", Severity::Notice),
            (
                NoticeCode::LimitReached,
                "log_limit_reached",
                Severity::Info,
            ),
            (
                NoticeCode::UnknownName,
                "log_unknown_name",
                Severity::Notice,
            ),
            (NoticeCode::EventsLost, "log_events_lost", Severity::Notice),
            (
                NoticeCode::CursorApproximate,
                "log_cursor_approximate",
                Severity::Notice,
            ),
        ];
        for (code, name, severity) in all {
            assert_eq!(code.as_str(), name);
            assert_eq!(code.severity(), severity);
        }
    }

    #[test]
    fn only_report_codes_convert_to_notable_entries() {
        let mut notice = LogNotice::new(NoticeCode::PermissionDeniedSkipped);
        notice.count = Some(4);
        let entry = notice.to_notable().unwrap();
        assert_eq!(entry.code, Notable::PermissionDeniedSkipped);
        assert_eq!(entry.count, Some(4));
        assert!(
            LogNotice::new(NoticeCode::FileSkipped)
                .to_notable()
                .is_none()
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
