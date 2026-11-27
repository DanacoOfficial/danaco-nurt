// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syslog::eventlog::xml
// Responsibility:   Parser of the XML form of Windows events.
// Dependencies:     nurt_contracts, quick_xml
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parser of the XML form of Windows events.
//!
//! `EvtRender` with `EvtRenderEventXml` renders an event as an `Event`
//! element of the event schema
//! (`http://schemas.microsoft.com/win/2004/08/events/event`) with the system
//! properties in `System` and the payload in `EventData` or `UserData`.
//! Elements are matched by their local names; namespaces are not resolved.

use std::borrow::Cow;

use nurt_contracts::value::Instant;
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};

/// Name of the field for an unnamed `Data` element, followed by its position
/// among the unnamed elements, from 1.
const UNNAMED_DATA_PREFIX: &str = "Data";

/// Error of [`EventXml::parse`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EventXmlError {
    /// The text is not well-formed XML.
    #[error("event XML is not well-formed: {0}")]
    Syntax(String),
    /// The root element is not `Event`.
    #[error("event XML has no `Event` root element")]
    NotAnEvent,
    /// A required system property is missing or has an invalid value.
    #[error("event XML has no valid `{0}`")]
    Missing(&'static str),
}

/// System properties and payload of one event.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EventXml {
    /// `Provider/@Name`.
    pub provider: String,
    /// `Provider/@Guid`.
    pub provider_guid: Option<String>,
    /// `EventID`.
    pub event_id: Option<u32>,
    /// `EventID/@Qualifiers`.
    pub qualifiers: Option<String>,
    /// `Version`.
    pub version: Option<String>,
    /// `Level`, as written.
    pub level: Option<String>,
    /// `Task`.
    pub task: Option<String>,
    /// `Opcode`.
    pub opcode: Option<String>,
    /// `Keywords`, in hexadecimal.
    pub keywords: Option<String>,
    /// `TimeCreated/@SystemTime`.
    pub time_created: Instant,
    /// `EventRecordID`.
    pub record_id: u64,
    /// `Correlation/@ActivityID`.
    pub activity_id: Option<String>,
    /// `Execution/@ProcessID`.
    pub process_id: Option<u64>,
    /// `Execution/@ThreadID`.
    pub thread_id: Option<u64>,
    /// `Channel`.
    pub channel: Option<String>,
    /// `Computer`.
    pub computer: Option<String>,
    /// `Security/@UserID`, a SID in text form.
    pub user_id: Option<String>,
    /// Payload in document order: named and unnamed `Data` and `Binary` of
    /// `EventData`, or the leaf elements of `UserData`.
    pub data: Vec<(String, String)>,
}

impl EventXml {
    /// Parses the XML form of an event.
    ///
    /// # Errors
    ///
    /// [`EventXmlError::Syntax`] for malformed XML,
    /// [`EventXmlError::NotAnEvent`] when the root element is not `Event`,
    /// [`EventXmlError::Missing`] when `TimeCreated/@SystemTime` or
    /// `EventRecordID` is missing or invalid.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_syslog::eventlog::xml::EventXml;
    /// let event = EventXml::parse(
    ///     "<Event><System><Provider Name='Disk'/><EventID>7</EventID><Level>2</Level>\
    ///      <TimeCreated SystemTime='2026-11-21T10:00:00.1234567Z'/>\
    ///      <EventRecordID>42</EventRecordID><Channel>System</Channel></System>\
    ///      <EventData><Data>\\Device\\Harddisk0</Data></EventData></Event>",
    /// )?;
    /// assert_eq!(event.provider, "Disk");
    /// assert_eq!(event.record_id, 42);
    /// assert_eq!(event.data, [("Data1".to_owned(), "\\Device\\Harddisk0".to_owned())]);
    /// # Ok::<(), nurt_syslog::eventlog::xml::EventXmlError>(())
    /// ```
    pub fn parse(xml: &str) -> Result<Self, EventXmlError> {
        let mut parser = Parser::default();
        let mut reader = Reader::from_str(xml);
        loop {
            let event = reader.read_event().map_err(syntax)?;
            match event {
                Event::Start(start) => parser.open(&start)?,
                Event::Empty(start) => {
                    parser.open(&start)?;
                    parser.close();
                }
                Event::End(_) => parser.close(),
                Event::Text(text) => parser.text(&text.xml10_content()),
                Event::CData(data) => parser.text(&data.xml10_content()),
                Event::GeneralRef(reference) => {
                    let name = reference.xml10_content();
                    match reference.resolve_char_ref().map_err(syntax)? {
                        Some(character) => parser.text(character.encode_utf8(&mut [0; 4])),
                        None => parser.text(&entity_text(&name)),
                    }
                }
                Event::Eof => break,
                Event::Comment(_) | Event::Decl(_) | Event::PI(_) | Event::DocType(_) => {}
            }
        }
        parser.finish()
    }
}

fn syntax(error: impl std::fmt::Display) -> EventXmlError {
    EventXmlError::Syntax(error.to_string())
}

fn entity_text(name: &str) -> Cow<'static, str> {
    resolve_predefined_entity(name).map_or_else(|| Cow::Owned(format!("&{name};")), Cow::Borrowed)
}

/// Section of the event that an element belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    System,
    EventData,
    UserData,
    Other,
}

#[derive(Debug, Default)]
struct Frame {
    name: String,
    text: String,
    has_child: bool,
    data_name: Option<String>,
}

#[derive(Debug, Default)]
struct Parser {
    stack: Vec<Frame>,
    event: EventXml,
    time_created: Option<Instant>,
    record_id: Option<u64>,
    unnamed_data: usize,
    seen_root: bool,
}

impl Parser {
    fn section(&self) -> Section {
        match self.stack.get(1).map(|frame| frame.name.as_str()) {
            Some("System") => Section::System,
            Some("EventData") => Section::EventData,
            Some("UserData") => Section::UserData,
            _ => Section::Other,
        }
    }

    fn open(&mut self, start: &BytesStart<'_>) -> Result<(), EventXmlError> {
        let name = local_name(start);
        if self.stack.is_empty() {
            if self.seen_root || name != "Event" {
                return Err(EventXmlError::NotAnEvent);
            }
            self.seen_root = true;
        }
        if let Some(parent) = self.stack.last_mut() {
            parent.has_child = true;
        }
        self.stack.push(Frame {
            name: name.clone(),
            ..Frame::default()
        });
        let section = self.section();
        let depth = self.stack.len();
        for attribute in start.attributes() {
            let attribute = attribute.map_err(syntax)?;
            let key = attribute.key.local_name();
            let value = attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(syntax)?
                .into_owned();
            self.attribute(section, depth, &name, key.as_ref(), value);
        }
        Ok(())
    }

    fn attribute(
        &mut self,
        section: Section,
        depth: usize,
        element: &str,
        key: &str,
        value: String,
    ) {
        let event = &mut self.event;
        match (section, depth, element, key) {
            (Section::System, 3, "Provider", "Name") => event.provider = value,
            (Section::System, 3, "Provider", "Guid") => event.provider_guid = Some(value),
            (Section::System, 3, "EventID", "Qualifiers") => event.qualifiers = Some(value),
            (Section::System, 3, "TimeCreated", "SystemTime") => {
                self.time_created = value.parse().ok();
            }
            (Section::System, 3, "Correlation", "ActivityID") => event.activity_id = Some(value),
            (Section::System, 3, "Execution", "ProcessID") => event.process_id = value.parse().ok(),
            (Section::System, 3, "Execution", "ThreadID") => event.thread_id = value.parse().ok(),
            (Section::System, 3, "Security", "UserID") => event.user_id = Some(value),
            (Section::EventData, 3, "Data", "Name") => {
                if let Some(frame) = self.stack.last_mut() {
                    frame.data_name = Some(value);
                }
            }
            _ => {}
        }
    }

    fn text(&mut self, text: &str) {
        if let Some(frame) = self.stack.last_mut() {
            frame.text.push_str(text);
        }
    }

    fn close(&mut self) {
        let section = self.section();
        let depth = self.stack.len();
        let Some(frame) = self.stack.pop() else {
            return;
        };
        if frame.has_child {
            return;
        }
        match section {
            Section::System if depth == 3 => self.system_value(&frame.name, frame.text),
            Section::EventData if depth == 3 => self.event_data(frame),
            Section::UserData if depth > 2 => self.push_data(frame.name, frame.text),
            Section::System | Section::EventData | Section::UserData | Section::Other => {}
        }
    }

    fn system_value(&mut self, element: &str, text: String) {
        let event = &mut self.event;
        match element {
            "EventID" => event.event_id = text.trim().parse().ok(),
            "Version" => event.version = Some(text),
            "Level" => event.level = Some(text),
            "Task" => event.task = Some(text),
            "Opcode" => event.opcode = Some(text),
            "Keywords" => event.keywords = Some(text),
            "EventRecordID" => self.record_id = text.trim().parse().ok(),
            "Channel" => event.channel = Some(text),
            "Computer" => event.computer = Some(text),
            _ => {}
        }
    }

    fn event_data(&mut self, frame: Frame) {
        match (frame.name.as_str(), frame.data_name) {
            ("Data", Some(name)) => self.push_data(name, frame.text),
            ("Data", None) => {
                self.unnamed_data += 1;
                let name = format!("{UNNAMED_DATA_PREFIX}{}", self.unnamed_data);
                self.push_data(name, frame.text);
            }
            ("Binary", _) => self.push_data(frame.name, frame.text),
            _ => {}
        }
    }

    fn push_data(&mut self, name: String, value: String) {
        self.event.data.push((name, value));
    }

    fn finish(mut self) -> Result<EventXml, EventXmlError> {
        if !self.seen_root {
            return Err(EventXmlError::NotAnEvent);
        }
        if !self.stack.is_empty() {
            return Err(EventXmlError::Syntax("unclosed element".to_owned()));
        }
        self.event.time_created = self
            .time_created
            .ok_or(EventXmlError::Missing("TimeCreated/@SystemTime"))?;
        self.event.record_id = self
            .record_id
            .ok_or(EventXmlError::Missing("EventRecordID"))?;
        Ok(self.event)
    }
}

fn local_name(start: &BytesStart<'_>) -> String {
    let name = start.local_name();
    let name: &str = name.as_ref();
    name.to_owned()
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
