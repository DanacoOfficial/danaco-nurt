// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-net/tests/interfaces_sysfs.rs
// Responsibility:   `network.interfaces` on Linux: a synthetic `/sys/class/net` and.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `network.interfaces` on Linux: a synthetic `/sys/class/net` and
//! `/proc/net` tree below a system root, and the interfaces of the host.

#![cfg(target_os = "linux")]
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test on an unexpected result"
)]

#[path = "support/exec_ctx.rs"]
mod exec_ctx;

use std::fs;
use std::path::Path;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::{InterfaceKind, NetworkInterface};
use nurt_net::interfaces::{COMMAND, InterfaceAddress, Interfaces, SysRoot, from_sysfs};

fn write(root: &Path, path: &str, text: &str) {
    let file = root.join(path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, text).unwrap();
}

fn mkdir(root: &Path, path: &str) {
    fs::create_dir_all(root.join(path)).unwrap();
}

fn address(interface: &str, address: &str, prefix: u8) -> InterfaceAddress {
    InterfaceAddress {
        interface: interface.to_owned(),
        address: address.parse().unwrap(),
        prefix,
    }
}

/// Four interfaces: a wired card with an IPv4 default route, the loopback
/// interface, a wireless card with an IPv6 default route and a virtual pair
/// end without a device.
fn fixture(root: &Path) {
    for (name, link_type, state, mac, mtu) in [
        ("eth0", "1", "up", "02:42:ac:11:00:02", "1500"),
        ("lo", "772", "unknown", "00:00:00:00:00:00", "65536"),
        ("wlan0", "1", "down", "3c:22:fb:00:00:01", "1500"),
        ("veth1", "1", "up", "6a:00:00:00:00:01", "1450"),
    ] {
        let dir = format!("sys/class/net/{name}");
        write(root, &format!("{dir}/type"), &format!("{link_type}\n"));
        write(root, &format!("{dir}/operstate"), &format!("{state}\n"));
        write(root, &format!("{dir}/address"), &format!("{mac}\n"));
        write(root, &format!("{dir}/mtu"), &format!("{mtu}\n"));
    }
    write(root, "sys/class/net/lo/flags", "0x9\n");
    mkdir(root, "sys/class/net/eth0/device");
    mkdir(root, "sys/class/net/wlan0/device");
    mkdir(root, "sys/class/net/wlan0/wireless");
    write(
        root,
        "proc/net/route",
        "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\tMTU\tWindow\tIRTT\n\
         eth0\t00000000\t0102000A\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
         eth0\t0000000A\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n",
    );
    let zero = "0".repeat(32);
    write(
        root,
        "proc/net/ipv6_route",
        &format!(
            "{zero} 00 {zero} 00 fe800000000000000000000000000001 00000400 00000001 00000000 00450003 wlan0\n"
        ),
    );
}

#[test]
fn interfaces_linux_sysroot() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let addresses = [
        address("eth0", "10.0.2.15", 24),
        address("lo", "127.0.0.1", 8),
        address("eth0", "2001:db8::15", 64),
        address("lo", "::1", 128),
    ];
    let found = from_sysfs(&SysRoot::new(dir.path()), &addresses).unwrap();
    let names: Vec<&str> = found.iter().map(|item| item.name.as_str()).collect();
    assert_eq!(names, ["eth0", "lo", "veth1", "wlan0"]);

    assert_eq!(
        found[0],
        NetworkInterface {
            name: "eth0".to_owned(),
            addresses: vec!["10.0.2.15/24".to_owned(), "2001:db8::15/64".to_owned()],
            up: true,
            mac: Some("02:42:ac:11:00:02".to_owned()),
            mtu: Some(1500),
            kind: Some(InterfaceKind::Ethernet),
            gateway: Some("10.0.2.1".to_owned()),
        }
    );
    let loopback = &found[1];
    assert!(loopback.up);
    assert_eq!(loopback.mac, None);
    assert_eq!(loopback.kind, Some(InterfaceKind::Loopback));
    assert_eq!(loopback.addresses, ["127.0.0.1/8", "::1/128"]);
    assert_eq!(found[2].kind, Some(InterfaceKind::Virtual));
    assert_eq!(found[2].mtu, Some(1450));
    let wireless = &found[3];
    assert!(!wireless.up);
    assert_eq!(wireless.kind, Some(InterfaceKind::Wireless));
    assert_eq!(wireless.gateway.as_deref(), Some("fe80::1"));
    assert!(wireless.addresses.is_empty());
}

#[test]
fn missing_sysfs_is_io_error() {
    let dir = tempfile::tempdir().unwrap();
    let error = from_sysfs(&SysRoot::new(dir.path()), &[]).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Io);
}

#[test]
fn stage_lists_loopback_of_host() {
    let args = exec_ctx::args(COMMAND, &[]);
    let records = exec_ctx::run(&Interfaces::new(), &args, &exec_ctx::ctx()).unwrap();
    let interfaces: Vec<NetworkInterface> = records
        .into_iter()
        .map(|record| NetworkInterface::try_from(record).unwrap())
        .collect();
    let loopback = interfaces
        .iter()
        .find(|item| item.kind == Some(InterfaceKind::Loopback))
        .unwrap();
    assert!(
        loopback
            .addresses
            .iter()
            .any(|address| address.starts_with("127."))
    );
}

#[test]
fn stage_reads_below_its_root() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let stage = Interfaces::with_root(SysRoot::new(dir.path()));
    let records = exec_ctx::run(&stage, &exec_ctx::args(COMMAND, &[]), &exec_ctx::ctx()).unwrap();
    let names: Vec<String> = records
        .into_iter()
        .map(|record| NetworkInterface::try_from(record).unwrap().name)
        .collect();
    assert_eq!(names, ["eth0", "lo", "veth1", "wlan0"]);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
