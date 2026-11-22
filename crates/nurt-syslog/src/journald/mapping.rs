// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Mapping of journal entries to log events.
//!
//! | Event field | Journal source |
//! |---|---|
//! | `time` | reception time of the entry |
//! | `source` | `SYSLOG_IDENTIFIER`, else `_COMM`, else the file name of `_EXE`, else `_TRANSPORT`, else `unknown` |
//! | `priority` | `PRIORITY`; missing or not a number 0–7 gives 6 |
//! | `message` | `MESSAGE`, empty when missing |
//! | `id` | `MESSAGE_ID` |
//! | `unit` | `UNIT` for messages of the service manager (`_PID=1`), `USER_UNIT` for messages of a user manager, else `_SYSTEMD_USER_UNIT`, else `_SYSTEMD_UNIT` |
//! | `host`, `pid`, `tid`, `user` | `_HOSTNAME`, `_PID`, `TID`, `_UID` |
//! | `boot_id` | boot identifier of the entry |
//! | `fields` | every field except `MESSAGE`, `MESSAGE_ID`, `_HOSTNAME`, `_PID` and `_BOOT_ID` |

use nurt_contracts::error::Error;
use nurt_contracts::value::Instant;

use crate::cursor::{Cursor, Id128};
use crate::event::{LogEvent, PRIORITY_DEBUG, PRIORITY_INFO, StoreKind};
use crate::limits::LogLimits;
use crate::query::NameMatch;

use super::object::{EntryObject, Payload};

/// Fields whose values can become the `unit` of an event.
pub(super) const UNIT_FIELDS: [&str; 4] = ["_SYSTEMD_UNIT", "_SYSTEMD_USER_UNIT", "UNIT", "USER_UNIT"];
/// Fields whose values can become the `source` of an event, besides `_EXE`.
pub(super) const SOURCE_FIELDS: [&str; 3] = ["SYSLOG_IDENTIFIER", "_COMM", "_TRANSPORT"];
/// Field with the executable path; its file name can become the `source`.
pub(super) const EXE_FIELD: &str = "_EXE";
/// Source of an entry without any identifying field.
pub(super) const UNKNOWN_SOURCE: &str = "unknown";

const UNIT_SUFFIXES: [&str; 11] = [
    ".service",
    ".socket",
    ".target",
    ".mount",
    ".automount",
    ".timer",
    ".path",
    ".slice",
    ".scope",
    ".device",
    ".swap",
];
const DEFAULT_UNIT_SUFFIX: &str = ".service";
const MANAGER_PID: &str = "1";
const USER_MANAGER_PREFIX: &str = "user@";
const NANOS_PER_MICRO: i64 = 1_000;
const OMITTED_FIELDS: [&str; 5] = ["MESSAGE", "MESSAGE_ID", "_HOSTNAME", "_PID", "_BOOT_ID"];

/// Field of an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Field {
    /// Name before `=`.
    pub(super) name: String,
    /// Value after `=`.
    pub(super) value: Vec<u8>,
    /// Whether the stored value exceeded the size limit.
    pub(super) truncated: bool,
}

impl Field {
    /// Splits a payload `NAME=value`; a payload without `=` or with an
    /// empty name is not a field.
    pub(super) fn split(payload: Payload) -> Option<Self> {
        let position = payload.bytes.iter().position(|byte| *byte == b'=')?;
        if position == 0 {
            return None;
        }
        let mut bytes = payload.bytes;
        let value = bytes.split_off(position + 1);
        bytes.truncate(position);
        Some(Self {
            name: String::from_utf8_lossy(&bytes).into_owned(),
            value,
            truncated: payload.truncated,
        })
    }
}

/// Unit name as `journalctl -u` reads it: a literal name without a known
/// unit suffix gets `.service`.
pub(super) fn normalize_unit(unit: &NameMatch) -> Result<NameMatch, Error> {
    match unit.literal() {
        Some(name) if !UNIT_SUFFIXES.iter().any(|suffix| name.ends_with(suffix)) => {
            NameMatch::parse(&format!("{name}{DEFAULT_UNIT_SUFFIX}"))
        }
        _ => Ok(unit.clone()),
    }
}

/// File name of an executable path.
pub(super) fn exe_name(path: &[u8]) -> &[u8] {
    path.rsplit(|byte| *byte == b'/').next().unwrap_or(path)
}

/// Whether `name` resembles `wanted`, for suggestions of existing names.
pub(super) fn similar(wanted: &str, name: &str) -> bool {
    const PREFIX: usize = 3;
    let stem = wanted.split('.').next().unwrap_or(wanted);
    name.contains(stem)
        || (stem.len() >= PREFIX && name.len() >= PREFIX && stem.get(..PREFIX) == name.get(..PREFIX))
}

/// Identity of the entry in its file.
#[derive(Debug, Clone, Copy)]
pub(super) struct Origin {
    /// Identifier of the file.
    pub(super) file_id: Id128,
    /// Sequence number space of the file.
    pub(super) seqnum_id: Id128,
}

/// Converts an entry and its fields into an event within the limits.
pub(super) fn to_event(origin: Origin, entry: &EntryObject, fields: Vec<Field>, limits: &LogLimits) -> LogEvent {
    let first = |name: &str| fields.iter().find(|field| field.name == name);
    let text = |name: &str| first(name).map(|field| String::from_utf8_lossy(&field.value).into_owned());
    let number = |name: &str| text(name).and_then(|value| value.parse::<u64>().ok());
    let source = first("SYSLOG_IDENTIFIER")
        .or_else(|| first("_COMM"))
        .map(|field| String::from_utf8_lossy(&field.value).into_owned())
        .or_else(|| first(EXE_FIELD).map(|field| String::from_utf8_lossy(exe_name(&field.value)).into_owned()))
        .or_else(|| text("_TRANSPORT"))
        .unwrap_or_else(|| UNKNOWN_SOURCE.to_owned());
    let priority = text("PRIORITY")
        .and_then(|value| value.parse::<u8>().ok())
        .filter(|value| *value <= PRIORITY_DEBUG)
        .unwrap_or(PRIORITY_INFO);
    let micros = i64::try_from(entry.realtime).unwrap_or(i64::MAX);
    let time = Instant::from_unix_nanos(micros.saturating_mul(NANOS_PER_MICRO));
    let cursor = Cursor::Journal {
        file_id: origin.file_id,
        seqnum_id: origin.seqnum_id,
        seqnum: entry.seqnum,
        realtime: entry.realtime,
    };
    let mut event = LogEvent::new(time, &source, priority, "", StoreKind::Journald, cursor);
    if let Some(message) = first("MESSAGE") {
        let (decoded, lossy) = decode(&message.value);
        event.message = decoded;
        event.flags.lossy |= lossy;
    }
    event.id = text("MESSAGE_ID");
    event.unit = unit_of(&text);
    event.host = text("_HOSTNAME");
    event.pid = number("_PID");
    event.tid = number("TID");
    event.user = text("_UID");
    event.boot_id = Some(entry.boot_id.to_string());
    event.flags.truncated = fields.iter().any(|field| field.truncated);
    for field in fields {
        if OMITTED_FIELDS.contains(&field.name.as_str()) {
            continue;
        }
        let (value, lossy) = decode(&field.value);
        event.flags.lossy |= lossy;
        event.fields.push((field.name, value));
    }
    limits.apply(&mut event);
    event
}

fn unit_of(text: &impl Fn(&str) -> Option<String>) -> Option<String> {
    let systemd_unit = text("_SYSTEMD_UNIT");
    if text("_PID").as_deref() == Some(MANAGER_PID)
        && let Some(unit) = text("UNIT")
    {
        return Some(unit);
    }
    if systemd_unit.as_deref().is_some_and(|unit| unit.starts_with(USER_MANAGER_PREFIX))
        && let Some(unit) = text("USER_UNIT")
    {
        return Some(unit);
    }
    text("_SYSTEMD_USER_UNIT").or(systemd_unit)
}

fn decode(bytes: &[u8]) -> (String, bool) {
    match String::from_utf8(bytes.to_vec()) {
        Ok(text) => (text, false),
        Err(error) => (String::from_utf8_lossy(error.as_bytes()).into_owned(), true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, value: &[u8]) -> Field {
        Field {
            name: name.to_owned(),
            value: value.to_vec(),
            truncated: false,
        }
    }

    fn entry() -> EntryObject {
        EntryObject {
            offset: 0,
            seqnum: 3,
            realtime: 1_500,
            monotonic: 0,
            boot_id: Id128::new([0xab; 16]),
            xor_hash: 0,
            items: Vec::new(),
        }
    }

    fn origin() -> Origin {
        Origin {
            file_id: Id128::new([1; 16]),
            seqnum_id: Id128::new([2; 16]),
        }
    }

    fn map(fields: Vec<Field>) -> LogEvent {
        to_event(origin(), &entry(), fields, &LogLimits::default())
    }

    #[test]
    fn maps_common_fields() {
        let event = map(vec![
            field("MESSAGE", b"started"),
            field("PRIORITY", b"3"),
            field("SYSLOG_IDENTIFIER", b"sshd"),
            field("_SYSTEMD_UNIT", b"ssh.service"),
            field("_PID", b"42"),
            field("TID", b"43"),
            field("_UID", b"0"),
            field("_HOSTNAME", b"host"),
            field("MESSAGE_ID", b"0123456789abcdef0123456789abcdef"),
            field("_TRANSPORT", b"syslog"),
        ]);
        assert_eq!(event.message, "started");
        assert_eq!(event.priority, 3);
        assert_eq!(event.source, "sshd");
        assert_eq!(event.unit.as_deref(), Some("ssh.service"));
        assert_eq!((event.pid, event.tid), (Some(42), Some(43)));
        assert_eq!(event.user.as_deref(), Some("0"));
        assert_eq!(event.host.as_deref(), Some("host"));
        assert_eq!(event.time.as_unix_nanos(), 1_500_000);
        assert_eq!(event.boot_id.as_deref(), Some("abababababababababababababababab"));
        let names: Vec<_> = event.fields.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["PRIORITY", "SYSLOG_IDENTIFIER", "_SYSTEMD_UNIT", "TID", "_UID", "_TRANSPORT"]);
        assert!(event.cursor.to_string().starts_with("v1:j:0101"));
    }

    #[test]
    fn source_falls_back_in_order() {
        assert_eq!(map(vec![field("_COMM", b"cron")]).source, "cron");
        assert_eq!(map(vec![field("_EXE", b"/usr/sbin/cron")]).source, "cron");
        assert_eq!(map(vec![field("_TRANSPORT", b"kernel")]).source, "kernel");
        assert_eq!(map(Vec::new()).source, "unknown");
    }

    #[test]
    fn logs_level_maps_priority() {
        assert_eq!(map(vec![field("PRIORITY", b"0")]).priority, 0);
        assert_eq!(map(vec![field("PRIORITY", b"9")]).priority, 6);
        assert_eq!(map(vec![field("PRIORITY", b"x")]).priority, 6);
        assert_eq!(map(Vec::new()).priority, 6);
    }

    #[test]
    fn unit_prefers_the_unit_an_event_is_about() {
        let manager = map(vec![
            field("_PID", b"1"),
            field("_SYSTEMD_UNIT", b"init.scope"),
            field("UNIT", b"cron.service"),
        ]);
        assert_eq!(manager.unit.as_deref(), Some("cron.service"));
        let user_manager = map(vec![
            field("_SYSTEMD_UNIT", b"user@1000.service"),
            field("USER_UNIT", b"app.service"),
        ]);
        assert_eq!(user_manager.unit.as_deref(), Some("app.service"));
        let user_unit = map(vec![
            field("_SYSTEMD_UNIT", b"user@1000.service"),
            field("_SYSTEMD_USER_UNIT", b"music.service"),
        ]);
        assert_eq!(user_unit.unit.as_deref(), Some("music.service"));
        assert_eq!(map(Vec::new()).unit, None);
    }

    #[test]
    fn binary_message_is_decoded_lossily() {
        let event = map(vec![field("MESSAGE", b"a\xffb")]);
        assert_eq!(event.message, "a\u{fffd}b");
        assert!(event.flags.lossy);
    }

    #[test]
    fn truncated_payload_marks_event() {
        let mut long = field("MESSAGE", b"abc");
        long.truncated = true;
        assert!(map(vec![long]).flags.truncated);
    }

    #[test]
    fn splits_payloads_at_first_equals_sign() {
        let payload = |bytes: &[u8]| Payload {
            bytes: bytes.to_vec(),
            truncated: false,
        };
        let split = Field::split(payload(b"A=b=c")).unwrap();
        assert_eq!((split.name.as_str(), split.value.as_slice()), ("A", b"b=c".as_slice()));
        assert!(Field::split(payload(b"=x")).is_none());
        assert!(Field::split(payload(b"novalue")).is_none());
    }

    #[test]
    fn normalizes_unit_names_like_journalctl() {
        let normalize = |text: &str| normalize_unit(&NameMatch::parse(text).unwrap()).unwrap().as_str().to_owned();
        assert_eq!(normalize("ssh"), "ssh.service");
        assert_eq!(normalize("ssh.socket"), "ssh.socket");
        assert_eq!(normalize("ssh*"), "ssh*");
    }

    #[test]
    fn suggests_similar_names() {
        assert!(similar("sshd.service", "ssh.service"));
        assert!(similar("cron", "crond.service"));
        assert!(!similar("cron", "dbus.service"));
        assert_eq!(exe_name(b"/usr/bin/env"), b"env");
        assert_eq!(exe_name(b"env"), b"env");
    }
}
