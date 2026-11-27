// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syslog/tests/eventlog_query.rs
// Responsibility:   Building of XPath queries and parsing and mapping of event XML.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

//! Building of XPath queries and parsing and mapping of event XML.

use std::fs;
use std::path::Path;

use nurt_contracts::record::LogLevel;
use nurt_contracts::value::Instant;
use nurt_syslog::eventlog::channels::nearest;
use nurt_syslog::eventlog::level::{priority, xpath_condition};
use nurt_syslog::eventlog::record::{Formatted, map_event, raw_message};
use nurt_syslog::eventlog::xml::{EventXml, EventXmlError};
use nurt_syslog::eventlog::xpath::{XPathFilter, literal};

fn fixture(name: &str) -> String {
    let path = Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("tests/fixtures/eventlog")
    .join(name);
    fs::read_to_string(path).unwrap()
}

fn instant(text: &str) -> Instant {
    text.parse().unwrap()
}

#[test]
fn eventlog_xpath_level_since_until() {
    let filter = XPathFilter {
        level: LogLevel::Warning,
        since: Some(instant("2026-11-21T10:00:00.0004Z")),
        until: Some(instant("2026-11-21T12:00:00.0004Z")),
        ..XPathFilter::default()
    };
    assert_eq!(
        filter.to_xpath(),
        "*[System[(Level=1 or Level=2 or Level=3) and \
         TimeCreated[@SystemTime>='2026-11-21T10:00:00.000Z' and \
         @SystemTime<'2026-11-21T12:00:00.001Z']]]"
    );
}

#[test]
fn eventlog_xpath_keeps_whole_milliseconds() {
    let filter = XPathFilter {
        level: LogLevel::Debug,
        since: Some(instant("2026-11-21T10:00:00Z")),
        until: Some(instant("2026-11-21T11:00:00.250Z")),
        ..XPathFilter::default()
    };
    assert_eq!(
        filter.to_xpath(),
        "*[System[TimeCreated[@SystemTime>='2026-11-21T10:00:00.000Z' and \
         @SystemTime<'2026-11-21T11:00:00.250Z']]]"
    );
    let only_until = XPathFilter {
        level: LogLevel::Debug,
        until: Some(instant("2026-11-21T11:00:00Z")),
        ..XPathFilter::default()
    };
    assert_eq!(
        only_until.to_xpath(),
        "*[System[TimeCreated[@SystemTime<'2026-11-21T11:00:00.000Z']]]"
    );
}

#[test]
fn eventlog_xpath_provider_escape() {
    let with = |provider| {
        XPathFilter {
            level: LogLevel::Debug,
            provider: Some(provider),
            ..XPathFilter::default()
        }
        .to_xpath()
    };
    assert_eq!(with("Example"), "*[System[Provider[@Name='Example']]]");
    assert_eq!(with("Bob's"), "*[System[Provider[@Name=\"Bob's\"]]]");
    assert_eq!(with("a'b\"c"), "*");
    assert_eq!(literal(""), Some("''".to_owned()));
}

#[test]
fn eventlog_xpath_record_bound_and_levels() {
    let filter = XPathFilter {
        level: LogLevel::Critical,
        before_record: Some(12_345),
        ..XPathFilter::default()
    };
    assert_eq!(
        filter.to_xpath(),
        "*[System[(Level=1) and EventRecordID<12345]]"
    );
    assert_eq!(
        xpath_condition(LogLevel::Info).as_deref(),
        Some("(Level<5 or Level>5)")
    );
    assert_eq!(
        XPathFilter::default().to_xpath(),
        "*[System[(Level<5 or Level>5)]]"
    );
}

#[test]
fn eventlog_level_maps_to_priority() {
    let cases = [
        (0, 6),
        (1, 2),
        (2, 3),
        (3, 4),
        (4, 6),
        (5, 7),
        (16, 6),
        (255, 6),
    ];
    for (level, expected) in cases {
        assert_eq!(priority(level), expected, "Level {level}");
    }
}

#[test]
fn eventlog_xml_named_data_and_system_properties() {
    let xml = EventXml::parse(&fixture("service_state.xml")).unwrap();
    assert_eq!(xml.provider, "Example-Service-Manager");
    assert_eq!(xml.event_id, Some(7036));
    assert_eq!(xml.qualifiers.as_deref(), Some("16384"));
    assert_eq!(xml.level.as_deref(), Some("4"));
    assert_eq!(xml.keywords.as_deref(), Some("0x8080000000000000"));
    assert_eq!(xml.time_created, instant("2026-11-21T10:15:30.1234567Z"));
    assert_eq!(xml.record_id, 1001);
    assert_eq!(
        xml.activity_id.as_deref(),
        Some("{00000000-0000-0000-0000-0000000000aa}")
    );
    assert_eq!((xml.process_id, xml.thread_id), (Some(812), Some(4400)));
    assert_eq!(xml.channel.as_deref(), Some("System"));
    assert_eq!(xml.computer.as_deref(), Some("host.example.com"));
    assert_eq!(xml.user_id.as_deref(), Some("S-1-5-18"));
    assert_eq!(
        xml.data,
        [
            ("param1".to_owned(), "Example Service".to_owned()),
            ("param2".to_owned(), "running".to_owned()),
        ]
    );
}

#[test]
fn eventlog_xml_unnamed_data_binary_and_references() {
    let xml = EventXml::parse(&fixture("unnamed_data.xml")).unwrap();
    assert_eq!(
        xml.data,
        [
            ("Data1".to_owned(), "\\Device\\Harddisk0\\DR0".to_owned()),
            ("Data2".to_owned(), "C:\\Temp & data A<".to_owned()),
            ("Binary".to_owned(), "0300480001".to_owned()),
        ]
    );
    assert_eq!(xml.user_id, None);
}

#[test]
fn eventlog_xml_user_data_leaves() {
    let xml = EventXml::parse(&fixture("user_data.xml")).unwrap();
    assert_eq!(
        xml.data,
        [
            ("SubjectUserName".to_owned(), "user".to_owned()),
            ("SubjectDomainName".to_owned(), "EXAMPLE".to_owned()),
            ("Reason".to_owned(), "manual".to_owned()),
        ]
    );
    assert_eq!(xml.channel.as_deref(), Some("Security"));
}

#[test]
fn eventlog_xml_rejects_invalid_documents() {
    assert_eq!(EventXml::parse("<Other/>"), Err(EventXmlError::NotAnEvent));
    assert_eq!(EventXml::parse(""), Err(EventXmlError::NotAnEvent));
    assert_eq!(
        EventXml::parse("<Event><System><EventRecordID>1</EventRecordID></System></Event>"),
        Err(EventXmlError::Missing("TimeCreated/@SystemTime"))
    );
    assert_eq!(
        EventXml::parse(
            "<Event><System><TimeCreated SystemTime='2026-11-21T10:00:00Z'/>\
             <EventRecordID>x</EventRecordID></System></Event>"
        ),
        Err(EventXmlError::Missing("EventRecordID"))
    );
    assert!(matches!(
        EventXml::parse("<Event><System></Event>"),
        Err(EventXmlError::Syntax(_))
    ));
    assert!(matches!(
        EventXml::parse("<Event><System>"),
        Err(EventXmlError::Syntax(_))
    ));
    assert_eq!(
        EventXml::parse("<Event/><Event/>"),
        Err(EventXmlError::NotAnEvent)
    );
}

#[test]
fn eventlog_unformatted_raw_message() {
    let xml = EventXml::parse(&fixture("service_state.xml")).unwrap();
    let event = map_event(&xml, Formatted::default(), "System");
    assert_eq!(event.message, "param1=Example Service; param2=running");
    assert!(event.flags.unformatted);
    assert_eq!(event.category, None);
    assert_eq!(raw_message(&[]), "");
}

#[test]
fn eventlog_formatted_event_maps_every_field() {
    let xml = EventXml::parse(&fixture("service_state.xml")).unwrap();
    let formatted = Formatted {
        message: Some("The Example Service service entered the running state.".to_owned()),
        task: Some("None".to_owned()),
    };
    let event = map_event(&xml, formatted, "system");
    assert_eq!(event.source, "Example-Service-Manager");
    assert_eq!(event.level(), LogLevel::Info);
    assert_eq!(event.id.as_deref(), Some("7036"));
    assert_eq!(event.unit.as_deref(), Some("System"));
    assert_eq!(event.host.as_deref(), Some("host.example.com"));
    assert_eq!((event.pid, event.tid), (Some(812), Some(4400)));
    assert_eq!(event.user.as_deref(), Some("S-1-5-18"));
    assert_eq!(event.category.as_deref(), Some("None"));
    assert!(!event.flags.unformatted);
    assert_eq!(event.cursor.to_string(), "v1:w:System:1001");
    assert_eq!(event.store.as_str(), "event_log");
    let names: Vec<&str> = event.fields.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        [
            "param1",
            "param2",
            "EventRecordID",
            "Level",
            "Task",
            "Opcode",
            "Keywords",
            "Qualifiers",
            "Version",
            "ProviderGuid",
            "ActivityID"
        ]
    );
    let record = event.to_record();
    assert_eq!(record.fields.unwrap()["Keywords"], "0x8080000000000000");
}

#[test]
fn eventlog_event_without_channel_uses_queried_channel() {
    let xml = EventXml::parse(
        "<Event><System><Provider Name='Example'/><EventID>70000</EventID>\
         <TimeCreated SystemTime='2026-11-21T10:00:00Z'/><EventRecordID>5</EventRecordID>\
         </System></Event>",
    )
    .unwrap();
    let event = map_event(&xml, Formatted::default(), "Application");
    assert_eq!(event.unit.as_deref(), Some("Application"));
    assert_eq!(event.id.as_deref(), Some("4464"));
    assert_eq!(event.level(), LogLevel::Info);
    assert_eq!(event.message, "");
}

#[test]
fn eventlog_channel_candidates_rank_containment_then_distance() {
    let available: Vec<String> = [
        "Application",
        "HardwareEvents",
        "Microsoft-Windows-Example/Operational",
        "Security",
        "Setup",
        "System",
    ]
    .map(String::from)
    .to_vec();
    assert_eq!(nearest("Sytem", &available)[0], "System");
    assert_eq!(
        nearest("EXAMPLE", &available)[0],
        "Microsoft-Windows-Example/Operational"
    );
    assert_eq!(nearest("x", &available).len(), available.len());
    let many: Vec<String> = (0..20).map(|n| format!("Channel{n}")).collect();
    assert_eq!(nearest("Channel", &many).len(), 8);
}

const EVENT_HEAD: &str = "<Event><System><Provider Name='Example'/><EventID>1</EventID>\
                          <TimeCreated SystemTime='2026-11-21T10:00:00Z'/>\
                          <EventRecordID>5</EventRecordID></System>";

#[test]
fn eventlog_xml_cdata_and_character_references() {
    let xml = EventXml::parse(&format!(
        "{EVENT_HEAD}<EventData><Data Name='a'><![CDATA[x<y & z]]></Data>\
         <Data>&#65;&#x42;&amp;&lt;&unknown;</Data></EventData></Event>"
    ))
    .unwrap();
    assert_eq!(
        xml.data,
        [
            ("a".to_owned(), "x<y & z".to_owned()),
            ("Data1".to_owned(), "AB&<&unknown;".to_owned()),
        ]
    );
}

#[test]
fn eventlog_xml_empty_data_elements_and_numbering_of_unnamed_data() {
    let xml = EventXml::parse(&format!(
        "{EVENT_HEAD}<EventData><Data Name='x'/><Data/><Data Name='y'>1</Data>\
         <Data>v</Data><ComplexData><Data>ignored</Data></ComplexData></EventData></Event>"
    ))
    .unwrap();
    assert_eq!(
        xml.data,
        [
            ("x".to_owned(), String::new()),
            ("Data1".to_owned(), String::new()),
            ("y".to_owned(), "1".to_owned()),
            ("Data2".to_owned(), "v".to_owned()),
        ]
    );
}

#[test]
fn eventlog_xml_keeps_white_space_of_values() {
    let xml = EventXml::parse(&format!(
        "{EVENT_HEAD}<EventData><Data Name='a'>  two  words\n</Data></EventData></Event>"
    ))
    .unwrap();
    assert_eq!(xml.data, [("a".to_owned(), "  two  words\n".to_owned())]);
}

#[test]
fn eventlog_xml_matches_elements_by_local_name() {
    let xml = EventXml::parse(
        "<ns:Event xmlns:ns='urn:example'><ns:System><ns:Provider ns:Name='P'/>\
         <ns:TimeCreated SystemTime='2026-11-21T10:00:00Z'/>\
         <ns:EventRecordID>9</ns:EventRecordID></ns:System></ns:Event>",
    )
    .unwrap();
    assert_eq!(xml.provider, "P");
    assert_eq!(xml.record_id, 9);
}

#[test]
fn eventlog_xml_values_that_are_not_numbers_are_absent() {
    let xml = EventXml::parse(
        "<Event><System><Provider Name='P'/><EventID>abc</EventID>\
         <TimeCreated SystemTime='2026-11-21T10:00:00Z'/><EventRecordID>1</EventRecordID>\
         <Execution ProcessID='x' ThreadID='-1'/></System></Event>",
    )
    .unwrap();
    assert_eq!(xml.event_id, None);
    assert_eq!((xml.process_id, xml.thread_id), (None, None));
    let event = map_event(&xml, Formatted::default(), "System");
    assert_eq!(event.id, None);
    assert_eq!(event.level(), LogLevel::Info);
}

#[test]
fn eventlog_xml_with_an_invalid_time_has_no_event() {
    assert_eq!(
        EventXml::parse(
            "<Event><System><TimeCreated SystemTime='yesterday'/>\
             <EventRecordID>1</EventRecordID></System></Event>"
        ),
        Err(EventXmlError::Missing("TimeCreated/@SystemTime"))
    );
}

#[test]
fn eventlog_xml_of_a_second_root_or_text_before_it_is_rejected() {
    assert!(EventXml::parse(&format!("{EVENT_HEAD}</Event><Event/>")).is_err());
    assert!(EventXml::parse("text").is_err());
}

#[test]
fn eventlog_xml_does_not_expand_declared_entities() {
    let xml = EventXml::parse(&format!(
        "<!DOCTYPE Event [<!ENTITY big 'aaaaaaaaaa'>]>{EVENT_HEAD}\
         <EventData><Data Name='a'>&big;</Data></EventData></Event>"
    ))
    .unwrap();
    assert_eq!(xml.data, [("a".to_owned(), "&big;".to_owned())]);
}

#[test]
fn eventlog_event_level_text_with_white_space_or_junk_maps_to_info() {
    let build = |level: &str| {
        let xml = EventXml::parse(&format!(
            "<Event><System><Provider Name='P'/><Level>{level}</Level>\
             <TimeCreated SystemTime='2026-11-21T10:00:00Z'/>\
             <EventRecordID>1</EventRecordID></System></Event>"
        ))
        .unwrap();
        map_event(&xml, Formatted::default(), "System").priority
    };
    assert_eq!(build(" 2 "), 3);
    assert_eq!(build("x"), 6);
    assert_eq!(build("300"), 6);
    assert_eq!(build(""), 6);
}

#[test]
fn eventlog_xpath_rounds_instants_before_the_epoch_outwards() {
    let filter = XPathFilter {
        level: LogLevel::Debug,
        since: Some(Instant::from_unix_nanos(-1)),
        until: Some(Instant::from_unix_nanos(-1)),
        ..XPathFilter::default()
    };
    assert_eq!(
        filter.to_xpath(),
        "*[System[TimeCreated[@SystemTime>='1969-12-31T23:59:59.999Z' and \
         @SystemTime<'1970-01-01T00:00:00.000Z']]]"
    );
}

#[test]
fn eventlog_xpath_terms_follow_a_fixed_order() {
    let filter = XPathFilter {
        level: LogLevel::Error,
        since: Some(instant("2026-11-21T10:00:00Z")),
        provider: Some("Example"),
        before_record: Some(9),
        ..XPathFilter::default()
    };
    assert_eq!(
        filter.to_xpath(),
        "*[System[(Level=1 or Level=2) and \
         TimeCreated[@SystemTime>='2026-11-21T10:00:00.000Z'] and \
         Provider[@Name='Example'] and EventRecordID<9]]"
    );
    let unusable = XPathFilter {
        level: LogLevel::Debug,
        provider: Some("a'b\"c"),
        before_record: Some(1),
        ..XPathFilter::default()
    };
    assert_eq!(unusable.to_xpath(), "*[System[EventRecordID<1]]");
}

#[test]
fn eventlog_channel_candidates_of_degenerate_input() {
    assert!(nearest("System", &[]).is_empty());
    let available = ["b", "a"].map(String::from);
    assert_eq!(nearest("", &available), ["a", "b"]);
    let accents = ["\u{c9}v\u{e9}nements".to_owned(), "System".to_owned()];
    assert_eq!(
        nearest("\u{e9}V\u{c9}", &accents)[0],
        "\u{c9}v\u{e9}nements"
    );
}

#[test]
fn eventlog_priority_of_every_level_is_in_the_range_of_priorities() {
    for level in 0..=u8::MAX {
        assert!(priority(level) <= 7, "Level {level}");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
