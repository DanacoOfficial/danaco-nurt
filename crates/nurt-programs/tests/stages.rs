// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Behaviour of `program.find`, `program.search`, `program.describe` and
//! `program.catalog`, including the parity of ranking and listings with
//! the reference algorithm on a synthetic catalog.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code fails loudly on broken fixtures"
)]

mod support;

use std::sync::Arc;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::value::{Record, Value};
use nurt_programs::model::{
    AliasKind, CategoryHint, CategoryScheme, GuideBody, GuideRef, LocalizedText, NotFor, RawAlias,
    RawCategory, RawGuide, RawProgram, SourceData,
};
use nurt_programs::search::SearchOptions;
use nurt_programs::source::{SourceKind, SourceRegistry};
use nurt_programs::stages::{CATALOG, DESCRIBE, FIND, ProgramsSetup, SEARCH};
use support::{Fixture, StaticSource, args, field, names, run, text, texts};

fn program(name: &str, category: &str, summary: &str, when: &[&str]) -> RawProgram {
    let mut raw = RawProgram::new(name);
    raw.summary = Some(LocalizedText::in_lang(summary, "en"));
    raw.when_to_use = when
        .iter()
        .map(|item| LocalizedText::in_lang(*item, "en"))
        .collect();
    raw.category_hints
        .push(CategoryHint::new(CategoryScheme::Catalog, category));
    raw.installed = Some(false);
    raw
}

fn category(id: &str, title: &str, parent: Option<&str>) -> RawCategory {
    RawCategory {
        id: id.to_owned(),
        title: LocalizedText::in_lang(title, "en"),
        description: Some(LocalizedText::in_lang(format!("{title} programs."), "en")),
        parent: parent.map(str::to_owned),
        ..RawCategory::default()
    }
}

fn guide(name: &str, description: &str) -> GuideRef {
    GuideRef {
        name: name.to_owned(),
        path: format!("guides/{name}.md"),
        description: Some(LocalizedText::in_lang(description, "en")),
    }
}

/// Synthetic catalog with the structure of the reference catalog: six
/// categories, eight subcategories, shared, optional and standalone guides,
/// aliases of three kinds and `not_for` entries.
fn reference_catalog() -> StaticSource {
    let mut categories = vec![
        category("video", "Video", None),
        category("audio", "Audio", None),
        category("documents", "Documents", None),
        category("archives", "Archives", None),
        category("system", "System", None),
        category("data", "Data", None),
        category("video-encoding", "Encoding", Some("video")),
        category("video-packaging", "Packaging", Some("video")),
        category("video-subtitles", "Subtitles", Some("video")),
        category("audio-editing", "Editing", Some("audio")),
        category("audio-speech", "Speech", Some("audio")),
        category("documents-pdf", "PDF", Some("documents")),
        category("documents-markup", "Markup", Some("documents")),
        category("archives-checksums", "Checksums", Some("archives")),
    ];
    categories[7].aliases = vec!["VideoPackaging".to_owned()];
    let mut ffmpeg = program(
        "ffmpeg",
        "video-encoding",
        "Converts, cuts and filters video and audio streams.",
        &["transcode video", "extract audio track"],
    );
    ffmpeg.guide = Some(guide("ffmpeg", "Encoding recipes for ffmpeg."));
    ffmpeg.aliases = vec![RawAlias::new("ffmpeg7", AliasKind::Versioned)];
    ffmpeg.not_for = vec![NotFor {
        task: LocalizedText::in_lang("subtitle track in mp4 container", "en"),
        instead: Some("MP4Box".to_owned()),
    }];
    let mut mp4box = program(
        "MP4Box",
        "video-packaging",
        "Packages MP4 files without re-encoding: tracks, subtitles, chapters, DASH.",
        &["subtitles as an MP4 track", "dash segments"],
    );
    mp4box.guide = Some(guide("mp4box", "Package MP4 without re-encoding."));
    mp4box.guide_required = true;
    mp4box.aliases = vec![RawAlias::new("mp4box", AliasKind::Alias)];
    mp4box.example = Some("MP4Box -add film.mp4 -new out.mp4".to_owned());
    let mut rsync = program(
        "rsync",
        "system",
        "Synchronizes files and directories.",
        &["file synchronization", "mirror directories"],
    );
    rsync.not_for = vec![NotFor {
        task: LocalizedText::in_lang("mirror with versioned backups", "en"),
        instead: Some("restic.".to_owned()),
    }];
    let mut tar = program(
        "tar",
        "archives",
        "Creates and extracts tape archives.",
        &[],
    );
    tar.guide = Some(guide("archives", "Archive formats compared."));
    tar.guide_optional = true;
    let mut zip = program("zip", "archives", "Packs files into ZIP archives.", &[]);
    zip.guide = Some(guide("archives", "Archive formats compared."));
    let mut rg = program(
        "rg",
        "system",
        "Searches file contents with regular expressions.",
        &[],
    );
    rg.aliases = vec![RawAlias::new("ripgrep", AliasKind::Companion)];
    let programs = vec![
        ffmpeg,
        program(
            "ffprobe",
            "video-encoding",
            "Reports streams and metadata of media files.",
            &["inspect codec"],
        ),
        mp4box,
        program(
            "mkvmerge",
            "video-packaging",
            "Merges tracks into Matroska files.",
            &["mkv container"],
        ),
        program(
            "subedit",
            "video-subtitles",
            "Edits subtitle timing in SRT files.",
            &["napisy przesuni\u{119}cie"],
        ),
        program("handbrake", "video", "Encodes video with presets.", &[]),
        program(
            "sox",
            "audio-editing",
            "Processes and converts sound files.",
            &["d\u{17a}wi\u{119}k konwersja", "normalize volume"],
        ),
        program("audacity", "audio-editing", "Records and edits audio.", &[]),
        program(
            "whisper",
            "audio-speech",
            "Transcribes speech to text.",
            &["transkrypcja nagrania"],
        ),
        program(
            "piper",
            "audio-speech",
            "Synthesizes speech from text.",
            &[],
        ),
        program("lame", "audio", "Encodes MP3 files.", &[]),
        program(
            "qpdf",
            "documents-pdf",
            "Transforms PDF files: merge, split, encrypt.",
            &["\u{142}\u{105}czenie pdf"],
        ),
        program(
            "pdftotext",
            "documents-pdf",
            "Extracts text from PDF files.",
            &[],
        ),
        {
            let mut pandoc = program(
                "pandoc",
                "documents-markup",
                "Converts documents between markup formats.",
                &["markdown to docx"],
            );
            pandoc.guide = Some(guide("pandoc", "Document conversion."));
            pandoc
        },
        program(
            "typst",
            "documents-markup",
            "Typesets documents from markup.",
            &[],
        ),
        program(
            "libreoffice",
            "documents",
            "Office suite for documents, spreadsheets and presentations.",
            &[],
        ),
        tar,
        zip,
        program(
            "7z",
            "archives",
            "Packs and unpacks 7z and many archive formats.",
            &[],
        ),
        program(
            "sha256sum",
            "archives-checksums",
            "Computes SHA-256 checksums.",
            &[],
        ),
        program(
            "b3sum",
            "archives-checksums",
            "Computes BLAKE3 checksums.",
            &[],
        ),
        rsync,
        program(
            "restic",
            "system",
            "Creates versioned backups.",
            &["backup repository"],
        ),
        program("htop", "system", "Shows processes interactively.", &[]),
        program("fd", "system", "Finds files by name.", &[]),
        rg,
        program(
            "jq",
            "data",
            "Filters and transforms JSON.",
            &["json query"],
        ),
        program(
            "duckdb",
            "data",
            "Runs analytical SQL on files.",
            &["sql on csv"],
        ),
        program(
            "sqlite3",
            "data",
            "Command line client of SQLite databases.",
            &[],
        ),
        program("csvkit", "data", "Tools for CSV tables.", &[]),
        program("yq", "data", "Filters YAML documents.", &[]),
        program("psql", "data", "PostgreSQL client.", &[]),
    ];
    let guides = vec![
        RawGuide {
            name: "streaming-basics".to_owned(),
            path: "guides/streaming-basics.md".to_owned(),
            summary: Some(LocalizedText::in_lang(
                "Basics of HLS and DASH streaming.",
                "en",
            )),
            category: Some("video".to_owned()),
            ..RawGuide::default()
        },
        RawGuide {
            name: "backup-policy".to_owned(),
            path: "guides/backup-policy.md".to_owned(),
            summary: Some(LocalizedText::in_lang("Retention rules for backups.", "en")),
            category: Some("system".to_owned()),
            ..RawGuide::default()
        },
    ];
    let mut source = StaticSource::catalog(
        "ref",
        SourceData {
            programs,
            categories,
            guides,
            ..SourceData::default()
        },
    );
    source.guides.insert(
        "guides/mp4box.md".to_owned(),
        GuideBody {
            text: "## Invocation\nMP4Box -add a.mp4 -new b.mp4\n## Pitfalls\nUse -new to keep the input.".to_owned(),
            version: Some("26.07".to_owned()),
            sections: vec!["Invocation".to_owned(), "Pitfalls".to_owned()],
            ..GuideBody::default()
        },
    );
    source.guides.insert(
        "guides/archives.md".to_owned(),
        GuideBody {
            text: "## Formats\ntar, zip".to_owned(),
            sections: vec!["Formats".to_owned()],
            ..GuideBody::default()
        },
    );
    source
}

fn catalog_only(fixture: &Fixture) -> ProgramsSetup {
    let registry = SourceRegistry::new().with(Arc::new(reference_catalog()));
    ProgramsSetup {
        ranking: Some(SearchOptions::COMPATIBLE),
        ..fixture.setup_with(registry)
    }
}

fn first_hit(setup: &ProgramsSetup, fixture: &Fixture, values: &[(&str, Value)]) -> Option<String> {
    let records = run(setup, &fixture.ctx(), args(SEARCH, values)).unwrap();
    records.first().and_then(|record| field(record, "name"))
}

#[test]
fn parity_first_hits_of_reference_queries() {
    let fixture = Fixture::new();
    let setup = catalog_only(&fixture);
    let cases: &[(&str, &str)] = &[
        ("ffmpeg", "ffmpeg"),
        ("ffprobe", "ffprobe"),
        ("mp4box", "MP4Box"),
        ("mkvmerge", "mkvmerge"),
        ("sox", "sox"),
        ("jq", "jq"),
        ("rg", "rg"),
        ("tar", "tar"),
        ("zip", "zip"),
        ("7z", "7z"),
        ("qpdf", "qpdf"),
        ("pandoc", "pandoc"),
        ("typst", "typst"),
        ("duckdb", "duckdb"),
        ("sqlite", "sqlite3"),
        ("psql", "psql"),
        ("htop", "htop"),
        ("restic", "restic"),
        ("rsync", "rsync"),
        ("whisper", "whisper"),
        ("piper", "piper"),
        ("lame", "lame"),
        ("b3sum", "b3sum"),
        ("sha256", "sha256sum"),
        ("ffm", "ffmpeg"),
        ("transkrypcja", "whisper"),
        ("d\u{17a}wi\u{119}k", "sox"),
        ("dzwiek", "sox"),
        ("napisy", "subedit"),
        ("\u{142}\u{105}czenie pdf", "qpdf"),
        ("markdown", "pandoc"),
        ("json", "jq"),
        ("yaml", "yq"),
        ("csv", "csvkit"),
        ("backup", "backup-policy"),
        ("versioned backups", "restic"),
        ("subtitles mp4", "MP4Box"),
        ("video", "ffmpeg"),
        ("processes", "htop"),
        ("regular expressions", "rg"),
        ("office", "libreoffice"),
        ("matroska", "mkvmerge"),
        ("chapters", "MP4Box"),
        ("speech", "piper"),
        ("streaming", "streaming-basics"),
        ("Ffmpeg i d\u{17a}wi\u{119}k", "ffmpeg"),
    ];
    assert!(cases.len() >= 40);
    for (query, expected) in cases {
        let hit = first_hit(&setup, &fixture, &[("query", text(query))]);
        assert_eq!(hit.as_deref(), Some(*expected), "query {query:?}");
    }
}

#[test]
fn parity_points_and_not_for() {
    let fixture = Fixture::new();
    let setup = catalog_only(&fixture);
    let records = run(
        &setup,
        &fixture.ctx(),
        args(SEARCH, &[("query", text("mirror versioned"))]),
    )
    .unwrap();
    assert_eq!(names(&records), vec!["restic", "rsync"]);
    assert_eq!(field(&records[0], "score").as_deref(), Some("5"));
    assert_eq!(field(&records[1], "score").as_deref(), Some("1"));
    assert_eq!(field(&records[1], "instead").as_deref(), Some("restic"));
    let exact = run(
        &setup,
        &fixture.ctx(),
        args(SEARCH, &[("query", text("ffmpeg"))]),
    )
    .unwrap();
    assert_eq!(field(&exact[0], "score").as_deref(), Some("118"));
}

#[test]
fn search_ranking_exact_name_first() {
    let fixture = Fixture::new();
    let setup = catalog_only(&fixture);
    let records = run(
        &setup,
        &fixture.ctx(),
        args(SEARCH, &[("query", text("ff"))]),
    )
    .unwrap();
    assert_eq!(
        names(&records)[..2],
        ["ffmpeg".to_owned(), "ffprobe".to_owned()]
    );
    let tie = run(
        &setup,
        &fixture.ctx(),
        args(SEARCH, &[("query", text("speech"))]),
    )
    .unwrap();
    assert_eq!(names(&tie), vec!["piper", "whisper"]);
}

#[test]
fn search_polish_query_folds_diacritics() {
    let fixture = Fixture::new();
    let setup = catalog_only(&fixture);
    for query in ["D\u{179}WI\u{118}K", "dzwiek", "d\u{17a}wi\u{119}k"] {
        assert_eq!(
            first_hit(&setup, &fixture, &[("query", text(query))]).as_deref(),
            Some("sox")
        );
    }
    let full = ProgramsSetup {
        ranking: Some(SearchOptions::FULL),
        ..setup
    };
    assert_eq!(
        first_hit(&full, &fixture, &[("query", text("wideo"))]).as_deref(),
        Some("ffmpeg")
    );
}

#[test]
fn search_kind_filter() {
    let fixture = Fixture::new();
    let setup = catalog_only(&fixture);
    let guides = run(
        &setup,
        &fixture.ctx(),
        args(
            SEARCH,
            &[("query", text("backup")), ("kind", text("guides"))],
        ),
    )
    .unwrap();
    assert_eq!(names(&guides), vec!["backup-policy"]);
    assert_eq!(field(&guides[0], "kind").as_deref(), Some("guide"));
    let programs = run(
        &setup,
        &fixture.ctx(),
        args(
            SEARCH,
            &[("query", text("backup")), ("kind", text("programs"))],
        ),
    )
    .unwrap();
    assert_eq!(names(&programs), vec!["restic"]);
    let error = run(
        &setup,
        &fixture.ctx(),
        args(SEARCH, &[("query", text("x1")), ("kind", text("skills"))]),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidValue);
    let category = run(
        &setup,
        &fixture.ctx(),
        args(
            SEARCH,
            &[("query", text("encode")), ("category", text("Audio"))],
        ),
    )
    .unwrap();
    assert_eq!(names(&category), vec!["lame"]);
}

#[test]
fn search_limit_bounds() {
    let fixture = Fixture::new();
    let setup = catalog_only(&fixture);
    let two = run(
        &setup,
        &fixture.ctx(),
        args(
            SEARCH,
            &[("query", text("files")), ("limit", Value::Int(2))],
        ),
    )
    .unwrap();
    assert_eq!(two.len(), 2);
    for limit in [0, 61] {
        let error = run(
            &setup,
            &fixture.ctx(),
            args(
                SEARCH,
                &[("query", text("files")), ("limit", Value::Int(limit))],
            ),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidValue);
    }
    let short = run(
        &setup,
        &fixture.ctx(),
        args(SEARCH, &[("query", text("x"))]),
    )
    .unwrap_err();
    assert_eq!(short.code, ErrorCode::Arg);
    let many = run(
        &setup,
        &fixture.ctx(),
        args(SEARCH, &[("query", texts(&["jq", "yq"]))]),
    )
    .unwrap();
    assert_eq!(names(&many), vec!["jq", "yq"]);
    assert_eq!(field(&many[1], "query").as_deref(), Some("yq"));
}

#[cfg(unix)]
#[test]
fn search_source_filter_catalog_id() {
    let fixture = Fixture::new();
    fixture.executable("/usr/bin/jq");
    fixture.executable("/usr/bin/jqfmt");
    let setup = ProgramsSetup {
        ranking: Some(SearchOptions::COMPATIBLE),
        ..fixture.setup(vec![Arc::new(reference_catalog())])
    };
    let all = run(
        &setup,
        &fixture.ctx(),
        args(SEARCH, &[("query", text("jq"))]),
    )
    .unwrap();
    assert_eq!(names(&all)[..2], ["jq".to_owned(), "jqfmt".to_owned()]);
    assert_eq!(field(&all[0], "installed").as_deref(), Some("true"));
    assert_eq!(
        field(&all[0], "sources").as_deref(),
        Some("path,catalog:ref")
    );
    let catalog = run(
        &setup,
        &fixture.ctx(),
        args(
            SEARCH,
            &[("query", text("jq")), ("source", texts(&["catalog:ref"]))],
        ),
    )
    .unwrap();
    assert_eq!(names(&catalog), vec!["jq"]);
    let path = run(
        &setup,
        &fixture.ctx(),
        args(
            SEARCH,
            &[("query", text("jq")), ("source", texts(&["path"]))],
        ),
    )
    .unwrap();
    assert_eq!(names(&path), vec!["jq", "jqfmt"]);
    let missing = run(
        &setup,
        &fixture.ctx(),
        args(
            SEARCH,
            &[("query", text("jq")), ("source", texts(&["catalog:none"]))],
        ),
    )
    .unwrap_err();
    assert_eq!(missing.code, ErrorCode::Catalog);
    let uninstalled = run(
        &setup,
        &fixture.ctx(),
        args(
            SEARCH,
            &[("query", text("yq")), ("installed", Value::Bool(false))],
        ),
    )
    .unwrap();
    assert_eq!(names(&uninstalled), vec!["yq"]);
}

fn describe(setup: &ProgramsSetup, fixture: &Fixture, values: &[(&str, Value)]) -> Record {
    run(setup, &fixture.ctx(), args(DESCRIBE, values))
        .unwrap()
        .remove(0)
}

#[test]
fn describe_include_guide_false() {
    let fixture = Fixture::new();
    let setup = catalog_only(&fixture);
    let full = describe(&setup, &fixture, &[("name", text("mp4box"))]);
    assert_eq!(field(&full, "name").as_deref(), Some("MP4Box"));
    assert_eq!(field(&full, "requested").as_deref(), Some("mp4box"));
    assert_eq!(field(&full, "alias_kind").as_deref(), Some("alias"));
    assert!(field(&full, "guide").unwrap().contains("## Pitfalls"));
    assert_eq!(
        field(&full, "guide_sections").as_deref(),
        Some("Invocation,Pitfalls")
    );
    assert_eq!(field(&full, "guide_required").as_deref(), Some("true"));
    assert_eq!(
        field(&full, "example").as_deref(),
        Some("MP4Box -add film.mp4 -new out.mp4")
    );
    let short = describe(
        &setup,
        &fixture,
        &[
            ("name", text("MP4Box")),
            ("include_guide", Value::Bool(false)),
        ],
    );
    assert!(short.get("guide").is_none());
    assert_eq!(field(&short, "etag"), field(&full, "etag"));
    let shared = describe(&setup, &fixture, &[("name", text("tar"))]);
    assert_eq!(field(&shared, "guide_shared_with").as_deref(), Some("zip"));
    assert_eq!(field(&shared, "guide_optional").as_deref(), Some("true"));
}

#[test]
fn describe_section_query() {
    let fixture = Fixture::new();
    let setup = catalog_only(&fixture);
    let section = describe(
        &setup,
        &fixture,
        &[("name", text("MP4Box")), ("section", text("pitfalls"))],
    );
    assert_eq!(
        field(&section, "guide").as_deref(),
        Some("## Pitfalls\nUse -new to keep the input.")
    );
    let query = describe(
        &setup,
        &fixture,
        &[("name", text("MP4Box")), ("query", text("keep input"))],
    );
    assert_eq!(field(&query, "guide_sections").as_deref(), Some("Pitfalls"));
    let unknown = describe(
        &setup,
        &fixture,
        &[("name", text("MP4Box")), ("section", text("Missing"))],
    );
    assert!(unknown.get("guide").is_none());
}

#[test]
fn describe_unchanged_since() {
    let fixture = Fixture::new();
    let setup = catalog_only(&fixture);
    let first = describe(&setup, &fixture, &[("name", text("ffmpeg"))]);
    let etag = field(&first, "etag").unwrap();
    assert_eq!(etag.len(), 8);
    let again = describe(
        &setup,
        &fixture,
        &[("name", text("ffmpeg")), ("unchanged_since", text(&etag))],
    );
    assert_eq!(field(&again, "unchanged").as_deref(), Some("true"));
    assert!(again.get("guide").is_none());
    let other = describe(
        &setup,
        &fixture,
        &[
            ("name", text("ffmpeg")),
            ("unchanged_since", text("00000000")),
        ],
    );
    assert!(other.get("unchanged").is_none());
}

#[test]
fn describe_unknown_name_with_candidates_and_guides() {
    let fixture = Fixture::new();
    let setup = catalog_only(&fixture);
    let error = run(
        &setup,
        &fixture.ctx(),
        args(DESCRIBE, &[("name", text("ffmpg"))]),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
    assert!(
        error
            .candidates
            .iter()
            .any(|candidate| candidate.program == "program.describe \"ffmpeg\"")
    );
    let guide = describe(&setup, &fixture, &[("name", text("streaming-basics"))]);
    assert_eq!(field(&guide, "kind").as_deref(), Some("guide"));
    let mixed = run(
        &setup,
        &fixture.ctx(),
        args(DESCRIBE, &[("name", texts(&["jq", "nothing-here"]))]),
    )
    .unwrap();
    assert_eq!(names(&mixed), vec!["jq"]);
    assert!(
        fixture
            .logs()
            .iter()
            .any(|message| message.contains("nothing-here"))
    );
}

#[test]
fn catalog_lists_categories_all_sources() {
    let fixture = Fixture::new();
    let setup = catalog_only(&fixture);
    let records = run(
        &setup,
        &fixture.ctx(),
        args(CATALOG, &[("source", text("catalog"))]),
    )
    .unwrap();
    assert_eq!(
        names(&records),
        vec!["data", "video", "archives", "audio", "documents", "system"]
    );
    assert_eq!(field(&records[0], "count").as_deref(), Some("6"));
    assert_eq!(
        field(&records[1], "subcategories").as_deref(),
        Some("video-encoding,video-packaging,video-subtitles")
    );
    assert_eq!(field(&records[1], "level").as_deref(), Some("category"));
}

#[test]
fn catalog_subcategory_other() {
    let fixture = Fixture::new();
    let setup = catalog_only(&fixture);
    let subs = run(
        &setup,
        &fixture.ctx(),
        args(CATALOG, &[("category", text("video"))]),
    )
    .unwrap();
    assert_eq!(
        names(&subs),
        vec![
            "video-encoding",
            "video-packaging",
            "video-subtitles",
            "_other"
        ]
    );
    assert_eq!(field(&subs[3], "count").as_deref(), Some("1"));
    assert_eq!(
        field(&subs[3], "title").as_deref(),
        Some("outside subcategories")
    );
    let other = run(
        &setup,
        &fixture.ctx(),
        args(
            CATALOG,
            &[("category", text("video")), ("subcategory", text("_other"))],
        ),
    )
    .unwrap();
    assert_eq!(names(&other), vec!["handbrake"]);
    let alias = run(
        &setup,
        &fixture.ctx(),
        args(
            CATALOG,
            &[
                ("category", text("Video")),
                ("subcategory", text("videopackaging")),
            ],
        ),
    )
    .unwrap();
    assert_eq!(names(&alias), vec!["MP4Box", "mkvmerge"]);
    let flat = run(
        &setup,
        &fixture.ctx(),
        args(CATALOG, &[("category", text("system"))]),
    )
    .unwrap();
    assert_eq!(names(&flat), vec!["fd", "htop", "restic", "rg", "rsync"]);
    let rejected = run(
        &setup,
        &fixture.ctx(),
        args(
            CATALOG,
            &[("category", text("system")), ("subcategory", text("x"))],
        ),
    )
    .unwrap_err();
    assert_eq!(rejected.code, ErrorCode::Arg);
    let orphan = run(
        &setup,
        &fixture.ctx(),
        args(CATALOG, &[("subcategory", text("x"))]),
    )
    .unwrap_err();
    assert_eq!(orphan.code, ErrorCode::Arg);
    let query = run(
        &setup,
        &fixture.ctx(),
        args(
            CATALOG,
            &[("category", text("audio")), ("query", text("speech"))],
        ),
    )
    .unwrap();
    assert_eq!(names(&query), vec!["piper", "whisper"]);
}

#[test]
fn catalog_unknown_category_candidates() {
    let fixture = Fixture::new();
    let setup = catalog_only(&fixture);
    let error = run(
        &setup,
        &fixture.ctx(),
        args(CATALOG, &[("category", text("vdeo"))]),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidValue);
    assert_eq!(error.candidates[0].program, "program.catalog \"video\"");
    let search = run(
        &setup,
        &fixture.ctx(),
        args(SEARCH, &[("query", text("x1")), ("category", text("nope"))]),
    )
    .unwrap_err();
    assert_eq!(search.code, ErrorCode::InvalidValue);
}

#[cfg(unix)]
#[test]
fn find_path_order_and_all() {
    let fixture = Fixture::new();
    fixture.executable("/usr/local/bin/tool");
    fixture.executable("/usr/bin/tool");
    fixture.executable("/usr/bin/vim");
    fixture.symlink("/usr/bin/vim", "/usr/bin/vi");
    let setup = fixture.setup(Vec::new());
    let records = run(
        &setup,
        &fixture.ctx(),
        args(
            FIND,
            &[("name", texts(&["tool", "vi"])), ("all", Value::Bool(true))],
        ),
    )
    .unwrap();
    assert_eq!(
        field(&records[0], "path").as_deref(),
        Some("/usr/local/bin/tool")
    );
    assert_eq!(
        field(&records[0], "all_paths").as_deref(),
        Some("/usr/local/bin/tool,/usr/bin/tool")
    );
    assert_eq!(field(&records[1], "alias_of").as_deref(), Some("vim"));
    let first_only = run(
        &setup,
        &fixture.ctx(),
        args(FIND, &[("name", texts(&["tool"]))]),
    )
    .unwrap();
    assert!(first_only[0].get("all_paths").is_none());
}

#[test]
fn find_not_found_record() {
    let fixture = Fixture::new();
    let setup = fixture.setup(Vec::new());
    let records = run(
        &setup,
        &fixture.ctx(),
        args(FIND, &[("name", texts(&["absent-program"]))]),
    )
    .unwrap();
    assert_eq!(field(&records[0], "found").as_deref(), Some("false"));
    assert!(records[0].get("path").is_none());
    let error = run(&setup, &fixture.ctx(), args(FIND, &[])).unwrap_err();
    assert_eq!(error.code, ErrorCode::MissingParam);
}

#[cfg(unix)]
fn debian_fixture(fixture: &Fixture) {
    fixture.executable("/usr/bin/tar");
    fixture.write(
        "/var/lib/dpkg/status",
        b"Package: tar\nStatus: install ok installed\nArchitecture: amd64\nVersion: 1.35+dfsg-3\nSection: utils\nDescription: GNU version of the tar archiving utility\n Tar is a program for packaging a set of files.\n",
    );
    fixture.write(
        "/var/lib/dpkg/info/tar.list",
        b"/.\n/usr/bin\n/usr/bin/tar\n/usr/share/doc/tar\n",
    );
    let mut page = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(
        &mut page,
        b".TH TAR 1\n.SH NAME\ntar \\- an archiving utility\n.SH SYNOPSIS\n",
    )
    .unwrap();
    fixture.write("/usr/share/man/man1/tar.1.gz", &page.finish().unwrap());
}

#[cfg(unix)]
#[test]
fn describe_merges_sources() {
    let fixture = Fixture::new();
    debian_fixture(&fixture);
    let setup = fixture.setup(Vec::new());
    let card = describe(&setup, &fixture, &[("name", text("tar"))]);
    assert_eq!(field(&card, "sources").as_deref(), Some("path,dpkg,man"));
    assert_eq!(
        field(&card, "summary").as_deref(),
        Some("an archiving utility")
    );
    assert_eq!(field(&card, "version").as_deref(), Some("1.35+dfsg-3"));
    assert_eq!(field(&card, "package").as_deref(), Some("tar"));
    assert_eq!(field(&card, "category").as_deref(), Some("archives"));
    assert_eq!(
        field(&card, "description").as_deref(),
        Some("Tar is a program for packaging a set of files.")
    );
    assert!(
        field(&card, "man")
            .unwrap()
            .starts_with("tar - an archiving utility")
    );
    let located = run(
        &setup,
        &fixture.ctx(),
        args(
            FIND,
            &[("name", texts(&["tar"])), ("versions", Value::Bool(true))],
        ),
    )
    .unwrap();
    assert_eq!(
        field(&located[0], "version").as_deref(),
        Some("1.35+dfsg-3")
    );
}

#[cfg(unix)]
#[test]
fn describe_help_only_on_request() {
    let fixture = Fixture::new();
    fixture.executable("/usr/bin/tool");
    let setup = fixture.setup(Vec::new());
    let plain = describe(&setup, &fixture, &[("name", text("tool"))]);
    assert!(plain.get("help").is_none());
    assert!(fixture.process.calls.lock().unwrap().is_empty());
    let help = describe(
        &setup,
        &fixture,
        &[("name", text("tool")), ("help", Value::Bool(true))],
    );
    assert_eq!(
        field(&help, "help").as_deref(),
        Some("usage: tool [options]")
    );
    assert_eq!(field(&help, "help_status").as_deref(), Some("ok"));
    assert_eq!(
        *fixture.process.calls.lock().unwrap(),
        vec![vec![
            "/usr/bin/tool".to_owned(),
            nurt_programs::help::HELP_ARGUMENT.to_owned()
        ]]
    );
    fixture.executable("/usr/sbin/reboot");
    let refused = run(
        &setup,
        &fixture.ctx(),
        args(
            DESCRIBE,
            &[("name", text("reboot")), ("help", Value::Bool(true))],
        ),
    );
    assert!(refused.is_err() || fixture.process.calls.lock().unwrap().len() == 1);
}

#[cfg(unix)]
#[test]
fn works_without_configuration_in_empty_root() {
    let fixture = Fixture::new();
    fixture.executable("/bin/sh");
    let setup = fixture.setup(Vec::new());
    let categories = run(&setup, &fixture.ctx(), args(CATALOG, &[])).unwrap();
    assert_eq!(names(&categories), vec!["system"]);
    let hits = run(
        &setup,
        &fixture.ctx(),
        args(SEARCH, &[("query", text("sh"))]),
    )
    .unwrap();
    assert_eq!(names(&hits), vec!["sh"]);
    assert_eq!(
        field(&hits[0], "summary").as_deref(),
        Some("Executable in /bin.")
    );
    let none = run(
        &setup,
        &fixture.ctx(),
        args(CATALOG, &[("source", text("catalog"))]),
    )
    .unwrap();
    assert!(none.is_empty());
    assert!(
        fixture
            .logs()
            .iter()
            .any(|message| message.starts_with("no_catalogs"))
    );
}

#[cfg(unix)]
#[test]
fn corrupt_source_warns_not_fails() {
    let fixture = Fixture::new();
    fixture.executable("/usr/bin/tool");
    fixture.write("/var/lib/rpm/rpmdb.sqlite", b"not a database at all");
    let failing: Arc<dyn nurt_programs::source::ProgramSource> =
        Arc::new(StaticSource::failing("broken"));
    let setup = fixture.setup(vec![failing]);
    let hits = run(
        &setup,
        &fixture.ctx(),
        args(SEARCH, &[("query", text("tool"))]),
    )
    .unwrap();
    assert_eq!(names(&hits), vec!["tool"]);
    let logs = fixture.logs();
    assert!(
        logs.iter()
            .any(|message| message.starts_with("source_unavailable: rpm")),
        "{logs:?}"
    );
    assert!(
        logs.iter()
            .any(|message| message.starts_with("source_unavailable: broken")),
        "{logs:?}"
    );
    assert_eq!(SourceKind::Package.as_str(), "package");
}
