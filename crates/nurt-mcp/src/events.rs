// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Delivery of engine events to the client.
//!
//! The engine emits [`Event`]s from the thread that runs the request. They
//! pass through a bounded channel to the task that serves the request, which
//! sends progress as `notifications/progress` (only when the client gave a
//! progress token) and log messages as `notifications/message` (only at or
//! above the level set with `logging/setLevel`).

// Logging notifications are deprecated in protocol revisions newer than the
// ones this server negotiates; the revisions it serves still define them.
#![allow(deprecated)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use nurt_contracts::stage::{Event, EventSink, LogEvent, LogLevel};
use rmcp::model::{
    LoggingLevel, LoggingMessageNotificationParam, ProgressNotificationParam, ProgressToken,
};
use rmcp::{Peer, RoleServer};
use tokio::sync::mpsc;

/// Number of events buffered between the engine and the client. Progress
/// and log messages are advisory, so events beyond the buffer are dropped
/// instead of slowing down the execution.
pub(crate) const EVENT_BUFFER: usize = 256;

/// Logger name used when an event does not name its source.
const DEFAULT_LOGGER: &str = "nurt";

/// Level below which log messages are not sent until the client sets one.
pub(crate) const DEFAULT_LOG_LEVEL: LoggingLevel = LoggingLevel::Warning;

/// Event sink that passes events to the serving task.
pub(crate) struct ChannelSink {
    sender: mpsc::Sender<Event>,
}

impl ChannelSink {
    /// Creates a sink and the receiver of its events.
    pub(crate) fn channel() -> (Arc<Self>, mpsc::Receiver<Event>) {
        let (sender, receiver) = mpsc::channel(EVENT_BUFFER);
        (Arc::new(Self { sender }), receiver)
    }
}

impl EventSink for ChannelSink {
    fn emit(&self, event: Event) {
        if self.sender.try_send(event).is_err() {
            tracing::debug!("event dropped: the buffer is full or the request has ended");
        }
    }
}

/// Logging threshold of one connection, set with `logging/setLevel`.
#[derive(Debug)]
pub(crate) struct LogThreshold(AtomicU8);

impl LogThreshold {
    /// Creates a threshold at [`DEFAULT_LOG_LEVEL`].
    pub(crate) fn new() -> Self {
        Self(AtomicU8::new(rank(DEFAULT_LOG_LEVEL)))
    }

    /// Sets the lowest level that is sent.
    pub(crate) fn set(&self, level: LoggingLevel) {
        self.0.store(rank(level), Ordering::Relaxed);
    }

    /// Whether a message of the level is sent.
    pub(crate) fn admits(&self, level: LoggingLevel) -> bool {
        rank(level) >= self.0.load(Ordering::Relaxed)
    }
}

/// Position of a level in the order of RFC 5424 severities, lowest first.
fn rank(level: LoggingLevel) -> u8 {
    match level {
        LoggingLevel::Debug => 0,
        LoggingLevel::Info => 1,
        LoggingLevel::Notice => 2,
        LoggingLevel::Warning => 3,
        LoggingLevel::Error => 4,
        LoggingLevel::Critical => 5,
        LoggingLevel::Alert => 6,
        LoggingLevel::Emergency => 7,
    }
}

/// Protocol level of a log event of the engine.
fn logging_level(level: LogLevel) -> LoggingLevel {
    match level {
        LogLevel::Error => LoggingLevel::Error,
        LogLevel::Warn => LoggingLevel::Warning,
        LogLevel::Info => LoggingLevel::Info,
        LogLevel::Debug | LogLevel::Trace => LoggingLevel::Debug,
    }
}

/// Text of a progress notification for an event, or `None` for events that
/// are not progress.
fn progress_message(event: &Event) -> Option<String> {
    match event {
        Event::Progress(progress) => {
            Some(
                progress
                    .message
                    .clone()
                    .unwrap_or_else(|| match progress.total {
                        Some(total) => format!(
                            "{} {}: {}/{}",
                            progress.command, progress.phase, progress.done, total
                        ),
                        None => {
                            format!("{} {}: {}", progress.command, progress.phase, progress.done)
                        }
                    }),
            )
        }
        Event::JobProgress(progress) => Some(match (&progress.line, progress.percent) {
            (Some(line), _) => line.clone(),
            (None, Some(percent)) => format!("{}: {percent}%", progress.id),
            (None, None) => progress.id.clone(),
        }),
        Event::JobOutput(output) => Some(output.lines.join("\n")),
        _ => None,
    }
}

/// Sends the events of one request to the client.
pub(crate) struct Forwarder<'a> {
    peer: &'a Peer<RoleServer>,
    progress_token: Option<ProgressToken>,
    threshold: &'a LogThreshold,
    sent: u32,
}

impl<'a> Forwarder<'a> {
    /// Creates a forwarder for a request with an optional progress token.
    pub(crate) fn new(
        peer: &'a Peer<RoleServer>,
        progress_token: Option<ProgressToken>,
        threshold: &'a LogThreshold,
    ) -> Self {
        Self {
            peer,
            progress_token,
            threshold,
            sent: 0,
        }
    }

    /// Sends one event, when the client asked for its kind.
    ///
    /// A notification that cannot be sent means that the connection is
    /// closing; the request ends on its own, so the failure is only traced.
    pub(crate) async fn forward(&mut self, event: Event) {
        match &event {
            Event::Log(log) => self.forward_log(log).await,
            _ => self.forward_progress(&event).await,
        }
    }

    async fn forward_log(&self, log: &LogEvent) {
        let level = logging_level(log.level);
        if !self.threshold.admits(level) {
            return;
        }
        let logger = log.source.as_deref().unwrap_or(DEFAULT_LOGGER);
        let param = LoggingMessageNotificationParam::new(level, log.message.clone().into())
            .with_logger(logger);
        if let Err(error) = self.peer.notify_logging_message(param).await {
            tracing::debug!(%error, "log notification not sent");
        }
    }

    async fn forward_progress(&mut self, event: &Event) {
        let (Some(token), Some(message)) = (&self.progress_token, progress_message(event)) else {
            return;
        };
        self.sent += 1;
        let param = ProgressNotificationParam::new(token.clone(), f64::from(self.sent))
            .with_message(message);
        if let Err(error) = self.peer.notify_progress(param).await {
            tracing::debug!(%error, "progress notification not sent");
        }
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::stage::{
        Event, EventSink, JobOutput, JobProgress, LogEvent, LogLevel, OutputStream, Progress,
    };
    use rmcp::model::LoggingLevel;

    use super::{ChannelSink, EVENT_BUFFER, LogThreshold, logging_level, progress_message};

    fn stage_progress(total: Option<u64>, message: Option<&str>) -> Event {
        Event::Progress(Progress {
            command: "find".to_owned(),
            phase: "scan".to_owned(),
            done: 40,
            total,
            bytes: None,
            message: message.map(str::to_owned),
        })
    }

    fn job_progress(percent: Option<f64>, line: Option<&str>) -> Event {
        Event::JobProgress(JobProgress {
            id: "j3".to_owned(),
            percent,
            line: line.map(str::to_owned),
        })
    }

    #[test]
    fn stage_progress_uses_its_message_or_counts() {
        assert_eq!(
            progress_message(&stage_progress(Some(80), None)).as_deref(),
            Some("find scan: 40/80")
        );
        assert_eq!(
            progress_message(&stage_progress(None, None)).as_deref(),
            Some("find scan: 40")
        );
        assert_eq!(
            progress_message(&stage_progress(None, Some("scanning /srv"))).as_deref(),
            Some("scanning /srv")
        );
    }

    #[test]
    fn job_progress_prefers_the_matching_line() {
        assert_eq!(
            progress_message(&job_progress(Some(50.0), Some("[5/10] build"))).as_deref(),
            Some("[5/10] build")
        );
        assert_eq!(
            progress_message(&job_progress(Some(50.0), None)).as_deref(),
            Some("j3: 50%")
        );
        assert_eq!(
            progress_message(&job_progress(None, None)).as_deref(),
            Some("j3")
        );
    }

    #[test]
    fn job_output_joins_lines_and_logs_are_not_progress() {
        let output = Event::JobOutput(JobOutput {
            id: "j3".to_owned(),
            stream: OutputStream::Stdout,
            cursor: "c1".to_owned(),
            lines: vec!["one".to_owned(), "two".to_owned()],
            bytes: 8,
        });
        assert_eq!(progress_message(&output).as_deref(), Some("one\ntwo"));
        let log = Event::Log(LogEvent {
            level: LogLevel::Info,
            message: "slow volume".to_owned(),
            source: None,
        });
        assert_eq!(progress_message(&log), None);
    }

    #[test]
    fn engine_levels_map_to_protocol_levels() {
        assert_eq!(logging_level(LogLevel::Error), LoggingLevel::Error);
        assert_eq!(logging_level(LogLevel::Warn), LoggingLevel::Warning);
        assert_eq!(logging_level(LogLevel::Info), LoggingLevel::Info);
        assert_eq!(logging_level(LogLevel::Debug), LoggingLevel::Debug);
        assert_eq!(logging_level(LogLevel::Trace), LoggingLevel::Debug);
    }

    #[test]
    fn threshold_admits_levels_at_or_above_it() {
        let threshold = LogThreshold::new();
        assert!(!threshold.admits(LoggingLevel::Info));
        assert!(threshold.admits(LoggingLevel::Warning));
        assert!(threshold.admits(LoggingLevel::Emergency));
        threshold.set(LoggingLevel::Debug);
        assert!(threshold.admits(LoggingLevel::Debug));
        threshold.set(LoggingLevel::Critical);
        assert!(!threshold.admits(LoggingLevel::Error));
        assert!(threshold.admits(LoggingLevel::Alert));
        threshold.set(LoggingLevel::Notice);
        assert!(!threshold.admits(LoggingLevel::Info));
    }

    #[test]
    fn sink_drops_events_beyond_the_buffer() {
        let (sink, mut receiver) = ChannelSink::channel();
        for _ in 0..=EVENT_BUFFER {
            sink.emit(stage_progress(None, None));
        }
        let mut received = 0;
        while receiver.try_recv().is_ok() {
            received += 1;
        }
        assert_eq!(received, EVENT_BUFFER);
    }
}
