// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::package::tests
// Responsibility:   Tests of the package stages with a directory root and recording mocks.
// Dependencies:     nurt_contracts, nurt_programs
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

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
    ServiceSpawnSpec, Services,
};
use nurt_contracts::spec::CatalogFile;
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelToken, EffectStage, ExecCtx, Extensions, ReadStage, StageData,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Record, Value};
use nurt_programs::model::RawPackage;

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

/// Host of a synthetic account named `user`.
struct TestHost(HostInfo);

impl TestHost {
    fn new(os: Os, home: &str) -> Self {
        Self(HostInfo {
            os,
            os_version: "test".to_owned(),
            architecture: "x86_64".to_owned(),
            hostname: "host".to_owned(),
            user: "user".to_owned(),
            home: PathBuf::from(home),
            temp: PathBuf::from("/tmp"),
            path_style: if os == Os::Windows {
                PathStyle::Windows
            } else {
                PathStyle::Posix
            },
            timezone: "UTC".to_owned(),
            locale: None,
            cpus: 1,
        })
    }

    fn linux() -> Self {
        Self::new(Os::Linux, "/home/user")
    }

    fn windows() -> Self {
        Self::new(Os::Windows, "C:\\Users\\user")
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
    missing: bool,
}

impl ProcessHost for RecordingProcess {
    fn run_program(&self, spec: &ProgramSpec, _cancel: &CancelToken) -> Result<ProgramOutcome> {
        self.calls.lock().expect("calls").push(spec.clone());
        if self.missing {
            return Err(Error::new(ErrorCode::ProgramNotFound, "program not found"));
        }
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
    Err(Error::new(
        ErrorCode::StageFailed,
        "not used by package stages",
    ))
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

/// Installed packages that change after the first read, as after a run of
/// the manager.
struct StagedSource {
    reads: AtomicUsize,
    first: Vec<(&'static str, &'static str)>,
    later: Vec<(&'static str, &'static str)>,
}

impl InstalledSource for StagedSource {
    fn installed(&self, _manager: PackageManager, _env: &HostEnv) -> Result<InstalledSet> {
        let rows = if self.reads.fetch_add(1, Ordering::SeqCst) == 0 {
            &self.first
        } else {
            &self.later
        };
        Ok(InstalledSet {
            packages: rows
                .iter()
                .map(|(name, version)| RawPackage {
                    name: (*name).to_owned(),
                    version: Some((*version).to_owned()),
                    ..RawPackage::default()
                })
                .collect(),
            warnings: vec!["index_partial: one entry skipped".to_owned()],
        })
    }
}

struct Holders(Vec<FileHolder>);

impl FileHolderService for Holders {
    fn holders(&self, _path: &Path) -> Result<Vec<FileHolder>> {
        Ok(self.0.clone())
    }
}

fn put(root: &Path, path: &str, content: &[u8]) {
    let target = root.join(path);
    fs::create_dir_all(target.parent().expect("parent")).expect("directories");
    fs::write(target, content).expect("file");
}

/// Debian-like root with a dpkg status file, the native architecture
/// `amd64` and the given APT package lists.
fn apt_root(status: &str, lists: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temporary directory");
    put(dir.path(), "var/lib/dpkg/status", status.as_bytes());
    put(dir.path(), "var/lib/dpkg/arch", b"amd64\n");
    for (name, content) in lists {
        put(
            dir.path(),
            &format!("var/lib/apt/lists/{name}"),
            content.as_bytes(),
        );
    }
    dir
}

/// Installed: `curl` and `vim` older than the list, `git` current.
const STATUS: &str = "Package: curl\nStatus: install ok installed\nVersion: 8.5.0-1\nArchitecture: amd64\nDescription: transfer tool\n\n\
    Package: git\nStatus: install ok installed\nVersion: 1:2.40-1\nArchitecture: amd64\n\n\
    Package: vim\nStatus: install ok installed\nVersion: 9.0-1~rc1\nArchitecture: amd64\n\n";
const LIST_FILE: &str = "deb.example.org_debian_dists_stable_main_binary-amd64_Packages";
const LIST: &str = "Package: curl\nVersion: 8.5.0-2\nArchitecture: amd64\nDescription: transfer tool\n\n\
    Package: git\nVersion: 1:2.40-1\nArchitecture: amd64\nDescription: version control\n\n\
    Package: vim\nVersion: 9.0-1\nArchitecture: amd64\nDescription: editor\n\n\
    Package: curl-extra\nVersion: 1\nArchitecture: amd64\nDescription: extras for CURL\n\n\
    Package: libcurl\nVersion: 1\nArchitecture: amd64\nDescription: library\n\n";

fn context_on(
    host: TestHost,
    process: Option<Arc<dyn ProcessHost>>,
    holders: Option<Arc<dyn FileHolderService>>,
) -> ExecCtx {
    let mut builder = ExecCtx::builder(Arc::new(TestClock), Arc::new(host));
    if let Some(process) = process {
        builder = builder.services(Arc::new(Services::new().with_process_host(process)));
    }
    let mut extensions = Extensions::new();
    if let Some(holders) = holders {
        extensions.insert::<dyn FileHolderService>(holders);
    }
    builder.extensions(extensions).build()
}

fn context(
    process: Option<Arc<dyn ProcessHost>>,
    holders: Option<Arc<dyn FileHolderService>>,
) -> ExecCtx {
    context_on(TestHost::linux(), process, holders)
}

fn text(value: &str) -> ArgValue {
    ArgValue::Value(Value::Text(value.to_owned()))
}

fn manager(value: &str) -> ArgValue {
    ArgValue::Value(Value::Enum(value.to_owned()))
}

fn names(items: &[&str]) -> ArgValue {
    ArgValue::Value(Value::List(
        items
            .iter()
            .map(|item| Value::Text((*item).to_owned()))
            .collect(),
    ))
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

fn env_for(dir: &tempfile::TempDir, superuser: bool) -> Arc<PackageEnv> {
    Arc::new(
        PackageEnv::system()
            .with_root(dir.path())
            .with_root_check(move || superuser),
    )
}

fn staged_env(
    dir: &tempfile::TempDir,
    first: Vec<(&'static str, &'static str)>,
    later: Vec<(&'static str, &'static str)>,
) -> Arc<PackageEnv> {
    let source = Arc::new(StagedSource {
        reads: AtomicUsize::new(0),
        first,
        later,
    });
    Arc::new(
        PackageEnv::new(source)
            .with_root(dir.path())
            .with_root_check(|| true),
    )
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
fn list_reads_dpkg_database_through_program_catalog_readers() {
    let dir = apt_root(STATUS, &[]);
    let stage = PackageList::new(env_for(&dir, true));
    let ctx = context(None, None);
    let result = packages(
        stage
            .execute(&BoundArgs::new("package.list"), input(), &ctx)
            .expect("list"),
    );
    let listed: Vec<_> = result
        .iter()
        .map(|p| (p.name.as_str(), p.version.as_str()))
        .collect();
    assert_eq!(
        listed,
        [
            ("curl", "8.5.0-1"),
            ("git", "1:2.40-1"),
            ("vim", "9.0-1~rc1")
        ]
    );
    assert!(
        result
            .iter()
            .all(|p| p.installed && p.manager == PackageManager::Apt)
    );
    assert_eq!(result[0].architecture.as_deref(), Some("amd64"));
    assert_eq!(result[0].description.as_deref(), Some("transfer tool"));
}

#[test]
fn list_outdated_sets_available_version() {
    let dir = apt_root(STATUS, &[(LIST_FILE, LIST)]);
    let stage = PackageList::new(env_for(&dir, true));
    let ctx = context(None, None);
    let args =
        BoundArgs::new("package.list").with("outdated", ArgValue::Value(Value::Bool(true)), false);
    let result = packages(stage.execute(&args, input(), &ctx).expect("list"));
    let summary: Vec<_> = result
        .iter()
        .map(|p| {
            (
                p.name.as_str(),
                p.version.as_str(),
                p.available_version.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("curl", "8.5.0-1", Some("8.5.0-2")),
            ("vim", "9.0-1~rc1", Some("9.0-1"))
        ]
    );
}

#[test]
fn list_filters_by_name_and_reads_without_starting_programs() {
    let dir = apt_root(STATUS, &[(LIST_FILE, LIST)]);
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
fn list_chocolatey_packages_on_windows_root() {
    let dir = tempfile::tempdir().expect("temporary directory");
    put(
        dir.path(),
        "c/ProgramData/chocolatey/lib/git/git.nuspec",
        br#"<package><metadata><id>git</id><version>2.47.0</version><summary>Version control</summary></metadata></package>"#,
    );
    let stage = PackageList::new(env_for(&dir, false));
    let ctx = context_on(TestHost::windows(), None, None);
    let args = BoundArgs::new("package.list").with("manager", manager("choco"), false);
    let result = packages(stage.execute(&args, input(), &ctx).expect("list"));
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "git");
    assert_eq!(result[0].version, "2.47.0");
    assert_eq!(result[0].manager, PackageManager::Choco);
}

#[test]
fn search_exact_name_first_case_insensitive() {
    let dir = apt_root(STATUS, &[(LIST_FILE, LIST)]);
    let stage = PackageSearch::new(env_for(&dir, true));
    let ctx = context(None, None);
    let args = BoundArgs::new("package.search").with("query", text("CURL"), false);
    let result = packages(stage.execute(&args, input(), &ctx).expect("search"));
    let found: Vec<_> = result.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(found, ["curl", "curl-extra", "libcurl"]);
    assert!(result[0].installed);
    assert_eq!(result[0].version, "8.5.0-1");
    assert_eq!(result[0].available_version.as_deref(), Some("8.5.0-2"));
    assert!(!result[1].installed);
    assert_eq!(
        result[1].source.as_deref(),
        Some("deb.example.org/debian stable/main")
    );
}

#[test]
fn search_with_explicit_manager_and_unreadable_index_is_format_error() {
    let dir = apt_root(STATUS, &[]);
    put(
        dir.path(),
        "var/lib/apt/lists/h_dists_s_main_binary-amd64_Packages.gz",
        b"\x1f\x8b damaged",
    );
    let stage = PackageSearch::new(env_for(&dir, true));
    let ctx = context(None, None);
    let explicit = BoundArgs::new("package.search")
        .with("query", text("curl"), false)
        .with("manager", manager("apt"), false);
    let error = stage
        .execute(&explicit, input(), &ctx)
        .expect_err("unreadable index");
    assert_eq!(error.code(), ErrorCode::Format);
    let auto = BoundArgs::new("package.search").with("query", text("curl"), false);
    assert!(packages(stage.execute(&auto, input(), &ctx).expect("search")).is_empty());
}

#[test]
fn search_rejects_non_text_query_and_unknown_manager() {
    let dir = apt_root(STATUS, &[]);
    let stage = PackageSearch::new(env_for(&dir, true));
    let ctx = context(None, None);
    let number =
        BoundArgs::new("package.search").with("query", ArgValue::Value(Value::Bool(true)), false);
    assert_eq!(
        stage
            .execute(&number, input(), &ctx)
            .expect_err("type")
            .code(),
        ErrorCode::Type
    );
    let unknown = BoundArgs::new("package.search")
        .with("query", text("x"), false)
        .with("manager", manager("portage"), false);
    assert_eq!(
        stage
            .execute(&unknown, input(), &ctx)
            .expect_err("value")
            .code(),
        ErrorCode::InvalidValue
    );
}

#[test]
fn explicit_manager_without_markers_is_external_missing() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let stage = PackageList::new(env_for(&dir, true));
    let ctx = context(None, None);
    let args = BoundArgs::new("package.list").with("manager", manager("pacman"), false);
    let error = stage.execute(&args, input(), &ctx).expect_err("error");
    assert_eq!(error.code(), ErrorCode::ExternalMissing);
    let auto = BoundArgs::new("package.list");
    assert_eq!(
        stage
            .execute(&auto, input(), &ctx)
            .expect_err("error")
            .code(),
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
    let env = staged_env(&dir, vec![], vec![("tool", "2.0-1")]);
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
    assert_eq!(tx.markers[0].detail.as_deref(), Some("apt"));
    assert_eq!(outcome.changes[0].path.as_str(), "package:tool");
    assert_eq!(outcome.changes[0].before, None);
    assert_eq!(outcome.changes[0].after.as_deref(), Some("2.0-1"));
    assert_eq!(outcome.irreversible.len(), 1);
    let effects = records(outcome.output);
    assert_eq!(effects.len(), 1);
    let calls = process.calls.lock().expect("calls");
    assert_eq!(calls[0].program, "apt-get");
    assert!(calls[0].env.iter().any(|v| v.name == "DEBIAN_FRONTEND"));
}

#[test]
fn upgrade_without_names_targets_outdated_packages() {
    let dir = apt_root(STATUS, &[(LIST_FILE, LIST)]);
    let process = Arc::new(RecordingProcess::default());
    let ctx = context(Some(process.clone()), None);
    let stage = PackageChange::new(env_for(&dir, true), Action::Upgrade);
    let plan = stage
        .plan(&BoundArgs::new("package.upgrade"), input(), &ctx)
        .expect("plan");
    let targets: Vec<_> = plan
        .changes
        .iter()
        .map(|c| c.path.as_str().to_owned())
        .collect();
    assert_eq!(targets, ["package:curl", "package:vim"]);
    assert!(plan.bulk);
    assert_eq!(plan.changes[0].before.as_deref(), Some("8.5.0-1"));
    let mut tx = RecordingTx::default();
    stage
        .apply(&BoundArgs::new("package.upgrade"), plan, &mut tx, &ctx)
        .expect("apply");
    let calls = process.calls.lock().expect("calls");
    assert_eq!(calls[0].args, ["upgrade", "-y", "-q"]);
}

#[test]
fn several_managers_change_through_the_first_with_warning() {
    let dir = apt_root(STATUS, &[]);
    put(dir.path(), "var/lib/pacman/local/ALPM_DB_VERSION", b"9\n");
    let process = Arc::new(RecordingProcess::default());
    let ctx = context(Some(process.clone()), None);
    let stage = PackageChange::new(env_for(&dir, true), Action::Remove);
    let args = BoundArgs::new("package.remove").with("names", names(&["vim"]), false);
    let plan = stage.plan(&args, input(), &ctx).expect("plan");
    assert_eq!(plan.irreversible[0].detail.as_deref(), Some("apt"));
    assert_eq!(plan.changes[0].before.as_deref(), Some("9.0-1~rc1"));
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
    let error = stage
        .apply(&args, plan, &mut tx, &ctx)
        .expect_err("failure");
    assert_eq!(error.code(), ErrorCode::StageFailed);
    assert!(error.data().message.contains("Unable to locate package"));
    assert_eq!(tx.markers.len(), 1);
}

#[test]
fn failed_manager_run_reports_the_standard_output_when_the_standard_error_is_empty() {
    let call = Invocation {
        program: "winget".to_owned(),
        args: Vec::new(),
        env: Vec::new(),
    };
    let outcome = ProgramOutcome {
        exit_code: Some(1),
        stdout: b"No package found matching input criteria.\r\n".to_vec(),
        stderr: b"\r\n".to_vec(),
        ..ProgramOutcome::default()
    };
    let error = call_failure(&call, &outcome);
    assert_eq!(error.code(), ErrorCode::StageFailed);
    assert_eq!(
        error.data().message,
        "winget exited with code 1: No package found matching input criteria."
    );
}

#[test]
fn missing_program_at_apply_is_external_missing() {
    let dir = apt_root(STATUS, &[]);
    let process = Arc::new(RecordingProcess {
        missing: true,
        ..RecordingProcess::default()
    });
    let ctx = context(Some(process), None);
    let stage = PackageChange::new(env_for(&dir, true), Action::Install);
    let args = BoundArgs::new("package.install").with("names", names(&["curl"]), false);
    let plan = stage.plan(&args, input(), &ctx).expect("plan");
    let mut tx = RecordingTx::default();
    let error = stage
        .apply(&args, plan, &mut tx, &ctx)
        .expect_err("missing");
    assert_eq!(error.code(), ErrorCode::ExternalMissing);
    let no_process = context(None, None);
    let plan = stage.plan(&args, input(), &no_process).expect("plan");
    let error = stage
        .apply(&args, plan, &mut tx, &no_process)
        .expect_err("no process host");
    assert_eq!(error.code(), ErrorCode::ExternalMissing);
}

#[test]
fn names_must_be_text() {
    let dir = apt_root(STATUS, &[]);
    let stage = PackageChange::new(env_for(&dir, true), Action::Install);
    let ctx = context(None, None);
    let args = BoundArgs::new("package.install").with(
        "names",
        ArgValue::Value(Value::List(vec![Value::Bool(true)])),
        false,
    );
    assert_eq!(
        stage.plan(&args, input(), &ctx).expect_err("type").code(),
        ErrorCode::Type
    );
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
    let args = BoundArgs::new("package.install").with("manager", manager("apt"), false);
    let info = PackageChange::new(env.clone(), Action::Install)
        .check(&args, &check_ctx)
        .expect("info");
    assert_eq!(info.programs.len(), 1);
    assert_eq!(info.programs[0].program, "apt-get");
    let list = PackageList::new(env);
    assert!(
        list.check(&args, &check_ctx)
            .expect("info")
            .programs
            .is_empty()
    );
}

fn catalog() -> CatalogFile {
    toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../catalog/package.toml"
    )))
    .expect("package catalog")
}

#[test]
fn catalog_signatures_match_frozen() {
    let catalog = catalog();
    let all = Arc::new(PackageEnv::system());
    for stage in stages(&all) {
        let descriptor = stage.descriptor();
        let spec = catalog
            .command(&descriptor.command)
            .expect("command in catalog");
        assert!(spec.param("manager").is_some(), "{}", descriptor.command);
        assert_eq!(
            stage.as_read().is_some(),
            format!("{:?}", spec.effect) == "Read",
            "{}",
            descriptor.command
        );
        assert_eq!(
            stage.as_effect().is_some(),
            format!("{:?}", spec.effect) == "External",
            "{}",
            descriptor.command
        );
    }
    assert_eq!(catalog.commands.len(), 5);
    let search = catalog.command("package.search").expect("search");
    assert!(search.param("query").is_some_and(|p| p.required));
    let install = catalog.command("package.install").expect("install");
    assert!(install.param("names").is_some_and(|p| p.required));
    assert!(install.param("version").is_some());
    let upgrade = catalog.command("package.upgrade").expect("upgrade");
    assert!(upgrade.param("names").is_some_and(|p| !p.required));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
