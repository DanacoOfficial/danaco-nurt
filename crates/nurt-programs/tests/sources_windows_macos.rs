// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Providers of Homebrew, winget, Scoop, Chocolatey, Windows applications
//! and macOS application bundles, read from synthetic trees below a
//! temporary root. Windows and macOS paths are host paths of the trees and
//! are resolved below the root on every system.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code fails loudly on broken fixtures"
)]

use std::path::Path;
use std::sync::Arc;

use nurt_contracts::env::Os;
use nurt_programs::model::{
    AliasKind, CategoryScheme, DetailWant, EntryKey, EntryKind, RawPackage, RawProgram, SourceData,
};
use nurt_programs::source::{
    Deadline, HostEnv, MemoryRegistry, ProgramSource, RegistryHive, RegistryReader, RegistryValue,
    RegistryView, SourceId, WarningCode,
};
use nurt_programs::sources::brew::{self, BrewSource};
use nurt_programs::sources::choco::{self, ChocoSource};
use nurt_programs::sources::macapp::{self, MacAppSource};
use nurt_programs::sources::scoop::{self, ScoopSource};
use nurt_programs::sources::winapp::{self, WinAppSource};
use nurt_programs::sources::winget::{self, WingetSource};

const UNINSTALL: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall";
const APP_PATHS: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths";

fn write(root: &Path, relative: &str, bytes: impl AsRef<[u8]>) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// Creates a symbolic link; returns `false` where the account may not
/// create links (Windows without the privilege).
fn link(target: &str, root: &Path, relative: &str) -> bool {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, path).is_ok()
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, path).is_ok()
    }
}

fn load(source: &dyn ProgramSource, env: &HostEnv) -> SourceData {
    source.load(env, &Deadline::none()).unwrap()
}

fn program<'a>(data: &'a SourceData, name: &str) -> &'a RawProgram {
    data.programs
        .iter()
        .find(|program| program.name == name)
        .unwrap_or_else(|| panic!("no program {name} in {:?}", data.programs))
}

fn package<'a>(data: &'a SourceData, name: &str) -> &'a RawPackage {
    data.packages
        .iter()
        .find(|package| package.name == name)
        .unwrap_or_else(|| panic!("no package {name} in {:?}", data.packages))
}

fn mac_env(root: &Path) -> HostEnv {
    HostEnv::new(Os::Macos)
        .with_root(root)
        .with_home("/Users/user")
}

fn windows_env(root: &Path) -> HostEnv {
    HostEnv::new(Os::Windows)
        .with_root(root)
        .with_home("C:\\Users\\user")
        .with_var("LOCALAPPDATA", "C:\\Users\\user\\AppData\\Local")
        .with_var("ProgramFiles", "C:\\Program Files")
}

#[test]
fn brew_receipt_version() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let cellar = "opt/homebrew/Cellar/jq";
    write(root, &format!("{cellar}/1.6/INSTALL_RECEIPT.json"), "{}");
    write(
        root,
        &format!("{cellar}/1.7.1/INSTALL_RECEIPT.json"),
        r#"{"installed_on_request": true, "time": 1700000000,
            "source": {"tap": "homebrew/core", "versions": {"stable": "1.7.1"}}}"#,
    );
    write(
        root,
        &format!("{cellar}/1.7.1/.brew/jq.rb"),
        "class Jq < Formula\n  desc \"Lightweight JSON processor\"\n  homepage \"https://jqlang.org\"\n  license \"MIT\"\nend\n",
    );
    let linked = link("../Cellar/jq/1.7.1/bin/jq", root, "opt/homebrew/bin/jq");
    let env = mac_env(root);
    let source = BrewSource::new();
    assert!(source.detect(&env));
    assert_eq!(source.id().as_str(), SourceId::BREW);
    let data = load(&source, &env);
    let jq = package(&data, "jq");
    assert_eq!(jq.version.as_deref(), Some("1.7.1"));
    assert_eq!(jq.explicit, Some(true));
    assert_eq!(jq.origin.as_deref(), Some("homebrew/core"));
    assert_eq!(jq.installed_at.as_deref(), Some("2023-11-14T22:13:20Z"));
    assert_eq!(jq.license.as_deref(), Some("MIT"));
    assert_eq!(
        jq.summary.as_ref().unwrap().text,
        "Lightweight JSON processor"
    );
    if linked {
        let command = program(&data, "jq");
        assert_eq!(command.version.as_deref(), Some("1.7.1"));
        assert_eq!(
            command.package.as_ref().unwrap().manager.as_str(),
            SourceId::BREW
        );
        assert_eq!(data.file_packages.len(), 1);
        assert_eq!(jq.executables.len(), 1);
    }
}

#[test]
fn brew_cask_app() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "usr/local/Caskroom/firefox/.metadata/128.0/20240701120000.000/Casks/firefox.json",
        r#"{"token": "firefox", "name": ["Mozilla Firefox"], "desc": "Web browser",
            "homepage": "https://www.mozilla.org/firefox/", "version": "128.0",
            "artifacts": [{"app": ["Firefox.app"]}, {"zap": []}]}"#,
    );
    let env = mac_env(root);
    let data = load(&BrewSource::new(), &env);
    let app = program(&data, "Firefox");
    assert_eq!(app.kind, EntryKind::App);
    assert_eq!(app.path.as_deref(), Some("/Applications/Firefox.app"));
    assert_eq!(app.app_name.as_ref().unwrap().text, "Mozilla Firefox");
    assert_eq!(app.version.as_deref(), Some("128.0"));
    assert_eq!(package(&data, "firefox").groups, ["cask"]);
}

#[test]
fn brew_api_cache_fills_missing_descriptions() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "opt/homebrew/Cellar/wget/1.24.5/INSTALL_RECEIPT.json",
        "{}",
    );
    write(
        root,
        "opt/homebrew/Caskroom/iterm2/.metadata/3.5.4/1/Casks/iterm2.rb",
        "cask \"iterm2\" do\nend\n",
    );
    let formulae = serde_json::json!([
        {"name": "wget", "desc": "Internet file retriever", "license": "GPL-3.0-or-later",
         "versions": {"stable": "1.24.5"}}
    ]);
    let casks = serde_json::json!([
        {"token": "iterm2", "name": ["iTerm2"], "desc": "Terminal emulator"}
    ]);
    let jws = |payload: &serde_json::Value| {
        serde_json::json!({"payload": payload.to_string(), "signatures": [{"signature": "c2ln"}]})
            .to_string()
    };
    write(root, "cache/api/formula.jws.json", jws(&formulae));
    write(root, "cache/api/cask.jws.json", jws(&casks));
    let env = mac_env(root).with_var("HOMEBREW_CACHE", "/cache");
    let data = load(&BrewSource::new(), &env);
    let wget = package(&data, "wget");
    assert_eq!(
        wget.summary.as_ref().unwrap().text,
        "Internet file retriever"
    );
    assert_eq!(wget.license.as_deref(), Some("GPL-3.0-or-later"));
    assert_eq!(
        package(&data, "iterm2").summary.as_ref().unwrap().text,
        "Terminal emulator"
    );
    assert!(brew::parse_api_cache(r#"{"payload": "[]"}"#).is_err());
    assert!(
        brew::parse_api_cache(r#"{"payload": "{}", "signatures": [{"signature": "x"}]}"#).is_err()
    );
}

#[test]
fn brew_prefixes_and_cache_follow_the_system() {
    let linux = HostEnv::new(Os::Linux).with_home("/home/user");
    assert_eq!(
        brew::prefixes(&linux),
        [Path::new("/home/linuxbrew/.linuxbrew")]
    );
    assert_eq!(
        brew::cache_dir(&linux).unwrap(),
        Path::new("/home/user/.cache/Homebrew")
    );
    let xdg = linux.clone().with_var("XDG_CACHE_HOME", "/var/cache/ada");
    assert_eq!(
        brew::cache_dir(&xdg).unwrap(),
        Path::new("/var/cache/ada/Homebrew")
    );
    let mac = HostEnv::new(Os::Macos)
        .with_home("/Users/user")
        .with_var("HOMEBREW_PREFIX", "/opt/custom");
    assert_eq!(brew::prefixes(&mac)[0], Path::new("/opt/custom"));
    assert_eq!(brew::prefixes(&mac).len(), 3);
    assert_eq!(
        brew::cache_dir(&mac).unwrap(),
        Path::new("/Users/user/Library/Caches/Homebrew")
    );
    assert!(brew::prefixes(&HostEnv::new(Os::Windows)).is_empty());
}

#[test]
fn brew_formula_reader_ignores_computed_values() {
    let info =
        brew::parse_formula_rb("  desc \"Tool #{version}\"\n  homepage \"https://example.org\"\n");
    assert_eq!(info.desc, None);
    assert_eq!(info.homepage.as_deref(), Some("https://example.org"));
    let receipt = brew::parse_receipt("{\"installed_on_request\": false}").unwrap();
    assert_eq!(receipt.on_request, Some(false));
    assert!(brew::parse_receipt("[]").is_err());
}

#[test]
fn brew_bad_receipt_warns() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "opt/homebrew/Cellar/tree/2.1.1/INSTALL_RECEIPT.json",
        "not json",
    );
    let data = load(&BrewSource::new(), &mac_env(dir.path()));
    assert_eq!(package(&data, "tree").version.as_deref(), Some("2.1.1"));
    assert_eq!(data.warnings.len(), 1);
    assert_eq!(data.warnings[0].code, WarningCode::SourceUnavailable);
}

fn tracking_db(path: &Path) {
    let connection = rusqlite::Connection::open(path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE ids(id TEXT);
             CREATE TABLE names(name TEXT);
             CREATE TABLE versions(version TEXT);
             CREATE TABLE monikers(moniker TEXT);
             CREATE TABLE tags(tag TEXT);
             CREATE TABLE manifest(id INTEGER, name INTEGER, version INTEGER, moniker INTEGER);
             CREATE TABLE tags_map(manifest INTEGER, tag INTEGER);
             INSERT INTO ids(rowid, id) VALUES (1, 'BurntSushi.ripgrep.MSVC'), (2, 'Git.Git');
             INSERT INTO names(rowid, name) VALUES (1, 'RipGrep MSVC'), (2, 'Git');
             INSERT INTO versions(rowid, version) VALUES (1, '14.1.1'), (2, '2.47.0');
             INSERT INTO monikers(rowid, moniker) VALUES (1, 'ripgrep');
             INSERT INTO tags(rowid, tag) VALUES (1, 'search'), (2, 'grep');
             INSERT INTO manifest(rowid, id, name, version, moniker) VALUES (1, 1, 1, 1, 1), (2, 2, 2, 2, NULL);
             INSERT INTO tags_map(manifest, tag) VALUES (1, 1), (1, 2);",
        )
        .unwrap();
}

#[test]
fn winget_links_registry_and_tracking_database() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let local = "c/Users/user/AppData/Local";
    let package_dir = "BurntSushi.ripgrep.MSVC_Microsoft.Winget.Source_8wekyb3d8bbwe";
    std::fs::create_dir_all(root.join(format!("{local}/Microsoft/WinGet/Packages/{package_dir}")))
        .unwrap();
    let target = format!(
        "C:\\Users\\user\\AppData\\Local\\Microsoft\\WinGet\\Packages\\{package_dir}\\rg.exe"
    );
    let linked = link(
        &target,
        root,
        &format!("{local}/Microsoft/WinGet/Links/rg.exe"),
    );
    let db_dir = root.join(format!(
        "{local}/{}",
        winget::TRACKING_DIR.replace('\\', "/")
    ));
    std::fs::create_dir_all(&db_dir).unwrap();
    tracking_db(&db_dir.join(winget::TRACKING_DB));
    let mut registry = MemoryRegistry::new();
    let key = format!("{UNINSTALL}\\Git_is1");
    registry.insert_value(
        RegistryHive::LocalMachine,
        RegistryView::Native,
        &key,
        "WinGetPackageIdentifier",
        RegistryValue::String("Git.Git".to_owned()),
    );
    registry.insert_value(
        RegistryHive::LocalMachine,
        RegistryView::Native,
        &key,
        "Publisher",
        RegistryValue::String("The Git Development Community".to_owned()),
    );
    let env = windows_env(root).with_registry(Arc::new(registry));
    let source = WingetSource::new();
    assert!(source.detect(&env));
    assert!(!source.detect(&mac_env(root)));
    let data = load(&source, &env);
    let ripgrep = package(&data, "BurntSushi.ripgrep.MSVC");
    assert_eq!(ripgrep.version.as_deref(), Some("14.1.1"));
    assert_eq!(ripgrep.groups, ["search", "grep"]);
    let git = package(&data, "Git.Git");
    assert_eq!(git.version.as_deref(), Some("2.47.0"));
    assert_eq!(git.origin.as_deref(), Some("The Git Development Community"));
    if linked {
        let rg = program(&data, "rg");
        assert_eq!(rg.package.as_ref().unwrap().name, "BurntSushi.ripgrep.MSVC");
        assert_eq!(rg.tags, ["search", "grep"]);
    }
    let first = source.fingerprint(&env).unwrap();
    assert_eq!(first, source.fingerprint(&env).unwrap());
}

#[test]
fn winget_unknown_tracking_schema_warns() {
    let dir = tempfile::tempdir().unwrap();
    let db_dir = dir.path().join(format!(
        "c/Users/user/AppData/Local/{}",
        winget::TRACKING_DIR.replace('\\', "/")
    ));
    std::fs::create_dir_all(&db_dir).unwrap();
    let connection = rusqlite::Connection::open(db_dir.join(winget::TRACKING_DB)).unwrap();
    connection
        .execute_batch("CREATE TABLE other(x TEXT);")
        .unwrap();
    drop(connection);
    let data = load(&WingetSource::new(), &windows_env(dir.path()));
    assert!(data.packages.is_empty());
    assert_eq!(data.warnings.len(), 1);
    assert!(data.warnings[0].reason.contains("unknown schema"));
}

#[test]
fn winget_package_names_from_directories_and_links() {
    assert_eq!(winget::package_of_dir("NoSource"), None);
    assert_eq!(
        winget::package_of_link("C:/Users/user/AppData/Local/Microsoft/WinGet/Packages/jqlang.jq_Microsoft.Winget.Source_8wekyb3d8bbwe/jq.exe")
            .as_deref(),
        Some("jqlang.jq")
    );
    assert_eq!(winget::package_of_link("C:\\Tools\\jq.exe"), None);
}

#[test]
fn scoop_manifest_shim() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let scoop = "c/Users/user/scoop";
    write(
        root,
        &format!("{scoop}/apps/ripgrep/current/manifest.json"),
        "\u{feff}{\"version\": \"14.1.1\", \"description\": \"Recursively searches directories\", \
         \"homepage\": \"https://github.com/BurntSushi/ripgrep\", \"license\": {\"identifier\": \"MIT\"}, \"bin\": \"rg.exe\"}",
    );
    write(
        root,
        &format!("{scoop}/apps/ripgrep/current/install.json"),
        r#"{"bucket": "main"}"#,
    );
    write(
        root,
        &format!("{scoop}/shims/rg.shim"),
        "path = \"C:\\Users\\user\\scoop\\apps\\ripgrep\\current\\rg.exe\"\n",
    );
    write(
        root,
        &format!("{scoop}/apps/fd/current/manifest.json"),
        r#"{"version": "10.2.0", "bin": [["fd.exe", "fdx"]]}"#,
    );
    write(
        root,
        &format!("{scoop}/apps/broken/current/manifest.json"),
        "[1, 2]",
    );
    let env = windows_env(root);
    let source = ScoopSource::new();
    assert!(source.detect(&env));
    let data = load(&source, &env);
    let rg = program(&data, "rg");
    assert_eq!(rg.version.as_deref(), Some("14.1.1"));
    assert_eq!(rg.license.as_deref(), Some("MIT"));
    assert_eq!(rg.package.as_ref().unwrap().name, "ripgrep");
    assert_eq!(package(&data, "ripgrep").origin.as_deref(), Some("main"));
    let fdx = program(&data, "fdx");
    assert_eq!(fdx.alias_of.as_deref(), Some("fd"));
    assert_eq!(fdx.alias_kind, Some(AliasKind::Shim));
    assert_eq!(data.warnings.len(), 1);
    assert_eq!(data.warnings[0].name.as_deref(), Some("broken"));
    assert_ne!(
        source.fingerprint(&env).unwrap(),
        source
            .fingerprint(&windows_env(root).with_var("SCOOP", "D:\\scoop"))
            .unwrap()
    );
}

#[test]
fn scoop_roots_and_shim_targets() {
    let env = HostEnv::new(Os::Windows)
        .with_var("SCOOP", "D:\\scoop")
        .with_var("SCOOP_GLOBAL", "E:\\scoop");
    assert_eq!(
        scoop::roots(&env),
        [Path::new("D:\\scoop"), Path::new("E:\\scoop")]
    );
    assert_eq!(scoop::shim_application("args = -v\n"), None);
    assert!(scoop::parse_manifest("not json").is_err());
    let manifest = scoop::parse_manifest(r#"{"bin": [["tools\\x.exe", ""], 7]}"#).unwrap();
    assert_eq!(manifest.bin.len(), 1);
    assert_eq!(manifest.bin[0].command, "x");
}

#[test]
fn choco_nuspec_summary() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let choco = "c/ProgramData/chocolatey";
    write(
        root,
        &format!("{choco}/lib/git/git.nuspec"),
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://schemas.microsoft.com/packaging/2015/06/nuspec.xsd">
  <metadata>
    <id>git</id>
    <version>2.47.0</version>
    <title>Git</title>
    <summary>Distributed version control</summary>
    <description>Git tracks changes &amp; merges branches.</description>
    <tags>vcs git scm</tags>
    <projectUrl>https://git-scm.com/</projectUrl>
    <license type="expression">GPL-2.0-only</license>
    <dependencies><dependency id="chocolatey-core.extension" /></dependencies>
  </metadata>
  <files />
</package>"#,
    );
    write(root, &format!("{choco}/lib/git/tools/cmd/git.exe"), "MZ");
    write(root, &format!("{choco}/bin/git.exe"), "MZ");
    write(root, &format!("{choco}/bin/orphan.exe"), "MZ");
    write(
        root,
        &format!("{choco}/lib/bad/bad.nuspec"),
        "<package><metadata>",
    );
    let env = windows_env(root);
    let source = ChocoSource::new();
    assert!(source.detect(&env));
    let data = load(&source, &env);
    let git = program(&data, "git");
    assert_eq!(
        git.summary.as_ref().unwrap().text,
        "Distributed version control"
    );
    assert_eq!(
        git.description.as_ref().unwrap().text,
        "Git tracks changes & merges branches."
    );
    assert_eq!(git.version.as_deref(), Some("2.47.0"));
    assert_eq!(git.tags, ["vcs", "git", "scm"]);
    assert_eq!(git.homepage.as_deref(), Some("https://git-scm.com/"));
    assert!(data.programs.iter().all(|program| program.name != "orphan"));
    assert_eq!(package(&data, "git").executables.len(), 1);
    assert_eq!(data.warnings.len(), 1);
    assert_eq!(
        choco::root(&HostEnv::new(Os::Windows)),
        Path::new(choco::DEFAULT_ROOT)
    );
}

fn text(value: &str) -> RegistryValue {
    RegistryValue::String(value.to_owned())
}

fn uninstall(
    registry: &mut MemoryRegistry,
    view: (RegistryHive, RegistryView),
    key: &str,
    values: &[(&str, RegistryValue)],
) {
    for (name, value) in values {
        registry.insert_value(
            view.0,
            view.1,
            &format!("{UNINSTALL}\\{key}"),
            name,
            value.clone(),
        );
    }
}

fn app_registry() -> MemoryRegistry {
    let native = (RegistryHive::LocalMachine, RegistryView::Native);
    let wow = (RegistryHive::LocalMachine, RegistryView::Wow32);
    let user = (RegistryHive::CurrentUser, RegistryView::Native);
    let editor = [
        ("DisplayName", text("Example Editor")),
        ("DisplayVersion", text("3.2")),
        ("Publisher", text("Example Ltd")),
        (
            "InstallLocation",
            RegistryValue::ExpandString("%ProgramFiles%\\Editor".to_owned()),
        ),
        (
            "DisplayIcon",
            text("\"C:\\Program Files\\Editor\\editor.exe\",0"),
        ),
        ("URLInfoAbout", text("https://editor.example")),
    ];
    let mut registry = MemoryRegistry::new();
    uninstall(&mut registry, native, "{6F1C}", &editor);
    uninstall(&mut registry, wow, "{6F1C}", &editor);
    uninstall(
        &mut registry,
        wow,
        "Viewer32",
        &[
            ("DisplayName", text("Legacy Viewer")),
            ("DisplayVersion", text("1.0")),
        ],
    );
    uninstall(
        &mut registry,
        user,
        "UserTool",
        &[("DisplayName", text("User Tool"))],
    );
    uninstall(
        &mut registry,
        native,
        "KB5005565",
        &[
            ("DisplayName", text("Security Update")),
            ("ParentKeyName", text("OperatingSystem")),
        ],
    );
    uninstall(
        &mut registry,
        native,
        "Runtime",
        &[
            ("DisplayName", text("Runtime Component")),
            ("SystemComponent", RegistryValue::Dword(1)),
        ],
    );
    uninstall(
        &mut registry,
        native,
        "NoName",
        &[("DisplayVersion", text("9"))],
    );
    registry.insert_value(
        native.0,
        native.1,
        &format!("{APP_PATHS}\\exed.exe"),
        "",
        RegistryValue::ExpandString("%ProgramFiles%\\Editor\\editor.exe".to_owned()),
    );
    registry.insert_value(
        native.0,
        native.1,
        &format!("{APP_PATHS}\\standalone.exe"),
        "",
        text("\"C:\\Tools\\standalone.exe\""),
    );
    registry.insert_value(
        user.0,
        user.1,
        &format!("{APP_PATHS}\\notes.txt"),
        "",
        text("C:\\Tools\\notes.txt"),
    );
    registry
}

#[test]
fn memory_registry_keeps_key_case_and_matches_without_case() {
    let (hive, view) = (RegistryHive::LocalMachine, RegistryView::Native);
    let mut registry = MemoryRegistry::new();
    registry.insert_key(hive, view, &format!("{UNINSTALL}\\{{6F1C}}"));
    registry.insert_key(hive, view, &format!("{UNINSTALL}\\Viewer32"));
    registry.insert_value(
        hive,
        view,
        &format!("{}\\viewer32", UNINSTALL.to_ascii_uppercase()),
        "DisplayName",
        text("Legacy Viewer"),
    );
    let mut names = registry
        .subkeys(hive, view, &UNINSTALL.to_ascii_lowercase())
        .unwrap()
        .unwrap();
    names.sort_unstable();
    assert_eq!(names, ["Viewer32", "{6F1C}"]);
    let values = registry
        .values(hive, view, &format!("{UNINSTALL}\\VIEWER32"))
        .unwrap()
        .unwrap();
    assert_eq!(
        values.get("DisplayName").and_then(RegistryValue::as_text),
        Some("Legacy Viewer")
    );
    assert!(
        registry
            .subkeys(hive, view, &format!("{UNINSTALL}\\Missing"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn winapp_uninstall_both_views() {
    let dir = tempfile::tempdir().unwrap();
    let env = windows_env(dir.path()).with_registry(Arc::new(app_registry()));
    let source = WinAppSource::new();
    assert!(source.detect(&env));
    let data = load(&source, &env);
    let mut names: Vec<&str> = data
        .programs
        .iter()
        .map(|program| program.name.as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["Example Editor", "Legacy Viewer", "User Tool", "standalone"]
    );
    let editor = program(&data, "Example Editor");
    assert_eq!(editor.kind, EntryKind::App);
    assert_eq!(editor.version.as_deref(), Some("3.2"));
    assert_eq!(editor.executable.as_deref(), Some("editor.exe"));
    assert_eq!(
        editor.path.as_deref(),
        Some("C:\\Program Files\\Editor\\editor.exe")
    );
    assert_eq!(editor.app_id.as_deref(), Some("{6F1C}"));
    assert_eq!(editor.homepage.as_deref(), Some("https://editor.example"));
    assert_eq!(editor.aliases.len(), 1);
    assert_eq!(editor.aliases[0].name, "exed");
    assert!(
        editor
            .category_hints
            .iter()
            .all(|hint| hint.scheme == CategoryScheme::Keyword)
    );
    let standalone = program(&data, "standalone");
    assert_eq!(
        standalone.path.as_deref(),
        Some("C:\\Tools\\standalone.exe")
    );
    assert_eq!(standalone.executable.as_deref(), Some("standalone.exe"));
    assert_ne!(
        source.fingerprint(&env).unwrap(),
        source.fingerprint(&windows_env(dir.path())).unwrap()
    );
}

#[test]
fn winapp_without_registry_warns() {
    let dir = tempfile::tempdir().unwrap();
    let data = load(&WinAppSource::new(), &windows_env(dir.path()));
    assert!(data.programs.is_empty());
    assert_eq!(data.warnings[0].code, WarningCode::SourceUnavailable);
    assert!(!WinAppSource::new().detect(&mac_env(dir.path())));
}

/// Version block of a `VS_VERSIONINFO` tree: header, key, value and
/// children, each aligned to four bytes.
fn version_block(
    key: &str,
    kind: u16,
    value: &[u8],
    value_length: u16,
    children: &[Vec<u8>],
) -> Vec<u8> {
    let pad = |bytes: &mut Vec<u8>| {
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
        }
    };
    let mut out = vec![0, 0];
    out.extend(value_length.to_le_bytes());
    out.extend(kind.to_le_bytes());
    out.extend(key.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
    pad(&mut out);
    out.extend(value);
    for child in children {
        pad(&mut out);
        out.extend(child);
    }
    let length = u16::try_from(out.len()).unwrap();
    out[..2].copy_from_slice(&length.to_le_bytes());
    out
}

fn string_entry(key: &str, value: &str) -> Vec<u8> {
    let units: Vec<u16> = value.encode_utf16().chain([0]).collect();
    let bytes: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    version_block(key, 1, &bytes, u16::try_from(units.len()).unwrap(), &[])
}

/// 64-bit PE image with one `.rsrc` section holding a version resource:
/// file version 3.2.0.7 and the strings of an English string table.
fn pe_with_version() -> Vec<u8> {
    const RSRC_RVA: u32 = 0x1000;
    const RSRC_FILE: usize = 0x200;
    let mut fixed = Vec::new();
    for word in [
        0xfeef_04bd_u32,
        0x0001_0000,
        (3 << 16) | 2,
        7,
        (3 << 16) | 2,
        7,
    ] {
        fixed.extend(word.to_le_bytes());
    }
    fixed.resize(52, 0);
    let table = version_block(
        "040904b0",
        1,
        &[],
        0,
        &[
            string_entry("CompanyName", "Example Ltd"),
            string_entry("FileDescription", "Example text editor"),
            string_entry("FileVersion", "3.2.0.7"),
            string_entry("ProductName", "Example Editor"),
            string_entry("ProductVersion", "3.2"),
        ],
    );
    let other = version_block(
        "041504b0",
        1,
        &[],
        0,
        &[string_entry("ProductName", "Edytor")],
    );
    let strings = version_block("StringFileInfo", 1, &[], 0, &[other, table]);
    let vars = version_block(
        "VarFileInfo",
        1,
        &[],
        0,
        &[version_block("Translation", 0, &[9, 4, 0xb0, 4], 4, &[])],
    );
    let version = version_block("VS_VERSION_INFO", 0, &fixed, 52, &[strings, vars]);

    let mut rsrc = Vec::new();
    let directory = |rsrc: &mut Vec<u8>, id: u32, target: u32| {
        rsrc.extend([0u8; 12]);
        rsrc.extend(0u16.to_le_bytes());
        rsrc.extend(1u16.to_le_bytes());
        rsrc.extend(id.to_le_bytes());
        rsrc.extend(target.to_le_bytes());
    };
    directory(&mut rsrc, 16, 0x8000_0000 | 0x18);
    directory(&mut rsrc, 1, 0x8000_0000 | 0x30);
    directory(&mut rsrc, 0x409, 0x48);
    rsrc.extend((RSRC_RVA + 0x58).to_le_bytes());
    rsrc.extend(u32::try_from(version.len()).unwrap().to_le_bytes());
    rsrc.extend([0u8; 8]);
    assert_eq!(rsrc.len(), 0x58);
    rsrc.extend(&version);
    let rsrc_len = u32::try_from(rsrc.len()).unwrap();
    let raw_size = rsrc_len.div_ceil(0x200) * 0x200;

    let mut image = vec![0u8; RSRC_FILE];
    image[..2].copy_from_slice(b"MZ");
    image[0x3c] = 0x40;
    image[0x40..0x44].copy_from_slice(b"PE\0\0");
    let coff = 0x44;
    image[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes());
    image[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
    image[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes());
    let optional = coff + 20;
    image[optional..optional + 2].copy_from_slice(&0x20bu16.to_le_bytes());
    image[optional + 108..optional + 112].copy_from_slice(&16u32.to_le_bytes());
    let resource = optional + 112 + 2 * 8;
    image[resource..resource + 4].copy_from_slice(&RSRC_RVA.to_le_bytes());
    image[resource + 4..resource + 8].copy_from_slice(&rsrc_len.to_le_bytes());
    let section = optional + 240;
    image[section..section + 5].copy_from_slice(b".rsrc");
    image[section + 8..section + 12].copy_from_slice(&rsrc_len.to_le_bytes());
    image[section + 12..section + 16].copy_from_slice(&RSRC_RVA.to_le_bytes());
    image[section + 16..section + 20].copy_from_slice(&raw_size.to_le_bytes());
    image[section + 20..section + 24]
        .copy_from_slice(&u32::try_from(RSRC_FILE).unwrap().to_le_bytes());
    image.extend(rsrc);
    image.resize(RSRC_FILE + raw_size as usize, 0);
    image
}

#[test]
fn winapp_pe_version_resource() {
    let image = pe_with_version();
    let info = winapp::parse_pe_version(&image).unwrap();
    assert_eq!(
        info.file_description.as_deref(),
        Some("Example text editor")
    );
    assert_eq!(info.product_name.as_deref(), Some("Example Editor"));
    assert_eq!(info.company_name.as_deref(), Some("Example Ltd"));
    assert_eq!(info.file_version.as_deref(), Some("3.2.0.7"));
    assert_eq!(info.product_version.as_deref(), Some("3.2"));
    assert_eq!(info.fixed_file_version.as_deref(), Some("3.2.0.7"));
    assert_eq!(
        info.description().as_deref(),
        Some("Example text editor\nExample Editor 3.2\nExample Ltd")
    );
    assert_eq!(winapp::parse_pe_version(&image[..0x180]), None);

    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "c/Program Files/Editor/editor.exe", &image);
    let env = windows_env(dir.path()).with_registry(Arc::new(app_registry()));
    let path = "C:\\Program Files\\Editor\\editor.exe";
    assert_eq!(winapp::read_pe_version(&env, Path::new(path)), Some(info));
    let source = WinAppSource::new();
    let want = DetailWant {
        description: true,
        ..DetailWant::default()
    };
    let key = EntryKey::new(source.id(), path);
    let detail = source.detail(&env, &key, &want).unwrap().unwrap();
    assert!(
        detail
            .description
            .unwrap()
            .starts_with("Example text editor")
    );
    let other = EntryKey::new(&SourceId::new(SourceId::PATH), path);
    assert_eq!(source.detail(&env, &other, &want).unwrap(), None);
    assert_eq!(
        source.detail(&env, &key, &DetailWant::default()).unwrap(),
        None
    );
}

#[test]
fn winapp_executable_values() {
    assert_eq!(
        winapp::executable_of("C:\\A\\b.EXE,-101").as_deref(),
        Some("C:\\A\\b.EXE")
    );
    assert_eq!(
        winapp::executable_of("C:\\A\\b,c.exe").as_deref(),
        Some("C:\\A\\b,c.exe")
    );
    assert_eq!(winapp::executable_of(""), None);
}

const INFO_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>org.example.Viewer</string>
  <key>CFBundleName</key><string>Viewer</string>
  <key>CFBundleDisplayName</key><string>Image Viewer</string>
  <key>CFBundleShortVersionString</key><string>2.4</string>
  <key>CFBundleVersion</key><string>240</string>
  <key>CFBundleExecutable</key><string>Viewer</string>
  <key>CFBundleDevelopmentRegion</key><string>English</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.graphics-design</string>
</dict>
</plist>"#;

fn utf16_strings(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xfe];
    bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    bytes
}

#[test]
fn macapp_info_plist_localized() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bundle = "Applications/Viewer.app/Contents";
    write(root, &format!("{bundle}/Info.plist"), INFO_PLIST);
    write(
        root,
        &format!("{bundle}/Resources/de.lproj/InfoPlist.strings"),
        utf16_strings("/* Localized */\n\"CFBundleDisplayName\" = \"Bildbetrachter\";\n"),
    );
    write(
        root,
        &format!("{bundle}/Resources/it.lproj/InfoPlist.strings"),
        "\"CFBundleName\" = \"Visualizzatore\";\n",
    );
    write(
        root,
        &format!("{bundle}/Resources/fr.lproj/InfoPlist.strings"),
        "\"CFBundleName\" = \"Visionneuse\";",
    );
    write(root, &format!("{bundle}/_MASReceipt/receipt"), "x");
    let env = mac_env(root).with_languages(vec!["de".to_owned(), "it".to_owned(), "en".to_owned()]);
    let source = MacAppSource::new();
    assert!(source.detect(&env));
    assert!(!source.detect(&HostEnv::new(Os::Linux).with_root(root)));
    let data = load(&source, &env);
    let viewer = program(&data, "Viewer");
    assert_eq!(viewer.kind, EntryKind::App);
    assert_eq!(
        viewer.path.as_deref().map(|path| path.replace('\\', "/")),
        Some("/Applications/Viewer.app".to_owned())
    );
    assert_eq!(viewer.app_id.as_deref(), Some("org.example.Viewer"));
    assert_eq!(viewer.version.as_deref(), Some("2.4"));
    assert_eq!(viewer.executable.as_deref(), Some("Viewer"));
    assert_eq!(viewer.tags, [macapp::APP_STORE_TAG]);
    assert_eq!(
        viewer.category_hints[0].scheme,
        CategoryScheme::MacosCategory
    );
    assert_eq!(
        viewer.category_hints[0].value,
        "public.app-category.graphics-design"
    );
    assert_localized_viewer_name(viewer);
}

/// Name of the bundle `Viewer.app`: the development region is English and
/// only the preferred languages German and Polish are read.
fn assert_localized_viewer_name(viewer: &RawProgram) {
    let name = viewer.app_name.as_ref().unwrap();
    assert_eq!(name.text, "Image Viewer");
    assert_eq!(name.lang.as_deref(), Some("en"));
    assert_eq!(name.pick(&["de".to_owned()]).0, "Bildbetrachter");
    assert_eq!(name.pick(&["it".to_owned()]).0, "Visualizzatore");
    assert!(!name.translations.contains_key("fr"));
}

#[test]
fn macapp_binary_plist_and_nested_bundles() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut dict = plist::Dictionary::new();
    dict.insert(
        "CFBundleIdentifier".to_owned(),
        plist::Value::String("com.example.Console".to_owned()),
    );
    dict.insert(
        "CFBundleVersion".to_owned(),
        plist::Value::String("1.1".to_owned()),
    );
    let mut binary = Vec::new();
    plist::Value::Dictionary(dict)
        .to_writer_binary(&mut binary)
        .unwrap();
    assert!(binary.starts_with(b"bplist00"));
    write(
        root,
        "System/Applications/Utilities/Console.app/Contents/Info.plist",
        &binary,
    );
    write(
        root,
        "Applications/Suite/Tool.app/Contents/Info.plist",
        INFO_PLIST,
    );
    write(
        root,
        "Users/user/Applications/Broken.app/Contents/Info.plist",
        "not a plist",
    );
    write(
        root,
        "Applications/.Hidden.app/Contents/Info.plist",
        INFO_PLIST,
    );
    write(root, "Applications/readme.txt", "x");
    let data = load(&MacAppSource::new(), &mac_env(root));
    let console = program(&data, "Console");
    assert_eq!(console.version.as_deref(), Some("1.1"));
    assert_eq!(console.app_name.as_ref().unwrap().text, "Console");
    assert_eq!(
        program(&data, "Tool")
            .path
            .as_deref()
            .map(|path| path.replace('\\', "/")),
        Some("/Applications/Suite/Tool.app".to_owned())
    );
    assert_eq!(data.programs.len(), 2);
    assert_eq!(data.warnings.len(), 1);
    assert_eq!(data.warnings[0].name.as_deref(), Some("Broken"));
    let strings = macapp::parse_strings(&binary);
    assert_eq!(strings.len(), 2);
}

/// Thin 64-bit Mach-O image with a `__TEXT,__info_plist` section.
fn macho_with_plist(plist: &[u8]) -> Vec<u8> {
    const DATA: usize = 256;
    let mut image = vec![0u8; DATA];
    image[..4].copy_from_slice(&0xfeed_facf_u32.to_le_bytes());
    image[16..20].copy_from_slice(&1u32.to_le_bytes());
    let command = 32;
    image[command..command + 4].copy_from_slice(&0x19u32.to_le_bytes());
    image[command + 8..command + 14].copy_from_slice(b"__TEXT");
    image[command + 64..command + 68].copy_from_slice(&2u32.to_le_bytes());
    let first = command + 72;
    image[first..first + 6].copy_from_slice(b"__text");
    let section = first + 80;
    image.resize(DATA.max(section + 80), 0);
    image[section..section + 12].copy_from_slice(b"__info_plist");
    image[section + 16..section + 22].copy_from_slice(b"__TEXT");
    image[section + 40..section + 48].copy_from_slice(&(plist.len() as u64).to_le_bytes());
    let offset = image.len();
    image[section + 48..section + 52]
        .copy_from_slice(&u32::try_from(offset).unwrap().to_le_bytes());
    image[20..24].copy_from_slice(&(72u32 + 2 * 80).to_le_bytes());
    image[command + 4..command + 8].copy_from_slice(&(72u32 + 2 * 80).to_le_bytes());
    image.extend(plist);
    image
}

#[test]
fn macapp_embedded_info_plist() {
    let thin = macho_with_plist(INFO_PLIST.as_bytes());
    assert_eq!(macapp::macho_info_plist(&thin), Some(INFO_PLIST.as_bytes()));
    let mut fat = Vec::new();
    fat.extend(0xcafe_babe_u32.to_be_bytes());
    fat.extend(1u32.to_be_bytes());
    for word in [
        0x0100_000c_u32,
        0,
        4096,
        u32::try_from(thin.len()).unwrap(),
        12,
    ] {
        fat.extend(word.to_be_bytes());
    }
    fat.resize(4096, 0);
    fat.extend(&thin);
    assert_eq!(macapp::macho_info_plist(&fat), Some(INFO_PLIST.as_bytes()));
    assert_eq!(macapp::macho_info_plist(&thin[..64]), None);

    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "Applications/Bare.app/Contents/MacOS/Bare",
        &fat,
    );
    let data = load(&MacAppSource::new(), &mac_env(dir.path()));
    let bare = program(&data, "Bare");
    assert_eq!(bare.app_id.as_deref(), Some("org.example.Viewer"));
}

#[test]
fn macapp_strings_in_utf8_and_big_endian() {
    let mut bytes = vec![0xfe, 0xff];
    bytes.extend(
        "a = \"\\U0041\\n\";"
            .encode_utf16()
            .flat_map(u16::to_be_bytes),
    );
    assert_eq!(
        macapp::parse_strings(&bytes).get("a").map(String::as_str),
        Some("A\n")
    );
    let utf8 = macapp::parse_strings("\u{feff}// c\nkey = value;".as_bytes());
    assert_eq!(utf8.get("key").map(String::as_str), Some("value"));
    assert!(macapp::parse_info_plist(b"<plist><array/></plist>").is_err());
}

#[test]
fn choco_and_scoop_read_files_with_damaged_bytes_lossily() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let choco = "c/ProgramData/chocolatey";
    let mut nuspec = b"<package><metadata><id>tool</id><version>1.0</version><summary>caf".to_vec();
    nuspec.extend_from_slice(&[0xff, 0xfe]);
    nuspec.extend_from_slice(b"</summary></metadata></package>");
    write(root, &format!("{choco}/lib/tool/tool.nuspec"), nuspec);
    write(root, &format!("{choco}/lib/tool/tools/tool.exe"), "MZ");
    write(root, &format!("{choco}/bin/tool.exe"), "MZ");
    let data = load(&ChocoSource::new(), &windows_env(root));
    assert_eq!(program(&data, "tool").version.as_deref(), Some("1.0"));

    let scoop = "c/Users/user/scoop";
    let mut manifest = br#"{"version":"2.0","description":"na"#.to_vec();
    manifest.extend_from_slice(&[0xff]);
    manifest.extend_from_slice(br#"ve"}"#);
    write(
        root,
        &format!("{scoop}/apps/tool/current/manifest.json"),
        manifest,
    );
    let data = load(&ScoopSource::new(), &windows_env(root));
    assert_eq!(package(&data, "tool").version.as_deref(), Some("2.0"));
}

#[test]
fn brew_prefers_the_linked_version_over_the_highest_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let cellar = "opt/homebrew/Cellar/jq";
    write(root, &format!("{cellar}/1.6/INSTALL_RECEIPT.json"), "{}");
    write(root, &format!("{cellar}/1.7.1/INSTALL_RECEIPT.json"), "{}");
    if !link("../Cellar/jq/1.6", root, "opt/homebrew/opt/jq") {
        return;
    }
    let data = load(&BrewSource::new(), &mac_env(root));
    assert_eq!(package(&data, "jq").version.as_deref(), Some("1.6"));
    std::fs::remove_file(root.join("opt/homebrew/opt/jq")).unwrap();
    let data = load(&BrewSource::new(), &mac_env(root));
    assert_eq!(package(&data, "jq").version.as_deref(), Some("1.7.1"));
}

#[test]
fn brew_ignores_relative_xdg_cache_directory() {
    let linux = HostEnv::new(Os::Linux)
        .with_home("/home/user")
        .with_var("XDG_CACHE_HOME", "relative/cache");
    assert_eq!(
        brew::cache_dir(&linux).unwrap(),
        Path::new("/home/user/.cache/Homebrew")
    );
}

#[test]
fn scoop_shim_application_with_apps_in_the_root_path() {
    let shim = |path: &str| scoop::shim_application(&format!("path = \"{path}\"\n"));
    assert_eq!(
        shim("D:\\apps\\scoop\\apps\\ripgrep\\current\\rg.exe").as_deref(),
        Some("ripgrep")
    );
    assert_eq!(
        shim("D:\\apps\\apps\\fd\\current\\fd.exe").as_deref(),
        Some("fd")
    );
    assert_eq!(
        shim("C:\\Users\\user\\scoop\\apps\\jq\\1.7\\jq.exe").as_deref(),
        Some("jq")
    );
    assert_eq!(shim("C:\\Tools\\jq.exe"), None);
}

#[test]
fn scoop_shim_files_follow_the_target_and_the_files_present() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let scoop_root = "d/apps/scoop";
    write(
        root,
        &format!("{scoop_root}/apps/tools/current/manifest.json"),
        r#"{"version": "1.0", "bin": ["tool.exe", "script.cmd", ["wrap.ps1", "wp"], "present.exe"]}"#,
    );
    write(root, &format!("{scoop_root}/shims/present.cmd"), "x");
    write(
        root,
        &format!("{scoop_root}/shims/tool.shim"),
        "path = \"D:\\apps\\scoop\\apps\\tools\\current\\tool.exe\"\n",
    );
    let env = windows_env(root).with_var("SCOOP", "D:\\apps\\scoop");
    let data = load(&ScoopSource::new(), &env);
    let path_of = |name: &str| program(&data, name).path.clone().unwrap();
    assert!(path_of("tool").ends_with("tool.exe"));
    assert!(path_of("script").ends_with("script.cmd"));
    assert!(path_of("wp").ends_with("wp.cmd"));
    assert!(path_of("present").ends_with("present.cmd"));
    assert_eq!(program(&data, "wp").alias_kind, Some(AliasKind::Shim));
    assert_eq!(program(&data, "wp").alias_of.as_deref(), Some("wrap"));
    assert_eq!(program(&data, "tool").alias_kind, None);
    assert_eq!(package(&data, "tools").executables.len(), 4);
}

#[test]
fn winget_identifiers_may_contain_underscores() {
    let source = "Microsoft.Winget.Source_8wekyb3d8bbwe";
    assert_eq!(
        winget::package_of_dir(&format!("Vendor.My_Tool_{source}")).as_deref(),
        Some("Vendor.My_Tool")
    );
    assert_eq!(
        winget::package_of_dir(&format!("Vendor.Tool_{source}")).as_deref(),
        Some("Vendor.Tool")
    );
    assert_eq!(
        winget::package_of_dir("Vendor.Tool_x").as_deref(),
        Some("Vendor.Tool")
    );
    assert_eq!(
        winget::package_of_dir("_Microsoft.Winget.Source_8wekyb3d8bbwe"),
        None
    );
    assert_eq!(
        winget::package_of_link(&format!(
            "C:\\L\\Packages\\Vendor.My_Tool_{source}\\tool.exe"
        ))
        .as_deref(),
        Some("Vendor.My_Tool")
    );
}
