// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Readers of Linux system files used by `host`.

use std::time::Duration;

use super::SysRoot;

/// Bytes in one kibibyte, the unit of `/proc/meminfo`.
const KIB: u64 = 1024;

/// Fields of `/etc/os-release`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct OsRelease {
    /// `ID`: lower-case identifier of the distribution.
    pub(crate) id: Option<String>,
    /// `VERSION_ID`: version of the distribution.
    pub(crate) version_id: Option<String>,
    /// `PRETTY_NAME`: name and version for display.
    pub(crate) pretty_name: Option<String>,
}

/// Parses `os-release`: `KEY=value` lines, values optionally in single or
/// double quotes with backslash escapes, comments starting with `#`.
pub(crate) fn parse_os_release(text: &str) -> OsRelease {
    let mut release = OsRelease::default();
    for line in text.lines().map(str::trim) {
        if line.starts_with('#') {
            continue;
        }
        let Some((key, raw)) = line.split_once('=') else {
            continue;
        };
        let value = unquote(raw.trim());
        let slot = match key.trim() {
            "ID" => &mut release.id,
            "VERSION_ID" => &mut release.version_id,
            "PRETTY_NAME" => &mut release.pretty_name,
            _ => continue,
        };
        *slot = Some(value).filter(|value| !value.is_empty());
    }
    release
}

fn unquote(raw: &str) -> String {
    let quote = raw
        .chars()
        .next()
        .filter(|first| matches!(first, '"' | '\''));
    let Some(quote) = quote else {
        return raw.to_owned();
    };
    let inner = raw[1..].strip_suffix(quote).unwrap_or(&raw[1..]);
    if quote == '\'' {
        return inner.to_owned();
    }
    let mut value = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            value.extend(chars.next());
        } else {
            value.push(c);
        }
    }
    value
}

/// Total and available memory in bytes from `/proc/meminfo`.
pub(crate) fn parse_meminfo(text: &str) -> (Option<u64>, Option<u64>) {
    let mut total = None;
    let mut available = None;
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let kib = rest
            .split_whitespace()
            .next()
            .and_then(|number| number.parse::<u64>().ok());
        match key {
            "MemTotal" => total = kib.map(|kib| kib.saturating_mul(KIB)),
            "MemAvailable" => available = kib.map(|kib| kib.saturating_mul(KIB)),
            _ => {}
        }
    }
    (total, available)
}

/// Time since boot from the first field of `/proc/uptime`.
pub(crate) fn parse_uptime(text: &str) -> Option<Duration> {
    let seconds: f64 = text.split_whitespace().next()?.parse().ok()?;
    Duration::try_from_secs_f64(seconds).ok()
}

/// Boot time in seconds since the Unix epoch from the `btime` line of
/// `/proc/stat`.
pub(crate) fn parse_btime(text: &str) -> Option<i64> {
    text.lines()
        .find_map(|line| line.strip_prefix("btime "))
        .and_then(|value| value.trim().parse().ok())
}

/// Values written by the systemd container detection to
/// `/run/systemd/container` and the names used for them in records.
const CONTAINER_NAMES: [(&str, &str); 6] = [
    ("docker", "docker"),
    ("podman", "podman"),
    ("lxc", "lxc"),
    ("lxc-libvirt", "lxc"),
    ("systemd-nspawn", "systemd-nspawn"),
    ("wsl", "wsl"),
];

/// Markers in `/proc/1/cgroup` that name the container engine.
const CGROUP_MARKERS: [(&str, &str); 4] = [
    ("/docker/", "docker"),
    ("/libpod-", "podman"),
    ("/lxc/", "lxc"),
    ("/kubepods", "kubernetes"),
];

/// Vendors in `/sys/class/dmi/id/sys_vendor` and product names in
/// `/sys/class/dmi/id/product_name` that identify a virtual machine.
const DMI_MARKERS: [(&str, &str); 9] = [
    ("qemu", "qemu"),
    ("kvm", "kvm"),
    ("vmware", "vmware"),
    ("innotek", "virtualbox"),
    ("virtualbox", "virtualbox"),
    ("xen", "xen"),
    ("parallels", "parallels"),
    ("amazon ec2", "amazon"),
    ("google", "google"),
];

/// Product name of Hyper-V guests, whose vendor is the vendor of the host
/// operating system.
const HYPERV_PRODUCT: &str = "virtual machine";
/// Vendor of Hyper-V guests.
const HYPERV_VENDOR: &str = "microsoft corporation";

/// Detects a container or a virtual machine; containers take precedence.
pub(crate) fn detect_virtualization(root: &SysRoot) -> Option<String> {
    detect_container(root).or_else(|| detect_machine(root))
}

fn detect_container(root: &SysRoot) -> Option<String> {
    if root.path("/.dockerenv").exists() {
        return Some("docker".to_owned());
    }
    if root.path("/run/.containerenv").exists() {
        return Some("podman".to_owned());
    }
    if let Ok(text) = root.read_to_string("/run/systemd/container") {
        let name = text.trim().to_ascii_lowercase();
        if !name.is_empty() {
            let known = CONTAINER_NAMES.iter().find(|(raw, _)| *raw == name);
            return Some(known.map_or(name, |(_, mapped)| (*mapped).to_owned()));
        }
    }
    let cgroup = root.read_to_string("/proc/1/cgroup").unwrap_or_default();
    if let Some((_, name)) = CGROUP_MARKERS
        .iter()
        .find(|(marker, _)| cgroup.contains(marker))
    {
        return Some((*name).to_owned());
    }
    let release = root
        .read_to_string("/proc/sys/kernel/osrelease")
        .unwrap_or_default()
        .to_ascii_lowercase();
    release.contains("microsoft").then(|| "wsl".to_owned())
}

fn detect_machine(root: &SysRoot) -> Option<String> {
    let read = |name: &str| {
        root.read_to_string(format!("/sys/class/dmi/id/{name}"))
            .map(|text| text.trim().to_ascii_lowercase())
            .unwrap_or_default()
    };
    let vendor = read("sys_vendor");
    let product = read("product_name");
    if vendor == HYPERV_VENDOR && product == HYPERV_PRODUCT {
        return Some("hyperv".to_owned());
    }
    DMI_MARKERS
        .iter()
        .find(|(marker, _)| vendor.contains(marker) || product.contains(marker))
        .map(|(_, name)| (*name).to_owned())
}
