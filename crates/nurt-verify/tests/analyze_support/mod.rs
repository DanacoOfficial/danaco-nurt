// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-verify/tests/analyze_support/mod.rs
// Responsibility:   Execution context, test project and tool programs for the tests of the analysis commands.
// Dependencies:     nurt_contracts, nurt_testkit, services
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Execution context, test project and tool programs for the tests of the
//! analysis commands.

#![allow(
    dead_code,
    reason = "each test file uses a different part of the shared helpers"
)]

mod disk;
mod services;

pub(crate) use services::{DiskFiles, FakeCode, FakeGit};

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use nurt_contracts::env::{BootId, Clock, ClockStamp, WallTime};
use nurt_contracts::envelope::RequestOptions;
use nurt_contracts::error::Result;
use nurt_contracts::journal::JournalStore;
use nurt_contracts::services::Services;
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelReason, CancelToken, EffectOutcome, EffectStage, ExecCtx, StageData,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Record, Value};
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::host::TestHost;
use nurt_testkit::journal::RecordingJournal;
use nurt_testkit::vfs::VirtualFs;

/// Wall time reported by the clock of the tests, as nanoseconds since the epoch.
const WALL_TIME: i128 = 1_800_000_000_000_000_000;

/// Upper limit of the records read from a stage output.
const RECORDS_MAX: u64 = 10_000;

struct RealClock {
    origin: Instant,
}

impl Clock for RealClock {
    fn now(&self) -> WallTime {
        WallTime::from_unix_nanos(WALL_TIME)
    }

    fn monotonic(&self) -> Duration {
        self.origin.elapsed()
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: self.now(),
            boot: BootId("test".to_owned()),
            since_boot_with_suspend: self.monotonic(),
            since_boot_active: self.monotonic(),
        }
    }

    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
        cancel.wait_timeout(duration).map_or(Ok(()), Err)
    }
}

/// Example of the crate that imitates analyzers and formatters.
const FIXTURE: &str = "tool_fixture";

/// Path of the `tool_fixture` example.
///
/// `cargo test` builds the example next to the test binaries; runners that
/// build test targets only (coverage measurement) leave it out, so the source
/// is then compiled once with `rustc`.
pub(crate) fn fixture_path() -> PathBuf {
    static COMPILED: OnceLock<PathBuf> = OnceLock::new();
    let exe = std::env::current_exe().unwrap();
    let built = exe
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join("examples")
        .join(format!("{FIXTURE}{}", std::env::consts::EXE_SUFFIX));
    if built.is_file() {
        return built;
    }
    COMPILED.get_or_init(compile_fixture).clone()
}

fn compile_fixture() -> PathBuf {
    let source = Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("examples")
    .join(format!("{FIXTURE}.rs"));
    let dir = std::env::temp_dir().join(format!("nurt-verify-{FIXTURE}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join(format!("{FIXTURE}{}", std::env::consts::EXE_SUFFIX));
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let status = std::process::Command::new(rustc)
        .args(["--edition", "2024", "-o"])
        .arg(&output)
        .arg(&source)
        .status()
        .unwrap();
    assert!(status.success(), "compiling the {FIXTURE} example failed");
    output
}

/// Directories of one test: a project, a state directory and a directory of
/// tool programs that the context puts on the search path.
pub(crate) struct Workspace {
    pub(crate) project: tempfile::TempDir,
    pub(crate) state: tempfile::TempDir,
    pub(crate) bin: tempfile::TempDir,
}

impl Workspace {
    /// Empty directories.
    pub(crate) fn new() -> Self {
        Self {
            project: tempfile::tempdir().unwrap(),
            state: tempfile::tempdir().unwrap(),
            bin: tempfile::tempdir().unwrap(),
        }
    }

    /// Writes a file of the project, creating its directories.
    pub(crate) fn file(&self, relative: &str, text: &str) -> &Self {
        let path = self.project.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
        self
    }

    /// Content of a file of the project.
    pub(crate) fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.project.path().join(relative)).unwrap()
    }

    /// Installs the test program under the name of an analyzer.
    pub(crate) fn install(&self, name: &str) -> &Self {
        let target = self
            .bin
            .path()
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        std::fs::copy(fixture_path(), target).unwrap();
        self
    }

    /// Context with the project as the base directory.
    pub(crate) fn context(&self) -> ExecCtx {
        self.builder(self.bin.path().as_os_str()).build()
    }

    /// Context that searches the programs of the system.
    pub(crate) fn system_context(&self) -> ExecCtx {
        let path = std::env::var_os("PATH").unwrap_or_default();
        self.builder(&path).build()
    }

    /// Context with the code service and the file listing of the doubles and,
    /// when given, repository queries.
    pub(crate) fn serviced(&self, git: Option<FakeGit>) -> ExecCtx {
        let mut services = Services::new()
            .with_code(Arc::new(FakeCode))
            .with_file_list(Arc::new(DiskFiles));
        if let Some(git) = git {
            services = services.with_git_query(Arc::new(git));
        }
        self.builder(self.bin.path().as_os_str())
            .services(Arc::new(services))
            .build()
    }

    /// Context that has a recording journal store.
    pub(crate) fn journaled(&self) -> (ExecCtx, Arc<RecordingJournal>) {
        let clock = Arc::new(VirtualClock::new());
        let journal = Arc::new(RecordingJournal::new(Arc::new(VirtualFs::new(clock))));
        let store: Arc<dyn JournalStore> = journal.clone();
        (
            self.builder(self.bin.path().as_os_str())
                .journal(store)
                .build(),
            journal,
        )
    }

    /// Like [`serviced`](Self::serviced), with `base` as the base directory
    /// of the request, for example a link to the project.
    pub(crate) fn serviced_at(&self, base: &Path, git: Option<FakeGit>) -> ExecCtx {
        let mut services = Services::new()
            .with_code(Arc::new(FakeCode))
            .with_file_list(Arc::new(DiskFiles));
        if let Some(git) = git {
            services = services.with_git_query(Arc::new(git));
        }
        self.builder_at(self.bin.path().as_os_str(), base)
            .services(Arc::new(services))
            .build()
    }

    fn builder(&self, path: &std::ffi::OsStr) -> nurt_contracts::stage::ExecCtxBuilder {
        self.builder_at(path, self.project.path())
    }

    fn builder_at(
        &self,
        path: &std::ffi::OsStr,
        base: &Path,
    ) -> nurt_contracts::stage::ExecCtxBuilder {
        let host = TestHost::native().with_var("PATH", path);
        let options = RequestOptions {
            base: Some(base.to_string_lossy().into_owned()),
            ..RequestOptions::default()
        };
        ExecCtx::builder(
            Arc::new(RealClock {
                origin: Instant::now(),
            }),
            Arc::new(host),
        )
        .fs(Arc::new(disk::DiskFs))
        .state_dir(self.state.path().to_path_buf())
        .options(Arc::new(options))
    }
}

/// Arguments of a command from name and value pairs.
pub(crate) fn args(command: &str, values: &[(&str, Value)]) -> BoundArgs {
    values
        .iter()
        .fold(BoundArgs::new(command), |args, (name, value)| {
            args.with(*name, ArgValue::Value(value.clone()), false)
        })
}

/// Text value.
pub(crate) fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

/// List of text values.
pub(crate) fn texts(values: &[&str]) -> Value {
    Value::List(values.iter().map(|value| text(value)).collect())
}

/// Header and records of a stage output.
pub(crate) struct Output {
    pub(crate) header: Option<Record>,
    pub(crate) records: Vec<Record>,
}

fn collect(output: StageOutput<'_>) -> Output {
    let header = output.header;
    let records = match output.data {
        StageData::Records(stream) => stream.collect_bounded(RECORDS_MAX).unwrap(),
        _ => Vec::new(),
    };
    Output { header, records }
}

/// Stage of a command from the public list.
pub(crate) fn stage(command: &str) -> Arc<dyn nurt_contracts::stage::StageImpl> {
    nurt_verify::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == command)
        .unwrap()
}

/// Runs a read stage.
pub(crate) fn read(ctx: &ExecCtx, command: &str, args: &BoundArgs) -> Result<Output> {
    let stage = stage(command);
    let output = stage
        .as_read()
        .unwrap()
        .execute(args, StageInput::none(), ctx)?;
    Ok(collect(output))
}

/// Plan of an effect stage.
pub(crate) fn plan(
    ctx: &ExecCtx,
    command: &str,
    args: &BoundArgs,
) -> Result<nurt_contracts::stage::EffectPlan> {
    stage(command)
        .as_effect()
        .unwrap()
        .plan(args, StageInput::none(), ctx)
}

/// Plans and applies an effect stage under a transaction that is not
/// observed.
pub(crate) fn apply(
    ctx: &ExecCtx,
    command: &str,
    args: &BoundArgs,
) -> Result<(Output, EffectOutcomeSummary)> {
    let stage = stage(command);
    let effect: &dyn EffectStage = stage.as_effect().unwrap();
    let plan = effect.plan(args, StageInput::none(), ctx)?;
    let clock = Arc::new(VirtualClock::new());
    let journal = RecordingJournal::new(Arc::new(VirtualFs::new(clock)));
    let mut tx = journal.begin(nurt_contracts::journal::BeginRequest {
        kind: nurt_contracts::journal::OpKind::command(command),
        session: None,
        program: nurt_contracts::bound::BoundDocument::new(
            String::new(),
            String::new(),
            nurt_contracts::bound::BoundProgram {
                statements: Vec::new(),
            },
        ),
        corrections: Vec::new(),
        bulk: false,
        counts_as_step: true,
        preserve_budget: None,
    });
    let outcome: EffectOutcome<'_> = effect.apply(args, plan, &mut *tx, ctx)?;
    let summary = EffectOutcomeSummary {
        changes: outcome
            .changes
            .iter()
            .map(|change| change.path.as_str().to_owned())
            .collect(),
        notable: outcome
            .notable
            .iter()
            .map(|entry| entry.code.as_str().to_owned())
            .collect(),
    };
    Ok((collect(outcome.output), summary))
}

/// Paths of the changes and codes of the remarks of an applied stage.
pub(crate) struct EffectOutcomeSummary {
    pub(crate) changes: Vec<String>,
    pub(crate) notable: Vec<String>,
}

/// Text of a field of a record.
pub(crate) fn field<'a>(record: &'a Record, name: &str) -> Option<&'a str> {
    record.get(name).and_then(Value::as_str)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
