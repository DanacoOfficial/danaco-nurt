// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the firewall, route and clock commands against the real system.
//!
//! They change the system and need root, `nft`, `ip` and `timedatectl`, so
//! they are `#[ignore]` and run by hand on the disposable Linux machine of
//! the pool: `cargo nextest run -p nurt-sys --test admin_root --run-ignored
//! only` as root. Every test removes what it created (the table
//! `nurt_test_fw`, the interface `nurt_test0`) and restores the time zone
//! and the state of network time.
#![cfg(target_os = "linux")]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::WriteHandle;
use nurt_contracts::journal::{
    Applied, ExemptWrite, ExternalChange, Mutation, MutationTx, OpHandle, OpId, Preserved,
    RollbackReason, RollbackReport, TargetIndex, TargetSpec,
};
use nurt_contracts::record::Change;
use nurt_contracts::report::IrreversibleEntry;
use nurt_contracts::services::{
    ProcessHost, ProgramInput, ProgramOutcome, ProgramSpec, ServiceChild, ServiceSpawnSpec,
};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelToken, EffectStage, ExecCtx, StageData, StageInput,
};
use nurt_contracts::value::Value;
use nurt_sys::admin::clock::ClockCommand;
use nurt_sys::admin::firewall::FirewallChange;
use nurt_sys::admin::inverse::{Undo, derive};
use nurt_sys::admin::network::RouteChange;
use nurt_testkit::ctx::TestContext;

const TABLE: &str = "nurt_test_fw";
const LINK: &str = "nurt_test0";

struct StdProcess;

impl ProcessHost for StdProcess {
    fn run_program(&self, spec: &ProgramSpec, _cancel: &CancelToken) -> Result<ProgramOutcome> {
        let mut child = Command::new(&spec.program)
            .args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| Error::new(ErrorCode::ProgramNotFound, error.to_string()))?;
        if let ProgramInput::Bytes(bytes) = &spec.stdin {
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(bytes)
                .expect("write");
        }
        let output = child.wait_with_output().expect("output");
        Ok(ProgramOutcome {
            exit_code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
            ..ProgramOutcome::default()
        })
    }

    fn resolve_program(&self, program: &str, _cwd: &Path) -> Result<PathBuf> {
        which::which(program)
            .map_err(|error| Error::new(ErrorCode::ProgramNotFound, error.to_string()))
    }

    fn spawn_service(&self, _spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
        Err(Error::new(ErrorCode::Spawn, "not available"))
    }
}

#[derive(Default)]
struct Tx {
    markers: Vec<IrreversibleEntry>,
}

fn unavailable<T>() -> Result<T> {
    Err(Error::new(ErrorCode::StageFailed, "not used"))
}

impl MutationTx for Tx {
    fn op(&self) -> Option<OpId> {
        None
    }
    fn preserve(&mut self, _t: &[TargetSpec]) -> Result<Vec<Preserved>> {
        unavailable()
    }
    fn stage(&mut self, _t: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        unavailable()
    }
    fn apply(&mut self, _t: TargetIndex, _m: Mutation) -> Result<Applied> {
        unavailable()
    }
    fn observe_external(&mut self, _c: &[ExternalChange]) -> Result<Vec<Applied>> {
        unavailable()
    }
    fn mark_irreversible(&mut self, marker: IrreversibleEntry) {
        self.markers.push(marker);
    }
    fn record_exempt(&mut self, _w: ExemptWrite) {}
    fn commit(self: Box<Self>) -> Result<OpHandle> {
        unavailable()
    }
    fn rollback(self: Box<Self>, _r: RollbackReason) -> Result<RollbackReport> {
        unavailable()
    }
}

fn context() -> ExecCtx {
    TestContext::new()
        .with_process_host(Arc::new(StdProcess))
        .ctx()
}

fn args(command: &str, values: &[(&str, Value)]) -> BoundArgs {
    values
        .iter()
        .fold(BoundArgs::new(command), |args, (name, value)| {
            args.with(*name, ArgValue::Value(value.clone()), false)
        })
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn word(value: &str) -> Value {
    Value::Enum(value.to_owned())
}

fn shell(program: &str, arguments: &[&str]) -> String {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .expect("program starts");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Changes reported by a stage.
struct Done {
    changes: Vec<Change>,
}

fn run(stage: &dyn EffectStage, request: &BoundArgs, ctx: &ExecCtx) -> Done {
    let input = StageInput {
        data: StageData::None,
        header: None,
        order: None,
        render: None,
    };
    let plan = stage.plan(request, input, ctx).expect("plan");
    let mut tx = Tx::default();
    let outcome = stage.apply(request, plan, &mut tx, ctx).expect("apply");
    assert!(!tx.markers.is_empty() || outcome.changes.is_empty());
    Done {
        changes: outcome.changes,
    }
}

struct Table;

impl Drop for Table {
    fn drop(&mut self) {
        let _ = Command::new("nft")
            .args(["delete", "table", "inet", TABLE])
            .output();
    }
}

#[test]
#[ignore = "needs root and nftables on the disposable Linux machine"]
fn nftables_rules_are_added_and_removed() {
    let _cleanup = Table;
    let ctx = context();
    let base = |port: i64| {
        vec![
            ("port", Value::Int(port)),
            ("backend", word("nftables")),
            ("table", text(TABLE)),
        ]
    };
    let allow = run(
        &FirewallChange::ALLOW,
        &args("firewall.allow", &base(22)),
        &ctx,
    );
    assert_eq!(allow.changes.len(), 1);
    assert!(shell("nft", &["list", "table", "inet", TABLE]).contains("tcp dport 22 accept"));
    let again = run(
        &FirewallChange::ALLOW,
        &args("firewall.allow", &base(22)),
        &ctx,
    );
    assert!(again.changes.is_empty());
    let mut deny = base(5353);
    deny.extend([
        ("protocol", word("udp")),
        ("direction", word("out")),
        ("source", text("192.0.2.0/24")),
    ]);
    let denied = run(&FirewallChange::DENY, &args("firewall.deny", &deny), &ctx);
    let listing = shell("nft", &["list", "table", "inet", TABLE]);
    assert!(listing.contains("udp dport 5353 drop"), "{listing}");
    let marker = IrreversibleEntry::new(nurt_contracts::report::Irreversible::ExternalEffect);
    let mut marker = marker;
    marker.detail = Some(format!("admin:firewall:nftables:{TABLE}"));
    let undo = derive("firewall.deny", &denied.changes, &[marker]);
    assert!(matches!(undo, Some(Undo::Programs(ref programs)) if programs.len() == 1));
    let mut remove = base(5353);
    remove.extend([
        ("protocol", word("udp")),
        ("direction", word("out")),
        ("source", text("192.0.2.0/24")),
        ("action", word("deny")),
    ]);
    run(
        &FirewallChange::REMOVE,
        &args("firewall.remove", &remove),
        &ctx,
    );
    run(
        &FirewallChange::REMOVE,
        &args("firewall.remove", &base(22)),
        &ctx,
    );
    let after = shell("nft", &["list", "table", "inet", TABLE]);
    assert!(!after.contains("dport"), "{after}");
}

struct Link;

impl Drop for Link {
    fn drop(&mut self) {
        let _ = Command::new("ip").args(["link", "delete", LINK]).output();
    }
}

#[test]
#[ignore = "needs root and iproute2 on the disposable Linux machine"]
fn routes_are_added_and_removed_on_a_dummy_interface() {
    let _cleanup = Link;
    for arguments in [
        vec!["link", "add", LINK, "type", "dummy"],
        vec!["addr", "add", "192.0.2.2/24", "dev", LINK],
        vec!["link", "set", LINK, "up"],
    ] {
        assert!(
            Command::new("ip")
                .args(&arguments)
                .status()
                .expect("ip")
                .success()
        );
    }
    let ctx = context();
    let request = args(
        "network.route_add",
        &[
            ("destination", text("198.51.100.0/24")),
            ("gateway", text("192.0.2.1")),
            ("interface", text(LINK)),
            ("metric", Value::Int(321)),
        ],
    );
    let added = run(&RouteChange::ADD, &request, &ctx);
    assert!(shell("ip", &["route", "show", "dev", LINK]).contains("198.51.100.0/24 via 192.0.2.1"));
    let Some(Undo::Programs(programs)) = derive("network.route_add", &added.changes, &[]) else {
        panic!("the route has an inverse");
    };
    assert_eq!(programs.len(), 1);
    assert!(programs[0].starts_with("network.route_remove \"198.51.100.0/24\""));
    let remove = args(
        "network.route_remove",
        &[("destination", text("198.51.100.0/24"))],
    );
    run(&RouteChange::REMOVE, &remove, &ctx);
    assert!(!shell("ip", &["route", "show", "dev", LINK]).contains("198.51.100.0/24"));
}

struct Clock {
    zone: String,
    ntp: bool,
}

impl Clock {
    fn read() -> Self {
        Self {
            zone: shell("timedatectl", &["show", "-p", "Timezone", "--value"])
                .trim()
                .to_owned(),
            ntp: shell("timedatectl", &["show", "-p", "NTP", "--value"]).trim() == "yes",
        }
    }
}

impl Drop for Clock {
    fn drop(&mut self) {
        let _ = Command::new("timedatectl")
            .args(["set-timezone", &self.zone])
            .output();
        let _ = Command::new("timedatectl")
            .args(["set-ntp", if self.ntp { "true" } else { "false" }])
            .output();
    }
}

#[test]
#[ignore = "needs root and systemd (timedatectl) on the disposable Linux machine"]
fn time_zone_and_network_time_are_changed_and_restored() {
    let original = Clock::read();
    let ctx = context();
    let other = if original.zone == "Pacific/Auckland" {
        "UTC"
    } else {
        "Pacific/Auckland"
    };
    let changed = run(
        &ClockCommand::TIMEZONE,
        &args("clock.timezone_set", &[("zone", text(other))]),
        &ctx,
    );
    assert_eq!(Clock::read().zone, other);
    let Some(Undo::Programs(programs)) = derive("clock.timezone_set", &changed.changes, &[]) else {
        panic!("the zone has an inverse");
    };
    assert_eq!(
        programs,
        [format!("clock.timezone_set \"{}\"", original.zone)]
    );
    run(
        &ClockCommand::TIMEZONE,
        &args("clock.timezone_set", &[("zone", text(&original.zone))]),
        &ctx,
    );
    assert_eq!(Clock::read().zone, original.zone);
    let wanted = if original.ntp { "off" } else { "on" };
    run(
        &ClockCommand::NTP,
        &args("clock.ntp", &[("state", word(wanted))]),
        &ctx,
    );
    assert_eq!(Clock::read().ntp, !original.ntp);
}
