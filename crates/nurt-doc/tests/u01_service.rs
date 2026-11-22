// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Format registry, encodings and the document service over the
//! `key = value` test format.
#![cfg(feature = "testing")]
// Helpers outside `#[test]` functions fail the test on invalid fixtures.
#![allow(clippy::expect_used)]

use std::collections::HashSet;
use std::path::Path;

use nurt_contracts::config::DocumentsConfig;
use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::{DocFormatId, DocService, SerializeOptions};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_doc::model::DocHandle;
use nurt_doc::registry::{
    Confidence, DocFormat, DocOptions, FormatsFn, InputKind, Registry, TextEncoding, open_with,
};
use nurt_doc::service::DocumentService;
use nurt_doc::testing::KeyValueFormat;
use serde_json::json;

const SAMPLE: &[u8] = b"# port\nport = 8080\nname = web\n";

fn id(name: &str) -> DocFormatId {
    DocFormatId::new(name).expect("valid format identifier")
}

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).expect("valid key path")
}

/// Test format with configurable names, extensions and sniffing; documents
/// are opened as `key = value` text.
struct Named {
    id: &'static str,
    extensions: &'static [&'static str],
    names: &'static [&'static str],
    sniffed: Confidence,
}

impl DocFormat for Named {
    fn id(&self) -> DocFormatId {
        id(self.id)
    }

    fn extensions(&self) -> &[&'static str] {
        self.extensions
    }

    fn file_names(&self) -> &[&'static str] {
        self.names
    }

    fn sniff(&self, _content: &[u8]) -> Confidence {
        self.sniffed
    }

    fn input(&self) -> InputKind {
        InputKind::Bytes
    }

    fn editable(&self) -> bool {
        false
    }

    fn open(&self, content: Vec<u8>, options: &DocOptions) -> Result<Box<dyn DocHandle>> {
        KeyValueFormat::new(self.id()).open(content, options)
    }
}

fn named(
    id: &'static str,
    extensions: &'static [&'static str],
    names: &'static [&'static str],
) -> Box<dyn DocFormat> {
    Box::new(Named {
        id,
        extensions,
        names,
        sniffed: Confidence::None,
    })
}

fn detection_registry() -> Registry {
    Registry::with([
        named("json", &["json"], &[]),
        named(
            "jsonc",
            &["jsonc"],
            &[".vscode/*.json", ".devcontainer/*.json", "tsconfig.json"],
        ),
        named("env", &[], &[".env", ".env.*"]),
        named("toml", &["toml"], &["cargo.lock"]),
        named("xml", &["xml"], &["pom.xml"]),
    ])
}

fn detected(registry: &Registry, path: &str) -> Option<String> {
    registry
        .detect(Some(Path::new(path)), b"")
        .map(|format| format.to_string())
}

#[test]
fn file_name_patterns_win_over_extensions() {
    let registry = detection_registry();
    assert_eq!(
        detected(&registry, "/work/app/.vscode/settings.json").as_deref(),
        Some("jsonc")
    );
    assert_eq!(
        detected(&registry, "/work/.devcontainer/devcontainer.json").as_deref(),
        Some("jsonc")
    );
    assert_eq!(
        detected(&registry, "/work/tsconfig.json").as_deref(),
        Some("jsonc")
    );
    assert_eq!(
        detected(&registry, "/work/settings.json").as_deref(),
        Some("json")
    );
    assert_eq!(
        detected(&registry, "/work/.vscode/notes/a.json").as_deref(),
        Some("json")
    );
    assert_eq!(detected(&registry, "/work/.env").as_deref(), Some("env"));
    assert_eq!(
        detected(&registry, "/work/.env.local").as_deref(),
        Some("env")
    );
    assert_eq!(
        detected(&registry, "/work/Cargo.lock").as_deref(),
        Some("toml")
    );
    assert_eq!(detected(&registry, "/work/POM.XML").as_deref(), Some("xml"));
    assert_eq!(
        detected(&registry, "relative/.vscode/launch.json").as_deref(),
        Some("jsonc")
    );
    assert_eq!(detected(&registry, "/work/readme.md"), None);
}

#[test]
fn content_decides_between_formats_of_one_extension_and_without_name() {
    let registry = Registry::with([
        Box::new(Named {
            id: "low",
            extensions: &["conf"],
            names: &[],
            sniffed: Confidence::Low,
        }) as Box<dyn DocFormat>,
        Box::new(Named {
            id: "high",
            extensions: &["conf"],
            names: &[],
            sniffed: Confidence::High,
        }),
    ]);
    assert_eq!(
        detected(&registry, "/etc/app.conf").as_deref(),
        Some("high")
    );
    assert_eq!(
        registry
            .detect(None, b"x")
            .map(|format| format.to_string())
            .as_deref(),
        Some("high")
    );
    let weak = Registry::with([Box::new(Named {
        id: "low",
        extensions: &[],
        names: &[],
        sniffed: Confidence::Low,
    }) as Box<dyn DocFormat>]);
    assert_eq!(weak.detect(None, b"x"), None);
}

#[test]
fn single_format_of_an_extension_needs_no_sniffing() {
    let registry = detection_registry();
    assert_eq!(
        detected(&registry, "/work/data.toml").as_deref(),
        Some("toml")
    );
}

#[test]
fn format_parameter_is_checked_against_registry() {
    let registry = detection_registry();
    assert_eq!(
        registry
            .resolve_format(Some("toml"), None, b"")
            .expect("known format"),
        id("toml")
    );
    let unknown = registry
        .resolve_format(Some("ini"), None, b"")
        .expect_err("ini is not registered");
    assert_eq!(unknown.code, ErrorCode::UnknownValue);
    assert!(
        unknown.message.contains("json, jsonc, env, toml, xml"),
        "{}",
        unknown.message
    );
    let undetected = registry
        .resolve_format(None, Some(Path::new("/work/readme.md")), b"text")
        .expect_err("no format");
    assert_eq!(undetected.code, ErrorCode::Format);
    assert_eq!(
        registry
            .resolve_format(None, Some(Path::new("/work/a.json")), b"")
            .expect("detected"),
        id("json")
    );
}

#[test]
fn registering_same_identifier_replaces_format() {
    let mut registry = detection_registry();
    registry.register(named("json", &["json5"], &[]));
    let infos = registry.infos();
    assert_eq!(infos.len(), 5);
    let json = infos
        .iter()
        .find(|info| info.id == id("json"))
        .expect("json is registered");
    assert_eq!(json.extensions, ["json5"]);
    assert!(!json.editable);
    assert!(registry.get(&id("yaml")).is_none());
    assert!(format!("{registry:?}").contains("jsonc"));
}

fn key_value_formats() -> Vec<Box<dyn DocFormat>> {
    vec![Box::new(KeyValueFormat::new(id("kv")))]
}

#[test]
fn registration_functions_provide_formats_in_order() {
    let functions: &[FormatsFn] = &[key_value_formats];
    let registry = Registry::from_functions(functions);
    assert_eq!(registry.infos().len(), 1);
    assert!(
        registry
            .get(&id("kv"))
            .is_some_and(|format| format.editable())
    );
}

#[test]
fn built_in_formats_have_distinct_identifiers() {
    let infos = Registry::builtin().infos();
    let ids: HashSet<String> = infos.iter().map(|info| info.id.to_string()).collect();
    assert_eq!(ids.len(), infos.len());
}

#[test]
fn opening_enforces_size_limit() {
    let format = KeyValueFormat::new(id("kv"));
    let options = DocOptions {
        max_bytes: 8,
        ..DocOptions::default()
    };
    let error = open_with(&format, SAMPLE, &options).expect_err("larger than the limit");
    assert_eq!(error.code, ErrorCode::Limit);
    assert_eq!(error.details.get("limit"), Some(&json!(8)));
}

#[test]
fn text_encodings_are_decoded_and_restored() {
    let format = KeyValueFormat::new(id("kv"));
    let options = DocOptions::default();
    let mut with_bom = b"\xEF\xBB\xBF".to_vec();
    with_bom.extend_from_slice(SAMPLE);
    let mut utf16 = b"\xFF\xFE".to_vec();
    utf16.extend(
        String::from_utf8_lossy(SAMPLE)
            .encode_utf16()
            .flat_map(u16::to_le_bytes),
    );
    let mut utf16be = b"\xFE\xFF".to_vec();
    utf16be.extend(
        String::from_utf8_lossy(SAMPLE)
            .encode_utf16()
            .flat_map(u16::to_be_bytes),
    );
    for (bytes, encoding) in [
        (SAMPLE.to_vec(), TextEncoding::Utf8),
        (with_bom, TextEncoding::Utf8Bom),
        (utf16, TextEncoding::Utf16Le),
        (utf16be, TextEncoding::Utf16Be),
    ] {
        let document = open_with(&format, &bytes, &options).expect("document opens");
        assert_eq!(document.encoding, encoding);
        assert_eq!(document.handle.text(), SAMPLE);
        assert_eq!(document.bytes(), bytes, "{}", encoding.as_str());
        assert!(format!("{document:?}").contains("encoding"));
    }
}

#[test]
fn invalid_encodings_are_rejected_with_offset() {
    let invalid_utf8 = TextEncoding::decode(b"ab\xFFcd").expect_err("invalid UTF-8");
    assert_eq!(invalid_utf8.code, ErrorCode::Encoding);
    assert_eq!(invalid_utf8.details.get("byte"), Some(&json!(2)));
    let after_bom = TextEncoding::decode(b"\xEF\xBB\xBFa\xC3").expect_err("truncated sequence");
    assert_eq!(after_bom.details.get("byte"), Some(&json!(4)));
    let odd = TextEncoding::decode(b"\xFF\xFEa\x00b").expect_err("odd length");
    assert_eq!(odd.details.get("byte"), Some(&json!(4)));
    let lone_surrogate =
        TextEncoding::decode(b"\xFF\xFE\x00\xD8a\x00").expect_err("lone surrogate");
    assert_eq!(lone_surrogate.code, ErrorCode::Encoding);
}

#[test]
fn options_come_from_configuration_and_never_resolve_external_entities() {
    let options = DocOptions::from_config(&DocumentsConfig::default());
    assert!(!options.xml_external_entities());
    assert_eq!(DocOptions::default().max_bytes, 256 * 1024 * 1024);
}

fn service() -> DocumentService {
    DocumentService::new(Registry::with(key_value_formats()), DocOptions::default())
}

#[test]
fn service_lists_and_detects_formats() {
    let service = service();
    assert_eq!(service.formats().len(), 1);
    assert_eq!(
        service.detect_format(Some(Path::new("/srv/app.kv")), b""),
        Some(id("kv"))
    );
    assert_eq!(service.options().max_bytes, DocOptions::default().max_bytes);
    assert_eq!(service.registry().infos().len(), 1);
}

#[test]
fn read_keys_returns_items_with_value_span_and_comment() {
    let items = service()
        .read_keys(SAMPLE, &id("kv"), &key("port"), None)
        .expect("key reads");
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.key.to_string(), "port");
    assert_eq!(item.value, Value::Int(8080));
    assert_eq!(item.comment.as_deref(), Some("port"));
    assert_eq!(item.document, None);
    let span = item.span.as_ref().expect("span");
    assert_eq!((span.bytes.start, span.bytes.end), (14, 18));
    assert_eq!((span.start.line, span.start.column), (2, 8));
    let all = service()
        .read_keys(SAMPLE, &id("kv"), &key("[*]"), Some(0))
        .expect("pattern reads");
    assert_eq!(all.len(), 2);
    assert!(
        service()
            .read_keys(SAMPLE, &id("kv"), &key("missing"), None)
            .expect("reads")
            .is_empty()
    );
}

#[test]
fn read_keys_reports_byte_ranges_of_the_file_encoding() {
    let mut with_bom = b"\xEF\xBB\xBF".to_vec();
    with_bom.extend_from_slice(SAMPLE);
    let items = service()
        .read_keys(&with_bom, &id("kv"), &key("port"), None)
        .expect("key reads");
    let span = items[0].span.as_ref().expect("span");
    assert_eq!((span.bytes.start, span.bytes.end), (17, 21));
    let mut utf16 = b"\xFE\xFF".to_vec();
    utf16.extend(
        String::from_utf8_lossy(SAMPLE)
            .encode_utf16()
            .flat_map(u16::to_be_bytes),
    );
    let items = service()
        .read_keys(&utf16, &id("kv"), &key("port"), None)
        .expect("key reads");
    let span = items[0].span.as_ref().expect("span");
    assert_eq!((span.bytes.start, span.bytes.end), (30, 38));
}

#[test]
fn read_keys_rejects_unknown_format() {
    let error = service()
        .read_keys(SAMPLE, &id("yaml"), &key("a"), None)
        .expect_err("not registered");
    assert_eq!(error.code, ErrorCode::UnknownValue);
}

#[test]
fn check_syntax_reports_issue_position() {
    let service = service();
    assert!(
        service
            .check_syntax(SAMPLE, &id("kv"))
            .expect("checks")
            .is_empty()
    );
    let issues = service
        .check_syntax(b"a = 1\noops\n", &id("kv"))
        .expect("checks");
    assert_eq!(issues.len(), 1);
    assert_eq!(
        (issues[0].span.start.line, issues[0].span.start.column),
        (2, 1)
    );
    assert_eq!(issues[0].kind.as_deref(), Some("e_doc_parse"));
    let encoding = service
        .check_syntax(b"a = \xFF\n", &id("kv"))
        .expect("checks");
    assert_eq!(encoding[0].kind.as_deref(), Some("e_encoding"));
    assert_eq!(encoding[0].span.bytes.start, 4);
    let unknown = service
        .check_syntax(SAMPLE, &id("yaml"))
        .expect_err("not registered");
    assert_eq!(unknown.code, ErrorCode::UnknownValue);
}

fn records() -> Vec<Record> {
    vec![
        Record::new(RecordKind::Item)
            .with("key", Value::Text("a".to_owned()))
            .with("value", Value::Int(1)),
        Record::new(RecordKind::Item)
            .with("key", Value::Text("b".to_owned()))
            .with("note", Value::Text("x, y".to_owned())),
    ]
}

fn serialized(format: &str, options: &SerializeOptions) -> String {
    let bytes = service()
        .serialize_records(&records(), &id(format), options)
        .expect("records serialize");
    String::from_utf8(bytes).expect("output is UTF-8")
}

#[test]
fn records_serialize_as_indented_and_compact_json() {
    let pretty = serialized("json", &SerializeOptions::default());
    assert_eq!(
        pretty,
        "[\n  {\n    \"key\": \"a\",\n    \"value\": 1\n  },\n  {\n    \"key\": \"b\",\n    \"note\": \"x, y\"\n  }\n]\n"
    );
    let compact = serialized(
        "json",
        &SerializeOptions {
            pretty: false,
            ..SerializeOptions::default()
        },
    );
    assert_eq!(
        compact,
        "[{\"key\":\"a\",\"value\":1},{\"key\":\"b\",\"note\":\"x, y\"}]\n"
    );
    let parsed: serde_json::Value = serde_json::from_str(&pretty).expect("valid JSON");
    assert_eq!(parsed[1]["note"], json!("x, y"));
}

#[test]
fn records_serialize_as_block_yaml_and_toml_tables() {
    assert_eq!(
        serialized("yaml", &SerializeOptions::default()),
        "- key: a\n  value: 1\n- key: b\n  note: x, y\n"
    );
    assert_eq!(
        serialized("toml", &SerializeOptions::default()),
        "[[item]]\nkey = \"a\"\nvalue = 1\n\n[[item]]\nkey = \"b\"\nnote = \"x, y\"\n"
    );
}

#[test]
fn records_serialize_as_tables_with_union_of_fields() {
    assert_eq!(
        serialized("csv", &SerializeOptions::default()),
        "key,value,note\r\na,1,\r\nb,,\"x, y\"\r\n"
    );
    assert_eq!(
        serialized("tsv", &SerializeOptions::default()),
        "key\tvalue\tnote\na\t1\t\nb\t\tx, y\n"
    );
    let options = SerializeOptions {
        header: false,
        delimiter: Some(';'),
        ..SerializeOptions::default()
    };
    assert_eq!(serialized("csv", &options), "a;1;\r\nb;;x, y\r\n");
}

#[test]
fn empty_record_lists_serialize_as_empty_documents() {
    let service = service();
    let options = SerializeOptions::default();
    let empty = |format: &str| {
        String::from_utf8(
            service
                .serialize_records(&[], &id(format), &options)
                .expect("serializes"),
        )
        .expect("UTF-8")
    };
    assert_eq!(empty("json"), "[]\n");
    assert_eq!(empty("yaml"), "[]\n");
    assert_eq!(empty("toml"), "");
    assert_eq!(empty("csv"), "");
}

#[test]
fn serialization_rejects_unknown_format_and_non_ascii_delimiter() {
    let service = service();
    let unknown = service
        .serialize_records(&records(), &id("xml"), &SerializeOptions::default())
        .expect_err("xml is not a record format");
    assert_eq!(unknown.code, ErrorCode::InvalidValue);
    let options = SerializeOptions {
        delimiter: Some('§'),
        ..SerializeOptions::default()
    };
    let delimiter = service
        .serialize_records(&records(), &id("csv"), &options)
        .expect_err("non-ASCII delimiter");
    assert_eq!(delimiter.code, ErrorCode::InvalidValue);
}

#[test]
fn yaml_output_quotes_text_that_would_be_typed() {
    let record = Record::new(RecordKind::Item)
        .with("flag", Value::Text("yes".to_owned()))
        .with("empty", Value::Text(String::new()))
        .with(
            "nested",
            Value::from_plain_json(json!({"a": [1, {"b": null}], "c": {}})),
        );
    let yaml = String::from_utf8(
        service()
            .serialize_records(&[record], &id("yaml"), &SerializeOptions::default())
            .expect("serializes"),
    )
    .expect("UTF-8");
    assert_eq!(
        yaml,
        "- flag: \"yes\"\n  empty: \"\"\n  nested:\n    a:\n      - 1\n      - b: null\n    c: {}\n"
    );
}
