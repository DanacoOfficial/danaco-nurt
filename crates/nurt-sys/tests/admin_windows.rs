// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module: nurt-sys tests admin_windows
// Responsibility: Tests the firewall, DNS and time zone commands against a disposable Windows machine.
// Dependencies: nurt-sys, nurt-contracts, nurt-testkit
// Author: Danaco Official
// Version: 0.1.1, 2026-11-27
// License: see the LICENSE file or https://www.danaco-group.pl
//! Tests of the firewall, DNS and time zone commands against Windows.
//!
//! They change the system and need an elevated session on the disposable
//! Windows machine, so they are `#[ignore]`: `cargo nextest run -p nurt-sys
//! --test admin_windows --run-ignored only`. Every test removes the rule it
//! created and restores the time zone and the name servers of the interface.
#![cfg(windows)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod admin_support;

use std::process::Command;

use admin_support::{args, context, run, text, word};
use nurt_contracts::value::Value;
use nurt_sys::admin::clock::ClockCommand;
use nurt_sys::admin::firewall::FirewallChange;
use nurt_sys::admin::inverse::{Undo, derive};
use nurt_sys::admin::network::DnsSet;

fn powershell(script: &str) -> String {
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .expect("powershell starts");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

struct Rule {
    port: i64,
}

impl Rule {
    fn request(&self) -> nurt_contracts::stage::BoundArgs {
        args(
            "firewall.allow",
            &[
                ("port", Value::Int(self.port)),
                ("protocol", word("tcp")),
                ("backend", word("windows")),
            ],
        )
    }
}

impl Drop for Rule {
    fn drop(&mut self) {
        let request = args(
            "firewall.remove",
            &[
                ("port", Value::Int(self.port)),
                ("protocol", word("tcp")),
                ("backend", word("windows")),
            ],
        );
        let ctx = context();
        let input = admin_support::no_input();
        if let Ok(plan) =
            nurt_contracts::stage::EffectStage::plan(&FirewallChange::REMOVE, &request, input, &ctx)
        {
            let mut tx = admin_support::Tx::default();
            let _ = nurt_contracts::stage::EffectStage::apply(
                &FirewallChange::REMOVE,
                &request,
                plan,
                &mut tx,
                &ctx,
            );
        }
    }
}

#[test]
#[ignore = "needs an elevated session on the disposable Windows machine (Windows Defender Firewall)"]
fn firewall_rule_is_added_and_removed() {
    let rule = Rule { port: 58321 };
    let ctx = context();
    let added = run(&FirewallChange::ALLOW, &rule.request(), &ctx);
    assert_eq!(added.changes.len(), 1);
    let listed = powershell(
        "Get-NetFirewallPortFilter -Protocol TCP | Where-Object LocalPort -eq 58321 | Measure-Object | ForEach-Object Count",
    );
    assert_ne!(listed, "0");
    let Some(Undo::Programs(programs)) = derive("firewall.allow", &added.changes, &[]) else {
        panic!("the rule has an inverse");
    };
    assert_eq!(programs.len(), 1);
    assert!(programs[0].starts_with("firewall.remove"));
}

struct Zone(String);

impl Drop for Zone {
    fn drop(&mut self) {
        let _ = Command::new("tzutil").args(["/s", &self.0]).output();
    }
}

#[test]
#[ignore = "needs an elevated session on the disposable Windows machine (tzutil)"]
fn time_zone_is_changed_and_recorded() {
    let current =
        String::from_utf8_lossy(&Command::new("tzutil").arg("/g").output().unwrap().stdout)
            .trim()
            .to_owned();
    let _restore = Zone(current.clone());
    let other = if current == "Tokyo Standard Time" {
        "UTC"
    } else {
        "Tokyo Standard Time"
    };
    let ctx = context();
    let changed = run(
        &ClockCommand::TIMEZONE,
        &args("clock.timezone_set", &[("zone", text(other))]),
        &ctx,
    );
    let now = String::from_utf8_lossy(&Command::new("tzutil").arg("/g").output().unwrap().stdout)
        .trim()
        .to_owned();
    assert_eq!(now, other);
    let Some(Undo::Programs(programs)) = derive("clock.timezone_set", &changed.changes, &[]) else {
        panic!("the zone has an inverse");
    };
    assert_eq!(programs, [format!("clock.timezone_set \"{current}\"")]);
}

struct Servers {
    alias: String,
    addresses: String,
}

impl Drop for Servers {
    fn drop(&mut self) {
        let script = if self.addresses.is_empty() {
            format!(
                "Set-DnsClientServerAddress -InterfaceAlias '{}' -ResetServerAddresses",
                self.alias
            )
        } else {
            format!(
                "Set-DnsClientServerAddress -InterfaceAlias '{}' -ServerAddresses {}",
                self.alias, self.addresses
            )
        };
        let _ = powershell(&script);
    }
}

#[test]
#[ignore = "needs an elevated session on the disposable Windows machine (DnsClient cmdlets)"]
fn name_servers_are_set_and_the_previous_state_is_recorded() {
    let alias =
        powershell("(Get-NetAdapter | Where-Object Status -eq 'Up' | Select-Object -First 1).Name");
    assert!(!alias.is_empty(), "no adapter is up");
    let before = powershell(&format!(
        "(Get-DnsClientServerAddress -InterfaceAlias '{alias}' -AddressFamily IPv4).ServerAddresses -join ','"
    ));
    let _restore = Servers {
        alias: alias.clone(),
        addresses: before,
    };
    let ctx = context();
    let changed = run(
        &DnsSet,
        &args(
            "network.dns_set",
            &[
                ("servers", Value::List(vec![text("192.0.2.53")])),
                ("interface", text(&alias)),
            ],
        ),
        &ctx,
    );
    assert_eq!(changed.changes.len(), 1);
    let now = powershell(&format!(
        "(Get-DnsClientServerAddress -InterfaceAlias '{alias}' -AddressFamily IPv4).ServerAddresses -join ','"
    ));
    assert!(now.contains("192.0.2.53"), "{now}");
    let Some(Undo::Programs(programs)) = derive("network.dns_set", &changed.changes, &[]) else {
        panic!("the name servers have an inverse");
    };
    assert_eq!(programs.len(), 1);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
