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

use nurt_sys::admin::clock::ClockCommand;
use nurt_sys::admin::firewall::FirewallChange;
use nurt_sys::admin::network::{DnsSet, RouteChange};
use nurt_sys::admin::undo::{Undo, derive};

mod admin_support;

use admin_support::{args, context, run, shell, text, word};
use nurt_contracts::report::IrreversibleEntry;
use nurt_contracts::value::Value;
use std::process::Command;

const TABLE: &str = "nurt_test_fw";
const LINK: &str = "nurt_test0";

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

/// Restores `/etc/resolv.conf` and removes the interface used by the DNS test.
struct Resolver {
    file: Option<Vec<u8>>,
}

impl Drop for Resolver {
    fn drop(&mut self) {
        if let Some(content) = &self.file {
            let _ = std::fs::write("/etc/resolv.conf", content);
        }
        let _ = Command::new("ip").args(["link", "delete", LINK]).output();
    }
}

#[test]
#[ignore = "needs root, iproute2 and a resolver (systemd-resolved or a plain resolv.conf) on the disposable Linux machine"]
fn name_servers_are_set_and_the_previous_state_is_recorded() {
    let plain =
        std::fs::symlink_metadata("/etc/resolv.conf").is_ok_and(|meta| meta.file_type().is_file());
    let _cleanup = Resolver {
        file: plain.then(|| std::fs::read("/etc/resolv.conf").expect("resolv.conf")),
    };
    assert!(
        Command::new("ip")
            .args(["link", "add", LINK, "type", "dummy"])
            .status()
            .expect("ip")
            .success()
    );
    let ctx = context();
    let request = args(
        "network.dns_set",
        &[
            ("servers", Value::List(vec![text("192.0.2.53")])),
            ("interface", text(LINK)),
        ],
    );
    let changed = run(&DnsSet, &request, &ctx);
    assert_eq!(changed.changes.len(), 1);
    let Some(Undo::Programs(programs)) = derive("network.dns_set", &changed.changes, &[]) else {
        panic!("the name servers have an inverse");
    };
    assert_eq!(programs.len(), 1);
    assert!(programs[0].starts_with("network.dns_set"));
    let seen = if plain {
        std::fs::read_to_string("/etc/resolv.conf").expect("resolv.conf")
    } else {
        shell("resolvectl", &["dns", LINK])
    };
    assert!(seen.contains("192.0.2.53"), "{seen}");
}
