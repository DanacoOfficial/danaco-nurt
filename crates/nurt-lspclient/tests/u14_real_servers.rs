// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-lspclient/tests/u14_real_servers.rs
// Responsibility:   Requests and `code.rename` against installed language servers.
// Dependencies:     nurt_code, nurt_contracts, u14_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Requests and `code.rename` against installed language servers.
//!
//! Each test builds a small project in a temporary directory and runs
//! definition, references, hover, document and workspace symbols, rename
//! (plan and apply) and diagnostics through [`LspProvider`]. A test whose
//! server or toolchain is not in `PATH` or does not start passes without
//! running and reports the reason on standard error. After the provider
//! stops, the private temporary directory of every server must be gone or
//! empty.

#![cfg(unix)]
// Tests report failures by panicking and skipped servers on standard error.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr
)]

#[path = "../../nurt-code/tests/u14_support/mod.rs"]
mod u14_support;

use std::io::{Read, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nurt_code::commands::rename::CodeRename;
use nurt_code::semantic::{DocumentPosition, SemanticProvider, SemanticScope};
use nurt_code::service::CodeAnalysis;
use nurt_contracts::config::{Config, DurationSetting};
use nurt_contracts::env::{BootId, Clock, ClockStamp, WallTime};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::DiagnosticSeverity;
use nurt_contracts::services::{
    ChildExit, LanguageId, ProcessHost, ProgramOutcome, ProgramSpec, ServiceChild,
    ServiceSpawnSpec, Services, TextPosition,
};
use nurt_contracts::stage::{CancelReason, CancelToken, ExecCtx, StageInput};
use nurt_contracts::value::{RecordKind, Value};
use nurt_lspclient::provider::LspProvider;
use u14_support::{JournalLog, StdFs, TestHost, args, execute, path_value, preview};

/// Time limit of one request; the first request waits for the project to
/// load.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// Interval between checks of a process that is expected to end.
const POLL: Duration = Duration::from_millis(10);

/// Wall clock and monotonic time of the test process.
struct SystemClock {
    base: Instant,
}

impl Clock for SystemClock {
    fn now(&self) -> WallTime {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                i128::try_from(since.as_nanos()).unwrap_or(i128::MAX)
            });
        WallTime::from_unix_nanos(since_epoch)
    }

    fn monotonic(&self) -> Duration {
        self.base.elapsed()
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: self.now(),
            boot: BootId("test".to_owned()),
            since_boot_with_suspend: self.monotonic(),
            since_boot_active: self.monotonic(),
        }
    }

    fn sleep(
        &self,
        duration: Duration,
        cancel: &CancelToken,
    ) -> std::result::Result<(), CancelReason> {
        match cancel.wait_timeout(duration) {
            Some(reason) => Err(reason),
            None => Ok(()),
        }
    }
}

/// Process host starting servers in their own process group, with the
/// private temporary directory as `TMPDIR`.
#[derive(Default)]
struct RealHost {
    temp_dirs: Mutex<Vec<PathBuf>>,
}

fn io_failure(error: &std::io::Error, program: &str) -> Error {
    Error::new(
        ErrorCode::ExternalFailed,
        format!("cannot start {program}: {error}"),
    )
}

impl ProcessHost for RealHost {
    fn resolve_program(&self, program: &str, _cwd: &Path) -> Result<PathBuf> {
        which::which(program).map_err(|_| {
            Error::new(
                ErrorCode::ExternalMissing,
                format!("program {program} is not found in PATH"),
            )
        })
    }

    fn run_program(&self, _spec: &ProgramSpec, _cancel: &CancelToken) -> Result<ProgramOutcome> {
        Err(Error::new(
            ErrorCode::Unsupported,
            "the test host runs only services",
        ))
    }

    fn spawn_service(&self, spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .current_dir(&spec.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        if !spec.inherit_env {
            command.env_clear();
        }
        for var in &spec.env {
            match &var.value {
                Some(value) => command.env(&var.name, value),
                None => command.env_remove(&var.name),
            };
        }
        if let Some(temp) = &spec.temp_dir {
            std::fs::create_dir_all(temp).map_err(|error| io_failure(&error, &spec.program))?;
            command
                .env("TMPDIR", temp)
                .env("TMP", temp)
                .env("TEMP", temp);
            self.temp_dirs.lock().unwrap().push(temp.clone());
        }
        let child = command
            .spawn()
            .map_err(|error| io_failure(&error, &spec.program))?;
        let started = SystemClock {
            base: Instant::now(),
        }
        .now();
        Ok(Box::new(RealChild { child, started }))
    }
}

struct RealChild {
    child: Child,
    started: WallTime,
}

fn exit_of(status: ExitStatus) -> ChildExit {
    ChildExit {
        exit_code: status.code(),
        signal: status.signal(),
    }
}

fn wait_failure(error: &std::io::Error) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("cannot wait for the server: {error}"),
    )
}

impl ServiceChild for RealChild {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn started(&self) -> WallTime {
        self.started
    }

    fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        self.child
            .stdin
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn Write + Send>)
    }

    fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        self.child
            .stdout
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>)
    }

    fn take_stderr(&mut self) -> Option<Box<dyn Read + Send>> {
        self.child
            .stderr
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>)
    }

    fn try_wait(&mut self) -> Result<Option<ChildExit>> {
        self.child
            .try_wait()
            .map(|status| status.map(exit_of))
            .map_err(|error| wait_failure(&error))
    }

    fn wait_timeout(&mut self, timeout: Duration) -> Result<Option<ChildExit>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(exit) = self.try_wait()? {
                return Ok(Some(exit));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(POLL);
        }
    }

    fn kill_tree(&mut self) -> Result<()> {
        if self.child.try_wait().ok().flatten().is_none() {
            let group = format!("-{}", self.child.id());
            let _ = Command::new("kill").args(["-KILL", "--", &group]).status();
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        Ok(())
    }
}

impl Drop for RealChild {
    fn drop(&mut self) {
        let _ = self.kill_tree();
    }
}

/// Project of one language server.
struct Fixture {
    /// Programs that must be in `PATH`: the server first.
    programs: &'static [&'static str],
    /// Commands that must succeed for the programs to be usable.
    probes: &'static [&'static [&'static str]],
    language: &'static str,
    files: &'static [(&'static str, &'static str)],
    /// File, line and column of the name in its definition.
    definition: (&'static str, u32, u32),
    /// File, line and column of a use of the name.
    usage: (&'static str, u32, u32),
    from: &'static str,
    to: &'static str,
    /// Text appended to the definition file to cause a syntax error.
    broken: &'static str,
}

const PYTHON: Fixture = Fixture {
    programs: &["pyright-langserver", "node"],
    probes: &[&["node", "--version"]],
    language: "python",
    files: &[
        (
            "pyproject.toml",
            "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        ),
        (
            "lib.py",
            "def compute_total(values):\n    return sum(values)\n",
        ),
        (
            "main.py",
            "from lib import compute_total\n\nprint(compute_total([1, 2]))\n",
        ),
    ],
    definition: ("lib.py", 1, 5),
    usage: ("main.py", 3, 7),
    from: "compute_total",
    to: "sum_values",
    broken: "\ndef broken(:\n",
};

const TYPESCRIPT: Fixture = Fixture {
    programs: &["typescript-language-server", "node"],
    probes: &[
        &["typescript-language-server", "--version"],
        &["node", "--version"],
    ],
    language: "typescript",
    files: &[
        (
            "tsconfig.json",
            "{\"compilerOptions\": {\"strict\": true}}\n",
        ),
        (
            "lib.ts",
            "export function computeTotal(values: number[]): number {\n  return values.reduce((a, b) => a + b, 0);\n}\n",
        ),
        (
            "main.ts",
            "import { computeTotal } from \"./lib\";\n\nconsole.log(computeTotal([1, 2]));\n",
        ),
    ],
    definition: ("lib.ts", 1, 17),
    usage: ("main.ts", 3, 13),
    from: "computeTotal",
    to: "sumValues",
    broken: "\nfunction broken( {\n",
};

const GO: Fixture = Fixture {
    programs: &["gopls", "go"],
    probes: &[&["gopls", "version"], &["go", "version"]],
    language: "go",
    files: &[
        ("go.mod", "module example.com/demo\n\ngo 1.21\n"),
        (
            "lib.go",
            "package main\n\nfunc computeTotal(values []int) int {\n\ttotal := 0\n\tfor _, value := range values {\n\t\ttotal += value\n\t}\n\treturn total\n}\n",
        ),
        (
            "main.go",
            "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Println(computeTotal([]int{1, 2}))\n}\n",
        ),
    ],
    definition: ("lib.go", 3, 6),
    usage: ("main.go", 6, 14),
    from: "computeTotal",
    to: "sumValues",
    broken: "\nfunc broken( {\n",
};

const RUST: Fixture = Fixture {
    programs: &["rust-analyzer", "cargo"],
    probes: &[&["rust-analyzer", "--version"], &["cargo", "--version"]],
    language: "rust",
    files: &[
        (
            "Cargo.toml",
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
        ),
        (
            "src/lib.rs",
            "mod other;\n\npub use other::three;\n\npub fn compute_total(values: &[i32]) -> i32 {\n    values.iter().sum()\n}\n",
        ),
        (
            "src/other.rs",
            "use crate::compute_total;\n\npub fn three() -> i32 {\n    compute_total(&[3])\n}\n",
        ),
    ],
    definition: ("src/lib.rs", 5, 8),
    usage: ("src/other.rs", 4, 5),
    from: "compute_total",
    to: "sum_values",
    broken: "\nfn broken( {\n",
};

const C: Fixture = Fixture {
    programs: &["clangd"],
    probes: &[&["clangd", "--version"]],
    language: "c",
    files: &[
        ("compile_flags.txt", "-std=c11\n"),
        (
            "main.c",
            "static int compute_total(int first, int second) {\n    return first + second;\n}\n\nint main(void) {\n    return compute_total(1, 2);\n}\n",
        ),
    ],
    definition: ("main.c", 1, 12),
    usage: ("main.c", 6, 12),
    from: "compute_total",
    to: "sum_values",
    broken: "\nint broken( {\n",
};

/// Reason to skip the test: a program missing from `PATH`, or a probe that
/// fails (for example a toolchain proxy without the server component).
fn unavailable(fixture: &Fixture) -> Option<String> {
    if let Some(program) = fixture
        .programs
        .iter()
        .find(|program| which::which(program).is_err())
    {
        return Some(format!("{program} is not in PATH"));
    }
    fixture.probes.iter().find_map(|probe| {
        let (program, arguments) = probe.split_first()?;
        let works = Command::new(program)
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        (!works).then(|| format!("`{}` fails", probe.join(" ")))
    })
}

fn context(clock: Arc<SystemClock>, config: Config) -> ExecCtx {
    ExecCtx::builder(clock, Arc::new(TestHost::new()))
        .fs(Arc::new(StdFs))
        .services(Arc::new(
            Services::new().with_code(Arc::new(CodeAnalysis::builtin())),
        ))
        .config(Arc::new(config))
        .build()
}

fn position(root: &Path, (file, line, column): (&str, u32, u32)) -> DocumentPosition {
    DocumentPosition {
        path: root.join(file),
        position: TextPosition { line, column },
    }
}

/// Runs every request and the rename against the server of `fixture`.
fn exercise(fixture: &Fixture) {
    if let Some(reason) = unavailable(fixture) {
        eprintln!("skipped: {reason}");
        return;
    }
    let (_dir, root) = u14_support::project();
    for (path, text) in fixture.files {
        u14_support::write(&root.join(path), text);
    }
    let mut config = Config::default();
    config.code.timeout = DurationSetting::from_secs(REQUEST_TIMEOUT.as_secs());
    let clock = Arc::new(SystemClock {
        base: Instant::now(),
    });
    let host = Arc::new(RealHost::default());
    let provider = Arc::new(LspProvider::new(
        Arc::clone(&host) as Arc<dyn ProcessHost>,
        Arc::clone(&clock) as Arc<dyn Clock>,
        &config.code,
    ));
    let ctx = context(clock, config);
    let scope = SemanticScope {
        root: root.clone(),
        language: Some(LanguageId::new(fixture.language).unwrap()),
        timeout: Some(REQUEST_TIMEOUT),
    };

    check_requests(fixture, provider.as_ref(), &ctx, &scope, &root);
    check_rename(fixture, &provider, &ctx, &root);
    check_diagnostics(fixture, provider.as_ref(), &ctx, &scope, &root);

    assert!(provider.shutdown() >= 1);
    for dir in host.temp_dirs.lock().unwrap().iter() {
        let empty = std::fs::read_dir(dir).map_or(true, |mut entries| entries.next().is_none());
        assert!(empty, "{} still holds files of the server", dir.display());
    }
}

fn check_requests(
    fixture: &Fixture,
    provider: &dyn SemanticProvider,
    ctx: &ExecCtx,
    scope: &SemanticScope,
    root: &Path,
) {
    let definition_file = root.join(fixture.definition.0);
    let usage = position(root, fixture.usage);
    let definition = position(root, fixture.definition);

    let outline = provider
        .document_symbols(ctx, scope, &definition_file)
        .unwrap();
    assert!(
        outline.iter().any(|symbol| symbol.name == fixture.from),
        "{outline:?}"
    );

    let found = provider
        .workspace_symbols(ctx, scope, fixture.from)
        .unwrap();
    assert!(
        found.iter().any(|symbol| symbol.name == fixture.from),
        "{found:?}"
    );

    let targets = provider.definition(ctx, scope, &usage).unwrap();
    assert!(
        targets
            .iter()
            .any(|target| target.path == definition_file
                && target.start.line == fixture.definition.1),
        "{targets:?}"
    );

    let references = provider.references(ctx, scope, &definition, true).unwrap();
    assert!(references.len() >= 2, "{references:?}");
    assert!(
        references.iter().any(|found| found.path == usage.path),
        "{references:?}"
    );

    let hover = provider.hover(ctx, scope, &definition).unwrap().unwrap();
    assert!(hover.contents.contains(fixture.from), "{}", hover.contents);
}

fn check_rename(fixture: &Fixture, provider: &Arc<LspProvider>, ctx: &ExecCtx, root: &Path) {
    let stage = CodeRename::new(Some(Arc::clone(provider) as Arc<dyn SemanticProvider>));
    let (file, line, column) = fixture.definition;
    let call = args(&[
        ("to", Value::Text(fixture.to.to_owned())),
        ("path", path_value(&root.join(file))),
        ("line", Value::Int(i64::from(line))),
        ("column", Value::Int(i64::from(column))),
        ("source", Value::Enum("lsp".to_owned())),
    ]);

    let plan = preview(&stage, &call, ctx).unwrap();
    let summary = plan
        .preview
        .iter()
        .find(|record| record.kind == RecordKind::Map)
        .unwrap();
    assert_eq!(summary.fields["source"], Value::Enum("lsp".to_owned()));
    assert!(
        plan.preview
            .iter()
            .any(|record| record.kind == RecordKind::RenameEdit)
    );

    let log = Arc::new(Mutex::new(JournalLog::default()));
    let outcome = execute(&stage, &call, StageInput::none(), ctx, &log).unwrap();
    assert!(!outcome.changes.is_empty());
    for (path, _) in fixture.files {
        let text = u14_support::read(&root.join(path));
        assert!(!text.contains(fixture.from), "{path}: {text}");
    }
    assert!(u14_support::read(&root.join(file)).contains(fixture.to));
}

fn check_diagnostics(
    fixture: &Fixture,
    provider: &dyn SemanticProvider,
    ctx: &ExecCtx,
    scope: &SemanticScope,
    root: &Path,
) {
    let path = root.join(fixture.definition.0);
    let broken = format!("{}{}", u14_support::read(&path), fixture.broken);
    let found = provider
        .diagnostics(ctx, scope, &path, Some(&broken))
        .unwrap();
    assert!(
        found
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error),
        "{found:?}"
    );
}

#[test]
fn pyright_langserver_answers_requests_and_renames_when_in_path() {
    exercise(&PYTHON);
}

#[test]
fn typescript_language_server_answers_requests_and_renames_when_in_path() {
    exercise(&TYPESCRIPT);
}

#[test]
fn gopls_answers_requests_and_renames_when_in_path() {
    exercise(&GO);
}

#[test]
fn rust_analyzer_answers_requests_and_renames_when_in_path() {
    exercise(&RUST);
}

#[test]
fn clangd_answers_requests_and_renames_when_in_path() {
    exercise(&C);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
