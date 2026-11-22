// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the stages independent of the service manager, and test doubles
//! shared with the tests of the backends.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
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
use nurt_contracts::record::{
    Effect, Service, ServiceManager, ServiceScope, ServiceState, StartType,
};
use nurt_contracts::report::{IrreversibleEntry, Notable};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelReason, CancelToken, ExecCtx, InputKinds, StageImpl,
};
use nurt_contracts::value::{Duration as ValueDuration, Record, RecordKind, Regex, Value};

use super::{
    DEFAULT_WAIT, NameFilter, PARAM_NAME, PARAM_SCOPE, PARAM_STATE, PARAM_WAIT, SERVICES_COMMAND,
    ServiceAction, ServiceBackend, ServiceCommand, ServiceControl, ServiceFilter, ServiceQuery,
    ServiceStatus, ServiceTransition, list_services, perform, plan_actions, stages,
};

/// Clock that advances only when slept on; optionally cancels a token at
/// the given sleep, counted from 1.
pub(crate) struct VirtualClock {
    monotonic: Mutex<Duration>,
    sleeps: AtomicUsize,
    cancel_at: Mutex<Option<(usize, CancelToken)>>,
}

impl VirtualClock {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            monotonic: Mutex::new(Duration::ZERO),
            sleeps: AtomicUsize::new(0),
            cancel_at: Mutex::new(None),
        })
    }

    pub(crate) fn cancel_at_sleep(&self, sleep: usize, token: CancelToken) {
        *self.cancel_at.lock().unwrap() = Some((sleep, token));
    }

    pub(crate) fn elapsed(&self) -> Duration {
        *self.monotonic.lock().unwrap()
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> WallTime {
        WallTime::from_unix_nanos(1_700_000_000_000_000_000).saturating_add(self.elapsed())
    }

    fn monotonic(&self) -> Duration {
        self.elapsed()
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: self.now(),
            boot: BootId("test-boot".to_owned()),
            since_boot_with_suspend: self.elapsed(),
            since_boot_active: self.elapsed(),
        }
    }

    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> std::result::Result<(), CancelReason> {
        let count = self.sleeps.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some((at, token)) = self.cancel_at.lock().unwrap().as_ref()
            && *at == count
        {
            token.cancel(CancelReason::Requested);
        }
        if let Some(reason) = cancel.reason() {
            return Err(reason);
        }
        *self.monotonic.lock().unwrap() += duration;
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

fn host() -> Arc<TestHost> {
    Arc::new(TestHost {
        info: HostInfo {
            os: Os::Linux,
            os_version: "6.12".to_owned(),
            architecture: "x86_64".to_owned(),
            hostname: "test".to_owned(),
            user: "user".to_owned(),
            home: PathBuf::from("/home/user"),
            temp: PathBuf::from("/tmp"),
            path_style: PathStyle::Posix,
            timezone: "UTC".to_owned(),
            locale: None,
            cpus: 2,
        },
    })
}

/// Context on a virtual clock with the cancellation token `cancel`.
pub(crate) fn context(clock: Arc<VirtualClock>, cancel: CancelToken) -> ExecCtx {
    ExecCtx::builder(clock, host()).cancel(cancel).build()
}

/// Clock that waits in real time, for tests against a real service
/// manager.
#[cfg(target_os = "linux")]
struct SystemClock {
    origin: std::time::Instant,
}

#[cfg(target_os = "linux")]
impl Clock for SystemClock {
    fn now(&self) -> WallTime {
        WallTime::from_unix_nanos(1_700_000_000_000_000_000).saturating_add(self.monotonic())
    }

    fn monotonic(&self) -> Duration {
        self.origin.elapsed()
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: self.now(),
            boot: BootId("system".to_owned()),
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

/// Context on a clock that waits in real time.
#[cfg(target_os = "linux")]
pub(crate) fn real_context() -> ExecCtx {
    let clock = Arc::new(SystemClock {
        origin: std::time::Instant::now(),
    });
    ExecCtx::builder(clock, host()).build()
}

/// Transaction that records the irreversible markers into a log shared with
/// a backend, so that tests see the order of markers and calls.
pub(crate) struct RecordingTx {
    log: Arc<Mutex<Vec<String>>>,
    pub(crate) markers: Vec<IrreversibleEntry>,
}

impl RecordingTx {
    pub(crate) fn new(log: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            log,
            markers: Vec::new(),
        }
    }
}

fn not_expected(what: &str) -> Error {
    Error::new(ErrorCode::StageFailed, format!("{what} is not used by service actions"))
}

impl MutationTx for RecordingTx {
    fn op(&self) -> Option<OpId> {
        None
    }

    fn preserve(&mut self, _targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        Err(not_expected("preserve"))
    }

    fn stage(&mut self, _target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        Err(not_expected("stage"))
    }

    fn apply(&mut self, _target: TargetIndex, _mutation: Mutation) -> Result<Applied> {
        Err(not_expected("apply"))
    }

    fn observe_external(&mut self, _changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        Err(not_expected("observe_external"))
    }

    fn mark_irreversible(&mut self, marker: IrreversibleEntry) {
        let path = marker.path.clone().unwrap_or_default();
        self.log.lock().unwrap().push(format!("mark {path}"));
        self.markers.push(marker);
    }

    fn record_exempt(&mut self, _write: ExemptWrite) {}

    fn commit(self: Box<Self>) -> Result<OpHandle> {
        Err(not_expected("commit"))
    }

    fn rollback(self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
        Err(not_expected("rollback"))
    }
}

/// [`ServiceControl`] over a given backend, as the executor offers it to
/// `stop`.
pub(crate) struct BackendControl<'b>(pub(crate) &'b dyn ServiceBackend);

impl ServiceControl for BackendControl<'_> {
    fn start(
        &self,
        service: &Service,
        wait: Duration,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<ServiceTransition> {
        perform(self.0, ServiceAction::Start, service, wait, tx, ctx)
    }

    fn stop(
        &self,
        service: &Service,
        wait: Duration,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<ServiceTransition> {
        perform(self.0, ServiceAction::Stop, service, wait, tx, ctx)
    }

    fn restart(
        &self,
        service: &Service,
        wait: Duration,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<ServiceTransition> {
        perform(self.0, ServiceAction::Restart, service, wait, tx, ctx)
    }

    fn enable(
        &self,
        service: &Service,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<ServiceTransition> {
        perform(self.0, ServiceAction::Enable, service, Duration::ZERO, tx, ctx)
    }

    fn disable(
        &self,
        service: &Service,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<ServiceTransition> {
        perform(self.0, ServiceAction::Disable, service, Duration::ZERO, tx, ctx)
    }
}

/// Backend over services held in memory; actions take effect at once.
struct MemoryBackend {
    manager: ServiceManager,
    services: Mutex<Vec<Service>>,
    log: Arc<Mutex<Vec<String>>>,
}

impl MemoryBackend {
    fn new(services: Vec<Service>) -> Self {
        Self {
            manager: ServiceManager::Windows,
            services: Mutex::new(services),
            log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn find(&self, name: &str, scope: ServiceScope) -> Result<Service> {
        self.services
            .lock()
            .unwrap()
            .iter()
            .find(|service| {
                service.name.eq_ignore_ascii_case(name) && service.scope == Some(scope)
            })
            .cloned()
            .ok_or_else(|| super::service_not_found(self.manager, name))
    }

    fn update(&self, action: &str, name: &str, change: impl Fn(&mut Service)) -> Result<()> {
        self.log.lock().unwrap().push(format!("{action} {name}"));
        let mut services = self.services.lock().unwrap();
        let service = services
            .iter_mut()
            .find(|service| service.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| super::service_not_found(self.manager, name))?;
        change(service);
        Ok(())
    }
}

impl ServiceBackend for MemoryBackend {
    fn manager(&self) -> ServiceManager {
        self.manager
    }

    fn list(&self, query: &ServiceQuery) -> Result<Vec<Service>> {
        Ok(self
            .services
            .lock()
            .unwrap()
            .iter()
            .filter(|service| service.scope == Some(query.scope))
            .cloned()
            .collect())
    }

    fn get(&self, name: &str, scope: ServiceScope) -> Result<Service> {
        self.find(name, scope)
    }

    fn status(&self, name: &str, scope: ServiceScope) -> Result<ServiceStatus> {
        let service = self.find(name, scope)?;
        Ok(ServiceStatus {
            state: service.state,
            state_raw: service.state_raw,
            result: None,
        })
    }

    fn start(&self, name: &str, _scope: ServiceScope) -> Result<()> {
        self.update("start", name, |service| service.state = ServiceState::Running)
    }

    fn stop(&self, name: &str, _scope: ServiceScope) -> Result<()> {
        self.update("stop", name, |service| service.state = ServiceState::Stopped)
    }

    fn restart(&self, name: &str, _scope: ServiceScope) -> Result<()> {
        self.update("restart", name, |service| service.state = ServiceState::Running)
    }

    fn enable(&self, name: &str, _scope: ServiceScope) -> Result<()> {
        self.update("enable", name, |service| {
            service.start_type = StartType::Automatic;
        })
    }

    fn disable(&self, name: &str, _scope: ServiceScope) -> Result<()> {
        self.update("disable", name, |service| {
            service.start_type = StartType::Disabled;
        })
    }
}

fn service(name: &str, state: ServiceState, start_type: StartType, raw: &str) -> Service {
    Service {
        name: name.to_owned(),
        state,
        start_type,
        manager: ServiceManager::Windows,
        state_raw: Some(raw.to_owned()),
        scope: Some(ServiceScope::System),
        ..Service::default()
    }
}

fn sample_services() -> Vec<Service> {
    vec![
        service("Spooler", ServiceState::Running, StartType::Automatic, "RUNNING"),
        service("W32Time", ServiceState::Stopped, StartType::Manual, "STOPPED"),
        service("WSearch", ServiceState::Running, StartType::Disabled, "RUNNING"),
    ]
}

fn names(records: &[Record]) -> Vec<String> {
    records
        .iter()
        .map(|record| match record.get("name") {
            Some(Value::Text(name)) => name.clone(),
            other => panic!("record without a name: {other:?}"),
        })
        .collect()
}

fn filter(args: &BoundArgs) -> ServiceFilter {
    ServiceFilter::from_args(args).unwrap()
}

fn text_arg(command: &str, name: &str, value: &str) -> BoundArgs {
    BoundArgs::new(command).with(name, ArgValue::Value(Value::Text(value.to_owned())), false)
}

#[test]
fn services_exact_name_uses_the_manager_resolution() {
    let backend = MemoryBackend::new(sample_services());
    let records =
        list_services(&backend, &filter(&text_arg(SERVICES_COMMAND, PARAM_NAME, "spooler")))
            .unwrap();
    assert_eq!(names(&records), ["Spooler"]);
    assert_eq!(records[0].kind, RecordKind::Service);
}

#[test]
fn services_unknown_exact_name_is_not_found() {
    let backend = MemoryBackend::new(sample_services());
    let error = list_services(&backend, &filter(&text_arg(SERVICES_COMMAND, PARAM_NAME, "nope")))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotFound);
    assert_eq!(error.fix, [SERVICES_COMMAND]);
}

#[test]
fn services_state_matches_enumeration_and_raw_state_ignoring_case() {
    let backend = MemoryBackend::new(sample_services());
    let by_state = list_services(&backend, &filter(&text_arg(SERVICES_COMMAND, PARAM_STATE, "Running")))
        .unwrap();
    assert_eq!(names(&by_state), ["Spooler", "WSearch"]);
    let by_raw = list_services(&backend, &filter(&text_arg(SERVICES_COMMAND, PARAM_STATE, "stopped")))
        .unwrap();
    assert_eq!(names(&by_raw), ["W32Time"]);
}

#[test]
fn services_regex_honours_flags() {
    let backend = MemoryBackend::new(sample_services());
    let mut pattern = Regex {
        pattern: "^w".to_owned(),
        flags: Default::default(),
    };
    pattern.flags.case_insensitive = true;
    let args = BoundArgs::new(SERVICES_COMMAND).with(
        PARAM_NAME,
        ArgValue::Value(Value::Regex(pattern)),
        false,
    );
    let records = list_services(&backend, &filter(&args)).unwrap();
    assert_eq!(names(&records), ["W32Time", "WSearch"]);
}

#[test]
fn services_rejects_invalid_arguments() {
    let invalid_regex = BoundArgs::new(SERVICES_COMMAND).with(
        PARAM_NAME,
        ArgValue::Value(Value::Regex(Regex {
            pattern: "(".to_owned(),
            flags: Default::default(),
        })),
        false,
    );
    let cases = [
        (invalid_regex, ErrorCode::Regex),
        (text_arg(SERVICES_COMMAND, PARAM_SCOPE, "machine"), ErrorCode::InvalidValue),
        (
            BoundArgs::new(SERVICES_COMMAND).with(PARAM_NAME, ArgValue::Value(Value::Int(1)), false),
            ErrorCode::Type,
        ),
        (
            BoundArgs::new(SERVICES_COMMAND).with(PARAM_STATE, ArgValue::Value(Value::Int(1)), false),
            ErrorCode::Type,
        ),
        (
            BoundArgs::new(SERVICES_COMMAND).with(PARAM_SCOPE, ArgValue::Value(Value::Int(1)), false),
            ErrorCode::Type,
        ),
    ];
    for (args, code) in cases {
        assert_eq!(ServiceFilter::from_args(&args).unwrap_err().code(), code);
    }
}

#[test]
fn services_defaults_to_every_system_service() {
    let parsed = filter(&BoundArgs::new(SERVICES_COMMAND));
    assert!(matches!(parsed.name, NameFilter::Any));
    assert_eq!(parsed.scope, ServiceScope::System);
    assert_eq!(parsed.state, None);
}

#[test]
fn wait_defaults_and_rejects_negative_values() {
    let none = BoundArgs::new("start");
    assert_eq!(ServiceCommand::START.wait(&none).unwrap(), DEFAULT_WAIT);
    let given = BoundArgs::new("start").with(
        PARAM_WAIT,
        ArgValue::Value(Value::Duration(ValueDuration::from_millis(1500))),
        false,
    );
    assert_eq!(
        ServiceCommand::RESTART.wait(&given).unwrap(),
        Duration::from_millis(1500)
    );
    let negative = BoundArgs::new("start").with(
        PARAM_WAIT,
        ArgValue::Value(Value::Duration(ValueDuration::from_millis(-1))),
        false,
    );
    assert_eq!(
        ServiceCommand::START.wait(&negative).unwrap_err().code(),
        ErrorCode::InvalidValue
    );
    assert_eq!(ServiceCommand::ENABLE.wait(&negative).unwrap(), Duration::ZERO);
}

#[test]
fn plan_previews_changes_and_marks_unchanged_services() {
    let backend = MemoryBackend::new(sample_services());
    let ctx = context(VirtualClock::new(), CancelToken::new());
    let plan = plan_actions(&backend, ServiceAction::Start, sample_services(), &ctx).unwrap();
    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.changes[0].path, "service:W32Time");
    assert_eq!(plan.changes[0].after.as_deref(), Some("running"));
    assert_eq!(plan.irreversible.len(), 1);
    assert_eq!(plan.notable.len(), 2);
    assert!(plan.notable.iter().all(|note| note.code == Notable::Unchanged));
    assert_eq!(plan.preview.len(), 3);
    let preview = Effect::try_from(plan.preview[1].clone()).unwrap();
    assert_eq!(preview.target.as_deref(), Some("service:W32Time"));
    assert!(backend.log.lock().unwrap().is_empty());

    let empty = plan_actions(&backend, ServiceAction::Start, Vec::new(), &ctx).unwrap();
    assert_eq!(empty.notable[0].code, Notable::NothingMatched);
}

#[test]
fn record_of_another_manager_is_rejected_before_any_call() {
    let backend = MemoryBackend::new(sample_services());
    let ctx = context(VirtualClock::new(), CancelToken::new());
    let mut foreign = sample_services().remove(1);
    foreign.manager = ServiceManager::Systemd;
    let mut tx = RecordingTx::new(backend.log.clone());
    let error = BackendControl(&backend)
        .start(&foreign, DEFAULT_WAIT, &mut tx, &ctx)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Unsupported);
    assert!(tx.markers.is_empty());
    assert!(backend.log.lock().unwrap().is_empty());
}

#[test]
fn service_control_enable_and_disable_report_start_type() {
    let backend = MemoryBackend::new(sample_services());
    let ctx = context(VirtualClock::new(), CancelToken::new());
    let mut tx = RecordingTx::new(backend.log.clone());
    let time = sample_services().remove(1);
    let enabled = BackendControl(&backend).enable(&time, &mut tx, &ctx).unwrap();
    assert_eq!((enabled.before.as_str(), enabled.after.as_str()), ("manual", "automatic"));
    let disabled = BackendControl(&backend).disable(&time, &mut tx, &ctx).unwrap();
    assert_eq!(disabled.after, "disabled");
    let again = BackendControl(&backend).disable(&time, &mut tx, &ctx).unwrap();
    assert!(!again.changed);
    assert_eq!(
        *backend.log.lock().unwrap(),
        [
            "mark service:W32Time",
            "enable W32Time",
            "mark service:W32Time",
            "disable W32Time"
        ]
    );
}

#[test]
fn transition_converts_to_change_and_effect() {
    let transition = ServiceTransition {
        name: "nginx".to_owned(),
        action: ServiceAction::Stop,
        before: "running".to_owned(),
        after: "stopped".to_owned(),
        changed: true,
    };
    let change = transition.to_change();
    assert_eq!(change.path, "service:nginx");
    assert_eq!(change.before.as_deref(), Some("running"));
    let effect = Effect::try_from(transition.to_effect()).unwrap();
    assert_eq!(effect.kind, "stop");
    assert_eq!(effect.after, Some(Value::Enum("stopped".to_owned())));
    assert_eq!(effect.irreversible.unwrap()[0].reason.as_str(), "service_changed");
    assert_eq!(effect.notable, None);
}

#[test]
fn stages_cover_the_catalog_commands() {
    let stages = stages();
    let commands: Vec<String> = stages.iter().map(|stage| stage.descriptor().command).collect();
    assert_eq!(commands, ["services", "start", "restart", "enable", "disable"]);
    assert!(stages[0].as_read().is_some());
    for stage in &stages[1..] {
        assert!(stage.as_effect().is_some());
        assert!(stage.descriptor().input.accepts(RecordKind::Service));
    }
    assert_eq!(stages[0].descriptor().input, InputKinds::None);
    let actions: Vec<&str> = ServiceCommand::ALL
        .iter()
        .map(|command| command.action().as_str())
        .collect();
    assert_eq!(actions, ["start", "restart", "enable", "disable"]);
}

/// One parameter of a command as declared in the catalog: name, type,
/// default and position.
type Param = (&'static str, &'static str, Option<&'static str>, Option<i64>);

#[test]
fn catalog_signatures_match_frozen() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../catalog/admin.toml");
    let catalog: toml::Table = std::fs::read_to_string(path).unwrap().parse().unwrap();
    let wait: &[Param] = &[(PARAM_WAIT, "duration", Some("10s"), None)];
    let expected: [(&str, &[Param], &str, &str); 5] = [
        (
            SERVICES_COMMAND,
            &[
                (PARAM_NAME, "text | regex", None, Some(1)),
                (PARAM_STATE, "text", None, None),
                (PARAM_SCOPE, "enum(system|user)", Some("system"), None),
            ],
            "none",
            "service",
        ),
        ("start", wait, "service", "effect"),
        ("restart", wait, "service", "effect"),
        ("enable", &[], "service", "effect"),
        ("disable", &[], "service", "effect"),
    ];
    let commands = catalog["command"].as_array().unwrap();
    for (name, params, input, output) in expected {
        let command = commands
            .iter()
            .find(|command| command["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("`{name}` is missing from the catalog"));
        assert_eq!(command["input"].as_array().unwrap()[0].as_str(), Some(input));
        assert_eq!(command["output"].as_array().unwrap()[0].as_str(), Some(output));
        let declared: Vec<(String, String, Option<String>, Option<i64>)> = command
            .get("param")
            .and_then(toml::Value::as_array)
            .map(|params| {
                params
                    .iter()
                    .map(|param| {
                        (
                            param["name"].as_str().unwrap().to_owned(),
                            param["type"].as_str().unwrap().to_owned(),
                            param
                                .get("default")
                                .and_then(toml::Value::as_str)
                                .map(str::to_owned),
                            param.get("position").and_then(toml::Value::as_integer),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let expected: Vec<(String, String, Option<String>, Option<i64>)> = params
            .iter()
            .map(|(name, kind, default, position)| {
                (
                    (*name).to_owned(),
                    (*kind).to_owned(),
                    default.map(str::to_owned),
                    *position,
                )
            })
            .collect();
        assert_eq!(declared, expected, "parameters of `{name}`");
    }
    assert_eq!(DEFAULT_WAIT, Duration::from_secs(10));
}

#[test]
fn already_holds_never_skips_restart() {
    assert!(ServiceAction::Start.already_holds("running"));
    assert!(ServiceAction::Stop.already_holds("stopped"));
    assert!(!ServiceAction::Restart.already_holds("running"));
    assert!(ServiceAction::Enable.already_holds("automatic"));
    assert!(ServiceAction::Disable.already_holds("disabled"));
    let all: HashMap<&str, ServiceAction> = [
        ServiceAction::Start,
        ServiceAction::Stop,
        ServiceAction::Restart,
        ServiceAction::Enable,
        ServiceAction::Disable,
    ]
    .into_iter()
    .map(|action| (action.as_str(), action))
    .collect();
    assert_eq!(all.len(), 5);
}
