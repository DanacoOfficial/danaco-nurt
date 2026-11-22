// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Test doubles and fixture builders shared by the integration tests.

#![allow(
    dead_code,
    reason = "each test binary uses a different subset of the helpers"
)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::{
    ProcessHost, ProgramOutcome, ProgramSpec, ServiceChild, ServiceSpawnSpec, Services,
};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelReason, CancelToken, Event, EventSink, ExecCtx, StageInput,
};
use nurt_contracts::value::{Record, Value};
use nurt_programs::index::{IndexCache, IndexOptions};
use nurt_programs::model::{Detail, DetailWant, EntryKey, GuideBody, SourceData};
use nurt_programs::source::{
    Deadline, Fingerprint, FingerprintBuilder, HostEnv, ProgramSource, SourceError,
    SourceErrorKind, SourceId, SourceKind, SourceRegistry,
};
use nurt_programs::stages::{ProgramsSetup, stages_with};

/// Wall time of the test clock: 2026-11-01T00:00:00Z.
pub(crate) const EPOCH_NANOS: i128 = 1_793_491_200_000_000_000;

/// Clock with a fixed wall time and a real monotonic reading.
#[derive(Debug)]
pub(crate) struct TestClock {
    start: std::time::Instant,
}

impl TestClock {
    pub(crate) fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
        }
    }
}

impl Clock for TestClock {
    fn now(&self) -> WallTime {
        WallTime::from_unix_nanos(EPOCH_NANOS + self.start.elapsed().as_nanos() as i128)
    }

    fn monotonic(&self) -> Duration {
        self.start.elapsed()
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

/// Host with a fixed description and no variables.
#[derive(Debug)]
pub(crate) struct TestHost {
    info: HostInfo,
}

impl TestHost {
    pub(crate) fn new(temp: &Path) -> Self {
        Self {
            info: HostInfo {
                os: Os::Linux,
                os_version: "6.8.0".to_owned(),
                architecture: "x86_64".to_owned(),
                hostname: "test".to_owned(),
                user: "user".to_owned(),
                home: PathBuf::from("/home/user"),
                temp: temp.to_path_buf(),
                path_style: PathStyle::Posix,
                timezone: "UTC".to_owned(),
                locale: None,
                cpus: 1,
            },
        }
    }
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

/// Process host that records calls and answers with a fixed output.
#[derive(Debug, Default)]
pub(crate) struct TestProcess {
    pub(crate) calls: Mutex<Vec<Vec<String>>>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) timed_out: bool,
}

impl ProcessHost for TestProcess {
    fn resolve_program(&self, program: &str, _cwd: &Path) -> Result<PathBuf> {
        Ok(PathBuf::from(program))
    }

    fn run_program(&self, spec: &ProgramSpec, _cancel: &CancelToken) -> Result<ProgramOutcome> {
        let mut call = vec![spec.program.clone()];
        call.extend(spec.args.iter().cloned());
        self.calls.lock().unwrap().push(call);
        Ok(ProgramOutcome {
            program_path: PathBuf::from(&spec.program),
            exit_code: Some(0),
            stdout: self.stdout.clone(),
            timed_out: self.timed_out,
            ..ProgramOutcome::default()
        })
    }

    fn spawn_service(&self, _spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
        Err(Error::new(ErrorCode::Unsupported, "not available in tests"))
    }
}

/// Event sink that keeps the messages of log events.
#[derive(Debug, Default)]
pub(crate) struct Logs(pub(crate) Mutex<Vec<String>>);

impl EventSink for Logs {
    fn emit(&self, event: Event) {
        if let Event::Log(log) = event {
            self.0.lock().unwrap().push(log.message);
        }
    }
}

/// Context of one test: clock, host, process host and logs.
pub(crate) struct Fixture {
    pub(crate) root: tempfile::TempDir,
    pub(crate) process: Arc<TestProcess>,
    pub(crate) logs: Arc<Logs>,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        Self {
            root: tempfile::tempdir().unwrap(),
            process: Arc::new(TestProcess {
                stdout: b"usage: tool [options]\n".to_vec(),
                ..TestProcess::default()
            }),
            logs: Arc::new(Logs::default()),
        }
    }

    pub(crate) fn path(&self) -> &Path {
        self.root.path()
    }

    pub(crate) fn env(&self) -> HostEnv {
        HostEnv::new(Os::Linux)
            .with_root(self.path())
            .with_var("PATH", "/usr/local/bin:/usr/bin:/bin")
            .with_home("/home/user")
    }

    pub(crate) fn ctx(&self) -> ExecCtx {
        let temp = self.path().join("tmp");
        std::fs::create_dir_all(&temp).unwrap();
        ExecCtx::builder(Arc::new(TestClock::new()), Arc::new(TestHost::new(&temp)))
            .services(Arc::new(
                Services::new().with_process_host(self.process.clone()),
            ))
            .events(self.logs.clone())
            .build()
    }

    /// Writes a file below the root, creating directories.
    pub(crate) fn write(&self, host_path: &str, bytes: &[u8]) -> PathBuf {
        let file = self.path().join(host_path.trim_start_matches('/'));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, bytes).unwrap();
        file
    }

    /// Writes an executable file below the root.
    pub(crate) fn executable(&self, host_path: &str) -> PathBuf {
        let file = self.write(host_path, b"#!/bin/sh\necho help\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        file
    }

    /// Creates a symbolic link below the root.
    #[cfg(unix)]
    pub(crate) fn symlink(&self, target: &str, host_path: &str) {
        let link = self.path().join(host_path.trim_start_matches('/'));
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    /// Setup with the built-in providers plus `extra`, a private cache and
    /// the fixture environment.
    pub(crate) fn setup(&self, extra: Vec<Arc<dyn ProgramSource>>) -> ProgramsSetup {
        let mut registry = nurt_programs::sources::builtin(&self.env(), &Default::default());
        for source in extra {
            registry.add(source);
        }
        self.setup_with(registry)
    }

    pub(crate) fn setup_with(&self, registry: SourceRegistry) -> ProgramsSetup {
        ProgramsSetup {
            registry: Some(registry),
            env: Some(self.env()),
            options: Some(IndexOptions::default()),
            cache: Some(Arc::new(IndexCache::new())),
            ranking: None,
        }
    }

    pub(crate) fn logs(&self) -> Vec<String> {
        self.logs.0.lock().unwrap().clone()
    }
}

/// Runs one command of the group and collects its records.
pub(crate) fn run(setup: &ProgramsSetup, ctx: &ExecCtx, args: BoundArgs) -> Result<Vec<Record>> {
    let stage = stages_with(setup.clone())
        .into_iter()
        .find(|stage| stage.descriptor().command == args.command())
        .unwrap();
    let output = stage
        .as_read()
        .unwrap()
        .execute(&args, StageInput::none(), ctx)?;
    match output.data {
        nurt_contracts::stage::StageData::Records(stream) => stream.collect(),
        _ => Ok(Vec::new()),
    }
}

/// Arguments of a command from `(name, value)` pairs.
pub(crate) fn args(command: &str, values: &[(&str, Value)]) -> BoundArgs {
    let mut args = BoundArgs::new(command);
    for (name, value) in values {
        args.insert(*name, ArgValue::Value(value.clone()), false);
    }
    args
}

pub(crate) fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

pub(crate) fn texts(values: &[&str]) -> Value {
    Value::List(values.iter().map(|value| text(value)).collect())
}

/// Text of a record field.
pub(crate) fn field(record: &Record, name: &str) -> Option<String> {
    match record.get(name)? {
        Value::Text(text) | Value::Enum(text) | Value::Path(text) => Some(text.clone()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Int(number) => Some(number.to_string()),
        Value::Number(number) => Some(number.to_string()),
        Value::List(items) => Some(
            items
                .iter()
                .map(|item| match item {
                    Value::Text(text) | Value::Path(text) => text.clone(),
                    other => format!("{other:?}"),
                })
                .collect::<Vec<_>>()
                .join(","),
        ),
        other => Some(format!("{other:?}")),
    }
}

pub(crate) fn names(records: &[Record]) -> Vec<String> {
    records
        .iter()
        .filter_map(|record| field(record, "name"))
        .collect()
}

/// Provider that returns fixed data, for catalogs and failure cases.
pub(crate) struct StaticSource {
    pub(crate) id: SourceId,
    pub(crate) kind: SourceKind,
    pub(crate) priority: i32,
    pub(crate) data: std::result::Result<SourceData, SourceErrorKind>,
    pub(crate) guides: HashMap<String, GuideBody>,
    pub(crate) loads: AtomicUsize,
}

impl StaticSource {
    pub(crate) fn catalog(id: &str, data: SourceData) -> Self {
        Self {
            id: SourceId::catalog(id),
            kind: SourceKind::Catalog,
            priority: 0,
            data: Ok(data),
            guides: HashMap::new(),
            loads: AtomicUsize::new(0),
        }
    }

    pub(crate) fn failing(id: &str) -> Self {
        Self {
            id: SourceId::new(id),
            kind: SourceKind::Package,
            priority: 0,
            data: Err(SourceErrorKind::Corrupt),
            guides: HashMap::new(),
            loads: AtomicUsize::new(0),
        }
    }
}

impl ProgramSource for StaticSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn kind(&self) -> SourceKind {
        self.kind
    }

    fn priority(&self) -> i32 {
        self.priority
    }

    fn detect(&self, _env: &HostEnv) -> bool {
        true
    }

    fn fingerprint(&self, env: &HostEnv) -> std::result::Result<Fingerprint, SourceError> {
        Ok(FingerprintBuilder::new(env)
            .text("static", self.id.as_str())
            .finish())
    }

    fn load(
        &self,
        _env: &HostEnv,
        _deadline: &Deadline<'_>,
    ) -> std::result::Result<SourceData, SourceError> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        self.data.clone().map_err(|kind| {
            SourceError::new(&self.id, kind, "damaged test data").at("/var/lib/test/db")
        })
    }

    fn detail(
        &self,
        _env: &HostEnv,
        key: &EntryKey,
        want: &DetailWant,
    ) -> std::result::Result<Option<Detail>, SourceError> {
        if !want.guide {
            return Ok(None);
        }
        Ok(self.guides.get(&key.key).map(|guide| Detail {
            guide: Some(guide.clone()),
            ..Detail::default()
        }))
    }
}

/// Builder of stored RPM headers.
#[derive(Default)]
pub(crate) struct HeaderBuilder {
    entries: Vec<(u32, u32, u32, u32)>,
    data: Vec<u8>,
}

impl HeaderBuilder {
    pub(crate) const STRING: u32 = 6;
    pub(crate) const STRING_ARRAY: u32 = 8;
    pub(crate) const I18N_STRING: u32 = 9;
    pub(crate) const INT32: u32 = 4;

    pub(crate) fn strings(mut self, tag: u32, kind: u32, values: &[&str]) -> Self {
        let offset = self.data.len() as u32;
        for value in values {
            self.data.extend(value.as_bytes());
            self.data.push(0);
        }
        self.entries.push((tag, kind, offset, values.len() as u32));
        self
    }

    pub(crate) fn int32(mut self, tag: u32, values: &[u32]) -> Self {
        while !self.data.len().is_multiple_of(4) {
            self.data.push(0);
        }
        let offset = self.data.len() as u32;
        for value in values {
            self.data.extend(value.to_be_bytes());
        }
        self.entries
            .push((tag, Self::INT32, offset, values.len() as u32));
        self
    }

    pub(crate) fn build(self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend((self.entries.len() as u32).to_be_bytes());
        out.extend((self.data.len() as u32).to_be_bytes());
        for (tag, kind, offset, count) in &self.entries {
            for value in [tag, kind, offset, count] {
                out.extend(value.to_be_bytes());
            }
        }
        out.extend(self.data);
        out
    }
}
