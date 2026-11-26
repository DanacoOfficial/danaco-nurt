// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Property lists: identity of every corpus file, XML read as the `plist`
//! crate reads it, a semantic round trip of binary property lists generated
//! from the XML samples, and changes of single values that leave every
//! other value and byte of the file as it was.
//!
//! The corpus is `tests/d5/corpus/plist`: XML in tabs, two and four spaces,
//! CRLF, comments, CDATA, character references and a byte order mark, and
//! ASCII property lists with comments, strings files, typed GNUstep values,
//! hexadecimal data and escapes. Binary property lists are written by the
//! `plist` crate from the XML samples.

// Helper functions of the tests report failures by unwrapping and
// panicking, as the test functions themselves do.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};

use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::{Bytes, Date, DateKind, Fields, Value};
use nurt_doc::formats::plist::{BINARY_REWRITTEN, PlistDocument, PlistKind};
use nurt_doc::model::MapSemantics;
use nurt_doc::registry::{DocOptions, Registry};
use nurt_doc::value::{apply_remove, apply_set};

/// Semantics of key paths in property lists.
const SEMANTICS: MapSemantics = MapSemantics::LAST_WINS;

/// Text that needs escaping in XML and quoting in ASCII property lists.
const CHANGED_TEXT: &str = "changed & <quoted> \"value\"";

/// Sample with a CDATA section. The `plist` crate reads CDATA sections as
/// empty text, so the sample is left out of the comparisons with the crate
/// and checked by `cdata_sections_read_as_character_data`.
const CDATA_SAMPLE: &str = "xml_cdata.plist";

fn corpus() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../tests/d5/corpus/plist")
}

fn files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(corpus())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext == "plist" || ext == "entitlements")
        })
        .collect();
    files.sort();
    files
}

fn documents(kind: PlistKind) -> Vec<(PathBuf, Vec<u8>, PlistDocument)> {
    files()
        .into_iter()
        .filter_map(|path| {
            let bytes = std::fs::read(&path).unwrap();
            let document = PlistDocument::parse(&bytes)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            (document.kind() == kind).then_some((path, bytes, document))
        })
        .collect()
}

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).unwrap()
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

/// Engine value of a property list value, as the format reads it.
fn engine_value(value: &plist::Value) -> Value {
    match value {
        plist::Value::String(text) => Value::Text(text.clone()),
        plist::Value::Integer(integer) => integer
            .as_signed()
            .map_or_else(|| Value::BigInt(integer.to_string()), Value::Int),
        plist::Value::Real(number) => Value::Number(*number),
        plist::Value::Boolean(flag) => Value::Bool(*flag),
        plist::Value::Date(date) => Value::Date(Date {
            kind: DateKind::OffsetDateTime,
            text: date.to_xml_format(),
        }),
        plist::Value::Data(bytes) => Value::Bytes(Bytes::new(bytes.clone())),
        plist::Value::Uid(uid) => Value::Int(i64::try_from(uid.get()).unwrap()),
        plist::Value::Array(items) => Value::List(items.iter().map(engine_value).collect()),
        plist::Value::Dictionary(dictionary) => Value::Map(
            dictionary
                .iter()
                .map(|(name, item)| (name.clone(), engine_value(item)))
                .collect::<Fields>(),
        ),
        _ => Value::Null,
    }
}

/// Every dictionary key in document order, depth first.
fn key_order(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Map(fields) => {
            for (name, item) in fields {
                out.push(name.clone());
                key_order(item, out);
            }
        }
        Value::List(items) => items.iter().for_each(|item| key_order(item, out)),
        _ => {}
    }
}

fn binary(value: &plist::Value) -> Vec<u8> {
    let mut bytes = Vec::new();
    value.to_writer_binary(&mut bytes).unwrap();
    bytes
}

fn plist_at<'a>(root: &'a mut plist::Value, key: &KeyPath) -> &'a mut plist::Value {
    key.segments()
        .iter()
        .fold(root, |value, segment| match (value, segment) {
            (plist::Value::Dictionary(dictionary), KeySegment::Key(name)) => {
                dictionary.get_mut(name).unwrap()
            }
            (plist::Value::Array(items), KeySegment::Index(index)) => {
                &mut items[usize::try_from(*index).unwrap()]
            }
            (other, segment) => panic!("{segment:?} does not apply to {other:?}"),
        })
}

/// Scalars of a document with their keys.
fn scalars(document: &PlistDocument) -> Vec<(KeyPath, Value)> {
    let Ok(items) = document.read(&key("**")) else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter(|item| !matches!(item.value, Value::Map(_) | Value::List(_)))
        .map(|item| (item.key, item.value))
        .collect()
}

fn assert_outside_unchanged(source: &[u8], after: &[u8], first: usize, last: usize, name: &str) {
    assert_eq!(
        &after[..first],
        &source[..first],
        "{name}: bytes before the change"
    );
    let tail = source.len() - last;
    assert_eq!(
        &after[after.len() - tail..],
        &source[last..],
        "{name}: bytes after the change"
    );
}

#[test]
fn corpus_has_xml_and_ascii_files() {
    assert!(documents(PlistKind::Xml).len() >= 18);
    assert!(documents(PlistKind::Ascii).len() >= 20);
}

#[test]
fn every_file_is_detected_and_opened_with_its_bytes() {
    let registry = Registry::builtin();
    for path in files() {
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            registry.detect(Some(&path), &bytes),
            Some(DocFormatId::PLIST),
            "{}",
            path.display()
        );
        let document = registry
            .open(&DocFormatId::PLIST, &bytes, &DocOptions::default())
            .unwrap();
        assert_eq!(document.bytes(), bytes, "{}", path.display());
    }
    let xml = std::fs::read(corpus().join("xml_nested.plist")).unwrap();
    assert_eq!(registry.detect(None, &xml), Some(DocFormatId::PLIST));
}

#[test]
fn xml_files_read_as_the_plist_crate_reads_them() {
    for (path, bytes, document) in documents(PlistKind::Xml) {
        if path.ends_with("xml_duplicate_keys.plist") || path.ends_with(CDATA_SAMPLE) {
            continue;
        }
        let reference = plist::Value::from_reader_xml(bytes.as_slice()).unwrap();
        let expected = engine_value(&reference);
        assert_eq!(document.to_value(), expected, "{}", path.display());
        let (mut ours, mut theirs) = (Vec::new(), Vec::new());
        key_order(&document.to_value(), &mut ours);
        key_order(&expected, &mut theirs);
        assert_eq!(ours, theirs, "{}", path.display());
    }
}

#[test]
fn cdata_sections_read_as_character_data() {
    let bytes = std::fs::read(corpus().join(CDATA_SAMPLE)).unwrap();
    let document = PlistDocument::parse(&bytes).unwrap();
    assert_eq!(
        document.read(&key("expr")).unwrap()[0].value,
        text("a < b && c")
    );
}

#[test]
fn ascii_scalars_take_dictionaries_and_arrays() {
    let source = b"{\n\ta = 1;\n\tb = x;\n}\n";
    let document = PlistDocument::parse(source).unwrap();
    let mut fields = Fields::new();
    fields.insert("inner".to_owned(), Value::Int(1));
    let after = document
        .set(&key("a"), &Value::Map(fields), false)
        .unwrap()
        .apply(source);
    let reopened = PlistDocument::parse(&after).unwrap();
    assert_eq!(reopened.read(&key("a.inner")).unwrap()[0].value, text("1"));
    assert_eq!(reopened.read(&key("b")).unwrap()[0].value, text("x"));
    let list = Value::List(vec![text("p"), text("q")]);
    let after = document.set(&key("b"), &list, false).unwrap().apply(source);
    let reopened = PlistDocument::parse(&after).unwrap();
    assert_eq!(reopened.read(&key("b")).unwrap()[0].value, list);
    assert_eq!(reopened.read(&key("a")).unwrap()[0].value, text("1"));
}

#[test]
fn repeated_dictionary_keys_select_the_last_entry() {
    let bytes = std::fs::read(corpus().join("xml_duplicate_keys.plist")).unwrap();
    let document = PlistDocument::parse(&bytes).unwrap();
    assert_eq!(document.read(&key("a")).unwrap()[0].value, Value::Int(2));
    let change = document.set(&key("a"), &Value::Int(3), false).unwrap();
    let text = String::from_utf8(change.apply(&bytes)).unwrap();
    assert!(
        text.contains("<integer>1</integer>") && text.contains("<integer>3</integer>"),
        "{text}"
    );
}

#[test]
fn ascii_files_read_the_expected_values() {
    let read = |name: &str, path: &str| {
        let bytes = std::fs::read(corpus().join(name)).unwrap();
        PlistDocument::parse(&bytes)
            .unwrap()
            .read(&key(path))
            .unwrap()[0]
            .value
            .clone()
    };
    let cases = [
        ("ascii_gnustep_typed.plist", "count", Value::Int(12)),
        ("ascii_gnustep_typed.plist", "ratio", Value::Number(0.5)),
        ("ascii_gnustep_typed.plist", "flag", Value::Bool(true)),
        ("ascii_gnustep_typed.plist", "off", Value::Bool(false)),
        (
            "ascii_hex_data.plist",
            "blob",
            Value::Bytes(Bytes::new(vec![0x0f, 0xa1, 0xb2, 0xc3, 0xd4, 0xe5])),
        ),
        (
            "ascii_hex_data.plist",
            "empty",
            Value::Bytes(Bytes::new(Vec::new())),
        ),
        ("ascii_quoted_escapes.plist", "path", text("C:\\dir")),
        ("ascii_quoted_escapes.plist", "text", text("line\nnext")),
        ("ascii_quoted_escapes.plist", "uni", text("\u{e9}")),
        ("ascii_quoted_escapes.plist", "octal", text("A")),
        ("ascii_quoted_escapes.plist", "quote", text("say \"hi\"")),
        ("ascii_strings_file.plist", "farewell", text("Bye")),
        ("ascii_strings_file.plist", "title", text("Main")),
        (
            "ascii_comments.plist",
            "c",
            Value::List(vec![text("d"), text("e")]),
        ),
        (
            "ascii_project_like.plist",
            "objects.1A2B3C.isa",
            text("PBXBuildFile"),
        ),
        (
            "ascii_project_like.plist",
            "objects.4D5E6F.sourceTree",
            text("<group>"),
        ),
        ("ascii_single_quoted.plist", "name", text("single quoted")),
        ("ascii_trailing_comma.plist", "[2]", text("c")),
        (
            "ascii_root_array.plist",
            "[2]",
            Value::Bytes(Bytes::new(vec![1])),
        ),
        ("ascii_unicode.plist", "tokyo", text("\u{6771}\u{4eac}")),
        ("ascii_info.plist", "LSRequiresIPhoneOS", text("YES")),
    ];
    for (name, path, expected) in cases {
        assert_eq!(read(name, path), expected, "{name} {path}");
    }
}

#[test]
fn binary_round_trip_keeps_values_types_and_key_order() {
    for (path, bytes, xml) in documents(PlistKind::Xml) {
        if path.ends_with(CDATA_SAMPLE) {
            continue;
        }
        let reference = plist::Value::from_reader_xml(bytes.as_slice()).unwrap();
        let encoded = binary(&reference);
        let document = PlistDocument::parse(&encoded).unwrap();
        assert_eq!(document.kind(), PlistKind::Binary);
        assert_eq!(document.source(), encoded.as_slice());
        assert_eq!(document.to_value(), xml.to_value(), "{}", path.display());
        let (mut ours, mut theirs) = (Vec::new(), Vec::new());
        key_order(&document.to_value(), &mut ours);
        key_order(&xml.to_value(), &mut theirs);
        assert_eq!(ours, theirs, "{}", path.display());
    }
}

/// New value of the same type as `value`.
fn other_value(value: &Value) -> Option<Value> {
    Some(match value {
        Value::Text(_) => text(CHANGED_TEXT),
        Value::Int(number) => Value::Int(number.wrapping_add(1)),
        Value::Number(_) => Value::Int(2),
        Value::Bool(flag) => Value::Bool(!flag),
        Value::Bytes(_) => Value::Bytes(Bytes::new(vec![9, 8, 7])),
        Value::Date(_) => Value::Date(Date {
            kind: DateKind::OffsetDateTime,
            text: "2027-02-01T00:00:00Z".to_owned(),
        }),
        _ => return None,
    })
}

/// Property list value stored for `value` over `old`.
fn stored(value: &Value, old: &plist::Value) -> plist::Value {
    match (value, old) {
        (Value::Int(number), plist::Value::Real(_)) => {
            plist::Value::Real(f64::from(i32::try_from(*number).unwrap()))
        }
        (Value::Int(number), plist::Value::Uid(_)) => {
            plist::Value::Uid(plist::Uid::new(u64::try_from(*number).unwrap()))
        }
        (Value::Int(number), _) => plist::Value::Integer((*number).into()),
        (Value::Text(text), _) => plist::Value::String(text.clone()),
        (Value::Bool(flag), _) => plist::Value::Boolean(*flag),
        (Value::Bytes(bytes), _) => plist::Value::Data(bytes.as_slice().to_vec()),
        (Value::Date(date), _) => {
            plist::Value::Date(plist::Date::from_xml_format(&date.text).unwrap())
        }
        (other, _) => panic!("no property list value for {other:?}"),
    }
}

#[test]
fn binary_changes_keep_every_other_value_and_type() {
    for (path, bytes, _) in documents(PlistKind::Xml) {
        if path.ends_with("xml_duplicate_keys.plist") {
            continue;
        }
        let reference = plist::Value::from_reader_xml(bytes.as_slice()).unwrap();
        let encoded = binary(&reference);
        let document = PlistDocument::parse(&encoded).unwrap();
        for (concrete, current) in scalars(&document) {
            let Some(new) = other_value(&current) else {
                continue;
            };
            let change = document.set(&concrete, &new, false).unwrap();
            assert_eq!(
                change.flags,
                [BINARY_REWRITTEN],
                "{} {concrete}",
                path.display()
            );
            let after = change.apply(&encoded);
            assert_eq!(
                document
                    .set(&concrete, &new, false)
                    .unwrap()
                    .apply(&encoded),
                after,
                "deterministic"
            );
            let mut expected = reference.clone();
            let slot = plist_at(&mut expected, &concrete);
            *slot = stored(&new, slot);
            assert_eq!(
                plist::Value::from_reader(std::io::Cursor::new(&after)).unwrap(),
                expected,
                "{} {concrete}",
                path.display()
            );
        }
    }
}

#[test]
fn binary_removals_insertions_and_appends() {
    let reference = plist::Value::from_file(corpus().join("xml_nested.plist")).unwrap();
    let encoded = binary(&reference);
    let document = PlistDocument::parse(&encoded).unwrap();
    let removed = document.remove(&key("level1.level2.items[0]")).unwrap();
    let after = PlistDocument::parse(&removed.apply(&encoded)).unwrap();
    assert_eq!(
        after.read(&key("level1.level2.items[0].name")).unwrap()[0].value,
        text("b")
    );
    let inserted = document
        .set(&key("level1.created.deep"), &Value::Int(1), true)
        .unwrap();
    let after = PlistDocument::parse(&inserted.apply(&encoded)).unwrap();
    assert_eq!(
        after.read(&key("level1.created.deep")).unwrap()[0].value,
        Value::Int(1)
    );
    let appended = document
        .set(&key("level1.level2.items[+]"), &text("x"), false)
        .unwrap();
    let after = PlistDocument::parse(&appended.apply(&encoded)).unwrap();
    assert_eq!(
        after.read(&key("level1.level2.items[2]")).unwrap()[0].value,
        text("x")
    );
    let error = document
        .set(&key("level1.missing"), &Value::Int(1), false)
        .unwrap_err();
    assert_eq!(error.code().as_str(), "E_KEY_NOT_FOUND");
}

#[test]
fn uid_values_keep_their_type() {
    let mut dictionary = plist::Dictionary::new();
    dictionary.insert("$top".into(), plist::Value::Uid(plist::Uid::new(1)));
    dictionary.insert("count".into(), plist::Value::Integer(4.into()));
    let encoded = binary(&plist::Value::Dictionary(dictionary));
    let document = PlistDocument::parse(&encoded).unwrap();
    let items = document.read(&key("$top")).unwrap();
    assert_eq!(
        (items[0].value.clone(), items[0].tag.as_deref()),
        (Value::Int(1), Some("uid"))
    );
    let after = document
        .set(&key("$top"), &Value::Int(5), false)
        .unwrap()
        .apply(&encoded);
    let value = plist::Value::from_reader(std::io::Cursor::new(after)).unwrap();
    assert_eq!(
        value.as_dictionary().unwrap().get("$top"),
        Some(&plist::Value::Uid(plist::Uid::new(5)))
    );
    let after = document
        .set(&key("$top"), &text("x"), false)
        .unwrap()
        .apply(&encoded);
    let value = plist::Value::from_reader(std::io::Cursor::new(after)).unwrap();
    assert_eq!(
        value.as_dictionary().unwrap().get("$top"),
        Some(&plist::Value::String("x".into()))
    );
}

#[test]
fn binary_versions_other_than_00_are_rejected() {
    let error = PlistDocument::parse(b"bplist15\x00\x00\x00").unwrap_err();
    assert_eq!(error.code().as_str(), "E_UNSUPPORTED_CONSTRUCT");
    assert_eq!(error.found.as_deref(), Some("plist.binary_layout"));
    let error = PlistDocument::parse(b"bplist00\x01\x02").unwrap_err();
    assert_eq!(error.code().as_str(), "E_DOC_PARSE");
}

#[test]
fn text_changes_touch_only_the_changed_value() {
    for kind in [PlistKind::Xml, PlistKind::Ascii] {
        for (path, bytes, document) in documents(kind) {
            let name = path.display().to_string();
            let original = document.to_value();
            for (concrete, _) in scalars(&document) {
                let change = document.set(&concrete, &text(CHANGED_TEXT), false).unwrap();
                assert_eq!(change.edits.len(), 1, "{name} {concrete}");
                let after = change.apply(&bytes);
                let edit = &change.edits[0];
                assert_outside_unchanged(&bytes, &after, edit.range.start, edit.range.end, &name);
                let mut expected = original.clone();
                apply_set(&mut expected, &concrete, text(CHANGED_TEXT), &SEMANTICS).unwrap();
                let reopened = PlistDocument::parse(&after)
                    .unwrap_or_else(|error| panic!("{name} {concrete}: {error}"));
                assert_eq!(reopened.to_value(), expected, "{name} {concrete}");
            }
        }
    }
}

#[test]
fn removing_any_value_removes_only_that_value() {
    for kind in [PlistKind::Xml, PlistKind::Ascii] {
        for (path, bytes, document) in documents(kind) {
            let name = path.display().to_string();
            let original = document.to_value();
            let Ok(items) = document.read(&key("**")) else {
                continue;
            };
            for item in items
                .into_iter()
                .filter(|item| !item.key.segments().is_empty())
            {
                let change = document.remove(&item.key).unwrap();
                let after = change.apply(&bytes);
                let mut expected = original.clone();
                apply_remove(&mut expected, &item.key, &SEMANTICS).unwrap();
                let reopened = PlistDocument::parse(&after)
                    .unwrap_or_else(|error| panic!("{name} {}: {error}", item.key));
                assert_eq!(reopened.to_value(), expected, "{name} {}", item.key);
            }
        }
    }
}

#[test]
fn setting_the_current_value_changes_nothing() {
    for kind in [PlistKind::Xml, PlistKind::Ascii] {
        for (path, _, document) in documents(kind) {
            let Ok(items) = document.read(&key("**")) else {
                continue;
            };
            for item in items {
                // An entry whose key is repeated later in its dictionary is
                // not the entry its key selects.
                if document.read(&item.key).unwrap()[0].value != item.value {
                    continue;
                }
                let change = document.set(&item.key, &item.value, false).unwrap();
                assert!(change.is_empty(), "{} {}", path.display(), item.key);
            }
        }
    }
}

#[test]
fn xml_type_changes_replace_the_value_element() {
    let bytes = std::fs::read(corpus().join("xml_scalars.plist")).unwrap();
    let document = PlistDocument::parse(&bytes).unwrap();
    let apply = |path: &str, value: &Value| {
        String::from_utf8(
            document
                .set(&key(path), value, false)
                .unwrap()
                .apply(&bytes),
        )
        .unwrap()
    };
    let source = String::from_utf8(bytes.clone()).unwrap();
    assert_eq!(
        apply("count", &text("8")),
        source.replace("<integer>7</integer>", "<string>8</string>")
    );
    assert_eq!(
        apply("count", &Value::Bool(true)),
        source.replace("<integer>7</integer>", "<true/>")
    );
    assert_eq!(
        apply("ratio", &Value::Int(4)),
        source.replace("<real>3.25</real>", "<real>4</real>")
    );
    assert_eq!(
        apply("enabled", &Value::Bool(true)),
        source.replace("<false/>", "<true/>")
    );
    assert_eq!(
        apply("empty", &text("now")),
        source.replace("<string></string>", "<string>now</string>")
    );
    let mut fields = Fields::new();
    fields.insert("inner".to_owned(), Value::Int(1));
    let nested = apply("text", &Value::Map(fields));
    assert!(nested.contains("\t<key>text</key>\n\t<dict>\n\t\t<key>inner</key>\n\t\t<integer>1</integer>\n\t</dict>\n"), "{nested}");
}

#[test]
fn new_entries_follow_the_indentation_and_line_breaks_of_siblings() {
    let set = |name: &str, path: &str, value: &Value| {
        let bytes = std::fs::read(corpus().join(name)).unwrap();
        let document = PlistDocument::parse(&bytes).unwrap();
        String::from_utf8(document.set(&key(path), value, true).unwrap().apply(&bytes)).unwrap()
    };
    let two = set("xml_unicode_two_spaces.plist", "list[+]", &text("x"));
    assert!(
        two.contains("<string>åäö</string>\n    <string>x</string>\n  </array>"),
        "{two}"
    );
    let four = set("xml_escapes_four_spaces.plist", "added", &Value::Int(1));
    assert!(
        four.contains("</string>\n    <key>added</key>\n    <integer>1</integer>\n</dict>"),
        "{four}"
    );
    let crlf = set(
        "xml_launchd_crlf.plist",
        "EnvironmentVariables.HOME",
        &text("/home/agent"),
    );
    assert!(crlf.contains("<string>/usr/bin:/bin</string>\r\n\t\t<key>HOME</key>\r\n\t\t<string>/home/agent</string>\r\n\t</dict>"), "{crlf}");
    let ascii = set(
        "ascii_nested_four_spaces.plist",
        "level1.level2.added",
        &text("two words"),
    );
    assert!(
        ascii.contains("            flag = YES;\n            added = \"two words\";\n        };"),
        "{ascii}"
    );
    let ascii_crlf = set(
        "ascii_launchd_crlf.plist",
        "ProgramArguments[+]",
        &text("--port"),
    );
    assert!(
        ascii_crlf.contains("\t\t\"--serve\",\r\n\t\t--port\r\n\t);"),
        "{ascii_crlf}"
    );
    let strings = set("ascii_strings_file.plist", "added", &text("New"));
    assert!(
        strings.ends_with("farewell = Bye;\nadded = New;\n"),
        "{strings}"
    );
}

#[test]
fn removing_entries_takes_their_lines() {
    let remove = |name: &str, path: &str| {
        let bytes = std::fs::read(corpus().join(name)).unwrap();
        let document = PlistDocument::parse(&bytes).unwrap();
        String::from_utf8(document.remove(&key(path)).unwrap().apply(&bytes)).unwrap()
    };
    let xml = remove("Info.plist", "CFBundleName");
    assert!(
        xml.contains("<string>com.example.app</string>\n\t<key>CFBundleVersion</key>"),
        "{xml}"
    );
    let ascii = remove("ascii_info.plist", "CFBundleName");
    assert!(
        ascii.contains("CFBundleIdentifier = com.example.app;\n\tCFBundleVersion = 42;"),
        "{ascii}"
    );
    let items = remove("xml_array_root.plist", "[*]");
    assert!(
        items.contains("<plist version=\"1.0\">\n<array>\n</array>\n</plist>"),
        "{items}"
    );
}

#[test]
fn invalid_keys_and_values_are_rejected() {
    let bytes = std::fs::read(corpus().join("xml_scalars.plist")).unwrap();
    let document = PlistDocument::parse(&bytes).unwrap();
    let cases = [
        (
            document
                .set(&key("missing"), &Value::Int(1), false)
                .unwrap_err(),
            "E_KEY_NOT_FOUND",
        ),
        (
            document
                .set(&key("text.deeper"), &Value::Int(1), true)
                .unwrap_err(),
            "E_KEY_CONFLICT",
        ),
        (
            document.set(&key("text"), &Value::Null, false).unwrap_err(),
            "E_TYPE",
        ),
        (
            document
                .set(
                    &key("created"),
                    &Value::Date(Date {
                        kind: DateKind::LocalDate,
                        text: "2026-01-31".into(),
                    }),
                    false,
                )
                .unwrap_err(),
            "E_INVALID_VALUE",
        ),
        (
            document.remove(&KeyPath::root()).unwrap_err(),
            "E_KEY_CONFLICT",
        ),
        (
            document.read(&key("nothing")).unwrap_err(),
            "E_KEY_NOT_FOUND",
        ),
        (
            PlistDocument::parse(b"<plist><foo/></plist>").unwrap_err(),
            "E_DOC_PARSE",
        ),
        (PlistDocument::parse(b"{ a = ").unwrap_err(), "E_DOC_PARSE"),
    ];
    for (error, code) in cases {
        assert_eq!(error.code().as_str(), code, "{}", error.message);
    }
}

/// `plutil` from the search path, when installed (macOS).
fn plutil() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("plutil"))
        .find(|candidate| candidate.is_file())
}

#[test]
fn json_of_plutil_matches_the_read_values_skipped_without_plutil() {
    let Some(plutil) = plutil() else { return };
    let (mut compared, mut skipped) = (0_usize, Vec::new());
    for kind in [PlistKind::Xml, PlistKind::Ascii] {
        for (path, _, document) in documents(kind) {
            let output = std::process::Command::new(&plutil)
                .args(["-convert", "json", "-o", "-"])
                .arg(&path)
                .output()
                .unwrap();
            if !output.status.success() {
                skipped.push(path.display().to_string());
                continue;
            }
            compared += 1;
            let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            let expected = Value::from_plain_json(json);
            assert!(
                nurt_doc::value::equivalent(&document.to_value(), &expected),
                "{}",
                path.display()
            );
        }
    }
    // `plutil` refuses a file whose values JSON cannot hold (dates, data); a
    // refusal of most files would leave the comparison without a subject.
    assert!(
        compared * 3 >= compared + skipped.len(),
        "plutil refused {} of {} files: {skipped:?}",
        skipped.len(),
        compared + skipped.len()
    );
}
