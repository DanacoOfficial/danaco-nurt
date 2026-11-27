// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-core/tests/admin_inverse.rs
// Responsibility:   The programs that undo firewall, network, clock and CIM changes are valid programs: they parse and bind against the catalog on the system they are made for.
// Dependencies:     nurt_bind, nurt_contracts, nurt_syntax, nurt_sys
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The programs that undo firewall, network, clock and CIM changes are
//! valid programs: they parse and bind against the catalog on the system
//! they are made for.

// Test code asserts with unwrap, expect and panic.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use nurt_bind::{BindContext, Catalog, bind};
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::record::{Change, ChangeAction};
use nurt_contracts::report::{Irreversible, IrreversibleEntry};
use nurt_contracts::spec::Os;
use nurt_syntax::parse::{ParseOptions, parse};
use nurt_sys::admin::undo::{Undo, derive};

fn change(path: &str, before: Option<&str>, after: Option<&str>) -> Change {
    let mut change = Change::new(path.to_owned(), ChangeAction::State);
    change.before = before.map(str::to_owned);
    change.after = after.map(str::to_owned);
    change
}

fn marker(detail: &str) -> IrreversibleEntry {
    let mut entry = IrreversibleEntry::new(Irreversible::ExternalEffect);
    entry.detail = Some(format!("admin:{detail}"));
    entry
}

fn programs(kind: &str, changes: &[Change], markers: &[IrreversibleEntry]) -> Vec<String> {
    match derive(kind, changes, markers) {
        Some(Undo::Programs(programs)) => programs,
        other => panic!("{kind}: {other:?}"),
    }
}

fn binds(text: &str, os: Os) {
    let catalog = Catalog::load_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../catalog"))
        .expect("catalog");
    let document = parse(text, SyntaxVariant::A, &ParseOptions::default())
        .unwrap_or_else(|error| panic!("`{text}` does not parse: {error}"));
    bind(&document, &catalog, &BindContext::new(os))
        .unwrap_or_else(|error| panic!("`{text}` does not bind: {error}"));
}

#[test]
fn firewall_inverses_bind() {
    let allow = programs(
        "firewall.allow",
        &[change(
            "firewall:allow tcp 8000-8100 in 10.0.0.0/8 public",
            Some("absent"),
            Some("present"),
        )],
        &[marker("firewall:firewalld:nurt")],
    );
    let removed = programs(
        "firewall.remove",
        &[change(
            "firewall:deny udp 53 out 2001:db8::/32 -",
            Some("present"),
            Some("absent"),
        )],
        &[marker("firewall:nftables:nurt_x")],
    );
    for text in allow.iter().chain(&removed) {
        binds(text, Os::Linux);
    }
}

#[test]
fn route_dns_and_clock_inverses_bind() {
    let state = "via=203.0.113.5 dev=eth0 metric=20";
    let mut all = programs(
        "network.route_add",
        &[change("route:10.9.0.0/16", None, Some(state))],
        &[],
    );
    all.extend(programs(
        "network.route_remove",
        &[change(
            "route:default",
            Some("via=203.0.113.1 dev=- metric=-"),
            None,
        )],
        &[],
    ));
    all.extend(programs(
        "network.dns_set",
        &[change(
            "dns:eth0",
            Some("servers=198.51.100.1,198.51.100.2;search=a.org,b.org"),
            Some("servers=1.1.1.1;search=a.org,b.org"),
        )],
        &[],
    ));
    all.extend(programs(
        "network.dns_set",
        &[change(
            "dns:eth0",
            Some("servers=;search="),
            Some("servers=1.1.1.1;search="),
        )],
        &[],
    ));
    all.extend(programs(
        "clock.timezone_set",
        &[change("clock:timezone", Some("Europe/Warsaw"), Some("UTC"))],
        &[],
    ));
    all.extend(programs(
        "clock.ntp",
        &[change("clock:ntp", Some("on"), Some("off"))],
        &[],
    ));
    assert_eq!(all.len(), 6);
    for text in &all {
        binds(text, Os::Linux);
    }
}

#[test]
fn cim_inverse_binds_on_windows() {
    let text = programs(
        "windows.cim",
        &[change(
            "cim:Win32_Service.StopService",
            None,
            Some("namespace=root/cimv2;where=Name='Spooler'"),
        )],
        &[],
    );
    assert_eq!(text.len(), 1);
    binds(&text[0], Os::Windows);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
