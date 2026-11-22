// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Violations of the format `nurt-program-catalog/1` found by the reader:
//! each case writes a small catalog to a temporary directory, reads it in
//! the strict mode of validation and expects one finding.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code fails loudly on broken fixtures"
)]

use std::path::Path;

use nurt_programs::catalog::guide::{FrontMatter, FrontValue, parse_front_matter, split_guide};
use nurt_programs::catalog::{
    self, CatalogErrorKind, Diagnostic, DiagnosticCode, Limits, LoadedCatalog, ReadOptions,
    Severity,
};
use nurt_programs::check::validate_catalog;
use serde_json::{Value, json};

const GUIDE: &str = "---\nname: tool\ndescription: Use the tool.\nversion: \"1.0\"\nprograms: [tool]\n---\n## Usage\n\ntool --help\n";

fn tool() -> Value {
    json!({"name": "tool", "category": "misc", "summary": "Does one thing.", "guide": "guides/tool.md"})
}

/// Index with the category `misc`, the program `tool` and the members of
/// `extra`, which replace members of the same name.
fn index(extra: Value) -> Value {
    let mut index = json!({
        "format": "nurt-program-catalog/1",
        "categories": [{"id": "misc", "title": "Miscellaneous"}],
        "programs": [tool()],
    });
    for (key, value) in extra.as_object().unwrap() {
        index[key] = value.clone();
    }
    index
}

fn programs(extra: &[Value]) -> Value {
    let mut list = vec![tool()];
    list.extend(extra.iter().cloned());
    json!({ "programs": list })
}

fn write(root: &Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// Catalog written to a temporary directory: the index, the guide of `tool`
/// and further files that may replace it.
struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new(index: &Value, files: &[(&str, &[u8])]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "catalog.json", index.to_string().as_bytes());
        write(dir.path(), "guides/tool.md", GUIDE.as_bytes());
        for (relative, bytes) in files {
            write(dir.path(), relative, bytes);
        }
        Self { dir }
    }

    fn read(&self, limits: Limits) -> LoadedCatalog {
        catalog::read_catalog(
            self.dir.path(),
            &ReadOptions {
                limits,
                strict: true,
            },
        )
        .unwrap()
    }
}

fn found(diagnostics: &[Diagnostic], code: DiagnosticCode, needle: &str) -> bool {
    diagnostics.iter().any(|diagnostic| {
        diagnostic.code == code
            && (diagnostic.file.contains(needle)
                || diagnostic
                    .field
                    .as_deref()
                    .is_some_and(|field| field.contains(needle))
                || diagnostic
                    .name
                    .as_deref()
                    .is_some_and(|name| name.contains(needle))
                || diagnostic.message.contains(needle))
    })
}

/// One faulty catalog: description, index, further files and the expected
/// finding.
struct Case {
    label: &'static str,
    index: Value,
    files: Vec<(&'static str, &'static [u8])>,
    code: DiagnosticCode,
    needle: &'static str,
}

fn case(label: &'static str, extra: Value, code: DiagnosticCode, needle: &'static str) -> Case {
    Case {
        label,
        index: index(extra),
        files: Vec::new(),
        code,
        needle,
    }
}

fn with_files(mut case: Case, files: &[(&'static str, &'static [u8])]) -> Case {
    case.files = files.to_vec();
    case
}

fn run(cases: Vec<Case>) {
    for case in cases {
        let loaded = Fixture::new(&case.index, &case.files).read(Limits::default());
        assert!(
            found(&loaded.diagnostics, case.code, case.needle),
            "{}: expected {:?} with `{}` in {:#?}",
            case.label,
            case.code,
            case.needle,
            loaded.diagnostics
        );
    }
}

use DiagnosticCode::{ControlCharacter, EntryInvalid, GuideMismatch, UnknownField};

#[test]
fn reports_faults_of_the_index_header() {
    run(vec![
        case("invalid id", json!({"id": "Bad Id!"}), EntryInvalid, "id"),
        case(
            "invalid language",
            json!({"languages": ["english!"]}),
            EntryInvalid,
            "languages",
        ),
        case(
            "short synonym group",
            json!({"synonyms": [["a"]]}),
            EntryInvalid,
            "synonyms[0]",
        ),
        case(
            "synonym group not a list",
            json!({"synonyms": ["a"]}),
            EntryInvalid,
            "synonyms[0]",
        ),
        case(
            "synonym not text",
            json!({"synonyms": [["a", 1]]}),
            EntryInvalid,
            "synonyms[0][1]",
        ),
        case(
            "include not text",
            json!({"include": [1]}),
            EntryInvalid,
            "include[0]",
        ),
        case(
            "include absolute",
            json!({"include": ["/etc/part.json"]}),
            EntryInvalid,
            "include[0]",
        ),
        case(
            "include of the index",
            json!({"include": ["catalog.json"]}),
            EntryInvalid,
            "include[0]",
        ),
        case(
            "include twice",
            json!({"include": ["a.json", "a.json"]}),
            EntryInvalid,
            "include[1]",
        ),
        case(
            "include missing",
            json!({"include": ["parts/missing.json"]}),
            EntryInvalid,
            "parts/missing.json",
        ),
        case(
            "extension not an object",
            json!({"x": 1}),
            EntryInvalid,
            "x",
        ),
        case(
            "unknown member",
            json!({"colour": "red"}),
            UnknownField,
            "colour",
        ),
        case(
            "guides not a list",
            json!({"guides": 5}),
            EntryInvalid,
            "guides",
        ),
    ]);
}

#[test]
fn reports_faults_of_categories() {
    let misc = json!({"id": "misc", "title": "Miscellaneous"});
    run(vec![
        case(
            "category not an object",
            json!({"categories": [misc, 42]}),
            EntryInvalid,
            "categories[1]",
        ),
        case(
            "category twice",
            json!({"categories": [misc, misc]}),
            EntryInvalid,
            "already declared",
        ),
        case(
            "parent undeclared",
            json!({"categories": [misc, {"id": "sub", "title": "Sub", "parent": "none"}]}),
            EntryInvalid,
            "parent",
        ),
        case(
            "three levels",
            json!({"categories": [
                misc,
                {"id": "sub", "title": "Sub", "parent": "misc"},
                {"id": "deeper", "title": "Deeper", "parent": "sub"}
            ]}),
            EntryInvalid,
            "subcategory",
        ),
    ]);
}

#[test]
fn reports_faults_of_program_entries() {
    let entry = |extra: Value| {
        let mut program =
            json!({"name": "other", "category": "misc", "summary": "Another program."});
        for (key, value) in extra.as_object().unwrap() {
            program[key] = value.clone();
        }
        program
    };
    run(vec![
        case(
            "program twice",
            programs(&[tool()]),
            EntryInvalid,
            "already defined",
        ),
        case(
            "optional and required guide",
            programs(&[entry(
                json!({"guide_optional": true, "guide_required": true}),
            )]),
            EntryInvalid,
            "guide_required",
        ),
        case(
            "unknown alias kind",
            programs(&[entry(
                json!({"aliases": [{"name": "o2", "kind": "cousin"}]}),
            )]),
            UnknownField,
            "kind",
        ),
        case(
            "alias kind not text",
            programs(&[entry(json!({"aliases": [{"name": "o3", "kind": 1}]}))]),
            EntryInvalid,
            "kind",
        ),
        case(
            "alias not a name",
            programs(&[entry(json!({"aliases": [5]}))]),
            EntryInvalid,
            "aliases[0]",
        ),
        case(
            "alias equals a program",
            programs(&[entry(json!({"aliases": ["tool"]}))]),
            EntryInvalid,
            "collides",
        ),
        case(
            "platform not text",
            programs(&[entry(json!({"platforms": [1]}))]),
            EntryInvalid,
            "platforms[0]",
        ),
        case(
            "platform twice",
            programs(&[entry(json!({"platforms": ["linux", "linux"]}))]),
            EntryInvalid,
            "repeated",
        ),
        case(
            "unknown platform",
            programs(&[entry(json!({"platforms": ["plan9"]}))]),
            UnknownField,
            "platforms[0]",
        ),
        case(
            "section without kind",
            programs(&[entry(json!({"sections": [{"text": "x"}]}))]),
            EntryInvalid,
            "kind",
        ),
        case(
            "section kind not text",
            programs(&[entry(json!({"sections": [{"kind": 5, "text": "x"}]}))]),
            EntryInvalid,
            "kind",
        ),
        case(
            "unknown section kind",
            programs(&[entry(
                json!({"sections": [{"kind": "trivia", "text": "x"}]}),
            )]),
            UnknownField,
            "kind",
        ),
        case(
            "section not an object",
            programs(&[entry(json!({"sections": [7]}))]),
            EntryInvalid,
            "sections[0]",
        ),
        case(
            "section without text",
            programs(&[entry(json!({"sections": [{"kind": "purpose"}]}))]),
            EntryInvalid,
            "text",
        ),
        case(
            "task not an object",
            programs(&[entry(json!({"not_for": [3]}))]),
            EntryInvalid,
            "not_for[0]",
        ),
        case(
            "guide not text",
            programs(&[entry(json!({"guide": 3}))]),
            EntryInvalid,
            "guide",
        ),
        case(
            "control character",
            programs(&[entry(json!({"summary": "Bell \u{7} sound."}))]),
            ControlCharacter,
            "summary",
        ),
    ]);
}

#[test]
fn reports_faults_of_parts_and_standalone_guides() {
    let part = json!({"include": ["parts/a.json"]});
    run(vec![
        with_files(
            case("part not JSON", part.clone(), EntryInvalid, "parts/a.json"),
            &[("parts/a.json", b"{")],
        ),
        with_files(
            case(
                "part without programs",
                part.clone(),
                EntryInvalid,
                "programs",
            ),
            &[("parts/a.json", b"{}")],
        ),
        with_files(
            case("part member unknown", part.clone(), UnknownField, "extra"),
            &[("parts/a.json", br#"{"programs": [], "extra": 1}"#)],
        ),
        with_files(
            case("part extension", part.clone(), EntryInvalid, "x"),
            &[("parts/a.json", br#"{"programs": [], "x": 1}"#)],
        ),
        with_files(
            case("part programs not a list", part, EntryInvalid, "programs"),
            &[("parts/a.json", br#"{"programs": 5}"#)],
        ),
        case(
            "guide not an object",
            json!({"guides": [5]}),
            EntryInvalid,
            "guides[0]",
        ),
        case(
            "guide without file",
            json!({"guides": [{"name": "intro"}]}),
            EntryInvalid,
            "guide",
        ),
        case(
            "guide named as a program",
            json!({"guides": [{"name": "tool", "guide": "guides/tool.md"}]}),
            EntryInvalid,
            "already used",
        ),
        case(
            "guide with unknown category",
            json!({"guides": [{"name": "intro", "guide": "guides/tool.md", "category": "none"}]}),
            EntryInvalid,
            "category",
        ),
    ]);
}

#[test]
fn reports_faults_of_guide_files() {
    let plain = || index(json!({}));
    let guide = |label, bytes: &'static [u8], code, needle| Case {
        label,
        index: plain(),
        files: vec![("guides/tool.md", bytes)],
        code,
        needle,
    };
    run(vec![
        guide(
            "no front matter",
            b"text only\n",
            EntryInvalid,
            "guides/tool.md",
        ),
        guide(
            "unclosed front matter",
            b"---\nname: tool\n",
            EntryInvalid,
            "guides/tool.md",
        ),
        guide(
            "invalid name",
            b"---\nname: bad name!\ndescription: d\n---\n",
            EntryInvalid,
            "front matter `name`",
        ),
        guide(
            "name of another guide",
            b"---\nname: other\ndescription: d\nversion: 1\n---\n",
            GuideMismatch,
            "front matter `name`",
        ),
        guide(
            "listed program without the guide",
            b"---\nname: tool\ndescription: d\nversion: 1\nprograms: [tool, ghost]\n---\n",
            GuideMismatch,
            "ghost",
        ),
        guide(
            "program not listed",
            b"---\nname: tool\ndescription: d\nversion: 1\nprograms: [other]\n---\n",
            GuideMismatch,
            "tool",
        ),
        guide(
            "not UTF-8",
            b"---\nname: t\xffol\n---\n",
            EntryInvalid,
            "UTF-8",
        ),
        guide(
            "repeated key",
            b"---\nname: tool\nname: tool\n---\n",
            EntryInvalid,
            "repeated",
        ),
    ]);
}

#[test]
fn reports_faults_of_overlays() {
    let french = || {
        index(
            json!({"languages": ["en", "fr"], "guides": [{"name": "intro", "guide": "guides/intro.md"}]}),
        )
    };
    let intro = (
        "guides/intro.md",
        b"---\nname: intro\ndescription: Start here.\nversion: 1\n---\n".as_slice(),
    );
    let overlay = |label, bytes: &'static [u8], code, needle| Case {
        label,
        index: french(),
        files: vec![intro, ("i18n/fr/catalog.json", bytes)],
        code,
        needle,
    };
    run(vec![
        Case {
            label: "overlay missing",
            index: french(),
            files: vec![intro],
            code: EntryInvalid,
            needle: "languages",
        },
        overlay("overlay not JSON", b"{", EntryInvalid, "i18n/fr/catalog.json"),
        overlay("overlay format", br#"{"format": "other", "language": "fr"}"#, EntryInvalid, "format"),
        overlay("overlay language", br#"{"format": "nurt-program-catalog/1", "language": "de"}"#, EntryInvalid, "language"),
        overlay(
            "overlay member",
            br#"{"format": "nurt-program-catalog/1", "language": "fr", "include": []}"#,
            UnknownField,
            "include",
        ),
        overlay(
            "overlay programs not a list",
            br#"{"format": "nurt-program-catalog/1", "language": "fr", "programs": 5}"#,
            EntryInvalid,
            "programs",
        ),
        overlay(
            "overlay item not an object",
            br#"{"format": "nurt-program-catalog/1", "language": "fr", "programs": [5]}"#,
            EntryInvalid,
            "programs[0]",
        ),
        overlay(
            "overlay category id",
            br#"{"format": "nurt-program-catalog/1", "language": "fr", "categories": [{"id": "Bad Id"}]}"#,
            EntryInvalid,
            "id",
        ),
        with_files(
            overlay(
                "translated guide renamed",
                br#"{"format": "nurt-program-catalog/1", "language": "fr", "programs": [{"name": "tool", "guide": "i18n/fr/guides/outil.md"}]}"#,
                EntryInvalid,
                "translated guide",
            ),
            &[
                intro,
                (
                    "i18n/fr/catalog.json",
                    br#"{"format": "nurt-program-catalog/1", "language": "fr", "programs": [{"name": "tool", "guide": "i18n/fr/guides/outil.md"}]}"#,
                ),
                ("i18n/fr/guides/outil.md", b"---\nname: outil\ndescription: d\nversion: 1\n---\n"),
            ],
        ),
    ]);
}

#[test]
fn overlay_translates_standalone_guides_and_ignores_unknown_entries() {
    let index = index(
        json!({"languages": ["fr"], "guides": [{"name": "intro", "guide": "guides/intro.md"}]}),
    );
    let overlay = json!({
        "format": "nurt-program-catalog/1",
        "language": "fr",
        "categories": [{"id": "misc", "title": "Divers"}, {"id": "absent", "title": "Absent"}],
        "programs": [{"name": "ghost", "summary": "Inconnu."}],
        "guides": [{"name": "intro", "summary": "Commencer ici.", "tags": ["début"], "guide": "i18n/fr/guides/intro.md"}]
    })
    .to_string();
    let fixture = Fixture::new(
        &index,
        &[
            (
                "guides/intro.md",
                b"---\nname: intro\ndescription: Start here.\nversion: 1\n---\n",
            ),
            ("i18n/fr/catalog.json", overlay.as_bytes()),
            (
                "i18n/fr/guides/intro.md",
                b"---\nname: intro\ndescription: Commencer.\nversion: 1\n---\n",
            ),
        ],
    );
    let loaded = fixture.read(Limits::default());
    let french = loaded.catalog.overlay("fr").unwrap();
    assert_eq!(french.categories.len(), 1);
    assert!(french.programs.is_empty());
    let intro = &french.guides["intro"];
    assert_eq!(intro.guide.as_deref(), Some("i18n/fr/guides/intro.md"));
    assert_eq!(intro.tags.as_deref(), Some(["début".to_owned()].as_slice()));
    assert!(
        loaded
            .catalog
            .guide_headers
            .contains_key("i18n/fr/guides/intro.md")
    );
}

#[test]
fn enforces_count_and_size_limits() {
    let index = index(json!({
        "categories": [{"id": "misc", "title": "Miscellaneous"}, {"id": "more", "title": "More"}],
        "guides": [
            {"name": "g1", "guide": "guides/tool.md"},
            {"name": "g2", "guide": "guides/tool.md"}
        ],
        "include": ["parts/a.json"],
        "programs": [tool(), {"name": "second", "category": "misc", "summary": "Second program."}]
    }));
    let part =
        br#"{"programs": [{"name": "third", "category": "misc", "summary": "Third program."}]}"#;
    let fixture = Fixture::new(&index, &[("parts/a.json", part)]);
    let cases = [
        (
            Limits {
                categories: 1,
                ..Limits::default()
            },
            "more than 1 categories",
        ),
        (
            Limits {
                programs: 1,
                ..Limits::default()
            },
            "more than 1 programs",
        ),
        (
            Limits {
                guides: 1,
                ..Limits::default()
            },
            "more than 1 standalone guides",
        ),
        (
            Limits {
                entry_bytes: 60,
                ..Limits::default()
            },
            "larger than 60 bytes",
        ),
        (
            Limits {
                total_bytes: u64::try_from(index.to_string().len()).unwrap() + 10,
                ..Limits::default()
            },
            "in total",
        ),
        (
            Limits {
                part_bytes: 20,
                ..Limits::default()
            },
            "parts/a.json",
        ),
        (
            Limits {
                guide_bytes: 20,
                ..Limits::default()
            },
            "guide is larger",
        ),
    ];
    for (limits, needle) in cases {
        let loaded = fixture.read(limits);
        assert!(
            found(&loaded.diagnostics, EntryInvalid, needle),
            "{needle}: {:#?}",
            loaded.diagnostics
        );
    }
    let deep = Fixture::new(&index, &[]);
    let error = catalog::read_catalog(
        deep.dir.path(),
        &ReadOptions {
            limits: Limits {
                depth: 2,
                ..Limits::default()
            },
            strict: false,
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, CatalogErrorKind::Limit);
}

#[test]
fn reads_catalog_named_by_its_index_and_reports_errors_with_location() {
    let fixture = Fixture::new(
        &index(
            json!({"format_minor": 3, "programs": [{"name": "tool", "category": "misc", "summary": "s", "guide": "guides/tool.md", "colour": "red"}]}),
        ),
        &[],
    );
    let index_path = fixture.dir.path().join("catalog.json");
    let loaded = catalog::read_catalog(
        &index_path,
        &ReadOptions {
            limits: Limits::default(),
            strict: true,
        },
    )
    .unwrap();
    assert_eq!(loaded.catalog.format_minor, 3);
    let unknown = loaded
        .diagnostics
        .iter()
        .find(|item| item.code == UnknownField)
        .unwrap();
    assert_eq!(unknown.severity, Severity::Warning);

    write(fixture.dir.path(), "catalog.json", b"\xff\xfe{}");
    let error = catalog::read_catalog(fixture.dir.path(), &ReadOptions::default()).unwrap_err();
    assert_eq!(error.kind, CatalogErrorKind::Syntax);
    write(fixture.dir.path(), "catalog.json", b"{\n  \"format\": \"nurt-program-catalog/1\",\n  \"categories\": 5,\n  \"programs\": []\n}");
    let error = catalog::read_catalog(fixture.dir.path(), &ReadOptions::default()).unwrap_err();
    assert_eq!(error.kind, CatalogErrorKind::Structure);
    assert_eq!(
        error.to_string(),
        format!("catalog.json:{}: {}", error.line.unwrap(), error.message)
    );
    write(
        fixture.dir.path(),
        "catalog.json",
        br#"{"format": "nurt-program-catalog/x", "categories": [], "programs": []}"#,
    );
    assert_eq!(
        catalog::read_catalog(fixture.dir.path(), &ReadOptions::default())
            .unwrap_err()
            .kind,
        CatalogErrorKind::Format
    );
    write(
        fixture.dir.path(),
        "catalog.json",
        br#"{"format": 1, "categories": [], "programs": []}"#,
    );
    let error = catalog::read_catalog(fixture.dir.path(), &ReadOptions::default()).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("catalog.json: {}", error.message)
    );
}

#[test]
fn check_reports_guides_that_fail_a_full_read() {
    let mut body =
        b"---\nname: tool\ndescription: Use the tool.\nversion: 1\nprograms: [tool]\n---\n"
            .to_vec();
    body.extend([0xff, 0xfe, b'\n']);
    let fixture = Fixture::new(&index(json!({})), &[]);
    write(fixture.dir.path(), "guides/tool.md", &body);
    let report = validate_catalog(fixture.dir.path());
    assert!(!report.is_valid());
    let finding = report
        .diagnostics
        .iter()
        .find(|item| item.file == "guides/tool.md")
        .unwrap();
    assert!(finding.message.contains("UTF-8"), "{finding}");
    let expected = format!(
        "guides/tool.md:1: error [{}]: {}",
        finding.code.as_str(),
        finding.message
    );
    assert_eq!(finding.to_string(), expected);
}

#[test]
fn front_matter_scalars_sequences_and_mappings() {
    let front = "# comment\n\nname: tool\nsummary: first\n  second # note\nquoted: \"a\\n\\t\\\"\\\\\\/\\u00e9\" # c\nsingle: 'it''s'\nempty:\nlist:\n  - a\n  - \"b\"\n  -\nflow: [a, \"b\", 'c']\nnone: []\nmap:\n  maintainer: team\n  url: \"https://example.org\"\n";
    let entries = parse_front_matter(front, 2).unwrap();
    let value = |key: &str| {
        entries
            .iter()
            .find(|(name, _)| name == key)
            .unwrap()
            .1
            .clone()
    };
    let text = |key: &str| FrontValue::Text(key.to_owned());
    assert_eq!(value("summary"), text("first second"));
    assert_eq!(value("quoted"), text("a\n\t\"\\/\u{e9}"));
    assert_eq!(value("single"), text("it's"));
    assert_eq!(value("empty"), text(""));
    assert_eq!(
        value("list"),
        FrontValue::List(vec!["a".to_owned(), "b".to_owned(), String::new()])
    );
    assert_eq!(
        value("flow"),
        FrontValue::List(vec!["a".to_owned(), "b".to_owned(), "c".to_owned()])
    );
    assert_eq!(value("none"), FrontValue::List(Vec::new()));
    assert_eq!(
        value("map"),
        FrontValue::Map(vec![
            ("maintainer".to_owned(), "team".to_owned()),
            ("url".to_owned(), "https://example.org".to_owned())
        ])
    );
}

#[test]
fn front_matter_block_scalars() {
    let front = "literal: |\n  one\n  two\n\nfolded: >\n  one\n  two\n\n  three\nstrip: |-\n  kept\n\nkeep: |+\n  kept\n\nclip_empty: |\nlast: x\n";
    let entries = parse_front_matter(front, 2).unwrap();
    let value = |key: &str| match &entries.iter().find(|(name, _)| name == key).unwrap().1 {
        FrontValue::Text(text) => text.clone(),
        other => panic!("{key}: {other:?}"),
    };
    assert_eq!(value("literal"), "one\ntwo\n");
    assert_eq!(value("folded"), "one two\nthree\n");
    assert_eq!(value("strip"), "kept");
    assert_eq!(value("keep"), "kept\n\n");
    assert_eq!(value("clip_empty"), "");
    assert_eq!(value("last"), "x");
}

#[test]
fn front_matter_rejects_constructs_outside_the_subset() {
    let cases = [
        (" name: x\n", "indentation"),
        ("just text\n", "expected `key: value`"),
        ("a: 1\nb: 2\na: 3\n", "repeated"),
        ("a: \"open\n", "not closed"),
        ("a: 'open\n", "not closed"),
        ("a: \"x\" y\n", "after the closing quote"),
        ("a: \"\\q\"\n", "escape `\\q`"),
        ("a: \"\\uZZZZ\"\n", "not a valid character"),
        ("a: &anchor x\n", "anchors"),
        ("a: |x\n", "anchors"),
        ("a: [x\n", "not closed"),
        ("a: [[x]]\n", "scalars"),
        ("a: [\"x\" y]\n", "expected `,` or `]`"),
        ("a: [] x\n", "after the closing quote"),
        ("a:\n  -x\n", "sequence item"),
        ("a:\n  - \"x\n", "not closed"),
        ("a:\n  b\n", "nested mapping"),
        ("a:\n  b:\n", "deeper"),
        ("a:\n  b: \"x\n", "not closed"),
    ];
    for (front, needle) in cases {
        let error = parse_front_matter(front, 2).unwrap_err();
        assert!(
            error.message.contains(needle),
            "{front:?}: {}",
            error.message
        );
    }
    assert_eq!(
        parse_front_matter("a: 1\nb: 2\na: 3\n", 2)
            .unwrap_err()
            .line,
        4
    );
}

#[test]
fn guide_split_requires_delimiters() {
    assert!(split_guide("no newline", 100).is_err());
    assert!(split_guide("title\n---\n", 100).is_err());
    assert!(split_guide("---\nname: a\n", 100).is_err());
    assert!(split_guide("---\nname: a\n---\nbody", 8).is_err());
    let split = split_guide("\u{feff}---\r\nname: a\r\n---\r\nbody\r\n", 100).unwrap();
    assert_eq!(split.front, "name: a\r\n");
    assert_eq!(split.body, "body\r\n");
}

#[test]
fn front_matter_keys_are_typed() {
    let list =
        |items: &[&str]| FrontValue::List(items.iter().map(|item| (*item).to_owned()).collect());
    let text = |value: &str| FrontValue::Text(value.to_owned());
    let (front, faults) = FrontMatter::from_entries(vec![
        ("name".to_owned(), text("tool")),
        ("description".to_owned(), text("  Describes.  ")),
        ("language".to_owned(), text("pt-BR")),
        ("license".to_owned(), text("CC-BY-4.0")),
        (
            "metadata".to_owned(),
            FrontValue::Map(vec![("a".to_owned(), "b".to_owned())]),
        ),
        ("other".to_owned(), list(&["ignored"])),
    ]);
    assert!(faults.is_empty(), "{faults:?}");
    assert_eq!(front.description.as_deref(), Some("Describes."));
    assert_eq!(front.metadata["a"], "b");
    let invalid = [
        ("name", list(&["x"])),
        ("description", text("   ")),
        ("version", list(&["1"])),
        ("license", text(&"x".repeat(1000))),
        ("language", text("english!")),
        ("programs", text("tool")),
        ("programs", list(&["bad name!"])),
        ("programs", list(&["a", "a"])),
        ("metadata", text("x")),
    ];
    for (key, value) in invalid {
        let (_, faults) = FrontMatter::from_entries(vec![(key.to_owned(), value.clone())]);
        assert!(
            faults.iter().any(|fault| fault.key == key),
            "{key} {value:?}: {faults:?}"
        );
    }
    let (_, faults) = FrontMatter::from_entries(Vec::new());
    let keys: Vec<&str> = faults.iter().map(|fault| fault.key.as_str()).collect();
    assert_eq!(keys, ["name", "description"]);
}

#[test]
fn validates_identifiers_languages_and_dates() {
    assert!(catalog::is_valid_language("en"));
    assert!(catalog::is_valid_language("pt-BR"));
    assert!(!catalog::is_valid_language("e"));
    assert!(!catalog::is_valid_language("en_US"));
    assert!(catalog::is_valid_date("2024-02-29"));
    assert!(!catalog::is_valid_date("2026-13-01"));
    assert!(!catalog::is_valid_date("2026-04-32"));
    assert!(!catalog::is_valid_date("2026-1-01"));
    assert!(catalog::is_valid_catalog_id("media-tools"));
    assert!(!catalog::is_valid_catalog_id("Media Tools"));
    assert!(catalog::is_valid_category_id("video-packaging"));
    assert!(!catalog::is_valid_name(""));
}
