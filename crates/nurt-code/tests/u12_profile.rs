// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `code.profile` on project trees created in temporary directories.
//!
//! The repository is described by its metadata files (`.git/HEAD` and a
//! loose reference) and by a repository query service with fixed history and
//! status. Manifests are read through a document service that understands
//! JSON and the subset of TOML used by the fixtures.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::ffi::OsString;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use nurt_code::commands::profile::CodeProfile;
use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::{
    DocFormatId, DocFormatInfo, DocItem, DocService, FileHistoryStat, GitChange, GitPathStatus,
    GitQueryService, SerializeOptions, Services, SyntaxIssue,
};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelReason, CancelToken, ExecCtx, StageData, StageImpl, StageInput,
};
use nurt_contracts::value::{Record, Value};

/// Commit the fixture repository is on.
const HEAD: &str = "1111111111111111111111111111111111111111";

/// Start of the test clock: 2023-11-14 22:13:20 UTC.
const START_NS: i64 = 1_700_000_000_000_000_000;

/// Lifetime of a cached profile plus one second.
const PAST_TTL_NS: i64 = 601 * 1_000_000_000;

struct TestClock {
    now_ns: AtomicI64,
}

impl TestClock {
    fn advance(&self, nanos: i64) {
        self.now_ns.fetch_add(nanos, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now(&self) -> WallTime {
        WallTime::from_unix_nanos(i128::from(self.now_ns.load(Ordering::SeqCst)))
    }

    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: self.now(),
            boot: BootId("test".to_owned()),
            since_boot_with_suspend: Duration::ZERO,
            since_boot_active: Duration::ZERO,
        }
    }

    fn sleep(
        &self,
        _duration: Duration,
        _cancel: &CancelToken,
    ) -> std::result::Result<(), CancelReason> {
        Ok(())
    }
}

struct TestHost {
    info: HostInfo,
}

impl Host for TestHost {
    fn info(&self) -> &HostInfo {
        &self.info
    }

    fn env_var(&self, _name: &str) -> Option<OsString> {
        None
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        Vec::new()
    }
}

/// Repository with a fixed history and a status that tests can change.
struct TestGit {
    status: Mutex<Vec<GitPathStatus>>,
}

impl TestGit {
    fn new() -> Self {
        Self {
            status: Mutex::new(vec![status("src/lib.rs", GitChange::Modified)]),
        }
    }
}

fn status(path: &str, worktree: GitChange) -> GitPathStatus {
    GitPathStatus {
        path: path.to_owned(),
        index: GitChange::Unmodified,
        worktree,
        original_path: None,
    }
}

impl GitQueryService for TestGit {
    fn repo_root(&self, path: &Path) -> Result<Option<PathBuf>> {
        Ok(path.join(".git").exists().then(|| path.to_path_buf()))
    }

    fn status(&self, _root: &Path, _paths: &[String]) -> Result<Vec<GitPathStatus>> {
        Ok(self.status.lock().unwrap().clone())
    }

    fn changed_files(&self, _root: &Path, _against: Option<&str>) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn tracked_files(&self, _root: &Path) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn is_ignored(&self, _root: &Path, _path: &str) -> Result<bool> {
        Ok(false)
    }

    fn file_history(
        &self,
        _root: &Path,
        _since_days: u32,
        _limit: u32,
    ) -> Result<Vec<FileHistoryStat>> {
        let stat = |path: &str, commits| FileHistoryStat {
            path: path.to_owned(),
            commits,
            authors: 1,
            last_change: Some(WallTime::from_unix_nanos(i128::from(START_NS))),
        };
        Ok(vec![
            stat("Cargo.lock", 9),
            stat("src/main.rs", 5),
            stat("Makefile", 2),
        ])
    }
}

/// Document service for JSON and for TOML tables with string and integer
/// values.
struct TestDocs;

impl DocService for TestDocs {
    fn formats(&self) -> Vec<DocFormatInfo> {
        Vec::new()
    }

    fn detect_format(&self, _path: Option<&Path>, _bytes: &[u8]) -> Option<DocFormatId> {
        None
    }

    fn read_keys(
        &self,
        bytes: &[u8],
        format: &DocFormatId,
        _key: &KeyPath,
        _depth: Option<u32>,
    ) -> Result<Vec<DocItem>> {
        let json = match format.as_str() {
            "json" => serde_json::from_slice(bytes)
                .map_err(|error| Error::new(ErrorCode::DocParse, error.to_string()))?,
            "toml" => mini_toml(&String::from_utf8_lossy(bytes)),
            other => {
                return Err(Error::new(
                    ErrorCode::DocParse,
                    format!("format {other} is not supported"),
                ));
            }
        };
        Ok(vec![DocItem {
            key: KeyPath::root(),
            value: Value::from_plain_json(json),
            span: None,
            comment: None,
            document: None,
            anchor: None,
            alias_of: None,
            tag: None,
        }])
    }

    fn check_syntax(&self, _bytes: &[u8], _format: &DocFormatId) -> Result<Vec<SyntaxIssue>> {
        Ok(Vec::new())
    }

    fn serialize_records(
        &self,
        _records: &[Record],
        _format: &DocFormatId,
        _options: &SerializeOptions,
    ) -> Result<Vec<u8>> {
        Ok(Vec::new())
    }
}

/// TOML tables (`[a.b]`), arrays of tables (`[[a]]`) and `key = value`
/// lines with string or integer values.
fn mini_toml(text: &str) -> serde_json::Value {
    let mut root = serde_json::Map::new();
    let mut table: Vec<String> = Vec::new();
    let mut array_item = false;
    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        if let Some(name) = line.strip_prefix("[[").and_then(|l| l.strip_suffix("]]")) {
            table = vec![name.trim().to_owned()];
            array_item = true;
            let items = root
                .entry(name.trim())
                .or_insert_with(|| serde_json::json!([]));
            if let Some(items) = items.as_array_mut() {
                items.push(serde_json::json!({}));
            }
        } else if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            table = name.split('.').map(|part| part.trim().to_owned()).collect();
            array_item = false;
            target(&mut root, &table, false);
        } else if let Some((key, value)) = line.split_once('=') {
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .map_or_else(
                    || {
                        value
                            .parse::<i64>()
                            .map_or(serde_json::Value::Null, serde_json::Value::from)
                    },
                    serde_json::Value::from,
                );
            target(&mut root, &table, array_item).insert(key.trim().to_owned(), value);
        }
    }
    serde_json::Value::Object(root)
}

fn target<'m>(
    root: &'m mut serde_json::Map<String, serde_json::Value>,
    table: &[String],
    array_item: bool,
) -> &'m mut serde_json::Map<String, serde_json::Value> {
    let mut current = root;
    for (index, name) in table.iter().enumerate() {
        let entry = current
            .entry(name.clone())
            .or_insert_with(|| serde_json::json!({}));
        let entry = if array_item && index + 1 == table.len() {
            entry
                .as_array_mut()
                .and_then(|items| items.last_mut())
                .unwrap()
        } else {
            entry
        };
        current = entry.as_object_mut().unwrap();
    }
    current
}

struct Fixture {
    dir: tempfile::TempDir,
    state: tempfile::TempDir,
    clock: Arc<TestClock>,
    git: Arc<TestGit>,
}

impl Fixture {
    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn write(&self, relative: &str, content: &str) {
        let path = self.root().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn set_modified(&self, relative: &str, seconds: u64) {
        let file = File::options()
            .write(true)
            .open(self.root().join(relative))
            .unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
            .unwrap();
    }

    fn context(&self, git: bool, docs: bool, state: bool) -> ExecCtx {
        let info = HostInfo {
            os: Os::Linux,
            os_version: "6.0".to_owned(),
            architecture: "x86_64".to_owned(),
            hostname: "test".to_owned(),
            user: "test".to_owned(),
            home: self.state.path().to_path_buf(),
            temp: self.state.path().to_path_buf(),
            path_style: PathStyle::Posix,
            timezone: "UTC".to_owned(),
            locale: None,
            cpus: 1,
        };
        let mut services = Services::new();
        if git {
            services = services.with_git_query(Arc::clone(&self.git) as Arc<dyn GitQueryService>);
        }
        if docs {
            services = services.with_doc(Arc::new(TestDocs));
        }
        let builder = ExecCtx::builder(
            Arc::clone(&self.clock) as Arc<dyn Clock>,
            Arc::new(TestHost { info }),
        )
        .services(Arc::new(services));
        let builder = if state {
            builder.state_dir(self.state.path().join("state"))
        } else {
            builder
        };
        builder.build()
    }
}

/// A project with npm, Cargo and Make manifests, a Python package and a
/// repository on [`HEAD`].
fn project() -> Fixture {
    let fixture = Fixture {
        dir: tempfile::tempdir().unwrap(),
        state: tempfile::tempdir().unwrap(),
        clock: Arc::new(TestClock {
            now_ns: AtomicI64::new(START_NS),
        }),
        git: Arc::new(TestGit::new()),
    };
    fixture.write(
        "package.json",
        r#"{"name": "web", "scripts": {"build": "tsc", "test": "jest"}, "bin": {"tool": "bin/tool.js"}}"#,
    );
    fixture.write("bin/tool.js", "console.log(1);\n");
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    );
    fixture.write("Cargo.lock", "version = 3\n");
    fixture.write("src/main.rs", "fn main() {}\n");
    fixture.write("src/lib.rs", "pub fn f() {}\n");
    fixture.write("Makefile", "all: build\n\nbuild:\n\tcargo build\n");
    fixture.write("app/__main__.py", "print(1)\n");
    fixture.write(".git/HEAD", "ref: refs/heads/main\n");
    fixture.write(".git/refs/heads/main", &format!("{HEAD}\n"));
    fixture
}

fn args(root: &Path, extra: &[(&str, Value)]) -> BoundArgs {
    let mut args = BoundArgs::new("code.profile").with(
        "root",
        ArgValue::Value(Value::Path(root.display().to_string())),
        false,
    );
    for (name, value) in extra {
        args.insert(*name, ArgValue::Value(value.clone()), false);
    }
    args
}

fn run(ctx: &ExecCtx, root: &Path, extra: &[(&str, Value)]) -> Result<(Record, Vec<Record>)> {
    let args = args(root, extra);
    let stage = CodeProfile;
    let output = stage
        .as_read()
        .unwrap()
        .execute(&args, StageInput::none(), ctx)?;
    let header = output.header.unwrap();
    let records = match output.data {
        StageData::Records(stream) => stream.collect::<Result<Vec<_>>>()?,
        _ => Vec::new(),
    };
    Ok((header, records))
}

fn field(record: &Record, name: &str) -> Option<String> {
    match record.fields.get(name)? {
        Value::Text(text) | Value::Enum(text) | Value::Path(text) => Some(text.clone()),
        Value::Int(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn section<'r>(records: &'r [Record], name: &str) -> Vec<&'r Record> {
    records
        .iter()
        .filter(|record| field(record, "section").as_deref() == Some(name))
        .collect()
}

fn has(records: &[&Record], pairs: &[(&str, &str)]) -> bool {
    records.iter().any(|record| {
        pairs
            .iter()
            .all(|(name, value)| field(record, name).as_deref() == Some(*value))
    })
}

fn cached(header: &Record) -> bool {
    field(header, "cached").as_deref() == Some("true")
}

#[test]
fn commands_come_from_manifests_with_their_origin() {
    let fixture = project();
    let (_, records) = run(&fixture.context(true, true, true), fixture.root(), &[]).unwrap();
    let commands = section(&records, "commands");
    assert!(has(
        &commands,
        &[
            ("name", "build"),
            ("command", "npm run build"),
            ("origin", "package.json:scripts.build")
        ]
    ));
    assert!(has(
        &commands,
        &[("name", "test"), ("command", "npm run test")]
    ));
    assert!(has(
        &commands,
        &[
            ("name", "build"),
            ("command", "cargo build"),
            ("origin", "Cargo.toml")
        ]
    ));
    assert!(has(
        &commands,
        &[
            ("name", "all"),
            ("command", "make all"),
            ("origin", "Makefile:1")
        ]
    ));
    assert!(has(
        &commands,
        &[
            ("name", "build"),
            ("command", "make build"),
            ("origin", "Makefile:3")
        ]
    ));
}

#[test]
fn script_names_with_shell_metacharacters_are_not_offered_as_commands() {
    let fixture = project();
    fixture.write(
        "package.json",
        r#"{"name": "web", "scripts": {"build": "tsc", "x; touch pwned": "id", "a$(id)": "id", "p|q": "id"}}"#,
    );
    let (_, records) = run(&fixture.context(true, true, false), fixture.root(), &[]).unwrap();
    let commands = section(&records, "commands");
    assert!(has(&commands, &[("command", "npm run build")]));
    for record in &commands {
        let command = field(record, "command").unwrap_or_default();
        assert!(
            !command.contains([';', '$', '|']),
            "unsafe command offered: {command}"
        );
    }
}

#[test]
fn entry_points_name_their_target_and_origin() {
    let fixture = project();
    let root = fixture.root().display().to_string();
    let (_, records) = run(&fixture.context(true, true, true), fixture.root(), &[]).unwrap();
    let entries = section(&records, "entry_points");
    let tool = format!("{root}/bin/tool.js");
    assert!(has(
        &entries,
        &[
            ("name", "tool"),
            ("path", tool.as_str()),
            ("origin", "package.json:bin.tool")
        ]
    ));
    let main = format!("{root}/src/main.rs");
    assert!(has(
        &entries,
        &[
            ("name", "demo"),
            ("path", main.as_str()),
            ("origin", "Cargo.toml:package.name")
        ]
    ));
    assert!(has(
        &entries,
        &[("command", "python -m app"), ("origin", "__main__.py")]
    ));
}

#[test]
fn hotspots_come_from_history_without_lock_files() {
    let fixture = project();
    let (header, records) = run(&fixture.context(true, true, true), fixture.root(), &[]).unwrap();
    assert_eq!(field(&header, "repository").as_deref(), Some("true"));
    assert_eq!(field(&header, "revision").as_deref(), Some(HEAD));
    let hotspots = section(&records, "hotspots");
    assert!(has(
        &hotspots,
        &[
            ("name", "src/main.rs"),
            ("changes", "5"),
            ("origin", "history, 90 days")
        ]
    ));
    assert!(has(
        &hotspots,
        &[("name", "src/lib.rs"), ("origin", "uncommitted")]
    ));
    assert!(!has(&hotspots, &[("name", "Cargo.lock")]));
    let manifests = section(&records, "manifests");
    assert!(!manifests.is_empty());
    assert!(!section(&records, "languages").is_empty());
}

#[test]
fn cached_profile_is_reused_and_uncommitted_changes_are_read_again() {
    let fixture = project();
    let ctx = fixture.context(true, true, true);
    let (first, _) = run(&ctx, fixture.root(), &[]).unwrap();
    assert!(!cached(&first));
    fixture
        .git
        .status
        .lock()
        .unwrap()
        .push(status("src/new.rs", GitChange::Untracked));
    let (second, records) = run(&ctx, fixture.root(), &[]).unwrap();
    assert!(cached(&second));
    assert!(second.fields.contains_key("cache_age_ms"));
    let hotspots = section(&records, "hotspots");
    assert!(has(
        &hotspots,
        &[("name", "src/new.rs"), ("origin", "uncommitted")]
    ));
}

#[test]
fn cache_is_invalidated_by_revision_manifests_period_age_and_refresh() {
    let fixture = project();
    let ctx = fixture.context(true, true, true);
    let root = fixture.root();
    assert!(!cached(&run(&ctx, root, &[]).unwrap().0));
    assert!(cached(&run(&ctx, root, &[]).unwrap().0));

    fixture.write(
        ".git/refs/heads/main",
        "2222222222222222222222222222222222222222\n",
    );
    assert!(!cached(&run(&ctx, root, &[]).unwrap().0));
    assert!(cached(&run(&ctx, root, &[]).unwrap().0));

    fixture.set_modified("Cargo.toml", 1_000);
    assert!(!cached(&run(&ctx, root, &[]).unwrap().0));
    assert!(cached(&run(&ctx, root, &[]).unwrap().0));

    assert!(!cached(
        &run(&ctx, root, &[("days", Value::Int(30))]).unwrap().0
    ));

    fixture.clock.advance(PAST_TTL_NS);
    assert!(!cached(&run(&ctx, root, &[]).unwrap().0));

    assert!(!cached(
        &run(&ctx, root, &[("refresh", Value::Bool(true))])
            .unwrap()
            .0
    ));
}

#[test]
fn sections_and_limit_select_the_entries() {
    let fixture = project();
    let extra = [
        (
            "sections",
            Value::List(vec![Value::Enum("commands".to_owned())]),
        ),
        ("limit", Value::Int(1)),
    ];
    let (_, records) = run(&fixture.context(true, true, false), fixture.root(), &extra).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(field(&records[0], "section").as_deref(), Some("commands"));
}

#[test]
fn tree_outside_a_repository_reports_recently_modified_files() {
    let fixture = project();
    fs::remove_dir_all(fixture.root().join(".git")).unwrap();
    fixture.set_modified("src/lib.rs", 2_000_000_000);
    let (header, records) = run(&fixture.context(true, true, false), fixture.root(), &[]).unwrap();
    assert_eq!(field(&header, "repository").as_deref(), Some("false"));
    assert_eq!(header.fields.get("revision"), Some(&Value::Null));
    let hotspots = section(&records, "hotspots");
    assert_eq!(field(hotspots[0], "name").as_deref(), Some("src/lib.rs"));
    assert_eq!(
        field(hotspots[0], "origin").as_deref(),
        Some("modification time")
    );
}

#[test]
fn toml_manifests_without_a_document_service_are_reported_unavailable() {
    let fixture = project();
    let ctx = fixture.context(false, false, false);
    let (_, records) = run(&ctx, fixture.root(), &[]).unwrap();
    let commands = section(&records, "commands");
    assert!(has(&commands, &[("command", "cargo build")]));
    assert!(has(&commands, &[("command", "npm run test")]));
    let notes = ctx.take_notes();
    assert!(notes.iter().any(|note| {
        note.detail
            .as_deref()
            .is_some_and(|detail| detail.starts_with("Cargo.toml"))
    }));
}

/// A tree without a repository holding one manifest of every other build
/// tool, each with the settings its commands and entry points are read from.
fn polyglot() -> Fixture {
    let fixture = Fixture {
        dir: tempfile::tempdir().unwrap(),
        state: tempfile::tempdir().unwrap(),
        clock: Arc::new(TestClock {
            now_ns: AtomicI64::new(START_NS),
        }),
        git: Arc::new(TestGit::new()),
    };
    let files = [
        (
            "justfile",
            "set shell := [\"bash\", \"-c\"]\nbuild:\n    cargo build\n@check arg:\n    cargo check\n",
        ),
        (
            "pyproject.toml",
            "[build-system]\nrequires = \"hatchling\"\n[project.scripts]\ncli = \"pkg.main:run\"\n[tool.pytest]\nminversion = \"6\"\n[tool.ruff]\nline-length = 100\n",
        ),
        (
            "setup.cfg",
            "[tool:pytest]\naddopts = -q\n[options.entry_points]\nconsole_scripts =\n    tool2 = pkg.cli:main\n",
        ),
        ("tox.ini", "[tox]\nenvlist = py311, lint\n"),
        ("go.mod", "module example.com/app\n\ngo 1.22\n"),
        ("main.go", "package main\n"),
        ("cmd/srv/main.go", "package main\n"),
        (
            "CMakeLists.txt",
            "project(x)\nenable_testing()\nadd_executable(app main.c)\n",
        ),
        (
            "build.gradle.kts",
            "application {\n    mainClass.set(\"com.example.App\")\n}\n",
        ),
        (
            "pom.xml",
            "<project>\n  <mainClass>com.example.Main</mainClass>\n</project>\n",
        ),
        ("mvnw", "#!/bin/sh\n"),
        (
            "app.csproj",
            "<Project>\n  <OutputType>Exe</OutputType>\n  <PackageReference Include=\"Microsoft.NET.Test.Sdk\" />\n</Project>\n",
        ),
        (
            "composer.json",
            r#"{"scripts": {"lint": "phpcs"}, "bin": ["bin/run"]}"#,
        ),
        ("Gemfile", "source \"https://rubygems.org\"\n"),
        ("Rakefile", "task :default\n"),
        ("spec/app_spec.rb", "describe 1\n"),
        (
            "deno.json",
            r#"{"tasks": {"dev": "deno run main.ts"}, "exports": "./mod.ts"}"#,
        ),
    ];
    for (path, content) in files {
        fixture.write(path, content);
    }
    fixture
}

#[test]
fn every_supported_manifest_contributes_commands_and_entry_points() {
    let fixture = polyglot();
    let (_, records) = run(&fixture.context(false, true, false), fixture.root(), &[]).unwrap();
    let commands = section(&records, "commands");
    let expected_commands = [
        ("just build", "justfile:2"),
        ("just check", "justfile:4"),
        ("python -m build", "pyproject.toml:build-system"),
        ("pytest", "pyproject.toml:tool.pytest"),
        ("ruff check .", "pyproject.toml:tool.ruff"),
        ("pytest", "setup.cfg:1"),
        ("tox", "tox.ini"),
        ("tox -e py311", "tox.ini:2"),
        ("tox -e lint", "tox.ini:2"),
        ("go test ./...", "go.mod"),
        ("cmake --build build", "CMakeLists.txt"),
        ("ctest --test-dir build", "CMakeLists.txt"),
        ("gradle build", "build.gradle.kts"),
        ("./mvnw test", "pom.xml"),
        ("dotnet build app.csproj", "app.csproj"),
        ("dotnet test app.csproj", "app.csproj"),
        ("composer run-script lint", "composer.json:scripts.lint"),
        ("bundle install", "Gemfile"),
        ("bundle exec rake", "Rakefile"),
        ("bundle exec rspec", "spec/"),
        ("deno task dev", "deno.json:tasks.dev"),
    ];
    for (command, origin) in expected_commands {
        assert!(
            has(&commands, &[("command", command), ("origin", origin)]),
            "{command} from {origin}"
        );
    }
    let entries = section(&records, "entry_points");
    let expected_entries = [
        ("cli", "pyproject.toml:project.scripts.cli"),
        ("tool2", "setup.cfg:5"),
        ("example.com/app", "go.mod:module"),
        ("srv", "cmd/"),
        ("app", "CMakeLists.txt:3"),
        ("com.example.App", "build.gradle.kts:2"),
        ("com.example.Main", "pom.xml:2"),
        ("app", "app.csproj:2"),
        ("./mod.ts", "deno.json:exports"),
    ];
    for (name, origin) in expected_entries {
        assert!(
            has(&entries, &[("name", name), ("origin", origin)]),
            "{name} from {origin}"
        );
    }
}

#[test]
fn root_that_is_not_a_directory_fails_with_not_found() {
    let fixture = project();
    let file = fixture.root().join("Makefile");
    let error = run(&fixture.context(true, true, false), &file, &[]).unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotFound);
}
