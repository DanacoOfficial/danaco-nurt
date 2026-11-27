// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::services::linux::tests
// Responsibility:   Tests of the systemd backend against a manager served in the process.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests of the systemd backend against a manager served in the process,
//! and tests against the systemd of the running system.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::{
    Effect, Service, ServiceManager, ServiceScope, ServiceState, StartType,
};
use nurt_contracts::report::{Irreversible, Notable};
use nurt_contracts::stage::{ArgValue, BoundArgs, CancelToken, ExecCtx, StageData};
use nurt_contracts::value::{Record, Regex, Value};

use super::mock::{MockUnit, Plan, Served, serve};
use super::{
    OS_CODE_DBUS, SYSTEMD_RUNTIME_DIR, SystemdBackend, connect, map_active_state, map_dbus_error,
    map_unit_file_state, unit_name,
};
use crate::services::tests::{BackendControl, RecordingTx, VirtualClock, context};
use crate::services::{
    ServiceAction, ServiceBackend, ServiceControl, ServiceFilter, ServiceQuery, apply_actions,
    list_services, perform,
};

const WAIT: Duration = Duration::from_secs(10);

fn output_records(data: StageData<'_>) -> Vec<Record> {
    match data {
        StageData::Records(stream) => stream.collect::<Result<Vec<_>, _>>().unwrap(),
        _ => panic!("stage output is not a record stream"),
    }
}

struct Fixture {
    backend: SystemdBackend,
    served: Served,
    clock: Arc<VirtualClock>,
    cancel: CancelToken,
    ctx: ExecCtx,
}

impl Fixture {
    fn new(units: Vec<MockUnit>) -> Self {
        let served = serve(units);
        let backend = SystemdBackend::with_connections(Some(served.client.clone()), None);
        let clock = VirtualClock::new();
        let cancel = CancelToken::new();
        let ctx = context(clock.clone(), cancel.clone());
        Self {
            backend,
            served,
            clock,
            cancel,
            ctx,
        }
    }

    fn calls(&self) -> Arc<Mutex<Vec<String>>> {
        self.served.state.lock().unwrap().calls.clone()
    }

    fn tx(&self) -> RecordingTx {
        RecordingTx::new(self.calls())
    }

    fn service(&self, name: &str) -> Service {
        self.backend.get(name, ServiceScope::System).unwrap()
    }

    fn called(&self, call: &str) -> bool {
        self.calls()
            .lock()
            .unwrap()
            .iter()
            .any(|entry| entry == call)
    }
}

fn host_units() -> Vec<MockUnit> {
    vec![
        MockUnit::new("nginx", "active", "running"),
        MockUnit::new("cron", "active", "running").with_file_state(None),
        MockUnit::new("backup", "inactive", "dead")
            .with_file_state(Some("disabled"))
            .not_loaded(),
        MockUnit::new("getty@", "inactive", "dead")
            .with_file_state(Some("static"))
            .not_loaded(),
        MockUnit::new("getty@tty1", "active", "running").with_file_state(None),
        MockUnit::new("ghost", "inactive", "dead")
            .with_file_state(None)
            .not_loaded(),
    ]
}

fn listed(backend: &SystemdBackend) -> Vec<Service> {
    backend
        .list(&ServiceQuery {
            scope: ServiceScope::System,
        })
        .unwrap()
}

#[test]
fn systemd_list_merges_units_and_unit_files() {
    let fixture = Fixture::new(host_units());
    let services = listed(&fixture.backend);
    let names: Vec<&str> = services
        .iter()
        .map(|service| service.name.as_str())
        .collect();
    assert_eq!(names, ["backup", "cron", "getty@tty1", "nginx"]);
    let backup = &services[0];
    assert_eq!(backup.state, ServiceState::Stopped);
    assert_eq!(backup.start_type, StartType::Disabled);
    assert_eq!(backup.state_raw.as_deref(), Some("inactive/dead"));
    assert_eq!(
        backup.path.as_ref().map(|path| path.as_str()),
        Some("/etc/systemd/system/backup.service")
    );
    let cron = &services[1];
    assert_eq!(cron.start_type, StartType::Manual);
    assert_eq!(cron.path, None);
    let nginx = &services[3];
    assert_eq!(nginx.state, ServiceState::Running);
    assert_eq!(nginx.start_type, StartType::Automatic);
    assert_eq!(nginx.manager, ServiceManager::Systemd);
    assert_eq!(nginx.scope, Some(ServiceScope::System));
    assert_eq!(nginx.description.as_deref(), Some("nginx daemon"));
    assert!(fixture.called("ListUnitsByPatterns *.service"));
    assert!(fixture.called("ListUnitFilesByPatterns *.service"));
}

#[test]
fn systemd_list_falls_back_to_methods_without_patterns() {
    let fixture = Fixture::new(host_units());
    fixture.served.state.lock().unwrap().legacy = true;
    let services = listed(&fixture.backend);
    assert_eq!(services.len(), 4);
    assert!(fixture.called("ListUnits"));
    assert!(fixture.called("ListUnitFiles"));
}

#[test]
fn systemd_name_suffix_and_template_instance() {
    let fixture = Fixture::new(host_units());
    let short = fixture.service("nginx");
    let long = fixture.service("nginx.service");
    assert_eq!(short, long);
    assert_eq!(short.name, "nginx");
    let instance = fixture.service("getty@tty1");
    assert_eq!(instance.name, "getty@tty1");
    assert_eq!(instance.state, ServiceState::Running);
    assert_eq!(unit_name("getty@tty1"), "getty@tty1.service");
    assert_eq!(unit_name("nginx.service"), "nginx.service");
    let loaded_on_demand = fixture.service("backup");
    assert_eq!(loaded_on_demand.start_type, StartType::Disabled);
}

#[test]
fn systemd_empty_name_is_not_found_without_a_call_to_the_manager() {
    let fixture = Fixture::new(host_units());
    for name in ["", ".service"] {
        let scope = ServiceScope::System;
        let errors = [
            fixture.backend.get(name, scope).map(|_| ()),
            fixture.backend.status(name, scope).map(|_| ()),
            fixture.backend.start(name, scope),
            fixture.backend.stop(name, scope),
            fixture.backend.restart(name, scope),
            fixture.backend.enable(name, scope),
            fixture.backend.disable(name, scope),
        ];
        for result in errors {
            assert_eq!(result.unwrap_err().code, ErrorCode::NotFound, "{name:?}");
        }
    }
    assert!(fixture.calls().lock().unwrap().is_empty());
}

#[test]
fn systemd_unit_names_with_escapes_and_non_ascii_text_are_passed_unchanged() {
    for escaped in [
        r"systemd-cryptsetup@dev-disk-by\x2duuid-1",
        r"home-caf\xc3\xa9",
        "caf\u{e9}-\u{65e5}\u{672c}",
    ] {
        let unit = format!("{escaped}.service");
        assert_eq!(unit_name(escaped), unit);
        assert_eq!(unit_name(&unit), unit);
        let fixture = Fixture::new(vec![MockUnit::new(escaped, "active", "running")]);
        let service = fixture.service(escaped);
        assert_eq!(service.name, escaped);
        assert_eq!(service.state, ServiceState::Running);
        assert_eq!(fixture.service(&unit), service);
    }
}

#[test]
fn systemd_failure_without_a_dbus_error_name_is_a_service_error() {
    let failure = zbus::Error::Failure("the connection was closed".to_owned());
    let error = map_dbus_error(&failure, "nginx.service");
    assert_eq!(error.code(), ErrorCode::Service);
    assert_eq!(error.details["os_code"], OS_CODE_DBUS);
    assert_eq!(error.details["manager"], "systemd");
}

#[test]
fn systemd_get_reads_service_properties() {
    let mut nginx = MockUnit::new("nginx", "active", "running");
    nginx.main_pid = 812;
    nginx.user = "www-data".to_owned();
    let fixture = Fixture::new(vec![nginx]);
    let service = fixture.service("nginx");
    assert_eq!(service.pid, Some(812));
    assert_eq!(service.account.as_deref(), Some("www-data"));
    let os_raw = service.os_raw.unwrap();
    assert_eq!(
        os_raw.get("result"),
        Some(&Value::Text("success".to_owned()))
    );
    assert_eq!(os_raw.get("exec_main_status"), Some(&Value::Int(0)));
    assert_eq!(
        os_raw.get("load_state"),
        Some(&Value::Text("loaded".to_owned()))
    );
}

#[test]
fn systemd_active_state_mapping_table() {
    let table = [
        ("active", "running", ServiceState::Running),
        ("active", "exited", ServiceState::Stopped),
        ("reloading", "reload", ServiceState::Running),
        ("inactive", "dead", ServiceState::Stopped),
        ("activating", "start", ServiceState::Starting),
        ("deactivating", "stop-sigterm", ServiceState::Stopping),
        ("failed", "failed", ServiceState::Failed),
        ("maintenance", "clean", ServiceState::Unknown),
        ("refreshing", "running", ServiceState::Unknown),
        ("", "", ServiceState::Unknown),
    ];
    for (active, sub, expected) in table {
        assert_eq!(map_active_state(active, sub), expected, "{active}/{sub}");
    }
}

#[test]
fn systemd_unit_file_state_mapping_table() {
    let table = [
        (Some("enabled"), StartType::Automatic),
        (Some("enabled-runtime"), StartType::Automatic),
        (Some("disabled"), StartType::Disabled),
        (Some("masked"), StartType::Disabled),
        (Some("masked-runtime"), StartType::Disabled),
        (Some("static"), StartType::Static),
        (Some("indirect"), StartType::OnDemand),
        (Some("linked"), StartType::Unknown),
        (Some("linked-runtime"), StartType::Unknown),
        (Some("generated"), StartType::Unknown),
        (Some("transient"), StartType::Unknown),
        (Some("alias"), StartType::Unknown),
        (Some("bad"), StartType::Unknown),
        (Some(""), StartType::Manual),
        (None, StartType::Manual),
    ];
    for (state, expected) in table {
        assert_eq!(map_unit_file_state(state), expected, "{state:?}");
    }
}

fn stopped_nginx(plan: Plan) -> Vec<MockUnit> {
    vec![MockUnit::new("nginx", "inactive", "dead").with_start(plan)]
}

#[test]
fn start_waits_until_running_virtual_clock() {
    let fixture = Fixture::new(stopped_nginx(Plan::running_after(3)));
    let mut tx = fixture.tx();
    let nginx = fixture.service("nginx");
    let transition = perform(
        &fixture.backend,
        ServiceAction::Start,
        &nginx,
        WAIT,
        &mut tx,
        &fixture.ctx,
    )
    .unwrap();
    assert_eq!(transition.before, "stopped");
    assert_eq!(transition.after, "running");
    assert!(transition.changed);
    assert_eq!(fixture.clock.elapsed(), Duration::from_millis(100 + 200));
    assert!(fixture.called("StartUnit nginx.service"));
}

#[test]
fn start_wait_zero_returns_starting() {
    let fixture = Fixture::new(stopped_nginx(Plan::running_after(3)));
    let mut tx = fixture.tx();
    let nginx = fixture.service("nginx");
    let transition = perform(
        &fixture.backend,
        ServiceAction::Start,
        &nginx,
        Duration::ZERO,
        &mut tx,
        &fixture.ctx,
    )
    .unwrap();
    assert_eq!(transition.after, "starting");
    assert_eq!(fixture.clock.elapsed(), Duration::ZERO);
}

#[test]
fn start_timeout_reports_actual_state_and_effect() {
    let fixture = Fixture::new(stopped_nginx(Plan::running_after(u32::MAX)));
    let mut tx = fixture.tx();
    let nginx = fixture.service("nginx");
    let error = perform(
        &fixture.backend,
        ServiceAction::Start,
        &nginx,
        Duration::from_secs(1),
        &mut tx,
        &fixture.ctx,
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Timeout);
    assert_eq!(error.actual, Some(serde_json::json!("starting")));
    assert_eq!(error.expected, Some(serde_json::json!("running")));
    assert!(error.effect_applied);
    assert_eq!(error.details["state_raw"], "activating/start");
    assert_eq!(fixture.clock.elapsed(), Duration::from_secs(1));
    assert_eq!(tx.markers.len(), 1);
    assert_eq!(tx.markers[0].reason, Irreversible::ServiceChanged);
}

#[test]
fn start_failed_unit_service_error_with_logs_hint() {
    let fixture = Fixture::new(stopped_nginx(Plan::failed_after(2)));
    let mut tx = fixture.tx();
    let nginx = fixture.service("nginx");
    let error = perform(
        &fixture.backend,
        ServiceAction::Start,
        &nginx,
        WAIT,
        &mut tx,
        &fixture.ctx,
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Service);
    assert_eq!(error.details["result"], "exit-code");
    assert_eq!(error.details["os_code"], "exit-code");
    assert_eq!(error.details["manager"], "systemd");
    assert_eq!(error.next_steps, ["logs unit: \"nginx\""]);
    assert!(error.effect_applied);
}

#[test]
fn start_already_running_no_call_no_change() {
    let fixture = Fixture::new(vec![MockUnit::new("nginx", "active", "running")]);
    let mut tx = fixture.tx();
    let nginx = fixture.service("nginx");
    let outcome = apply_actions(
        &fixture.backend,
        ServiceAction::Start,
        std::slice::from_ref(&nginx),
        WAIT,
        &mut tx,
        &fixture.ctx,
    )
    .unwrap();
    assert!(!fixture.called("StartUnit nginx.service"));
    assert!(tx.markers.is_empty());
    assert!(outcome.changes.is_empty());
    assert_eq!(outcome.notable[0].code, Notable::Unchanged);
    let records = output_records(outcome.output.data);
    let effect = Effect::try_from(records[0].clone()).unwrap();
    assert_eq!(effect.before, effect.after);
    assert_eq!(effect.notable, Some(vec!["unchanged".to_owned()]));
}

#[test]
fn restart_effect_before_after() {
    let fixture = Fixture::new(vec![
        MockUnit::new("nginx", "active", "running").with_start(Plan::running_after(2)),
    ]);
    let mut tx = fixture.tx();
    let nginx = fixture.service("nginx");
    let outcome = apply_actions(
        &fixture.backend,
        ServiceAction::Restart,
        std::slice::from_ref(&nginx),
        WAIT,
        &mut tx,
        &fixture.ctx,
    )
    .unwrap();
    assert!(fixture.called("RestartUnit nginx.service"));
    assert_eq!(outcome.changes.len(), 1);
    assert_eq!(outcome.changes[0].path.as_str(), "service:nginx");
    let records = output_records(outcome.output.data);
    let effect = Effect::try_from(records[0].clone()).unwrap();
    assert_eq!(effect.kind, "restart");
    assert_eq!(effect.target.as_deref(), Some("service:nginx"));
    assert_eq!(effect.action.as_deref(), Some("restart"));
    assert_eq!(effect.before, Some(Value::Enum("running".to_owned())));
    assert_eq!(effect.after, Some(Value::Enum("running".to_owned())));
}

#[test]
fn enable_disable_call_reload_and_report_start_type() {
    let fixture = Fixture::new(host_units());
    let mut tx = fixture.tx();
    let backup = fixture.service("backup");
    let control = BackendControl(&fixture.backend);
    let enabled = control.enable(&backup, &mut tx, &fixture.ctx).unwrap();
    assert_eq!(
        (enabled.before.as_str(), enabled.after.as_str()),
        ("disabled", "automatic")
    );
    let disabled = control.disable(&backup, &mut tx, &fixture.ctx).unwrap();
    assert_eq!(disabled.after, "disabled");
    let calls = fixture.calls();
    let calls = calls.lock().unwrap();
    let changing: Vec<&str> = calls
        .iter()
        .map(String::as_str)
        .filter(|call| !call.starts_with("GetUnit") && !call.starts_with("LoadUnit"))
        .collect();
    assert_eq!(
        changing,
        [
            "mark service:backup",
            "EnableUnitFiles backup.service",
            "Reload",
            "mark service:backup",
            "DisableUnitFiles backup.service",
            "Reload"
        ]
    );
}

#[test]
fn access_denied_maps_to_privilege_root() {
    let fixture = Fixture::new(stopped_nginx(Plan::running_after(1)));
    fixture.served.state.lock().unwrap().deny = true;
    let mut tx = fixture.tx();
    let nginx = fixture.service("nginx");
    let error = perform(
        &fixture.backend,
        ServiceAction::Start,
        &nginx,
        WAIT,
        &mut tx,
        &fixture.ctx,
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Privilege);
    assert_eq!(error.details["required"], "root");
    assert_eq!(
        error.problem.as_deref(),
        Some("org.freedesktop.DBus.Error.AccessDenied")
    );
    assert!(!error.effect_applied);
}

#[test]
fn no_such_unit_maps_to_not_found() {
    let fixture = Fixture::new(host_units());
    let error = fixture
        .backend
        .get("absent", ServiceScope::System)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotFound);
    assert_eq!(error.details["unit"], "absent");
    let status = fixture
        .backend
        .status("ghost", ServiceScope::System)
        .unwrap();
    assert_eq!(status.state, ServiceState::Stopped);
    let start = fixture
        .backend
        .start("absent", ServiceScope::System)
        .unwrap_err();
    assert_eq!(start.code(), ErrorCode::NotFound);
}

#[test]
fn cancel_during_wait_returns_cancelled() {
    let fixture = Fixture::new(stopped_nginx(Plan::running_after(u32::MAX)));
    fixture.clock.cancel_at_sleep(2, fixture.cancel.clone());
    let mut tx = fixture.tx();
    let nginx = fixture.service("nginx");
    let error = perform(
        &fixture.backend,
        ServiceAction::Start,
        &nginx,
        WAIT,
        &mut tx,
        &fixture.ctx,
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Cancelled);
    assert!(error.effect_applied);
    assert_eq!(fixture.clock.elapsed(), Duration::from_millis(100));
}

#[test]
fn service_control_stop_used_by_core_stop() {
    let fixture = Fixture::new(vec![MockUnit::new("nginx", "active", "running")]);
    let mut tx = fixture.tx();
    let nginx = fixture.service("nginx");
    let control: &dyn ServiceControl = &BackendControl(&fixture.backend);
    let stopped = control.stop(&nginx, WAIT, &mut tx, &fixture.ctx).unwrap();
    assert_eq!(
        (stopped.before.as_str(), stopped.after.as_str()),
        ("running", "stopped")
    );
    assert!(fixture.called("StopUnit nginx.service"));
    let again = control.stop(&nginx, WAIT, &mut tx, &fixture.ctx).unwrap();
    assert!(!again.changed);
    assert_eq!(tx.markers.len(), 1);
}

#[test]
fn services_filter_regex_state_scope_user() {
    let system = serve(host_units());
    let user = serve(vec![
        MockUnit::new("syncthing", "active", "running"),
        MockUnit::new("sync-mail", "failed", "failed"),
        MockUnit::new("pipewire", "active", "running"),
    ]);
    let backend =
        SystemdBackend::with_connections(Some(system.client.clone()), Some(user.client.clone()));
    let args = BoundArgs::new("services")
        .with(
            "name",
            ArgValue::Value(Value::Regex(Regex {
                pattern: "^sync".to_owned(),
                flags: Default::default(),
            })),
            false,
        )
        .with(
            "state",
            ArgValue::Value(Value::Text("running".to_owned())),
            false,
        )
        .with(
            "scope",
            ArgValue::Value(Value::Enum("user".to_owned())),
            false,
        );
    let filter = ServiceFilter::from_args(&args).unwrap();
    let records = list_services(&backend, &filter).unwrap();
    assert_eq!(records.len(), 1);
    let service = Service::try_from(records[0].clone()).unwrap();
    assert_eq!(service.name, "syncthing");
    assert_eq!(service.scope, Some(ServiceScope::User));
    let failed = ServiceFilter::from_args(&args.with(
        "state",
        ArgValue::Value(Value::Text("FAILED".to_owned())),
        false,
    ))
    .unwrap();
    let records = list_services(&backend, &failed).unwrap();
    assert_eq!(
        Service::try_from(records[0].clone()).unwrap().name,
        "sync-mail"
    );
}

#[test]
fn mutation_tx_irreversible_service_state_set_before_apply() {
    let fixture = Fixture::new(stopped_nginx(Plan::running_after(1)));
    let mut tx = fixture.tx();
    let nginx = fixture.service("nginx");
    perform(
        &fixture.backend,
        ServiceAction::Start,
        &nginx,
        WAIT,
        &mut tx,
        &fixture.ctx,
    )
    .unwrap();
    let calls = fixture.calls();
    let calls = calls.lock().unwrap();
    let mark = calls
        .iter()
        .position(|call| call == "mark service:nginx")
        .unwrap();
    let start = calls
        .iter()
        .position(|call| call == "StartUnit nginx.service")
        .unwrap();
    assert!(mark < start);
    assert_eq!(tx.markers[0].path.as_deref(), Some("service:nginx"));
    assert_eq!(tx.markers[0].detail.as_deref(), Some("start"));
}

#[test]
fn missing_manager_is_reported_as_service_error() {
    let backend = SystemdBackend::new();
    if std::path::Path::new(SYSTEMD_RUNTIME_DIR).is_dir() {
        return;
    }
    let error = backend
        .list(&ServiceQuery {
            scope: ServiceScope::System,
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Service);
    assert_eq!(error.details["os_code"], "no_systemd");
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        let error = connect(ServiceScope::User).unwrap_err();
        assert_eq!(error.details["os_code"], "no_user_bus");
        assert_eq!(error.fix, ["services scope: system"]);
    }
}

/// Whether the running system is managed by systemd with a reachable
/// system bus.
fn systemd_available() -> bool {
    std::path::Path::new(SYSTEMD_RUNTIME_DIR).is_dir() && connect(ServiceScope::System).is_ok()
}

fn running_as_root() -> bool {
    rustix::process::geteuid().is_root()
}

#[test]
#[ignore = "requires systemd"]
fn it_systemd_lists_cron_service() {
    if !systemd_available() {
        return;
    }
    let backend = SystemdBackend::new();
    let services = listed(&backend);
    assert!(!services.is_empty());
    if let Some(cron) = services.iter().find(|service| service.name == "cron") {
        assert_eq!(cron.manager, ServiceManager::Systemd);
        assert!(cron.state_raw.is_some());
    }
}

#[test]
#[ignore = "requires systemd, root and a running cron service"]
fn it_systemd_restart_cron_running_after() {
    if !systemd_available() || !running_as_root() {
        return;
    }
    let backend = SystemdBackend::new();
    let Ok(cron) = backend.get("cron", ServiceScope::System) else {
        return;
    };
    if cron.state != ServiceState::Running {
        return;
    }
    let ctx = crate::services::tests::real_context();
    let mut tx = RecordingTx::new(Arc::new(Mutex::new(Vec::new())));
    let transition = perform(&backend, ServiceAction::Restart, &cron, WAIT, &mut tx, &ctx).unwrap();
    assert_eq!(transition.after, "running");
}

#[test]
#[ignore = "requires systemd and root"]
fn it_systemd_enable_disable_test_unit() {
    if !systemd_available() || !running_as_root() {
        return;
    }
    let name = format!("nurt-test-{}", std::process::id());
    let file = std::path::Path::new(SYSTEMD_RUNTIME_DIR).join(unit_name(&name));
    std::fs::write(
        &file,
        "[Unit]\nDescription=Danaco Nurt test unit\n\n[Service]\nType=oneshot\nExecStart=/bin/true\n\n[Install]\nWantedBy=multi-user.target\n",
    )
    .unwrap();
    let backend = SystemdBackend::new();
    let reload = |backend: &SystemdBackend| {
        backend
            .manager_call::<_, ()>(ServiceScope::System, "", "Reload", &())
            .unwrap();
    };
    reload(&backend);
    let enabled = backend.enable(&name, ServiceScope::System);
    let after_enable = backend
        .get(&name, ServiceScope::System)
        .map(|service| service.start_type);
    let disabled = backend.disable(&name, ServiceScope::System);
    let after_disable = backend
        .get(&name, ServiceScope::System)
        .map(|service| service.start_type);
    std::fs::remove_file(&file).unwrap();
    reload(&backend);
    enabled.unwrap();
    disabled.unwrap();
    assert_eq!(after_enable.unwrap(), StartType::Automatic);
    assert_eq!(after_disable.unwrap(), StartType::Disabled);
}

#[test]
#[ignore = "requires a systemd user manager on the session bus"]
fn it_systemd_user_scope_lists_user_units() {
    if connect(ServiceScope::User).is_err() {
        return;
    }
    let backend = SystemdBackend::new();
    let services = backend
        .list(&ServiceQuery {
            scope: ServiceScope::User,
        })
        .unwrap();
    assert!(
        services
            .iter()
            .all(|service| service.scope == Some(ServiceScope::User))
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
