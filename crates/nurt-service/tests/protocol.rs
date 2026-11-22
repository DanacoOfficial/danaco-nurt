// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Service host over its control channel: protocol, heartbeat, instance
//! lock, module supervision and `status`, with mock modules and the schedule
//! module of `nurt-sched`.
//!
//! Every test runs its own service in a temporary state directory. The
//! tests use the Unix socket transport; on Windows the named pipe of an
//! account admits one service at a time.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "an integration test reports failures by panicking"
)]

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::net::UnixListener;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use nurt_contracts::config::{Config, ServiceMode};
use nurt_contracts::env::WallTime;
use nurt_contracts::error::ErrorCode;
use nurt_sched::module::ServiceLink;
use nurt_service::heartbeat::{self, Heartbeat, ModuleState};
use nurt_service::ipc::client::{self, ControlLink};
use nurt_service::ipc::protocol::{self, Message};
use nurt_service::module::ServiceModule;
use nurt_service::status::{self, InactiveReason, Liveness, StatusRequest};
use nurt_service::{Backend, Manifest, ScheduleService, ServicePaths, ServiceSetup, run};
use serde_json::{Value, json};

use support::clock::VirtualClock;
use support::d2_client::{self, Verdict};
use support::modules::{Behaviour, MockModule};
use support::{Options, eventually, launch, next_event};

const PING_LIMIT: Duration = Duration::from_millis(50);
const NANOS_PER_MS: i128 = 1_000_000;

fn modules(space: &Arc<MockModule>, schedule: &Arc<MockModule>) -> Vec<Arc<dyn ServiceModule>> {
    vec![
        Arc::clone(space) as Arc<dyn ServiceModule>,
        Arc::clone(schedule) as Arc<dyn ServiceModule>,
    ]
}

fn mocks() -> (Arc<MockModule>, Arc<MockModule>) {
    (MockModule::space(Behaviour::Normal), MockModule::schedule())
}

fn error_code(outcome: Result<Value, Value>) -> String {
    outcome.unwrap_err()["code"].as_str().unwrap().to_owned()
}

#[test]
fn protocol_roundtrip_ping_status_reload() {
    let (space, schedule) = mocks();
    let service = launch(modules(&space, &schedule), Options::default());
    let mut client = service.client();

    let ping = client.call("ping", json!({})).unwrap().unwrap();
    assert_eq!(ping["pid"], std::process::id());

    let status = client.call("status", json!({})).unwrap().unwrap();
    assert_eq!(status["protocol"], 1);
    assert_eq!(status["version"], env!("CARGO_PKG_VERSION"));
    assert!(status["modules"]["space"]["state"].is_string());
    assert!(status["modules"]["schedule"]["state"].is_string());

    let reload = client.call("reload", json!({})).unwrap().unwrap();
    assert_eq!(reload["modules"]["space"], "reloaded");
    assert_eq!(reload["modules"]["schedule"], "reloaded");
    assert_eq!((space.reloads(), schedule.reloads()), (1, 1));

    client
        .send_line(br#"{"v":1,"id":"liveness","op":"ping"}"#)
        .unwrap();
    let Some(Message::Response { id, outcome }) = client.read_message().unwrap() else {
        panic!("a response was expected");
    };
    assert_eq!(id, "liveness");
    assert_eq!(outcome.unwrap()["pid"], std::process::id());

    client.send_line(br#"{"v":1,"op":"ping"}"#).unwrap();
    let Some(Message::Response { id, outcome }) = client.read_message().unwrap() else {
        panic!("a response without identifier was expected");
    };
    assert_eq!(id, Value::Null);
    assert!(outcome.is_ok());

    drop(client);
    service.shutdown().unwrap();
}

#[test]
fn protocol_unknown_op_and_unsupported_version() {
    let (space, schedule) = mocks();
    let service = launch(modules(&space, &schedule), Options::default());
    let mut client = service.client();

    let error = client.call("frobnicate", json!({})).unwrap().unwrap_err();
    assert_eq!(error["code"], protocol::ERR_UNKNOWN_OP);
    assert!(error["ops"].as_array().unwrap().contains(&json!("ping")));
    assert_eq!(
        error_code(client.call("journal.compact", json!({})).unwrap()),
        protocol::ERR_UNKNOWN_OP
    );
    assert_eq!(
        error_code(client.call("space.compact", json!({})).unwrap()),
        "E_UNSUPPORTED"
    );

    client.send_line(br#"{"v":2,"id":7,"op":"ping"}"#).unwrap();
    let Some(Message::Response { id, outcome }) = client.read_message().unwrap() else {
        panic!("a response was expected");
    };
    assert_eq!(id, 7);
    let error = outcome.unwrap_err();
    assert_eq!(error["code"], protocol::ERR_UNSUPPORTED_VERSION);
    assert_eq!(error["supported"], json!([1]));

    client.send_line(b"ping").unwrap();
    client.send_line(b"   ").unwrap();
    client
        .send_line(br#"{"v":1,"id":8,"op":"ping","body":[1]}"#)
        .unwrap();
    for _ in 0..2 {
        let Some(Message::Response { outcome, .. }) = client.read_message().unwrap() else {
            panic!("a response was expected");
        };
        assert_eq!(error_code(outcome), protocol::ERR_BAD_REQUEST);
    }
    assert!(client.call("ping", json!({})).unwrap().is_ok());
}

#[test]
fn protocol_line_too_long_closes_connection() {
    let (space, schedule) = mocks();
    let service = launch(modules(&space, &schedule), Options::default());
    let mut client = service.client();
    let line = vec![b'x'; protocol::MAX_LINE + 1];
    client.send_line(&line).unwrap();
    let Some(Message::Response { outcome, .. }) = client.read_message().unwrap() else {
        panic!("a response was expected");
    };
    assert_eq!(error_code(outcome), protocol::ERR_LINE_TOO_LONG);
    assert!(
        client.read_message().is_err(),
        "the service must close the connection"
    );
    assert!(service.client().call("ping", json!({})).unwrap().is_ok());
}

#[test]
fn protocol_ping_under_50ms_while_module_blocked() {
    let (space, schedule) = mocks();
    let service = launch(modules(&space, &schedule), Options::default());
    schedule.hold();
    let mut blocked = service.client();
    let pending = thread::spawn(move || blocked.call("schedule.status", json!({})).unwrap());
    thread::sleep(Duration::from_millis(50));
    assert!(
        !pending.is_finished(),
        "the request must wait for the held module"
    );

    let fastest = (0..5)
        .map(|_| client::ping(&service.paths, Duration::from_secs(5)).unwrap())
        .map(|(pid, elapsed)| {
            assert_eq!(pid, std::process::id());
            elapsed
        })
        .min()
        .unwrap();
    assert!(fastest < PING_LIMIT, "ping took {fastest:?}");
    let beat = heartbeat::read(&service.paths.heartbeat_file())
        .unwrap()
        .unwrap();
    assert_eq!(beat.pid, std::process::id());

    schedule.release();
    assert_eq!(pending.join().unwrap().unwrap()["module"], "schedule");
}

#[test]
fn protocol_schedule_changed_triggers_module_reload() {
    let mock = MockModule::schedule();
    let service = launch(
        vec![Arc::clone(&mock) as Arc<dyn ServiceModule>],
        Options::default(),
    );
    let reply = service
        .client()
        .call("schedule.changed", json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(reply["schedules"], 0);
    assert_eq!(mock.changes(), 1);
    service.shutdown().unwrap();

    let service = launch(
        vec![Arc::new(ScheduleService::new()) as Arc<dyn ServiceModule>],
        Options::default(),
    );
    eventually("the schedule module to run", || {
        service.status()["modules"]["schedule"]["state"] == "running"
    });
    let mut client = service.client();
    let reply = client.call("schedule.changed", json!({})).unwrap().unwrap();
    assert_eq!(reply["schedules"], 0);
    let reply = client.call("schedule.status", json!({})).unwrap().unwrap();
    assert_eq!(reply["schedules"], 0);
    assert_eq!(reply["running"], json!([]));
    let reply = client
        .call(
            "schedule/register_action",
            json!({"kind": "session_message", "description": "message to the session"}),
        )
        .unwrap()
        .unwrap();
    assert_eq!(reply["kind"], "session_message");
    assert_eq!(
        error_code(client.call("schedule.frobnicate", json!({})).unwrap()),
        "E_UNSUPPORTED"
    );
}

#[test]
fn protocol_schedule_run_busy() {
    let (space, schedule) = mocks();
    let service = launch(modules(&space, &schedule), Options::default());
    schedule.hold();
    let mut first = service.client();
    let running = thread::spawn(move || first.call("schedule.run", json!({"id": "s1"})).unwrap());
    eventually("the first run to start", || schedule.is_running("s1"));

    let error = service
        .client()
        .call("schedule.run", json!({"id": "s1"}))
        .unwrap()
        .unwrap_err();
    assert_eq!(error["code"], "E_JOB_STATE");
    assert_eq!(error["details"]["actual"], "s1#1");

    schedule.release();
    let reply = running.join().unwrap().unwrap();
    assert_eq!(reply["run"]["state"], "ok");
}

#[test]
fn protocol_remote_schedule_action_register_fire_ack() {
    let (space, schedule) = mocks();
    let service = launch(modules(&space, &schedule), Options::default());
    eventually("the schedule module to start", || schedule.started());
    let mut console = service.client();
    let reply = console
        .call(
            "schedule/register_action",
            json!({"kind": "session_message", "description": "message to the session"}),
        )
        .unwrap()
        .unwrap();
    assert_eq!(reply["kind"], "session_message");

    let token = schedule
        .fire("session_message")
        .expect("the kind is registered");
    let (mut console, event, data) = next_event(console);
    assert_eq!(event, "schedule.fired");
    assert_eq!(data["ack_token"], token.as_str());
    assert_eq!(data["action_kind"], "session_message");

    let reply = console
        .call(
            "schedule/ack",
            json!({"token": token, "state": "ok", "summary": "delivered"}),
        )
        .unwrap();
    assert_eq!(reply, Ok(json!({})));
    assert_eq!(schedule.acks(), [(token.clone(), "ok".to_owned())]);
    let error = console
        .call("schedule/ack", json!({"token": token, "state": "ok"}))
        .unwrap();
    assert_eq!(error_code(error), "E_NOT_FOUND");
}

#[test]
fn remote_action_without_client_skipped() {
    let (space, schedule) = mocks();
    let service = launch(modules(&space, &schedule), Options::default());
    eventually("the schedule module to start", || schedule.started());
    let mut watcher = service.client();
    assert_eq!(
        watcher.call("subscribe", json!({})).unwrap(),
        Ok(json!({"subscribed": true}))
    );
    assert_eq!(service.status()["subscribers"], 1);

    assert_eq!(schedule.fire("session_message"), None);
    let (watcher, event, data) = next_event(watcher);
    assert_eq!(event, "schedule.skipped");
    assert_eq!(data["reason"], "action_unavailable");

    let mut console = service.client();
    console
        .call(
            "schedule/register_action",
            json!({"kind": "session_message"}),
        )
        .unwrap()
        .unwrap();
    drop(console);
    eventually("the module to learn about the closed connection", || {
        !schedule.registered("session_message")
    });
    assert_eq!(schedule.fire("session_message"), None);
    let (_watcher, event, data) = next_event(watcher);
    assert_eq!(event, "schedule.skipped");
    assert_eq!(data["reason"], "action_unavailable");
}

#[test]
fn client_compat_d2_journal_inactive_rules() {
    let (space, schedule) = mocks();
    let service = launch(modules(&space, &schedule), Options::default());
    let state_dir = service.paths.state_dir().to_owned();
    let paths = service.paths.clone();
    let attempts = 3;
    assert!(
        (0..attempts).any(|_| {
            d2_client::journal_active(&state_dir, jiff::Timestamp::now()) == Verdict::Active
        }),
        "the reference client must see a running service within {attempts} attempts"
    );
    let liveness = status::liveness(
        &paths,
        status::wall_now(),
        Duration::from_millis(2_000),
        Duration::from_secs(1),
    );
    assert!(matches!(liveness, Liveness::Active { .. }));
    let later =
        jiff::Timestamp::now() + jiff::SignedDuration::from_millis(d2_client::MAX_AGE_MS + 1_500);
    assert_eq!(
        d2_client::journal_active(&state_dir, later),
        Verdict::Inactive("stale_heartbeat")
    );
    service.shutdown().unwrap();

    assert_eq!(
        d2_client::journal_active(&state_dir, jiff::Timestamp::now()),
        Verdict::Inactive("no_heartbeat")
    );
    let cases = [
        (
            std::process::id(),
            2_001,
            "stale_heartbeat",
            InactiveReason::StaleHeartbeat,
        ),
        (
            std::process::id(),
            1_000,
            "no_response",
            InactiveReason::NoResponse,
        ),
        (u32::MAX, 0, "process_gone", InactiveReason::ProcessGone),
    ];
    for (pid, age_ms, verdict, reason) in cases {
        let now = status::wall_now();
        let beat = WallTime::from_unix_nanos(now.unix_nanos() - i128::from(age_ms) * NANOS_PER_MS);
        let heartbeat = Heartbeat {
            pid,
            started: heartbeat::format_instant(beat),
            beat: heartbeat::format_instant(beat),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            protocol: 1,
            modules: BTreeMap::new(),
        };
        fs::write(
            paths.heartbeat_file(),
            serde_json::to_vec(&heartbeat).unwrap(),
        )
        .unwrap();
        let now_text = heartbeat::format_instant(now);
        let reference = d2_client::journal_active(&state_dir, now_text.parse().unwrap());
        assert_eq!(
            reference,
            Verdict::Inactive(verdict),
            "age {age_ms} ms, pid {pid}"
        );
        match status::liveness(&paths, now, Duration::from_millis(2_000), PING_LIMIT) {
            Liveness::Inactive { reason: actual, .. } => assert_eq!(actual, reason),
            Liveness::Active { .. } => panic!("the service must be inactive for {verdict}"),
        }
    }
}

#[test]
fn heartbeat_atomic_reader_never_sees_partial() {
    let (space, schedule) = mocks();
    let service = launch(
        modules(&space, &schedule),
        Options {
            heartbeat_ms: Some(50),
            ..Options::default()
        },
    );
    let path = service.paths.heartbeat_file();
    let mut complete = 0;
    let deadline = std::time::Instant::now() + Duration::from_millis(600);
    while std::time::Instant::now() < deadline {
        match fs::read(&path) {
            Ok(bytes) => {
                let heartbeat: Heartbeat = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                    panic!(
                        "partial heartbeat {:?}: {error}",
                        String::from_utf8_lossy(&bytes)
                    )
                });
                assert_eq!(heartbeat.pid, std::process::id());
                complete += 1;
            }
            Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::NotFound),
        }
    }
    assert!(complete > 10, "only {complete} heartbeats were read");
    let first = heartbeat::read(&path).unwrap().unwrap().beat;
    eventually("a renewed heartbeat", || {
        heartbeat::read(&path)
            .unwrap()
            .is_some_and(|heartbeat| heartbeat.beat != first)
    });
    service.shutdown().unwrap();
    assert!(!path.exists(), "the heartbeat file is removed on shutdown");
}

#[test]
fn heartbeat_interval_virtual_clock() {
    let clock = Arc::new(VirtualClock::new(Duration::from_secs(1)));
    let (space, schedule) = mocks();
    let service = launch(
        modules(&space, &schedule),
        Options {
            clock: Some(Arc::clone(&clock) as _),
            ..Options::default()
        },
    );
    let path = service.paths.heartbeat_file();
    let beat_at = |offset_ms: u64| {
        heartbeat::format_instant(
            VirtualClock::origin().saturating_add(Duration::from_millis(offset_ms)),
        )
    };
    let current = || {
        heartbeat::read(&path)
            .unwrap()
            .map(|heartbeat| heartbeat.beat)
    };
    assert_eq!(current(), Some(beat_at(0)));
    let started = heartbeat::read(&path).unwrap().unwrap().started;
    assert_eq!(started, beat_at(0));

    clock.advance(Duration::from_millis(500));
    eventually("the second heartbeat", || current() == Some(beat_at(500)));
    clock.advance(Duration::from_millis(499));
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        current(),
        Some(beat_at(500)),
        "no heartbeat before the period ends"
    );
    clock.advance(Duration::from_millis(1));
    eventually("the third heartbeat", || current() == Some(beat_at(1_000)));
    let modules = heartbeat::read(&path).unwrap().unwrap().modules;
    assert_eq!(
        modules.keys().map(String::as_str).collect::<Vec<_>>(),
        ["schedule", "space"]
    );
}

#[test]
fn lock_second_instance_exists_error() {
    let (space, schedule) = mocks();
    let service = launch(modules(&space, &schedule), Options::default());
    let second =
        ServiceSetup::with_paths(Config::default(), service.paths.clone()).without_signals();
    let error = run(second, Vec::new()).unwrap_err();
    assert_eq!(error.code, ErrorCode::Exists);
    assert_eq!(error.details["pid"], std::process::id());
    assert!(service.client().call("ping", json!({})).unwrap().is_ok());

    let twins: Vec<Arc<dyn ServiceModule>> = vec![MockModule::schedule(), MockModule::schedule()];
    let dir = tempfile::tempdir().unwrap();
    let setup = ServiceSetup::with_paths(Config::default(), ServicePaths::new(dir.path()))
        .without_signals();
    assert_eq!(run(setup, twins).unwrap_err().code, ErrorCode::InvalidValue);
}

#[test]
fn stale_socket_removed_after_lock() {
    let (space, schedule) = mocks();
    let service = launch(
        modules(&space, &schedule),
        Options {
            prepare: Some(Box::new(|state_dir| {
                fs::create_dir_all(state_dir).unwrap();
                drop(UnixListener::bind(state_dir.join("service.sock")).unwrap());
                assert!(state_dir.join("service.sock").exists());
            })),
            ..Options::default()
        },
    );
    assert!(service.client().call("ping", json!({})).unwrap().is_ok());
    let socket = service.paths.socket_file();
    service.shutdown().unwrap();
    assert!(!socket.exists(), "the socket is removed on shutdown");

    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("service.sock"), b"not a socket").unwrap();
    let setup =
        ServiceSetup::with_paths(Config::default(), ServicePaths::new(&state)).without_signals();
    let error = run(setup, Vec::new()).unwrap_err();
    assert_eq!(error.code, ErrorCode::Io);
    assert!(error.message.contains("not a socket"));
}

#[test]
fn module_panic_isolated_other_keeps_running() {
    let space = MockModule::space(Behaviour::PanicOnStart);
    let schedule = MockModule::schedule();
    let service = launch(modules(&space, &schedule), Options::default());
    eventually("the schedule module to run while space fails", || {
        let status = service.status();
        status["modules"]["schedule"]["state"] == "running"
            && status["modules"]["space"]["restarts"].as_u64() >= Some(1)
    });
    let status = service.status();
    assert!(
        status["modules"]["space"]["reason"]
            .as_str()
            .unwrap()
            .contains("panicked")
    );
    let reply = service.client().call("schedule.status", json!({})).unwrap();
    assert_eq!(reply.unwrap()["module"], "schedule");
    assert_eq!(schedule.starts(), 1);
}

#[test]
fn module_restart_backoff_and_rate_limit() {
    let clock = Arc::new(VirtualClock::new(Duration::from_secs(1)));
    let space = MockModule::space(Behaviour::PanicOnStart);
    let schedule = MockModule::schedule();
    let service = launch(
        modules(&space, &schedule),
        Options {
            clock: Some(Arc::clone(&clock) as _),
            ..Options::default()
        },
    );
    eventually("the space module to fail for good", || {
        service.status()["modules"]["space"]["state"] == "failed"
    });
    assert_eq!(space.starts(), 6, "one start and five restarts");
    assert_eq!(service.status()["modules"]["space"]["restarts"], 5);
    assert_eq!(
        clock.elapsed(),
        Duration::from_secs(1 + 2 + 4 + 8 + 16),
        "delays double from one second"
    );
    // The heartbeat sleeps on the virtual clock; one period lets it record
    // the state reached after the last restart delay.
    clock.advance(Duration::from_millis(500));
    eventually("the heartbeat to report the failure", || {
        heartbeat::read(&service.paths.heartbeat_file())
            .unwrap()
            .is_some_and(|heartbeat| heartbeat.modules.get("space") == Some(&ModuleState::Failed))
    });

    let error = service
        .client()
        .call("space.status", json!({}))
        .unwrap()
        .unwrap_err();
    assert_eq!(error["code"], protocol::ERR_MODULE_UNAVAILABLE);
    let reply = service.client().call("reload", json!({})).unwrap().unwrap();
    assert_eq!(reply["modules"]["space"], "restarted");
    eventually("the space module to start again", || space.starts() >= 7);
}

#[test]
fn service_never_writes_schedule_definitions() {
    let service = launch(
        vec![Arc::new(ScheduleService::new()) as Arc<dyn ServiceModule>],
        Options {
            prepare: Some(Box::new(|state_dir| {
                fs::create_dir_all(state_dir.join("schedules").join("definitions")).unwrap();
            })),
            ..Options::default()
        },
    );
    let definitions = service
        .paths
        .state_dir()
        .join("schedules")
        .join("definitions");
    eventually("the schedule module to run", || {
        service.status()["modules"]["schedule"]["state"] == "running"
    });
    let mut client = service.client();
    client.call("schedule.changed", json!({})).unwrap().unwrap();
    client.call("schedule.status", json!({})).unwrap().unwrap();
    client.call("reload", json!({})).unwrap().unwrap();
    drop(client);
    service.shutdown().unwrap();
    let written: Vec<_> = fs::read_dir(&definitions).unwrap().collect();
    assert!(written.is_empty(), "the service wrote {written:?}");
}

#[test]
fn status_assembles_installed_running_modules() {
    let (space, schedule) = mocks();
    let service = launch(modules(&space, &schedule), Options::default());
    let manifest = Manifest {
        format: "nurt-service-install/1".to_owned(),
        mode: ServiceMode::User,
        backend: Backend::SystemdUser,
        version: "0.0.9".to_owned(),
        executable: "/opt/nurt/bin/nurt".into(),
        account: None,
        domain: None,
        files: vec!["/home/user/.config/systemd/user/nurt-service.service".into()],
        linger: Some(false),
    };
    fs::write(
        service.paths.install_manifest(),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let mut request = StatusRequest::new(service.paths.clone(), &Config::default());
    request.ping_timeout = Duration::from_secs(5);

    eventually("both modules to run", || {
        let status = status::status(&request).unwrap();
        status.modules.get("space") == Some(&ModuleState::Running)
            && status.modules.get("schedule") == Some(&ModuleState::Running)
    });
    let report = status::status(&request).unwrap();
    assert!(report.installed && report.running);
    assert_eq!(report.mode, Some(ServiceMode::User));
    assert_eq!(report.pid, Some(std::process::id()));
    assert_eq!(report.linger, Some(false));
    assert_eq!(report.version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert!(report.heartbeat_age_ms.is_some());
    assert!(report.started.is_some());

    let link = ControlLink::new(
        service.paths.clone(),
        Duration::from_millis(2_000),
        Duration::from_secs(5),
    );
    assert!(link.running());
    let reply = link
        .request(nurt_sched::module::ModuleRequest::new(
            "schedule.status",
            json!({}),
        ))
        .unwrap();
    assert_eq!(reply["module"], "schedule");
    let error = link
        .request(nurt_sched::module::ModuleRequest::new(
            "schedule.frobnicate",
            json!({}),
        ))
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Unsupported);

    service.shutdown().unwrap();
    let report = status::status(&request).unwrap();
    assert!(report.installed && !report.running);
    assert_eq!(report.reason, Some(InactiveReason::NoHeartbeat));
    assert_eq!(report.version.as_deref(), Some("0.0.9"));
    assert!(report.modules.is_empty() && report.pid.is_none());
}
