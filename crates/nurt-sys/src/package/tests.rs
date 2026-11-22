// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the package stages with a directory root and recording mocks.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::WriteHandle;
use nurt_contracts::journal::{
    Applied, ExemptWrite, ExternalChange, Mutation, MutationTx, OpHandle, OpId, Preserved,
    RollbackReason, RollbackReport, TargetIndex, TargetSpec,
};
use nurt_contracts::kinds::Implementation;
use nurt_contracts::record::{Package, PackageManager};
use nurt_contracts::report::{Irreversible, IrreversibleEntry};
use nurt_contracts::services::{
    FileHolder, FileHolderService, ProcessHost, ProgramOutcome, ProgramSpec, ServiceChild,
    ServiceSpawnSpec,
};
use nurt_contracts::spec::CatalogFile;
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelToken, EffectStage, ExecCtx, Extensions, ReadStage, StageData,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Record, Value};

use super::*;

struct TestClock;

impl Clock for TestClock {
    fn now(&self) -> WallTime {
        WallTime(0)
    }
    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: WallTime(0),
            boot: BootId("test".to_owned()),
            since_boot_with_suspend: Duration::ZERO,
            since_boot_active: Duration::ZERO,
        }
    }
    fn sleep(
        &self,
        _duration: Duration,
        _cancel: &CancelToken,
    ) -> std::result::Result<(), nurt_contracts::stage::CancelReason> {
        Ok(())
    }
}

struct TestHost(HostInfo);

impl TestHost {
    fn linux() -> Self {
        Self(HostInfo {
            os: Os::Linux,
            os_version: "6".to_owned(),
            architecture: "x86_64".to_owned(),
            hostname: "host".to_owned(),
            user: "user".to_owned(),
            home: PathBuf::from("/home/user"),
            temp: PathBuf::from("/tmp"),
            path_style: PathStyle::Posix,
            timezone: "UTC".to_owned(),
            locale: None,
            cpus: 1,
        })
    }
}

impl Host for TestHost {
    fn info(&self) -> &HostInfo {
        &self.0
    }
    fn env_var(&self, _name: &str) -> Option<OsString> {
        None
    }
    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        Vec::new()
    }
}

#[derive(Default)]
struct RecordingProcess {
    calls: Mutex<Vec<ProgramSpec>>,
    exit_code: i32,
    stderr: &'static str,
}

impl ProcessHost for RecordingProcess {
    fn run_program(&self, spec: &ProgramSpec, _cancel: &CancelToken) -> Result<ProgramOutcome> {
        self.calls.lock().expect("calls").push(spec.clone());
        Ok(ProgramOutcome {
            exit_code: Some(self.exit_code),
            stderr: self.stderr.as_bytes().to_vec(),
            ..ProgramOutcome::default()
        })
    }
    fn resolve_program(&self, program: &str, _cwd: &Path) -> Result<PathBuf> {
        Ok(PathBuf::from(program))
    }
    fn spawn_service(&self, _spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
        Err(Error::new(ErrorCode::Spawn, "not available in tests"))
    }
}

#[derive(Default)]
struct RecordingTx {
    markers: Vec<IrreversibleEntry>,
}

fn unavailable<T>() -> Result<T> {
    Err(Error::new(ErrorCode::StageFailed, "not used by package stages"))
}

impl MutationTx for RecordingTx {
    fn op(&self) -> Option<OpId> {
        None
    }
    fn preserve(&mut self, _targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        unavailable()
    }
    fn stage(&mut self, _target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        unavailable()
    }
    fn apply(&mut self, _target: TargetIndex, _mutation: Mutation) -> Result<Applied> {
        unavailable()
    }
    fn observe_external(&mut self, _changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        unavailable()
    }
    fn mark_irreversible(&mut self, marker: IrreversibleEntry) {
        self.markers.push(marker);
    }
    fn record_exempt(&mut self, _write: ExemptWrite) {}
    fn commit(self: Box<Self>) -> Result<OpHandle> {
        unavailable()
    }
    fn rollback(self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
        unavailable()
    }
}

/// Installed packages that change after the first read, as after a manager run.
struct StagedSource {
    reads: AtomicUsize,
    first: Vec<(&'static str, &'static str)>,
    later: Vec<(&'static str, &'static str)>,
}

impl InstalledSource for StagedSource {
    fn installed(&self, _manager: PackageManager, _env: &HostEnv) -> Result<InstalledSet> {
        let rows = if self.reads.fetch_add(1, Ordering::SeqCst) == 0 { &self.first } else { &self.later };
        Ok(InstalledSet {
            packages: rows
                .iter()
                .map(|(name, version)| InstalledPackage {
                    name: (*name).to_owned(),
                    version: Some((*version).to_owned()),
                    ..InstalledPackage::default()
                })
                .collect(),
            warnings: Vec::new(),
        })
    }
}

struct Holders(Vec<FileHolder>);

impl FileHolderService for Holders {
    fn holders(&self, _path: &Path) -> Result<Vec<FileHolder>> {
        Ok(self.0.clone())
    }
}

fn apt_root(status: &str, lists: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temporary directory");
    let dpkg = dir.path().join("var/lib/dpkg");
    fs::create_dir_all(&dpkg).expect("dpkg directory");
    fs::write(dpkg.join("status"), status).expect("status");
    fs::write(dpkg.join("arch"), "amd64\n").expect("arch");
    let apt = dir.path().join("var/lib/apt/lists");
    fs::create_dir_all(&apt).expect("lists directory");
    for (name, content) in lists {
        fs::write(apt.join(name), content).expect("list");
    }
    dir
}

const STATUS: &str = "Package: curl\nStatus: install ok installed\nVersion: 8.5.0-1\nArchitecture: amd64\n\n\
    Package: git\nStatus: install ok installed\nVersion: 1:2.40-1\nArchitecture: amd64\n\n\
    Package: vim\nStatus: install ok installed\nVersion: 9.0-1~rc1\nArchitecture: amd64\n\n";
const LIST: &str = "Package: curl\nVersion: 8.5.0-2\nArchitecture: amd64\nDescription: transfer tool\n\n\
    Package: git\nVersion: 1:2.40-1\nArchitecture: amd64\nDescription: version control\n\n\
    Package: vim\nVersion: 9.0-1\nArchitecture: amd64\nDescription: editor\n\n\
    Package: curl-extra\nVersion: 1\nArchitecture: amd64\nDescription: extras for CURL\n\n\
    Package: libcurl\nVersion: 1\nArchitecture: amd64\nDescription: library\n\n";

fn context(process: Option<Arc<dyn ProcessHost>>, holders: Option<Arc<dyn FileHolderService>>) -> ExecCtx {
    let mut builder = ExecCtx::builder(Arc::new(TestClock), Arc::new(TestHost::linux()));
    if let Some(process) = process {
        builder = builder.process(process);
    }
    let mut extensions = Extensions::new();
    if let Some(holders) = holders {
        extensions.insert::<dyn FileHolderService>(holders);
    }
    builder.extensions(extensions).build()
}

fn text(value: &str) -> ArgValue {
    ArgValue::Value(Value::Text(value.to_owned()))
}

fn names(items: &[&str]) -> ArgValue {
    ArgValue::Value(Value::List(items.iter().map(|i| Value::Text((*i).to_owned())).collect()))
}

fn records(output: StageOutput<'_>) -> Vec<Record> {
    match output.data {
        StageData::Records(stream) => stream.collect::<Result<Vec<_>>>().expect("records"),
        _ => panic!("expected records"),
    }
}

fn packages(output: StageOutput<'_>) -> Vec<Package> {
    records(output)
        .into_iter()
        .map(|record| Package::try_from(record).expect("package record"))
        .collect()
}

fn env_for(dir: &tempfile::TempDir, process_root: bool) -> Arc<PackageEnv> {
    Arc::new(PackageEnv::new(SysRoot::at(dir.path()), Arc::new(DpkgStatusSource)).with_root_check(move || process_root))
}

fn input() -> StageInput<'static> {
    StageInput {
        data: StageData::None,
        header: None,
        order: None,
        render: None,
    }
}

#[test]
fn list_outdated_sets_available_version() {
    let dir = apt_root(STATUS, &[("h_dists_s_main_binary-amd64_Packages", LIST)]);
    let stage = PackageList::new(env_for(&dir, true));
    let ctx = context(None, None);
    let args = BoundArgs::new("package.list").with("outdated", ArgValue::Value(Value::Bool(true)), false);
    let result = packages(stage.execute(&args, input(), &ctx).expect("list"));
    let summary: Vec<_> = result
        .iter()
        .map(|p| (p.name.as_str(), p.version.as_str(), p.available_version.as_deref()))
        .collect();
    assert_eq!(summary, [("curl", "8.5.0-1", Some("8.5.0-2")), ("vim", "9.0-1~rc1", Some("9.0-1"))]);
    assert!(result.iter().all(|p| p.installed && p.manager == PackageManager::Apt));
}

#[test]
fn list_filters_by_name_and_reads_without_starting_programs() {
    let dir = apt_root(STATUS, &[]);
    let process = Arc::new(RecordingProcess::default());
    let ctx = context(Some(process.clone()), None);
    let stage = PackageList::new(env_for(&dir, true));
    let args = BoundArgs::new("package.list").with("name", text("GI"), false);
    let result = packages(stage.execute(&args, input(), &ctx).expect("list"));
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "git");
    assert!(result[0].available_version.is_none());
    let search = PackageSearch::new(env_for(&dir, true));
    let query = BoundArgs::new("package.search").with("query", text("curl"), false);
    search.execute(&query, input(), &ctx).expect("search");
    assert!(process.calls.lock().expect("calls").is_empty());
}

#[test]
fn search_exact_name_first_case_insensitive() {
    let dir = apt_root(STATUS, &[("h_dists_s_main_binary-amd64_Packages", LIST)]);
    let stage = PackageSearch::new(env_for(&dir, true));
    let ctx = context(None, None);
    let args = BoundArgs::new("package.search").with("query", text("CURL"), false);
    let result = packages(stage.execute(&args, input(), &ctx).expect("search"));
    let found: Vec<_> = result.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(found, ["curl", "curl-extra", "libcurl"]);
    assert!(result[0].installed);
    assert_eq!(result[0].available_version.as_deref(), Some("8.5.0-2"));
    assert!(!result[1].installed);
}

#[test]
fn explicit_manager_without_markers_is_external_missing() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let stage = PackageList::new(env_for(&dir, true));
    let ctx = context(None, None);
    let args = BoundArgs::new("package.list").with("manager", ArgValue::Value(Value::Enum("pacman".to_owned())), false);
    let error = stage.execute(&args, input(), &ctx).expect_err("error");
    assert_eq!(error.code(), ErrorCode::ExternalMissing);
    let auto = BoundArgs::new("package.list");
    assert_eq!(
        stage.execute(&auto, input(), &ctx).expect_err("error").code(),
        ErrorCode::ExternalMissing
    );
}

#[test]
fn install_missing_manager_external_missing() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let stage = PackageChange::new(env_for(&dir, true), Action::Install);
    let ctx = context(None, None);
    let args = BoundArgs::new("package.install").with("names", names(&["curl"]), false);
    let error = stage.plan(&args, input(), &ctx).expect_err("error");
    assert_eq!(error.code(), ErrorCode::ExternalMissing);
}

#[test]
fn install_without_root_is_privilege_error_before_any_program() {
    let dir = apt_root(STATUS, &[]);
    let process = Arc::new(RecordingProcess::default());
    let stage = PackageChange::new(env_for(&dir, false), Action::Install);
    let ctx = context(Some(process.clone()), None);
    let args = BoundArgs::new("package.install").with("names", names(&["curl"]), false);
    let error = stage.plan(&args, input(), &ctx).expect_err("error");
    assert_eq!(error.code(), ErrorCode::Privilege);
    assert!(process.calls.lock().expect("calls").is_empty());
}

#[test]
fn install_lock_held_in_use_with_holders() {
    let dir = apt_root(STATUS, &[]);
    let holders = Arc::new(Holders(vec![FileHolder {
        pid: 42,
        name: "unattended-upgr".to_owned(),
        kind: None,
    }]));
    let stage = PackageChange::new(env_for(&dir, true), Action::Install);
    let ctx = context(None, Some(holders));
    let args = BoundArgs::new("package.install").with("names", names(&["curl"]), false);
    let error = stage.plan(&args, input(), &ctx).expect_err("error");
    assert_eq!(error.code(), ErrorCode::InUse);
    let detail = serde_json::to_string(&error.data()).expect("serialized error");
    assert!(detail.contains("unattended-upgr"), "{detail}");
}

#[test]
fn install_effect_before_after_irreversible_package_change() {
    let dir = apt_root(STATUS, &[]);
    let source = Arc::new(StagedSource {
        reads: AtomicUsize::new(0),
        first: vec![],
        later: vec![("tool", "2.0-1")],
    });
    let env = Arc::new(PackageEnv::new(SysRoot::at(dir.path()), source).with_root_check(|| true));
    let process = Arc::new(RecordingProcess::default());
    let ctx = context(Some(process.clone()), None);
    let stage = PackageChange::new(env, Action::Install);
    let args = BoundArgs::new("package.install").with("names", names(&["tool"]), false);
    let plan = stage.plan(&args, input(), &ctx).expect("plan");
    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.irreversible[0].reason, Irreversible::PackageChanged);
    assert_eq!(plan.preview.len(), 1);
    let mut tx = RecordingTx::default();
    let outcome = stage.apply(&args, plan, &mut tx, &ctx).expect("apply");
    assert_eq!(tx.markers.len(), 1);
    assert_eq!(tx.markers[0].reason, Irreversible::PackageChanged);
    assert_eq!(outcome.changes[0].path, "package:tool");
    assert_eq!(outcome.changes[0].before, None);
    assert_eq!(outcome.changes[0].after.as_deref(), Some("2.0-1"));
    let effects = records(outcome.output);
    assert_eq!(effects.len(), 1);
    let calls = process.calls.lock().expect("calls");
    assert_eq!(calls[0].program, "apt-get");
    assert!(calls[0].env.iter().any(|v| v.name == "DEBIAN_FRONTEND"));
}

#[test]
fn failed_manager_run_reports_stage_failed_with_stderr_tail() {
    let dir = apt_root(STATUS, &[]);
    let process = Arc::new(RecordingProcess {
        exit_code: 100,
        stderr: "E: Unable to locate package nothing",
        ..RecordingProcess::default()
    });
    let ctx = context(Some(process), None);
    let stage = PackageChange::new(env_for(&dir, true), Action::Install);
    let args = BoundArgs::new("package.install").with("names", names(&["nothing"]), false);
    let plan = stage.plan(&args, input(), &ctx).expect("plan");
    let mut tx = RecordingTx::default();
    let error = stage.apply(&args, plan, &mut tx, &ctx).expect_err("failure");
    assert_eq!(error.code(), ErrorCode::StageFailed);
    assert!(error.data().message.contains("Unable to locate package"));
    assert_eq!(tx.markers.len(), 1);
}

#[test]
fn stage_spec_external_marks_p_for_changes_only() {
    let catalog = catalog();
    for name in ["package.list", "package.search"] {
        let spec = catalog.command(name).expect("command");
        assert_eq!(spec.implementation, Implementation::PackageManager);
        assert_eq!(format!("{:?}", spec.effect), "Read");
    }
    for name in ["package.install", "package.remove", "package.upgrade"] {
        let spec = catalog.command(name).expect("command");
        assert_eq!(format!("{:?}", spec.effect), "External");
    }
    let env = Arc::new(PackageEnv::system());
    let host = TestHost::linux();
    let config = nurt_contracts::config::Config::default();
    let spec = catalog.command("package.install").expect("command");
    let check_ctx = nurt_contracts::stage::CheckCtx::new(spec, &host, &config);
    let args = BoundArgs::new("package.install").with("manager", ArgValue::Value(Value::Enum("apt".to_owned())), false);
    let info = PackageChange::new(env.clone(), Action::Install).check(&args, &check_ctx).expect("info");
    assert_eq!(info.programs.len(), 1);
    assert_eq!(info.programs[0].program, "apt-get");
    let list = PackageList::new(env);
    assert!(list.check(&args, &check_ctx).expect("info").programs.is_empty());
}

fn catalog() -> CatalogFile {
    toml::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../catalog/package.toml")))
        .expect("package catalog")
}

#[test]
fn catalog_signatures_match_frozen() {
    let catalog = catalog();
    let all = Arc::new(PackageEnv::system());
    for stage in stages(&all) {
        let descriptor = stage.descriptor();
        let spec = catalog.command(&descriptor.command).expect("command in catalog");
        assert!(spec.param("manager").is_some(), "{}", descriptor.command);
        assert_eq!(stage.as_read().is_some(), (format!("{:?}", spec.effect) == "Read"), "{}", descriptor.command);
    }
    assert_eq!(catalog.commands.len(), 5);
    let search = catalog.command("package.search").expect("search");
    assert!(search.param("query").is_some_and(|p| p.required));
}
