// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Log store over the Windows event log.
//!
//! [`EventLogStore`] selects the channels of the parameter `unit`, opens one
//! query per channel with an XPath filter, merges the channel streams newest
//! first and applies the final filter and the size limits of the crate.
//!
//! A cursor continues its own channel by record identifier. The other
//! channels continue from the creation time of the event of the cursor,
//! which is read from the event log. When that event no longer exists, the
//! other channels are read from their newest event and the notice
//! `log_cursor_approximate` is added.

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::value::Instant;
use serde_json::json;

use crate::cursor::Cursor;
use crate::event::LogEvent;
use crate::filter::EventFilter;
use crate::limits::LogLimits;
use crate::merge::{Merge, Source};
use crate::query::{LogNotice, LogQuery, LogStore, LogStream, NameMatch, NoticeCode, Notices};

use super::channels::{self, DEFAULT_CHANNELS};
use super::record::{self, Formatted};
use super::source::{EventSource, MessagePart, SourceError};
use super::xml::EventXml;
use super::xpath::XPathFilter;

/// Value of the detail `required` of `E_PRIVILEGE`.
const REQUIRED_ADMINISTRATOR: &str = "administrator";

/// Class of a skipped event in the notice `log_file_skipped`.
const SKIPPED_CORRUPT: &str = "corrupt";

/// Class of an event that could not be read in the notice `log_file_skipped`.
const SKIPPED_IO: &str = "io";

/// Log store of the Windows event log.
#[derive(Debug)]
pub struct EventLogStore<S> {
    source: S,
    limits: LogLimits,
}

impl<S: EventSource> EventLogStore<S> {
    /// Store over an event source with the given size limits.
    #[must_use]
    pub const fn new(source: S, limits: LogLimits) -> Self {
        Self { source, limits }
    }

    fn select(&self, unit: Option<&NameMatch>) -> Result<(Vec<String>, bool), Error> {
        let Some(unit) = unit else {
            return Ok((DEFAULT_CHANNELS.map(str::to_owned).to_vec(), false));
        };
        if let Some(name) = unit.literal() {
            return Ok((vec![name.to_owned()], true));
        }
        let available = self
            .source
            .channels()
            .map_err(|error| source_error(&error, unit.as_str(), ""))?;
        let pattern = NameMatch::parse_ignore_case(unit.as_str())?;
        let matched: Vec<String> = available
            .iter()
            .filter(|channel| pattern.matches(channel))
            .cloned()
            .collect();
        if matched.is_empty() {
            return Err(unknown_channel(unit.as_str(), &available));
        }
        Ok((matched, false))
    }

    fn resume(&self, cursor: Option<&Cursor>, notices: &Notices) -> Result<Option<Resume>, Error> {
        match cursor {
            None => Ok(None),
            Some(Cursor::EventLog { channel, record_id }) => {
                let time = self.record_time(channel, *record_id);
                if time.is_none() {
                    let mut notice = LogNotice::new(NoticeCode::CursorApproximate);
                    notice.path = Some(channel.clone());
                    notices.add(notice);
                }
                Ok(Some(Resume {
                    cursor: Cursor::EventLog {
                        channel: channel.clone(),
                        record_id: *record_id,
                    },
                    time,
                }))
            }
            Some(other) => Err(Error::new(
                ErrorCode::InvalidValue,
                format!(
                    "cursor `{other}` belongs to another log store; pass a cursor returned by `logs` on this system"
                ),
            )
            .with_found(other.to_string())),
        }
    }

    /// Creation time of a record, read with a query of its identifier.
    fn record_time(&self, channel: &str, record_id: u64) -> Option<Instant> {
        let xpath = format!("*[System[EventRecordID={record_id}]]");
        let event = self.source.query(channel, &xpath).ok()?.next()?.ok()?;
        let text = self.source.render_xml(&event).ok()?;
        EventXml::parse(&text).ok().map(|xml| xml.time_created)
    }
}

impl<S: EventSource> LogStore for EventLogStore<S> {
    /// Reads the channels of the query, newest event first.
    ///
    /// # Errors
    ///
    /// - `E_UNKNOWN_VALUE` when a named channel does not exist or a pattern
    ///   matches no channel; `expected` lists the nearest channel names.
    /// - `E_PATTERN_SYNTAX` when the pattern of `unit` is malformed.
    /// - `E_INVALID_VALUE` for a cursor of another log store.
    /// - `E_PRIVILEGE` with the detail `required: administrator` when access
    ///   to a named channel, or to every selected channel, is denied.
    /// - `E_NOT_FOUND` when the event log service is not available.
    /// - `E_STAGE_FAILED` when the event log rejects the query or fails
    ///   otherwise; the details carry the XPath query or the Win32 error code.
    fn query<'a>(&'a self, query: &'a LogQuery) -> Result<LogStream<'a>, Error> {
        let notices = Notices::new();
        let (selected, named) = self.select(query.unit.as_ref())?;
        let resume = self.resume(query.cursor.as_ref(), &notices)?;
        let provider = query.source.as_ref().and_then(NameMatch::literal);
        let mut sources = Vec::new();
        let mut denied = Vec::new();
        for channel in selected {
            let xpath = channel_filter(query, &channel, provider, resume.as_ref()).to_xpath();
            match self.source.query(&channel, &xpath) {
                Ok(events) => sources.push(Source::eager(ChannelEvents {
                    source: &self.source,
                    channel,
                    events,
                    resume: resume.clone(),
                    notices: notices.clone(),
                })),
                Err(SourceError::AccessDenied) if !named => denied.push(channel),
                Err(SourceError::ChannelNotFound) if named => {
                    let available = self.source.channels().unwrap_or_default();
                    return Err(unknown_channel(&channel, &available));
                }
                Err(SourceError::ChannelNotFound) => {}
                Err(error) => return Err(source_error(&error, &channel, &xpath)),
            }
        }
        if sources.is_empty() && !denied.is_empty() {
            return Err(privilege_error(&denied.join(", ")));
        }
        for channel in denied {
            let mut notice = LogNotice::new(NoticeCode::PermissionDeniedSkipped);
            notice.path = Some(channel);
            notice.count = Some(1);
            notices.add(notice);
        }
        let max_open = sources.len().max(1);
        let filter = EventFilter::new(query).with_unit(None);
        let limits = &self.limits;
        let counted = notices.clone();
        let events = Merge::new(sources, max_open).filter_map(move |item| match item {
            Ok(mut event) if filter.matches(&event) => {
                if event.flags.unformatted {
                    let mut notice = LogNotice::new(NoticeCode::Unformatted);
                    notice.count = Some(1);
                    counted.add(notice);
                }
                limits.apply(&mut event);
                Some(Ok(event))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        });
        Ok(LogStream::new(events, notices, query.limit))
    }
}

/// Position of a cursor and the creation time of its event, when known.
#[derive(Debug, Clone)]
struct Resume {
    cursor: Cursor,
    time: Option<Instant>,
}

impl Resume {
    fn record_in(&self, channel: &str) -> Option<u64> {
        match &self.cursor {
            Cursor::EventLog {
                channel: own,
                record_id,
            } if own.eq_ignore_ascii_case(channel) => Some(*record_id),
            _ => None,
        }
    }

    /// Whether an event comes after the cursor in the order of `logs`.
    fn admits(&self, event: &LogEvent, channel: &str) -> bool {
        if let Some(record_id) = self.record_in(channel) {
            return matches!(&event.cursor, Cursor::EventLog { record_id: own, .. } if *own < record_id);
        }
        self.time
            .is_none_or(|time| (event.time, &event.cursor) < (time, &self.cursor))
    }
}

/// XPath filter of one channel. Events at the time of the cursor are kept by
/// the query and decided by [`Resume::admits`].
fn channel_filter<'a>(
    query: &LogQuery,
    channel: &str,
    provider: Option<&'a str>,
    resume: Option<&Resume>,
) -> XPathFilter<'a> {
    let mut filter = XPathFilter {
        level: query.level,
        since: query.since,
        until: query.until,
        provider,
        before_record: None,
    };
    if let Some(resume) = resume {
        if let Some(record_id) = resume.record_in(channel) {
            filter.before_record = Some(record_id);
        } else if let Some(bound) = resume
            .time
            .and_then(|time| time.as_unix_nanos().checked_add(1))
        {
            let bound = Instant::from_unix_nanos(bound);
            filter.until = Some(filter.until.map_or(bound, |until| until.min(bound)));
        }
    }
    filter
}

/// Events of one channel, newest first.
///
/// An event that cannot be rendered or parsed is skipped with the notice
/// `log_file_skipped` for the channel; the stream ends after an error of the
/// query.
struct ChannelEvents<'s, S: EventSource> {
    source: &'s S,
    channel: String,
    events: S::Events<'s>,
    resume: Option<Resume>,
    notices: Notices,
}

impl<S: EventSource> ChannelEvents<'_, S> {
    fn read(&self, event: &S::Event) -> Option<LogEvent> {
        let Ok(text) = self.source.render_xml(event) else {
            self.skip(SKIPPED_IO);
            return None;
        };
        let Ok(xml) = EventXml::parse(&text) else {
            self.skip(SKIPPED_CORRUPT);
            return None;
        };
        let formatted = Formatted {
            message: self.source.format(event, &xml.provider, MessagePart::Event),
            task: self.source.format(event, &xml.provider, MessagePart::Task),
        };
        let mapped = record::map_event(&xml, formatted, &self.channel);
        self.resume
            .as_ref()
            .is_none_or(|resume| resume.admits(&mapped, &self.channel))
            .then_some(mapped)
    }

    fn skip(&self, class: &str) {
        let mut notice = LogNotice::new(NoticeCode::FileSkipped);
        notice.path = Some(self.channel.clone());
        notice.detail = Some(class.to_owned());
        notice.count = Some(1);
        self.notices.add(notice);
    }
}

impl<S: EventSource> Iterator for ChannelEvents<'_, S> {
    type Item = Result<LogEvent, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.events.next()? {
                Ok(event) => {
                    if let Some(mapped) = self.read(&event) {
                        return Some(Ok(mapped));
                    }
                }
                Err(error) => return Some(Err(source_error(&error, &self.channel, ""))),
            }
        }
    }
}

fn unknown_channel(name: &str, available: &[String]) -> Error {
    Error::new(
        ErrorCode::UnknownValue,
        format!("unknown event log channel `{name}`; use a channel listed in `expected`"),
    )
    .with_found(name)
    .with_expected(json!(channels::nearest(name, available)))
}

fn privilege_error(channels: &str) -> Error {
    Error::new(
        ErrorCode::Privilege,
        format!(
            "reading channel `{channels}` requires administrator rights; run the command from an elevated prompt"
        ),
    )
    .with_detail("required", json!(REQUIRED_ADMINISTRATOR))
    .with_detail("channel", json!(channels))
}

fn source_error(error: &SourceError, channel: &str, xpath: &str) -> Error {
    match error {
        SourceError::AccessDenied => privilege_error(channel),
        SourceError::ChannelNotFound => unknown_channel(channel, &[]),
        SourceError::ServiceUnavailable => Error::new(
            ErrorCode::NotFound,
            "the event log service is not available; start the service `EventLog`",
        ),
        SourceError::InvalidQuery => Error::new(
            ErrorCode::StageFailed,
            format!("the event log rejected the query of channel `{channel}`"),
        )
        .with_detail("channel", json!(channel))
        .with_detail("xpath", json!(xpath)),
        SourceError::System { code, message } => Error::new(
            ErrorCode::StageFailed,
            format!("reading channel `{channel}` failed: {message}"),
        )
        .with_detail("channel", json!(channel))
        .with_detail("os_error", json!(code)),
    }
}
