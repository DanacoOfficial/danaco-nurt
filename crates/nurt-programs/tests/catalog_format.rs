// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reader, provider and validation of catalogs in the format
//! `nurt-program-catalog/1`. The examples of the specification are read from
//! `tests/fixtures/catalog/spec` and validated against its JSON schemas;
//! faulty catalogs are written to temporary directories.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code fails loudly on broken fixtures"
)]

use std::path::{Path, PathBuf};

use nurt_contracts::env::Os;
use nurt_programs::catalog::guide::{self, FrontValue};
use nurt_programs::catalog::{
    self, CatalogErrorKind, DiagnosticCode, Limits, PathError, ReadOptions, Severity,
};
use nurt_programs::check::{validate_catalog, validate_catalog_with};
use nurt_programs::model::{AliasKind, DetailWant, EntryKey, RawProgram, SourceData};
use nurt_programs::source::{
    CatalogSpec, Deadline, HostEnv, ProgramSource, SourceId, SourceKind, WarningCode,
};
use nurt_programs::sources::catalog::{self as provider, CatalogProvider, MAX_WARNINGS};
use serde_json::{Value, json};

fn spec_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/catalog/spec")
}

fn example(name: &str) -> PathBuf {
    spec_dir().join("examples").join(name)
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn write(root: &Path, relative: &str, text: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

const GUIDE: &str =
    "---\nname: tool\ndescription: Use the tool.\nversion: \"1.0\"\n---\n## Usage\n\ntool --help\n";

/// Catalog with one valid program `tool` and the given further programs.
fn catalog_with(root: &Path, programs: &[Value]) {
    let mut list = vec![
        json!({"name": "tool", "category": "misc", "summary": "Does one thing.", "guide": "guides/tool.md"}),
    ];
    list.extend(programs.iter().cloned());
    let index = json!({
        "format": "nurt-program-catalog/1",
        "categories": [{"id": "misc", "title": "Miscellaneous"}],
        "programs": list,
    });
    write(root, "catalog.json", &index.to_string());
    write(root, "guides/tool.md", GUIDE);
}

fn spec(id: &str, path: &Path, priority: i32, position: usize) -> CatalogSpec {
    CatalogSpec {
        id: id.to_owned(),
        path: path.to_path_buf(),
        priority,
        position,
    }
}

fn env() -> HostEnv {
    HostEnv::new(Os::Linux)
}

fn load(provider: &CatalogProvider, env: &HostEnv) -> SourceData {
    provider.load(env, &Deadline::none()).unwrap()
}

fn program<'a>(data: &'a SourceData, name: &str) -> &'a RawProgram {
    data.programs
        .iter()
        .find(|program| program.name == name)
        .unwrap_or_else(|| panic!("no program {name}"))
}

/// Schema of the definition `name` of the catalog schema, as a document of
/// its own that keeps the shared definitions.
fn definition_schema(schema: &Value, name: &str) -> Value {
    let mut definition = schema["$defs"][name].clone();
    definition["$defs"] = schema["$defs"].clone();
    definition["$schema"] = schema["$schema"].clone();
    definition
}

fn assert_schema_valid(schema: &Value, instance: &Value, label: &str) {
    let validator = jsonschema::validator_for(schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{label}: {errors:?}");
}

fn front_matter_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap();
    let split = guide::split_guide(&text, Limits::default().front_matter_bytes).unwrap();
    let entries = guide::parse_front_matter(split.front, guide::FRONT_FIRST_LINE).unwrap();
    let mut object = serde_json::Map::new();
    for (key, value) in entries {
        let value = match value {
            FrontValue::Text(text) => Value::String(text),
            FrontValue::List(items) => Value::Array(items.into_iter().map(Value::String).collect()),
            FrontValue::Map(pairs) => Value::Object(
                pairs
                    .into_iter()
                    .map(|(key, value)| (key, Value::String(value)))
                    .collect(),
            ),
        };
        object.insert(key, value);
    }
    Value::Object(object)
}

fn guide_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name != "references") {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|extension| extension == "md") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn catalog_minimal_example_valid() {
    let root = example("minimal");
    let report = validate_catalog(&root);
    assert!(report.is_valid(), "{report}");
    let summary = report.summary.unwrap();
    assert_eq!(summary.programs, 1);
    assert_eq!(summary.categories, 1);
    assert_eq!(summary.guide_files, 1);
    let schema = read_json(&spec_dir().join("catalog.schema.json"));
    assert_schema_valid(
        &schema,
        &read_json(&root.join("catalog.json")),
        "minimal catalog.json",
    );
    let front = read_json(&spec_dir().join("guide-frontmatter.schema.json"));
    assert_schema_valid(
        &front,
        &front_matter_json(&root.join("guides/ffmpeg.md")),
        "minimal guide",
    );
}

#[test]
fn catalog_full_example_valid() {
    let root = example("full");
    let report = validate_catalog(&root);
    assert_eq!(report.errors(), 0, "{report}");
    let summary = report.summary.clone().unwrap();
    assert_eq!(summary.id.as_deref(), Some("media-tools"));
    assert_eq!(summary.programs, 5);
    assert_eq!(summary.categories, 4);
    assert_eq!(summary.guides, 1);
    assert_eq!(summary.parts, 1);
    assert_eq!(summary.overlays, 1);
    assert!(
        report.to_string().ends_with("0 errors, 0 warnings"),
        "{report}"
    );
    let schema = read_json(&spec_dir().join("catalog.schema.json"));
    assert_schema_valid(
        &schema,
        &read_json(&root.join("catalog.json")),
        "full catalog.json",
    );
    assert_schema_valid(
        &definition_schema(&schema, "part"),
        &read_json(&root.join("programs/archives.json")),
        "part",
    );
    assert_schema_valid(
        &definition_schema(&schema, "overlay"),
        &read_json(&root.join("i18n/de/catalog.json")),
        "overlay",
    );
    let front = read_json(&spec_dir().join("guide-frontmatter.schema.json"));
    let guides = guide_files(&root);
    assert_eq!(guides.len(), 5);
    for path in guides {
        assert_schema_valid(
            &front,
            &front_matter_json(&path),
            &path.display().to_string(),
        );
    }
}

#[test]
fn schema_rejects_documents_outside_the_format() {
    let schema = read_json(&spec_dir().join("catalog.schema.json"));
    let validator = jsonschema::validator_for(&schema).unwrap();
    assert!(!validator.is_valid(&json!({"format": "nurt-program-catalog/1", "programs": []})));
    assert!(!validator.is_valid(&json!({
        "format": "nurt-program-catalog/1",
        "categories": [],
        "programs": [{"name": "x", "category": "misc", "summary": "s", "guide": "/etc/passwd"}]
    })));
}

#[test]
fn catalog_guide_sections() {
    let root = example("full");
    let source = CatalogProvider::new(spec("media", &root, 0, 0));
    let env = env();
    let want = DetailWant {
        description: true,
        guide: true,
        ..DetailWant::default()
    };
    let key = EntryKey::new(source.id(), "MP4Box");
    let detail = source.detail(&env, &key, &want).unwrap().unwrap();
    assert!(detail.description.unwrap().starts_with("MP4Box edits"));
    let body = detail.guide.unwrap();
    assert_eq!(body.sections, ["Invocation", "Pitfalls"]);
    assert_eq!(body.version.as_deref(), Some("26.07"));
    assert_eq!(body.language.as_deref(), Some("en"));
    assert!(!body.text.starts_with("---"));
    assert!(
        body.files
            .iter()
            .any(|file| file.ends_with("dash-profiles.md")),
        "{:?}",
        body.files
    );
    let pitfalls = guide::find_section(&body.text, "pitfalls").unwrap();
    assert!(pitfalls.text.contains("-new"));
    assert!(!guide::search_sections(&body.text, "track", 5).is_empty());
    assert!(guide::introduction(&body.text).contains("MP4Box changes"));
    let unknown = EntryKey::new(source.id(), "absent");
    assert_eq!(source.detail(&env, &unknown, &want).unwrap(), None);
    let foreign = EntryKey::new(&SourceId::new(SourceId::PATH), "MP4Box");
    assert_eq!(source.detail(&env, &foreign, &want).unwrap(), None);
    let standalone = EntryKey::new(source.id(), "streaming-basics");
    let guide = source
        .detail(&env, &standalone, &want)
        .unwrap()
        .unwrap()
        .guide
        .unwrap();
    assert!(!guide.text.is_empty());
}

#[test]
fn catalog_shared_guide() {
    let loaded = catalog::read_catalog(&example("full"), &ReadOptions::default()).unwrap();
    assert_eq!(loaded.catalog.guide_shared_with("tar"), ["zstd"]);
    assert!(loaded.catalog.guide_shared_with("MP4Box").is_empty());
    let data = provider::convert(&loaded.catalog, &SourceId::catalog("media"));
    let tar = program(&data, "tar").guide.clone().unwrap();
    let zstd = program(&data, "zstd").guide.clone().unwrap();
    assert_eq!(tar.path, "guides/archives.md");
    assert_eq!(tar, zstd);
}

#[test]
fn catalog_optional_guide_flag() {
    let loaded = catalog::read_catalog(&example("full"), &ReadOptions::default()).unwrap();
    let data = provider::convert(&loaded.catalog, &SourceId::catalog("media"));
    let ls = program(&data, "ls");
    assert!(ls.guide_optional);
    assert!(ls.guide.is_none());
    let ffmpeg = program(&data, "ffmpeg");
    assert!(!ffmpeg.guide_optional);
    assert!(ffmpeg.guide.is_some());
    let mp4box = program(&data, "MP4Box");
    assert!(mp4box.guide_required);
    assert_eq!(mp4box.package_name.as_deref(), Some("gpac"));
    assert_eq!(mp4box.catalog_path.as_deref(), Some("/usr/bin/MP4Box"));
    let english = |lang: Option<&str>| lang == Some("en");
    assert_eq!(
        mp4box
            .not_for
            .iter()
            .filter(|item| english(item.task.lang.as_deref()))
            .count(),
        2
    );
    assert_eq!(mp4box.not_for[0].instead.as_deref(), Some("ffmpeg"));
    assert_eq!(
        mp4box
            .sections
            .iter()
            .filter(|section| english(section.text.lang.as_deref()))
            .count(),
        3
    );
    assert_eq!(
        mp4box
            .when_to_use
            .iter()
            .filter(|item| english(item.lang.as_deref()))
            .count(),
        3
    );
}

#[test]
fn catalog_alias_expands_to_target() {
    let loaded = catalog::read_catalog(&example("full"), &ReadOptions::default()).unwrap();
    let data = provider::convert(&loaded.catalog, &SourceId::catalog("media"));
    let aliases = |name: &str| -> Vec<(String, AliasKind)> {
        program(&data, name)
            .aliases
            .iter()
            .map(|alias| (alias.name.clone(), alias.kind))
            .collect()
    };
    assert_eq!(
        aliases("ffmpeg"),
        [
            ("ffprobe".to_owned(), AliasKind::Companion),
            ("ffmpeg7".to_owned(), AliasKind::Versioned)
        ]
    );
    assert_eq!(
        aliases("zstd"),
        [
            ("unzstd".to_owned(), AliasKind::Companion),
            ("zstdcat".to_owned(), AliasKind::Alias)
        ]
    );
    let mp4box = &program(&data, "MP4Box").aliases[0];
    assert_eq!(mp4box.path.as_deref(), Some("/usr/local/bin/mp4box"));
    let packaging = data
        .categories
        .iter()
        .find(|category| category.id == "video-packaging")
        .unwrap();
    assert_eq!(packaging.parent.as_deref(), Some("video"));
    assert_eq!(packaging.aliases, ["VideoPackaging", "packaging"]);
    assert_eq!(data.synonyms.len(), 2);
    assert_eq!(data.guides[0].name, "streaming-basics");
}

#[test]
fn catalog_i18n_overlay() {
    let root = example("full");
    let source = CatalogProvider::new(spec("media", &root, 0, 0));
    let env = env().with_languages(vec!["de".to_owned(), "en".to_owned()]);
    let data = load(&source, &env);
    let mp4box = program(&data, "MP4Box");
    let summary = mp4box.summary.as_ref().unwrap();
    assert_eq!(summary.lang.as_deref(), Some("en"));
    assert!(summary.translations["de"].starts_with("Verpackt"));
    assert!(mp4box.tags.contains(&"untertitel".to_owned()));
    assert!(
        mp4box
            .sections
            .iter()
            .any(|section| section.text.translations.contains_key("de")
                || section.text.lang.as_deref() == Some("de"))
    );
    let video = data
        .categories
        .iter()
        .find(|category| category.id == "video")
        .unwrap();
    assert!(
        video
            .summary
            .as_ref()
            .unwrap()
            .translations
            .contains_key("de")
    );
    let want = DetailWant {
        guide: true,
        ..DetailWant::default()
    };
    let detail = source
        .detail(&env, &EntryKey::new(source.id(), "MP4Box"), &want)
        .unwrap()
        .unwrap();
    assert_eq!(detail.guide.unwrap().language.as_deref(), Some("de"));
    let english = source
        .detail(&self::env(), &EntryKey::new(source.id(), "MP4Box"), &want)
        .unwrap()
        .unwrap();
    assert_eq!(english.guide.unwrap().language.as_deref(), Some("en"));
}

#[test]
fn catalog_path_escape_rejected() {
    assert!(catalog::check_relpath("guides/tool.md").is_ok());
    for path in [
        "../outside.md",
        "/etc/passwd",
        "C:\\guide.md",
        "guides\\tool.md",
        "",
    ] {
        assert!(catalog::check_relpath(path).is_err(), "{path}");
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("catalog");
    catalog_with(
        &root,
        &[
            json!({"name": "up", "category": "misc", "summary": "Escapes.", "guide": "../outside.md"}),
            json!({"name": "absolute", "category": "misc", "summary": "Absolute.", "guide": "/etc/hosts"}),
        ],
    );
    write(dir.path(), "outside.md", GUIDE);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.path().join("outside.md"), root.join("guides/link.md"))
            .unwrap();
        let index = read_json(&root.join("catalog.json"));
        let mut index = index;
        index["programs"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "linked", "category": "misc", "summary": "Linked.", "guide": "guides/link.md"}));
        write(&root, "catalog.json", &index.to_string());
    }
    let loaded = catalog::read_catalog(&root, &ReadOptions::default()).unwrap();
    let names: Vec<&str> = loaded
        .catalog
        .programs
        .iter()
        .map(|program| program.name.as_str())
        .collect();
    let expected: &[&str] = if cfg!(unix) {
        &["tool", "linked"]
    } else {
        &["tool"]
    };
    assert_eq!(names, expected);
    assert!(
        loaded
            .catalog
            .programs
            .iter()
            .all(|program| program.name == "tool" || program.guide.is_none())
    );
    let invalid = loaded
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == DiagnosticCode::EntryInvalid)
        .count();
    assert_eq!(
        invalid,
        if cfg!(unix) { 3 } else { 2 },
        "{:?}",
        loaded.diagnostics
    );
    assert!(matches!(
        catalog::check_relpath("a/../../b"),
        Err(PathError::Syntax(_) | PathError::Outside)
    ));
}

#[test]
fn catalog_unsupported_format_major_e_catalog() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "catalog.json",
        r#"{"format": "nurt-program-catalog/2", "categories": [], "programs": []}"#,
    );
    let error = catalog::read_catalog(dir.path(), &ReadOptions::default()).unwrap_err();
    assert_eq!(error.kind, CatalogErrorKind::Format);
    assert!(error.message.contains("version 2"));
    let source = CatalogProvider::new(spec("future", dir.path(), 0, 0));
    assert_eq!(
        source.read(&env()).unwrap_err().kind,
        CatalogErrorKind::Format
    );
    let data = load(&source, &env());
    assert!(data.programs.is_empty());
    assert_eq!(data.warnings.len(), 1);
    assert_eq!(data.warnings[0].code, WarningCode::CatalogFormat);
    let report = validate_catalog(dir.path());
    assert!(!report.is_valid());
    assert_eq!(report.diagnostics[0].code, DiagnosticCode::CatalogFormat);

    write(
        dir.path(),
        "catalog.json",
        r#"{"categories": [], "programs": []}"#,
    );
    let missing = catalog::read_catalog(dir.path(), &ReadOptions::default()).unwrap_err();
    assert_eq!(missing.kind, CatalogErrorKind::Structure);
    write(dir.path(), "catalog.json", "{ not json");
    let syntax = catalog::read_catalog(dir.path(), &ReadOptions::default()).unwrap_err();
    assert_eq!(syntax.kind, CatalogErrorKind::Syntax);
    let data = load(&source, &env());
    assert_eq!(data.warnings[0].code, WarningCode::CatalogInvalid);
    let absent = tempfile::tempdir().unwrap();
    let unreadable = catalog::read_catalog(absent.path(), &ReadOptions::default()).unwrap_err();
    assert_eq!(unreadable.kind, CatalogErrorKind::Unreadable);
    assert!(!CatalogProvider::new(spec("absent", absent.path(), 0, 0)).detect(&env()));
}

#[test]
fn catalog_bad_entry_warns_and_skips() {
    let dir = tempfile::tempdir().unwrap();
    catalog_with(
        dir.path(),
        &[
            json!({"name": "nosummary", "category": "misc"}),
            json!({"name": "bad name!", "category": "misc", "summary": "Bad name."}),
            json!({"name": "nocat", "category": "absent", "summary": "Unknown category."}),
            json!({"name": "noguide", "category": "misc", "summary": "Missing guide.", "guide": "guides/absent.md"}),
            json!(42),
        ],
    );
    let source = CatalogProvider::new(spec("bad", dir.path(), 0, 0));
    assert!(source.detect(&env()));
    assert_eq!(source.kind(), SourceKind::Catalog);
    let data = load(&source, &env());
    let names: Vec<&str> = data
        .programs
        .iter()
        .map(|program| program.name.as_str())
        .collect();
    assert_eq!(names, ["tool", "noguide"]);
    assert!(program(&data, "noguide").guide.is_none());
    let invalid: Vec<_> = data
        .warnings
        .iter()
        .filter(|warning| warning.code == WarningCode::CatalogEntryInvalid)
        .collect();
    assert_eq!(invalid.len(), 5, "{:?}", data.warnings);
    assert!(invalid.iter().all(|warning| {
        warning
            .path
            .as_deref()
            .is_some_and(|path| path.ends_with("catalog.json"))
    }));
    assert!(
        invalid
            .iter()
            .any(|warning| warning.name.as_deref() == Some("nocat"))
    );
}

#[test]
fn catalog_warnings_are_truncated() {
    let dir = tempfile::tempdir().unwrap();
    let bad: Vec<Value> = (0..MAX_WARNINGS + 5)
        .map(|index| json!({"name": format!("p{index}"), "category": "misc"}))
        .collect();
    catalog_with(dir.path(), &bad);
    let data = load(
        &CatalogProvider::new(spec("many", dir.path(), 0, 0)),
        &env(),
    );
    assert_eq!(data.warnings.len(), MAX_WARNINGS + 1);
    let last = data.warnings.last().unwrap();
    assert_eq!(last.code, WarningCode::WarningsTruncated);
    assert!(last.reason.starts_with("5 "));
}

#[test]
fn catalog_guide_without_version_warns() {
    let dir = tempfile::tempdir().unwrap();
    catalog_with(dir.path(), &[]);
    write(
        dir.path(),
        "guides/tool.md",
        "---\nname: tool\ndescription: Use the tool.\n---\nText.\n",
    );
    let data = load(
        &CatalogProvider::new(spec("noversion", dir.path(), 0, 0)),
        &env(),
    );
    assert_eq!(data.programs.len(), 1);
    assert!(
        data.warnings
            .iter()
            .any(|warning| warning.code == WarningCode::GuideVersionMissing),
        "{:?}",
        data.warnings
    );
}

#[test]
fn catalog_priority_two_catalogs() {
    let dir = tempfile::tempdir().unwrap();
    let second = dir.path().join("media-tools");
    catalog_with(&second, &[]);
    let specs = provider::resolve_ids(
        vec![
            spec("full", &example("full"), 10, 0),
            spec("media-tools", &second, 5, 1),
            spec("minimal", &example("minimal").join("catalog.json"), 0, 2),
        ],
        &env(),
    );
    let ids: Vec<&str> = specs.iter().map(|spec| spec.id.as_str()).collect();
    assert_eq!(ids, ["media-tools", "media-tools-2", "minimal"]);
    let sources: Vec<CatalogProvider> = specs.into_iter().map(CatalogProvider::new).collect();
    assert_eq!(sources[0].id().as_str(), "catalog:media-tools");
    assert_eq!(sources[0].priority(), 10);
    assert_eq!(sources[1].priority(), 5);
    assert!(sources[0].priority() > sources[1].priority());
    assert_eq!(sources[2].spec().position, 2);
    let first = load(&sources[0], &env());
    let other = load(&sources[2], &env());
    assert!(
        first
            .programs
            .iter()
            .any(|program| program.name == "ffmpeg")
    );
    assert!(
        other
            .programs
            .iter()
            .any(|program| program.name == "ffmpeg")
    );
    assert_eq!(
        program(&first, "ffmpeg")
            .key
            .as_ref()
            .unwrap()
            .source
            .as_str(),
        "catalog:media-tools"
    );
    let marker = sources[2].fingerprint(&env()).unwrap();
    assert_eq!(marker, sources[2].fingerprint(&env()).unwrap());
    assert_ne!(marker, sources[0].fingerprint(&env()).unwrap());
}

#[test]
fn check_reports_file_and_field() {
    let dir = tempfile::tempdir().unwrap();
    catalog_with(
        dir.path(),
        &[
            json!({"name": "dated", "category": "misc", "summary": "Bad date.", "verified": "2026-13-45"}),
        ],
    );
    let report = validate_catalog(dir.path());
    assert!(!report.is_valid());
    let finding = report
        .diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic
                .field
                .as_deref()
                .is_some_and(|field| field.contains("verified"))
        })
        .unwrap_or_else(|| panic!("{report}"));
    assert_eq!(finding.file, "catalog.json");
    assert_eq!(finding.severity, Severity::Error);
    assert!(finding.line.is_some_and(|line| line >= 1));
    let text = finding.to_string();
    assert!(text.starts_with("catalog.json:"), "{text}");
    assert!(text.contains("verified"), "{text}");
    assert!(report.to_string().contains("1 errors"), "{report}");

    write(dir.path(), "guides/tool.md", "no front matter\n");
    let report = validate_catalog(dir.path());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.file == "guides/tool.md"),
        "{report}"
    );
}

#[test]
fn check_enforces_size_limits() {
    let dir = tempfile::tempdir().unwrap();
    catalog_with(dir.path(), &[]);
    let limits = Limits {
        index_bytes: 64,
        ..Limits::default()
    };
    let report = validate_catalog_with(dir.path(), &limits);
    assert!(!report.is_valid());
    assert!(report.summary.is_none());
    assert_eq!(report.diagnostics[0].code, DiagnosticCode::CatalogInvalid);
    let error = catalog::read_catalog(
        dir.path(),
        &ReadOptions {
            limits,
            strict: false,
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, CatalogErrorKind::Limit);

    let guide_limit = Limits {
        guide_bytes: 16,
        ..Limits::default()
    };
    let report = validate_catalog_with(dir.path(), &guide_limit);
    assert!(!report.is_valid(), "{report}");
}

#[test]
fn check_reports_unknown_members_as_warnings() {
    let dir = tempfile::tempdir().unwrap();
    catalog_with(
        dir.path(),
        &[
            json!({"name": "extra", "category": "misc", "summary": "Unknown member.", "colour": "red"}),
        ],
    );
    let report = validate_catalog(dir.path());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::UnknownField),
        "{report}"
    );
    let lenient = catalog::read_catalog(dir.path(), &ReadOptions::default()).unwrap();
    assert!(lenient.catalog.program("extra").is_some());
}

#[test]
fn guide_material_stops_at_the_file_limit_in_name_order() {
    let dir = tempfile::tempdir().unwrap();
    catalog_with(dir.path(), &[]);
    for name in ["d", "a", "c", "b", "e"] {
        write(dir.path(), &format!("guides/tool/{name}.txt"), name);
    }
    write(dir.path(), "guides/tool/sub/z.txt", "z");
    write(dir.path(), "guides/tool/sub/deep/y.txt", "y");
    let loaded = catalog::read_catalog(dir.path(), &ReadOptions::default()).unwrap();
    let few = Limits {
        guide_files: 3,
        ..Limits::default()
    };
    assert_eq!(
        loaded.catalog.guide_material("guides/tool.md", &few),
        [
            "guides/tool/a.txt",
            "guides/tool/b.txt",
            "guides/tool/c.txt"
        ]
    );
    let shallow = Limits {
        guide_files_depth: 2,
        ..Limits::default()
    };
    let files = loaded.catalog.guide_material("guides/tool.md", &shallow);
    assert!(files.contains(&"guides/tool/sub/z.txt".to_owned()));
    assert!(!files.iter().any(|file| file.ends_with("y.txt")));
    assert!(
        loaded
            .catalog
            .guide_material("guides/missing.md", &Limits::default())
            .is_empty()
    );
}
