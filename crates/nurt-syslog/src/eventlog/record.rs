// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Mapping of Windows events to log events.
//!
//! | Field | Source |
//! |---|---|
//! | `time` | `TimeCreated/@SystemTime` |
//! | `source` | `Provider/@Name` |
//! | `priority` | `Level`, see [`level::priority`] |
//! | `message` | message of the provider, or the payload as `name=value` pairs |
//! | `id` | `EventID`, its lower 16 bits |
//! | `unit` | `Channel` |
//! | `host` | `Computer` |
//! | `pid`, `tid` | `Execution/@ProcessID`, `Execution/@ThreadID` |
//! | `user` | `Security/@UserID` |
//! | `category` | name of the task |
//! | `fields` | payload of the event, then the system properties of [`SYSTEM_FIELDS`] |
//! | `cursor` | `v1:w:<channel>:<EventRecordID>` |

use crate::cursor::Cursor;
use crate::event::{LogEvent, PRIORITY_INFO, StoreKind};

use super::level;
use super::xml::EventXml;

/// Names of the fields that carry system properties without a field of their
/// own in the event, in the order in which they follow the payload.
pub const SYSTEM_FIELDS: [&str; 9] = [
    "EventRecordID",
    "Level",
    "Task",
    "Opcode",
    "Keywords",
    "Qualifiers",
    "Version",
    "ProviderGuid",
    "ActivityID",
];

/// Separator of the `name=value` pairs of a message without provider metadata.
const RAW_SEPARATOR: &str = "; ";

/// Bits of `EventID` that identify the event within its provider. The upper
/// bits of classic events are qualifiers.
const EVENT_ID_MASK: u32 = 0xFFFF;

/// Texts formatted from the metadata of the provider of an event.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Formatted {
    /// Message of the event.
    pub message: Option<String>,
    /// Name of the task of the event.
    pub task: Option<String>,
}

/// Maps an event to a log event.
///
/// The channel the event was read from is used when the event XML does not
/// name its channel. Without a formatted message the
/// message is the raw payload and the event has the flag `unformatted`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::record::LogLevel;
/// use nurt_syslog::eventlog::record::{map_event, Formatted};
/// use nurt_syslog::eventlog::xml::EventXml;
/// let xml = EventXml::parse(
///     "<Event><System><Provider Name='Disk'/><EventID Qualifiers='49156'>7</EventID>\
///      <Level>2</Level><TimeCreated SystemTime='2026-11-21T10:00:00Z'/>\
///      <EventRecordID>42</EventRecordID><Channel>System</Channel></System>\
///      <EventData><Data>\\Device\\Harddisk0</Data></EventData></Event>",
/// )?;
/// let event = map_event(&xml, Formatted::default(), "System");
/// assert_eq!(event.level(), LogLevel::Error);
/// assert_eq!(event.id.as_deref(), Some("7"));
/// assert_eq!(event.message, "Data1=\\Device\\Harddisk0");
/// assert!(event.flags.unformatted);
/// assert_eq!(event.cursor.to_string(), "v1:w:System:42");
/// # Ok::<(), nurt_syslog::eventlog::xml::EventXmlError>(())
/// ```
#[must_use]
pub fn map_event(xml: &EventXml, formatted: Formatted, channel: &str) -> LogEvent {
    let channel = xml.channel.clone().unwrap_or_else(|| channel.to_owned());
    let priority = xml
        .level
        .as_deref()
        .and_then(|text| text.trim().parse().ok())
        .map_or(PRIORITY_INFO, level::priority);
    let unformatted = formatted.message.is_none();
    let message = formatted
        .message
        .unwrap_or_else(|| raw_message(&xml.data));
    let cursor = Cursor::EventLog {
        channel: channel.clone(),
        record_id: xml.record_id,
    };
    let mut event = LogEvent::new(
        xml.time_created,
        &xml.provider,
        priority,
        &message,
        StoreKind::EventLog,
        cursor,
    );
    event.id = xml.event_id.map(|id| (id & EVENT_ID_MASK).to_string());
    event.unit = Some(channel);
    event.host.clone_from(&xml.computer);
    event.pid = xml.process_id;
    event.tid = xml.thread_id;
    event.user.clone_from(&xml.user_id);
    event.category = formatted.task;
    event.fields = fields(xml);
    event.flags.unformatted = unformatted;
    event
}

/// Message of an event whose provider has no message: the payload as
/// `name=value` pairs separated by `; `.
///
/// # Examples
///
/// ```
/// use nurt_syslog::eventlog::record::raw_message;
/// let data = [("param1", "Spooler"), ("param2", "stopped")]
///     .map(|(name, value)| (name.to_owned(), value.to_owned()));
/// assert_eq!(raw_message(&data), "param1=Spooler; param2=stopped");
/// ```
#[must_use]
pub fn raw_message(data: &[(String, String)]) -> String {
    data.iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join(RAW_SEPARATOR)
}

fn fields(xml: &EventXml) -> Vec<(String, String)> {
    let system = [
        Some(xml.record_id.to_string()),
        xml.level.clone(),
        xml.task.clone(),
        xml.opcode.clone(),
        xml.keywords.clone(),
        xml.qualifiers.clone(),
        xml.version.clone(),
        xml.provider_guid.clone(),
        xml.activity_id.clone(),
    ];
    let mut fields = xml.data.clone();
    fields.extend(
        SYSTEM_FIELDS
            .iter()
            .zip(system)
            .filter_map(|(name, value)| value.map(|value| ((*name).to_owned(), value))),
    );
    fields
}
