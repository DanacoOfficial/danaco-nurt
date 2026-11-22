// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interface between the reader and the event log of the system.
//!
//! [`EventSource`] is the only part of the reader that calls the operating
//! system. On Windows it is implemented over the Windows Event Log API; tests
//! implement it over recorded events.

/// Failure of the event log reported by an [`EventSource`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    /// The channel does not exist.
    #[error("the channel does not exist")]
    ChannelNotFound,
    /// The account may not read the channel.
    #[error("access to the channel is denied")]
    AccessDenied,
    /// The event log service does not answer.
    #[error("the event log service is not available")]
    ServiceUnavailable,
    /// The event log rejected the XPath query.
    #[error("the event log rejected the query")]
    InvalidQuery,
    /// Another error of the system, with its Win32 error code.
    #[error("event log error {code}: {message}")]
    System {
        /// Win32 error code.
        code: u32,
        /// Message of the system.
        message: String,
    },
}

/// Part of an event formatted from the metadata of its provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessagePart {
    /// The message of the event.
    Event,
    /// The name of the task of the event.
    Task,
}

/// Events of the event log of a system.
///
/// Implementations are shared by the channel readers of one query, which may
/// be moved to another thread together; they are therefore `Sync`.
pub trait EventSource: Sync {
    /// Handle of one event.
    type Event: Send;
    /// Events of one channel, newest first.
    type Events<'a>: Iterator<Item = Result<Self::Event, SourceError>> + Send
    where
        Self: 'a;

    /// Names of every channel of the system.
    ///
    /// # Errors
    ///
    /// A [`SourceError`] when the channels cannot be listed.
    fn channels(&self) -> Result<Vec<String>, SourceError>;

    /// Events of a channel that match an XPath query, in reverse order of
    /// their record identifiers.
    ///
    /// # Errors
    ///
    /// [`SourceError::ChannelNotFound`], [`SourceError::AccessDenied`],
    /// [`SourceError::InvalidQuery`] and other failures of the event log.
    fn query<'a>(&'a self, channel: &str, xpath: &str) -> Result<Self::Events<'a>, SourceError>;

    /// The XML form of an event.
    ///
    /// # Errors
    ///
    /// A [`SourceError`] when the event cannot be rendered.
    fn render_xml(&self, event: &Self::Event) -> Result<String, SourceError>;

    /// A part of an event formatted from the metadata of its provider, or
    /// `None` when the provider has no metadata or no text for the event.
    ///
    /// A message whose insertion strings the provider cannot resolve is
    /// returned with the unresolved insertions as the system leaves them.
    fn format(&self, event: &Self::Event, provider: &str, part: MessagePart) -> Option<String>;
}
