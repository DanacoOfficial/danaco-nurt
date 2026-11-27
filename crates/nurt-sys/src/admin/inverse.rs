// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Inverse operations of the administration commands.
//!
//! The report of a change records the state before and after of every
//! target. From it this module derives the programs that restore the state;
//! the engine keeps them with the number of the journal entry and runs them
//! on `undo`, with the rights of the original operation.
//!
//! | Command | Recorded target | Undo |
//! |---|---|---|
//! | `firewall.allow`, `firewall.deny` | `firewall:<rule>` absent → present | `firewall.remove` of the rule |
//! | `firewall.remove` | `firewall:<rule>` present → absent | `firewall.allow` or `firewall.deny` of the rule |
//! | `network.route_add` | `route:<destination>` | `network.route_remove` of the route |
//! | `network.route_remove` | `route:<destination>` | `network.route_add` of the route |
//! | `network.dns_set` | `dns:<interface>` | `network.dns_set` with the previous servers and domains |
//! | `clock.timezone_set` | `clock:timezone` | `clock.timezone_set` with the previous zone |
//! | `clock.ntp` | `clock:ntp` | `clock.ntp` with the previous state |
//! | `clock.set` | `clock:time` | irreversible |
//! | `windows.cim` with `invoke` | `cim:<class>.<method>` | the documented opposite method, otherwise irreversible |
//!
//! `network.dns_set` on `/etc/resolv.conf` records the file; the journal
//! restores it and no program is needed.

use nurt_contracts::record::Change;
use nurt_contracts::report::{Irreversible, IrreversibleEntry};

use super::clock::{NTP_TARGET, TIMEZONE_TARGET};
use super::effects::MARKER_PREFIX;
use super::firewall::{DEFAULT_TABLE, Rule, TARGET_PREFIX as FIREWALL_PREFIX, Verdict};
use super::network::{DNS_PREFIX, ROUTE_PREFIX, Route, parse_dns_text};

/// Prefix of the target of a CIM method call.
pub const CIM_PREFIX: &str = "cim:";

/// Result of the derivation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Undo {
    /// Programs that restore the state, run one after another.
    Programs(Vec<String>),
    /// The change cannot be undone, with the reason.
    Irreversible(String),
}

/// Whether `entry` is the marker of an administration change, which the
/// engine drops when it derives the inverse.
#[must_use]
pub fn is_marker(entry: &IrreversibleEntry) -> bool {
    entry.reason == Irreversible::ExternalEffect
        && entry
            .detail
            .as_deref()
            .is_some_and(|detail| detail.starts_with(MARKER_PREFIX))
}

/// Whether `kind` is a command of this module.
#[must_use]
pub fn handles(kind: &str) -> bool {
    kind.starts_with("firewall.")
        || kind.starts_with("network.")
        || kind.starts_with("clock.")
        || kind == "windows.cim"
}

/// Opposite of a CIM method, when the class documents one.
#[must_use]
pub fn opposite_method(class: &str, method: &str) -> Option<&'static str> {
    match (class, method) {
        ("Win32_Service", "StartService") => Some("StopService"),
        ("Win32_Service", "StopService") => Some("StartService"),
        ("Win32_NetworkAdapter" | "Win32_PnPEntity", "Enable") => Some("Disable"),
        ("Win32_NetworkAdapter" | "Win32_PnPEntity", "Disable") => Some("Enable"),
        ("Win32_Printer", "Pause") => Some("Resume"),
        ("Win32_Printer", "Resume") => Some("Pause"),
        _ => None,
    }
}

fn plain(text: &str) -> bool {
    !text.contains(['"', '\\', '\n', '\r'])
}

/// Programs that undo `kind`, or `None` when `kind` is not handled here.
#[must_use]
pub fn derive(kind: &str, changes: &[Change], markers: &[IrreversibleEntry]) -> Option<Undo> {
    if !handles(kind) {
        return None;
    }
    let detail = markers
        .iter()
        .find(|entry| is_marker(entry))
        .and_then(|entry| entry.detail.as_deref())
        .unwrap_or_default();
    let mut programs = Vec::new();
    for change in changes {
        match one(kind, change, detail) {
            Ok(Some(program)) => programs.push(program),
            Ok(None) => {}
            Err(reason) => return Some(Undo::Irreversible(reason)),
        }
    }
    if kind == "clock.set" {
        return Some(Undo::Irreversible(
            "the previous reading of the clock is lost when it is set".to_owned(),
        ));
    }
    Some(Undo::Programs(programs))
}

fn one(kind: &str, change: &Change, detail: &str) -> Result<Option<String>, String> {
    let path = change.path.as_str();
    let before = change.before.as_deref();
    let after = change.after.as_deref();
    if let Some(key) = path.strip_prefix(FIREWALL_PREFIX) {
        let rule = Rule::parse_key(key)
            .ok_or_else(|| format!("the rule `{key}` cannot be written in a program"))?;
        let mut parts = detail
            .strip_prefix(MARKER_PREFIX)
            .unwrap_or_default()
            .split(':');
        let (_, backend, table) = (parts.next(), parts.next(), parts.next());
        let table = backend
            .filter(|name| *name == "nftables")
            .map(|_| table.unwrap_or(DEFAULT_TABLE));
        return Ok(match (kind, before, after) {
            ("firewall.allow" | "firewall.deny", Some("absent"), Some("present")) => {
                Some(rule.program("remove", backend, table))
            }
            ("firewall.remove", Some("present"), Some("absent")) => {
                let verb = match rule.verdict {
                    Verdict::Allow => "allow",
                    Verdict::Deny => "deny",
                };
                Some(rule.program(verb, backend, table))
            }
            _ => None,
        });
    }
    if let Some(destination) = path.strip_prefix(ROUTE_PREFIX) {
        let state = after.or(before).unwrap_or_default();
        let route = Route::from_state(destination, state)
            .filter(|route| plain(&route.destination))
            .ok_or_else(|| {
                format!("the route to `{destination}` cannot be written in a program")
            })?;
        return match (kind, before, after) {
            ("network.route_add", None, Some(_)) => Ok(Some(route.program("network.route_remove"))),
            ("network.route_remove", Some(_), None) => {
                if route.gateway.is_none() && route.interface.is_none() {
                    Err(format!(
                        "the route to `{destination}` has neither a gateway nor an interface"
                    ))
                } else {
                    Ok(Some(route.program("network.route_add")))
                }
            }
            _ => Ok(None),
        };
    }
    if let Some(interface) = path.strip_prefix(DNS_PREFIX) {
        let state = before
            .and_then(parse_dns_text)
            .filter(|_| plain(interface))
            .ok_or_else(|| {
                format!("the DNS state of `{interface}` cannot be written in a program")
            })?;
        return Ok(Some(state.program(interface)));
    }
    if path == TIMEZONE_TARGET {
        return match before {
            Some(zone) if !zone.is_empty() && plain(zone) => {
                Ok(Some(format!("clock.timezone_set \"{zone}\"")))
            }
            _ => Err("the previous time zone is unknown".to_owned()),
        };
    }
    if path == NTP_TARGET {
        return match before {
            Some(state @ ("on" | "off")) => Ok(Some(format!("clock.ntp state: {state}"))),
            _ => Err("the previous state of network time is unknown".to_owned()),
        };
    }
    if let Some(call) = path.strip_prefix(CIM_PREFIX) {
        return cim(call, after);
    }
    Ok(None)
}

fn cim(call: &str, state: Option<&str>) -> Result<Option<String>, String> {
    let (class, method) = call
        .split_once('.')
        .ok_or_else(|| format!("the call `{call}` cannot be written in a program"))?;
    let opposite = opposite_method(class, method)
        .ok_or_else(|| format!("`{class}.{method}` has no documented opposite"))?;
    let mut namespace = "root/cimv2";
    let mut condition = "";
    for part in state.unwrap_or_default().split(';') {
        if let Some(value) = part.strip_prefix("namespace=") {
            namespace = value;
        } else if let Some(value) = part.strip_prefix("where=") {
            condition = value;
        }
    }
    if condition.is_empty() || !plain(condition) || !plain(namespace) {
        return Err(format!(
            "`{class}.{method}` was called without a condition that selects the instances again"
        ));
    }
    Ok(Some(format!(
        "windows.cim \"{class}\" where: \"{condition}\" namespace: \"{namespace}\" invoke: \"{opposite}\""
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::record::ChangeAction;

    fn change(path: &str, before: Option<&str>, after: Option<&str>) -> Change {
        let mut change = Change::new(path.to_owned(), ChangeAction::State);
        change.before = before.map(str::to_owned);
        change.after = after.map(str::to_owned);
        change
    }

    fn marker(detail: &str) -> IrreversibleEntry {
        let mut entry = IrreversibleEntry::new(Irreversible::ExternalEffect);
        entry.detail = Some(format!("{MARKER_PREFIX}{detail}"));
        entry
    }

    fn programs(undo: Option<Undo>) -> Vec<String> {
        match undo {
            Some(Undo::Programs(programs)) => programs,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn firewall_add_is_undone_by_remove_in_the_same_backend() {
        let undo = derive(
            "firewall.allow",
            &[change(
                "firewall:allow tcp 22 in any -",
                Some("absent"),
                Some("present"),
            )],
            &[marker("firewall:nftables:nurt")],
        );
        assert_eq!(
            programs(undo),
            [
                "firewall.remove \"22\" protocol: tcp direction: in action: allow backend: nftables table: \"nurt\""
            ]
        );
    }

    #[test]
    fn firewall_remove_is_undone_by_the_verdict_of_the_rule() {
        let undo = derive(
            "firewall.remove",
            &[change(
                "firewall:deny udp 53 out 10.0.0.1 -",
                Some("present"),
                Some("absent"),
            )],
            &[marker("firewall:ufw:nurt")],
        );
        assert_eq!(
            programs(undo),
            ["firewall.deny \"53\" protocol: udp direction: out source: \"10.0.0.1\" backend: ufw"]
        );
    }

    #[test]
    fn routes_are_undone_by_the_opposite_command() {
        let state = "via=192.168.1.5 dev=eth0 metric=20";
        assert_eq!(
            programs(derive(
                "network.route_add",
                &[change("route:10.9.0.0/16", None, Some(state))],
                &[]
            )),
            [
                "network.route_remove \"10.9.0.0/16\" gateway: \"192.168.1.5\" interface: \"eth0\" metric: 20"
            ]
        );
        assert_eq!(
            programs(derive(
                "network.route_remove",
                &[change("route:10.9.0.0/16", Some(state), None)],
                &[]
            )),
            [
                "network.route_add \"10.9.0.0/16\" gateway: \"192.168.1.5\" interface: \"eth0\" metric: 20"
            ]
        );
    }

    #[test]
    fn dns_clock_and_ntp_restore_the_previous_value() {
        assert_eq!(
            programs(derive(
                "network.dns_set",
                &[change(
                    "dns:eth0",
                    Some("servers=10.0.0.1;search=a.org"),
                    Some("servers=1.1.1.1;search=a.org")
                )],
                &[],
            )),
            ["network.dns_set [\"10.0.0.1\"] search: [\"a.org\"] interface: \"eth0\""]
        );
        assert_eq!(
            programs(derive(
                "clock.timezone_set",
                &[change("clock:timezone", Some("Europe/Warsaw"), Some("UTC"))],
                &[]
            )),
            ["clock.timezone_set \"Europe/Warsaw\""]
        );
        assert_eq!(
            programs(derive(
                "clock.ntp",
                &[change("clock:ntp", Some("on"), Some("off"))],
                &[]
            )),
            ["clock.ntp state: on"]
        );
    }

    #[test]
    fn setting_the_clock_is_irreversible() {
        assert!(matches!(
            derive(
                "clock.set",
                &[change("clock:time", None, Some("2026-11-21T09:12:00Z"))],
                &[]
            ),
            Some(Undo::Irreversible(_))
        ));
    }

    #[test]
    fn cim_methods_with_an_opposite_are_undone() {
        let state = "namespace=root/cimv2;where=Name='Spooler'";
        assert_eq!(
            programs(derive(
                "windows.cim",
                &[change("cim:Win32_Service.StopService", None, Some(state))],
                &[],
            )),
            [
                "windows.cim \"Win32_Service\" where: \"Name='Spooler'\" namespace: \"root/cimv2\" invoke: \"StartService\""
            ]
        );
        assert!(matches!(
            derive(
                "windows.cim",
                &[change("cim:Win32_Process.Create", None, Some(state))],
                &[]
            ),
            Some(Undo::Irreversible(_))
        ));
        assert!(matches!(
            derive(
                "windows.cim",
                &[change(
                    "cim:Win32_Service.StopService",
                    None,
                    Some("namespace=root/cimv2;where=")
                )],
                &[]
            ),
            Some(Undo::Irreversible(_))
        ));
    }

    #[test]
    fn other_commands_are_not_handled() {
        assert_eq!(derive("start", &[], &[]), None);
        assert!(is_marker(&marker("clock.set")));
        assert!(!is_marker(&IrreversibleEntry::new(
            Irreversible::ExternalEffect
        )));
    }
}
