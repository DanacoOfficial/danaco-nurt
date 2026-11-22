// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Providers of system data and the index cache, read from synthetic files
//! below a test root: RPM SQLite, NDB and Berkeley DB databases, dpkg
//! status files, the pacman database, desktop entries and manual pages.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code fails loudly on broken fixtures"
)]
#![allow(
    clippy::cognitive_complexity,
    reason = "a fixture test checks every field read from one synthetic database"
)]

mod support;

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, SystemTime};

use nurt_contracts::env::Os;
use nurt_programs::index::{IndexOptions, ProgramIndex, cache_path};
use nurt_programs::model::{
    DetailWant, EntryKey, EntryKind, LocalizedText, RawProgram, SourceData,
};
use nurt_programs::source::{
    Deadline, HostEnv, ProgramSource, SourceErrorKind, SourceId, SourceKind, SourceRegistry,
    WarningCode,
};
use nurt_programs::sources::desktop::DesktopSource;
use nurt_programs::sources::man::{ManSource, man_dirs};
use nurt_programs::sources::path::PathSource;
use nurt_programs::sources::rpm::header::{
    TAG_BASENAMES, TAG_DIR_INDEXES, TAG_DIRNAMES, TAG_EPOCH, TAG_GROUP, TAG_I18N_TABLE,
    TAG_LICENSE, TAG_NAME, TAG_RELEASE, TAG_SUMMARY, TAG_URL, TAG_VERSION,
};
use nurt_programs::sources::rpm::{DbFormat, RpmSource};
use nurt_programs::sources::{dpkg, pacman, rpm};
use support::{Fixture, HeaderBuilder, StaticSource, TestClock};

/// Budget long enough for any fixture on a loaded build machine.
const TEST_BUDGET: Duration = Duration::from_secs(60);

fn options(cache: Option<PathBuf>) -> IndexOptions {
    IndexOptions {
        cache_dir: cache,
        budget: TEST_BUDGET,
    }
}

fn registry(sources: Vec<Arc<dyn ProgramSource>>) -> SourceRegistry {
    let mut registry = SourceRegistry::new();
    for source in sources {
        registry.add(source);
    }
    registry
}

fn load_index(
    fixture: &Fixture,
    sources: Vec<Arc<dyn ProgramSource>>,
    cache: Option<PathBuf>,
) -> ProgramIndex {
    ProgramIndex::load(
        &registry(sources),
        &fixture.env(),
        &options(cache),
        &TestClock::new(),
    )
}

fn card_summary(index: &ProgramIndex, name: &str) -> String {
    let (position, _) = index
        .resolve(name)
        .unwrap_or_else(|| panic!("no card {name}"));
    index.card(position).unwrap().summary.text.clone()
}

/// Header of the `gzip` package with an English and a German summary,
/// three files in two directories and an epoch.
fn gzip_header() -> Vec<u8> {
    HeaderBuilder::default()
        .strings(TAG_I18N_TABLE, HeaderBuilder::STRING_ARRAY, &["C", "de_DE"])
        .strings(TAG_NAME, HeaderBuilder::STRING, &["gzip"])
        .strings(TAG_VERSION, HeaderBuilder::STRING, &["1.12"])
        .strings(TAG_RELEASE, HeaderBuilder::STRING, &["3.fc40"])
        .int32(TAG_EPOCH, &[1])
        .strings(
            TAG_SUMMARY,
            HeaderBuilder::I18N_STRING,
            &[
                "The GNU data compression program",
                "Das GNU Datenkompressionsprogramm",
            ],
        )
        .strings(TAG_GROUP, HeaderBuilder::STRING, &["Applications/File"])
        .strings(
            TAG_URL,
            HeaderBuilder::STRING,
            &["https://www.gnu.org/software/gzip/"],
        )
        .strings(TAG_LICENSE, HeaderBuilder::STRING, &["GPL-3.0-or-later"])
        .strings(
            TAG_DIRNAMES,
            HeaderBuilder::STRING_ARRAY,
            &["/usr/bin/", "/usr/share/doc/gzip/"],
        )
        .strings(
            TAG_BASENAMES,
            HeaderBuilder::STRING_ARRAY,
            &["gzip", "zcat", "README"],
        )
        .int32(TAG_DIR_INDEXES, &[0, 0, 1])
        .build()
}

fn write_rpmdb(file: &Path, blobs: &[Vec<u8>]) {
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let connection = rusqlite::Connection::open(file).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE Packages (hnum INTEGER PRIMARY KEY AUTOINCREMENT, blob BLOB NOT NULL)",
        )
        .unwrap();
    for blob in blobs {
        connection
            .execute("INSERT INTO Packages (blob) VALUES (?1)", [blob])
            .unwrap();
    }
}

#[test]
fn rpm_sqlite_header_parse() {
    let fixture = Fixture::new();
    let damaged = vec![0, 0, 0, 9];
    write_rpmdb(
        &fixture.path().join("var/lib/rpm/rpmdb.sqlite"),
        &[gzip_header(), damaged],
    );
    let env = fixture.env();
    assert_eq!(rpm::locate(&env).unwrap().format, DbFormat::Sqlite);
    assert!(RpmSource::new().detect(&env));

    let data = rpm::read_packages(&env).unwrap();
    assert_eq!(data.packages.len(), 1);
    let package = &data.packages[0];
    assert_eq!(package.name, "gzip");
    assert_eq!(package.version.as_deref(), Some("1:1.12-3.fc40"));
    assert_eq!(package.section.as_deref(), Some("Applications/File"));
    assert_eq!(package.license.as_deref(), Some("GPL-3.0-or-later"));
    let summary = package.summary.as_ref().unwrap();
    assert_eq!(
        summary.pick(&["en".to_owned()]).0,
        "The GNU data compression program"
    );
    assert_eq!(
        summary.pick(&["de".to_owned()]),
        ("Das GNU Datenkompressionsprogramm", Some("de"))
    );
    assert_eq!(package.executables, vec!["/usr/bin/gzip", "/usr/bin/zcat"]);
    let programs: Vec<&str> = data
        .programs
        .iter()
        .map(|program| program.name.as_str())
        .collect();
    assert_eq!(programs, vec!["gzip", "zcat"]);
    assert_eq!(data.file_packages.len(), 2);
    assert_eq!(data.warnings.len(), 1);
    assert_eq!(data.warnings[0].code, WarningCode::SourceUnavailable);
    assert_eq!(data.warnings[0].line, Some(2));
}

#[test]
fn rpm_prefers_newest_layout_and_rejects_berkeley_db() {
    let fixture = Fixture::new();
    fixture.write("/var/lib/rpm/Packages", b"\x00\x06\x15\x61berkeley");
    let env = fixture.env();
    assert_eq!(rpm::locate(&env).unwrap().format, DbFormat::BerkeleyDb);
    let error = rpm::read_packages(&env).unwrap_err();
    assert_eq!(error.kind, SourceErrorKind::Unavailable);
    assert!(error.message.contains("berkeley"));

    fixture.write("/usr/lib/sysimage/rpm/Packages.db", b"not an ndb file");
    let located = rpm::locate(&env).unwrap();
    assert_eq!(located.format, DbFormat::Ndb);
    assert_eq!(
        located.path,
        PathBuf::from("/usr/lib/sysimage/rpm/Packages.db")
    );
    assert_eq!(
        rpm::read_packages(&env).unwrap_err().kind,
        SourceErrorKind::Corrupt
    );
}

#[test]
fn rpm_absent_database_is_not_detected() {
    let fixture = Fixture::new();
    let env = fixture.env();
    assert!(!RpmSource::new().detect(&env));
    assert_eq!(
        rpm::read_packages(&env).unwrap_err().kind,
        SourceErrorKind::Unavailable
    );
}

/// Status file with an installed command package, a removed package, a
/// library without executables and a description of two paragraphs.
const DPKG_STATUS: &str = "Package: gzip\n\
Status: install ok installed\n\
Architecture: amd64\n\
Version: 1.12-1\n\
Section: utils\n\
Homepage: https://www.gnu.org/software/gzip/\n\
Description: GNU compression utilities\n This package provides the standard GNU file compression utilities.\n .\n It also provides zcat.\n\
\n\
Package: removed-tool\n\
Status: deinstall ok config-files\n\
Version: 1.0\n\
Description: Removed package\n\
\n\
Package: libexample1\n\
Status: install ok installed\n\
Architecture: amd64\n\
Version: 2.0\n\
Section: libs\n\
Description: Example shared library\n";

fn dpkg_fixture() -> Fixture {
    let fixture = Fixture::new();
    fixture.write("/var/lib/dpkg/status", DPKG_STATUS.as_bytes());
    fixture.write(
        "/var/lib/dpkg/status.d/busybox",
        b"Package: busybox\nStatus: install ok installed\nVersion: 1.36\nDescription: Tiny utilities for small systems\n",
    );
    fixture.write(
        "/var/lib/dpkg/info/gzip.list",
        b"/.\n/usr\n/usr/bin\n/usr/bin/gzip\n/bin/zcat\n/usr/share/doc/gzip\n",
    );
    fixture.write(
        "/var/lib/dpkg/info/libexample1:amd64.list",
        b"/usr/lib/x86_64-linux-gnu/libexample.so.1\n",
    );
    fixture.write("/var/lib/dpkg/info/busybox.list", b"/bin/busybox\n");
    fixture
}

#[test]
fn dpkg_reads_status_status_directory_and_file_lists() {
    let fixture = dpkg_fixture();
    let data = dpkg::read_packages(&fixture.env()).unwrap();
    let mut packages: Vec<&str> = data
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect();
    packages.sort_unstable();
    assert_eq!(packages, vec!["busybox", "gzip", "libexample1"]);
    let gzip = data
        .packages
        .iter()
        .find(|package| package.name == "gzip")
        .unwrap();
    assert_eq!(
        gzip.summary.as_ref().unwrap().text,
        "GNU compression utilities"
    );
    assert!(
        gzip.description
            .as_ref()
            .unwrap()
            .text
            .contains("It also provides zcat.")
    );
    assert_eq!(gzip.executables, vec!["/usr/bin/gzip", "/bin/zcat"]);
    let library = data
        .packages
        .iter()
        .find(|package| package.name == "libexample1")
        .unwrap();
    assert!(library.executables.is_empty());
    let mut programs: Vec<&str> = data
        .programs
        .iter()
        .map(|program| program.name.as_str())
        .collect();
    programs.sort_unstable();
    assert_eq!(programs, vec!["busybox", "gzip", "zcat"]);
    assert!(
        data.file_packages
            .iter()
            .any(|(file, package)| file == "/usr/bin/gzip" && package.name == "gzip")
    );
}

#[test]
fn dpkg_package_summary_reaches_the_card_of_its_command() {
    let fixture = dpkg_fixture();
    fixture.executable("/usr/bin/gzip");
    let index = load_index(
        &fixture,
        vec![
            Arc::new(PathSource::new()),
            Arc::new(dpkg::DpkgSource::new()),
        ],
        None,
    );
    let (position, _) = index.resolve("gzip").unwrap();
    let card = index.card(position).unwrap();
    assert_eq!(card.summary.text, "GNU compression utilities");
    assert_eq!(card.version.as_deref(), Some("1.12-1"));
    assert_eq!(card.package.as_ref().unwrap().name, "gzip");
    assert_eq!(card.path.as_deref(), Some("/usr/bin/gzip"));
    assert!(card.installed);
}

#[test]
fn dpkg_without_status_is_unavailable() {
    let fixture = Fixture::new();
    assert!(!dpkg::DpkgSource::new().detect(&fixture.env()));
    assert_eq!(
        dpkg::read_packages(&fixture.env()).unwrap_err().kind,
        SourceErrorKind::Unavailable
    );
}

#[test]
fn pacman_reads_configured_database_path() {
    let fixture = Fixture::new();
    fixture.write(
        "/etc/pacman.conf",
        b"[options]\n# comment\nDBPath = /srv/pacman/\n",
    );
    fixture.write(
        "/srv/pacman/local/zstd-1.5.6-1/desc",
        b"%NAME%\nzstd\n\n%VERSION%\n1.5.6-1\n\n%DESC%\nZstandard compression\n\n%URL%\nhttps://facebook.github.io/zstd/\n\n%LICENSE%\nBSD-3-Clause\nGPL-2.0-only\n\n%GROUPS%\nbase-devel\n\n%REASON%\n1\n",
    );
    fixture.write(
        "/srv/pacman/local/zstd-1.5.6-1/files",
        b"%FILES%\nusr/\nusr/bin/\nusr/bin/zstd\nusr/bin/unzstd\nusr/lib/libzstd.so\n",
    );
    fixture.write("/srv/pacman/local/nameless-1-1/desc", b"%VERSION%\n1-1\n");
    fixture.write("/srv/pacman/local/ALPM_DB_VERSION", b"9\n");
    let env = fixture.env();
    assert_eq!(pacman::db_path(&env), PathBuf::from("/srv/pacman/"));
    assert!(pacman::PacmanSource::new().detect(&env));
    let data = pacman::read_packages(&env).unwrap();
    assert_eq!(data.packages.len(), 1);
    let zstd = &data.packages[0];
    assert_eq!(zstd.version.as_deref(), Some("1.5.6-1"));
    assert_eq!(
        zstd.license.as_deref(),
        Some("BSD-3-Clause AND GPL-2.0-only")
    );
    assert_eq!(zstd.groups, vec!["base-devel"]);
    assert_eq!(zstd.explicit, Some(false));
    assert_eq!(zstd.executables, vec!["/usr/bin/zstd", "/usr/bin/unzstd"]);
    assert_eq!(data.programs.len(), 2);
    assert_eq!(data.warnings.len(), 1);
    assert!(data.warnings[0].reason.contains("without name"));
}

#[test]
fn pacman_default_database_path_without_configuration() {
    let fixture = Fixture::new();
    assert_eq!(
        pacman::db_path(&fixture.env()),
        PathBuf::from("/var/lib/pacman")
    );
    assert_eq!(
        pacman::read_packages(&fixture.env()).unwrap_err().kind,
        SourceErrorKind::Unavailable
    );
}

fn desktop_entry(name: &str, exec: &str, extra: &str) -> Vec<u8> {
    format!("[Desktop Entry]\nType=Application\nName={name}\nComment={name} application\nExec={exec}\nCategories=Graphics;Viewer;\n{extra}").into_bytes()
}

#[test]
fn desktop_directories_prefer_user_entries_and_skip_hidden_ones() {
    let fixture = Fixture::new();
    fixture.write(
        "/home/user/.local/share/applications/viewer.desktop",
        &desktop_entry("User Viewer", "eog %U", ""),
    );
    fixture.write(
        "/usr/share/applications/viewer.desktop",
        &desktop_entry("System Viewer", "eog", ""),
    );
    fixture.write(
        "/usr/share/applications/hidden.desktop",
        &desktop_entry("Hidden", "hidden-tool", "NoDisplay=true\n"),
    );
    fixture.write(
        "/usr/share/applications/kde/editor.desktop",
        &desktop_entry("Editor", "kwrite", ""),
    );
    fixture.write("/usr/share/applications/notes.txt", b"not an entry");
    let env = fixture.env();
    let source = DesktopSource::new();
    assert!(source.detect(&env));
    assert!(!source.detect(&HostEnv::new(Os::Macos).with_root(fixture.path())));
    let data = source.load(&env, &Deadline::none()).unwrap();
    let mut programs: Vec<(String, String)> = data
        .programs
        .iter()
        .map(|program| {
            (
                program.name.clone(),
                program.app_name.as_ref().unwrap().text.clone(),
            )
        })
        .collect();
    programs.sort();
    assert_eq!(
        programs,
        vec![
            ("eog".to_owned(), "User Viewer".to_owned()),
            ("kwrite".to_owned(), "Editor".to_owned())
        ]
    );
    let editor = data
        .programs
        .iter()
        .find(|program| program.name == "kwrite")
        .unwrap();
    assert_eq!(editor.app_id.as_deref(), Some("kde-editor"));
}

#[test]
fn desktop_application_merges_with_command_in_path() {
    let fixture = Fixture::new();
    fixture.executable("/usr/bin/eog");
    fixture.write(
        "/usr/share/applications/viewer.desktop",
        &desktop_entry("Image Viewer", "eog", ""),
    );
    fixture.write(
        "/usr/share/applications/draw.desktop",
        &desktop_entry("Drawing", "draw-tool", ""),
    );
    let index = load_index(
        &fixture,
        vec![Arc::new(PathSource::new()), Arc::new(DesktopSource::new())],
        None,
    );
    let (position, _) = index.resolve("eog").unwrap();
    let card = index.card(position).unwrap();
    assert_eq!(card.kind, EntryKind::Program);
    assert_eq!(card.app_name.as_ref().unwrap().text, "Image Viewer");
    assert!(
        matches!(card.category.as_str(), "images" | "graphics"),
        "{}",
        card.category
    );
    let (position, alias) = index.resolve("Drawing").unwrap();
    assert_eq!(alias, None);
    assert_eq!(index.card(position).unwrap().kind, EntryKind::App);
}

fn gzip_bytes(text: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(text).unwrap();
    encoder.finish().unwrap()
}

fn xz_bytes(text: &[u8]) -> Vec<u8> {
    let mut encoder = liblzma::write::XzEncoder::new(Vec::new(), 6);
    encoder.write_all(text).unwrap();
    encoder.finish().unwrap()
}

fn bzip2_bytes(text: &[u8]) -> Vec<u8> {
    let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
    encoder.write_all(text).unwrap();
    encoder.finish().unwrap()
}

fn roff_page(name: &str, description: &str) -> Vec<u8> {
    format!(
        ".TH {} 1\n.SH NAME\n{name} \\- {description}\n.SH SYNOPSIS\n.B {name}\n",
        name.to_uppercase()
    )
    .into_bytes()
}

/// German page in ISO-8859-1: several umlauts and a sharp s, enough for
/// the encoding detector.
const GERMAN_PAGE: &[u8] = b".TH GZIP 1\n.SH NAME\ngzip \\- komprimiert und dekomprimiert Dateien f\xfcr die \xdcbertragung, gr\xf6\xdfere Dateien werden k\xfcrzer und \xe4ltere Formate bleiben lesbar\n";

fn man_fixture() -> Fixture {
    let fixture = Fixture::new();
    fixture.write(
        "/usr/share/man/man1/gzip.1.gz",
        &gzip_bytes(&roff_page("gzip", "compress or expand files")),
    );
    fixture.write(
        "/usr/share/man/man1/xz.1.xz",
        &xz_bytes(&roff_page("xz", "compress .xz files")),
    );
    fixture.write(
        "/usr/share/man/man1/bzip2.1.bz2",
        &bzip2_bytes(&roff_page("bzip2", "block-sorting file compressor")),
    );
    fixture.write(
        "/usr/share/man/man1/zstd.1.zst",
        &zstd::encode_all(roff_page("zstd", "zstandard compression").as_slice(), 3).unwrap(),
    );
    fixture.write(
        "/usr/share/man/man1/cat.1",
        b".Dd 2024\n.Dt CAT 1\n.Sh NAME\n.Nm cat\n.Nd concatenate and print files\n",
    );
    fixture.write(
        "/usr/share/man/man1/zcat.1.gz",
        &gzip_bytes(b".so man1/gzip.1\n"),
    );
    fixture.write(
        "/usr/share/man/man8/sshd.8",
        &roff_page("sshd", "OpenSSH daemon"),
    );
    fixture.write(
        "/usr/share/man/man3/printf.3",
        &roff_page("printf", "formatted output conversion"),
    );
    fixture.write("/usr/share/man/de/man1/gzip.1", GERMAN_PAGE);
    fixture.write("/usr/local/share/man/whatis", b"tool(1) - does tool things\nadmin(8) - administers the system\nlibcall(3) - library call\n");
    fixture
}

#[test]
fn man_pages_in_four_compressions_with_redirect_translation_and_whatis() {
    let fixture = man_fixture();
    let env = fixture
        .env()
        .with_languages(vec!["en".to_owned(), "de".to_owned()]);
    let source = ManSource::new();
    assert!(source.detect(&env));
    let data = source.load(&env, &Deadline::none()).unwrap();
    let summaries: HashMap<&str, &LocalizedText> = data
        .programs
        .iter()
        .map(|program| (program.name.as_str(), program.summary.as_ref().unwrap()))
        .collect();
    assert_eq!(summaries["gzip"].text, "compress or expand files");
    assert_eq!(summaries["xz"].text, "compress .xz files");
    assert_eq!(summaries["bzip2"].text, "block-sorting file compressor");
    assert_eq!(summaries["zstd"].text, "zstandard compression");
    assert_eq!(summaries["cat"].text, "concatenate and print files");
    assert_eq!(summaries["zcat"].text, "compress or expand files");
    assert_eq!(summaries["sshd"].text, "OpenSSH daemon");
    assert_eq!(summaries["tool"].text, "does tool things");
    assert_eq!(summaries["admin"].text, "administers the system");
    assert!(!summaries.contains_key("printf"));
    assert!(!summaries.contains_key("libcall"));
    let (german, lang) = summaries["gzip"].pick(&["de".to_owned()]);
    assert_eq!(lang, Some("de"));
    assert!(german.starts_with("komprimiert und dekomprimiert Dateien"));
    assert!(!german.contains('\u{fffd}'));
    assert!(data.warnings.is_empty());

    let gzip = data
        .programs
        .iter()
        .find(|program| program.name == "gzip")
        .unwrap();
    let detail = source
        .detail(
            &env,
            gzip.key.as_ref().unwrap(),
            &DetailWant {
                man: true,
                ..DetailWant::default()
            },
        )
        .unwrap()
        .unwrap();
    assert!(
        detail
            .man
            .unwrap()
            .starts_with("gzip - compress or expand files")
    );
    let none = source
        .detail(&env, gzip.key.as_ref().unwrap(), &DetailWant::default())
        .unwrap();
    assert!(none.is_none());
}

#[test]
fn man_damaged_page_warns_and_broken_redirect_is_reported() {
    let fixture = Fixture::new();
    fixture.write("/usr/share/man/man1/broken.1.gz", b"not gzip data");
    fixture.write("/usr/share/man/man1/lost.1", b".so man1/missing.1\n");
    fixture.write("/usr/share/man/man1/ok.1", &roff_page("ok", "works"));
    let data = ManSource::new()
        .load(&fixture.env(), &Deadline::none())
        .unwrap();
    assert_eq!(data.programs.len(), 1);
    assert_eq!(data.warnings.len(), 2);
    let key = EntryKey::new(&SourceId::new(SourceId::MAN), "/usr/share/man/man1/lost.1");
    let want = DetailWant {
        man: true,
        ..DetailWant::default()
    };
    assert!(
        ManSource::new()
            .detail(&fixture.env(), &key, &want)
            .is_err()
    );
}

#[test]
fn man_dirs_follow_manpath_and_configuration() {
    let fixture = Fixture::new();
    for dir in [
        "/opt/man/man1",
        "/usr/share/man/man1",
        "/srv/tools/man/man1",
        "/usr/local/share/man/man1",
    ] {
        std::fs::create_dir_all(fixture.path().join(dir.trim_start_matches('/'))).unwrap();
    }
    fixture.write(
        "/etc/manpath.config",
        b"MANDATORY_MANPATH /srv/tools/man\nMANPATH_MAP /usr/bin /usr/share/man\n",
    );
    let plain = man_dirs(&fixture.env());
    assert_eq!(plain[0], PathBuf::from("/usr/local/share/man"));
    assert!(plain.contains(&PathBuf::from("/srv/tools/man")));
    assert!(!plain.contains(&PathBuf::from("/opt/man")));
    let env = fixture.env().with_var("MANPATH", "/opt/man:");
    let dirs = man_dirs(&env);
    assert_eq!(dirs[0], PathBuf::from("/opt/man"));
    assert!(dirs.contains(&PathBuf::from("/usr/share/man")));
    let unique: std::collections::HashSet<&PathBuf> = dirs.iter().collect();
    assert_eq!(unique.len(), dirs.len());
}

#[test]
fn dpkg_translated_descriptions_from_apt_lists() {
    let fixture = dpkg_fixture();
    let translation = b"Package: gzip\nDescription-md5: 0123\nDescription-de: GNU Kompressionsprogramme\n Dieses Paket enthaelt die GNU Werkzeuge.\n\nPackage: unknown\nDescription-de: Nicht installiert\n\n";
    fixture.write(
        "/var/lib/apt/lists/deb.example.org_dists_stable_main_i18n_Translation-de.gz",
        &gzip_bytes(translation),
    );
    fixture.write(
        "/var/lib/apt/lists/deb.example.org_dists_stable_contrib_i18n_Translation-de_DE.xz",
        b"not xz data",
    );
    fixture.write(
        "/var/lib/apt/lists/deb.example.org_dists_stable_InRelease",
        b"signed index",
    );
    let env = fixture
        .env()
        .with_languages(vec!["en".to_owned(), "de".to_owned()]);
    let data = dpkg::read_packages(&env).unwrap();
    let gzip = data
        .packages
        .iter()
        .find(|package| package.name == "gzip")
        .unwrap();
    let summary = gzip.summary.as_ref().unwrap();
    assert_eq!(
        summary.pick(&["de".to_owned()]),
        ("GNU Kompressionsprogramme", Some("de"))
    );
    assert_eq!(
        summary.pick(&["en".to_owned()]).0,
        "GNU compression utilities"
    );
    let description = gzip.description.as_ref().unwrap();
    assert!(
        description
            .pick(&["de".to_owned()])
            .0
            .contains("GNU Werkzeuge")
    );
    assert_eq!(data.warnings.len(), 1);
    assert_eq!(data.warnings[0].source.as_str(), SourceId::DPKG);
    let english_only = dpkg::read_packages(&fixture.env()).unwrap();
    assert!(english_only.warnings.is_empty());
}

fn set_mtime(file: &Path, time: SystemTime) {
    std::fs::File::options()
        .write(true)
        .open(file)
        .unwrap()
        .set_modified(time)
        .unwrap();
}

#[test]
fn index_cache_invalidation_on_mtime() {
    let fixture = dpkg_fixture();
    fixture.executable("/usr/bin/gzip");
    let cache = fixture.path().join("cache");
    let status = fixture.path().join("var/lib/dpkg/status");
    let fixed = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
    set_mtime(&status, fixed);
    let sources = || -> Vec<Arc<dyn ProgramSource>> {
        vec![
            Arc::new(PathSource::new()),
            Arc::new(dpkg::DpkgSource::new()),
        ]
    };

    let first = load_index(&fixture, sources(), Some(cache.clone()));
    assert_eq!(card_summary(&first, "gzip"), "GNU compression utilities");
    let file = cache_path(&cache, &SourceId::new(SourceId::DPKG));
    assert!(file.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    let same_size = DPKG_STATUS.replace("GNU compression utilities", "GNU packing utilities ABC");
    assert_eq!(same_size.len(), DPKG_STATUS.len());
    std::fs::write(&status, same_size.as_bytes()).unwrap();
    set_mtime(&status, fixed);
    let cached = load_index(&fixture, sources(), Some(cache.clone()));
    assert_eq!(card_summary(&cached, "gzip"), "GNU compression utilities");

    set_mtime(&status, fixed + Duration::from_secs(60));
    let rebuilt = load_index(&fixture, sources(), Some(cache.clone()));
    assert_eq!(card_summary(&rebuilt, "gzip"), "GNU packing utilities ABC");
}

#[test]
fn index_rebuilds_damaged_cache_file() {
    let fixture = dpkg_fixture();
    fixture.executable("/usr/bin/gzip");
    let cache = fixture.path().join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(
        cache_path(&cache, &SourceId::new(SourceId::DPKG)),
        b"\xff\x00 damaged",
    )
    .unwrap();
    let index = load_index(
        &fixture,
        vec![
            Arc::new(PathSource::new()),
            Arc::new(dpkg::DpkgSource::new()),
        ],
        Some(cache),
    );
    assert_eq!(card_summary(&index, "gzip"), "GNU compression utilities");
    assert!(index.warnings().is_empty());
}

#[test]
fn cache_file_names_replace_reserved_characters() {
    let file = cache_path(Path::new("/c"), &SourceId::catalog("media"));
    assert_eq!(file, PathBuf::from("/c/catalog_media.cbor"));
}

fn static_source(id: &str, kind: SourceKind, programs: Vec<RawProgram>) -> StaticSource {
    StaticSource {
        id: SourceId::new(id),
        kind,
        priority: 0,
        data: Ok(SourceData {
            programs,
            ..SourceData::default()
        }),
        guides: HashMap::new(),
        loads: AtomicUsize::new(0),
    }
}

#[test]
fn index_joins_application_executable_and_catalog_package_name() {
    let fixture = dpkg_fixture();
    fixture.executable("/usr/bin/gzip");
    fixture.executable("/usr/bin/eog");
    let mut app = RawProgram::new("Image Viewer.app");
    app.kind = EntryKind::App;
    app.app_name = Some(LocalizedText::in_lang("Image Viewer", "en"));
    app.executable = Some("eog".to_owned());
    app.path = Some("/Applications/Image Viewer.app".to_owned());
    let mut entry = RawProgram::new("packer");
    entry.summary = Some(LocalizedText::in_lang("Packs files for transfer.", "en"));
    entry.package_name = Some("gzip".to_owned());
    entry.installed = Some(false);
    let index = load_index(
        &fixture,
        vec![
            Arc::new(PathSource::new()),
            Arc::new(dpkg::DpkgSource::new()),
            Arc::new(static_source("apps", SourceKind::App, vec![app])),
            Arc::new(static_source(
                "catalog:tools",
                SourceKind::Catalog,
                vec![entry],
            )),
        ],
        None,
    );
    let (position, _) = index.resolve("eog").unwrap();
    let eog = index.card(position).unwrap();
    assert_eq!(eog.kind, EntryKind::Program);
    assert_eq!(eog.app_name.as_ref().unwrap().text, "Image Viewer");
    assert!(eog.sources.iter().any(|source| source.as_str() == "apps"));
    assert!(index.resolve("Image Viewer").is_none());

    let (position, _) = index.resolve("packer").unwrap();
    let packer = index.card(position).unwrap();
    let package = packer.package.as_ref().unwrap();
    assert_eq!(
        (package.manager.as_str(), package.name.as_str()),
        (SourceId::DPKG, "gzip")
    );
    assert_eq!(packer.version.as_deref(), Some("1.12-1"));
    assert_eq!(packer.summary.text, "Packs files for transfer.");
}
