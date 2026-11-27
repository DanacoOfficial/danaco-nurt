// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-core::inverse
// Responsibility:   Inverse operations of changes outside the file system.
// Dependencies:     nurt_contracts, nurt_sys, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Inverse operations of changes outside the file system.
//!
//! Packages and services are not files, so the journal keeps no content to
//! restore. The report of such a change records the state before and after
//! (`before` and `after` of every change); from it the engine derives the
//! programs that restore the state and keeps them, with the number of the
//! journal entry, in the file `inverse-operations.jsonl` of its state
//! directory. `undo` of the entry runs the programs through the same engine,
//! so an elevated entry is undone by the elevated process that wrote it.
//!
//! | Operation | Recorded state | Undo |
//! |---|---|---|
//! | `package.install` | version before, absent when not installed | `package.remove`; reinstall of the recorded version when the version changed |
//! | `package.remove` | installed version | `package.install` of that version |
//! | `package.upgrade` | versions before | `package.install` of the recorded versions |
//! | `start`, `stop` | state before | the opposite command |
//! | `enable`, `disable` | start type before | the opposite command |
//! | `restart` | none | irreversible |
//! | `accounts.*`, `groups.*`, `mount`, `unmount`, `sysctl.set`, `modules.*`, `power.*`, `disks.*` | states before and after of the account, group, mount point, parameter or module | the inverse of `nurt_sys::admin::inverse` |

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nurt_contracts::bound::BoundDocument;
use nurt_contracts::envelope::{Request, Response};
use nurt_contracts::report::{Irreversible, IrreversibleEntry, Report, ReportStatus};
use nurt_contracts::stage::{CancelToken, EventSink};
use nurt_sys::admin::undo as admin;
use serde::{Deserialize, Serialize};

use crate::elevated::entry_id;
use crate::engine::{Engine, Prepared};

/// Whether the package manager `manager` still offers `version` of
/// `package`; true only when the manager was asked and said so.
fn repository_offers(manager: &str, package: &str, version: &str) -> bool {
    let (program, args): (&str, Vec<&str>) = match manager {
        "apt" => ("apt-cache", vec!["madison", package]),
        "dnf" => ("dnf", vec!["-q", "list", "--showduplicates", package]),
        _ => return false,
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    let program = program.to_owned();
    let args: Vec<String> = args.into_iter().map(str::to_owned).collect();
    std::thread::spawn(move || {
        let output = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output();
        let _ = sender.send(output);
    });
    receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .ok()
        .and_then(Result::ok)
        .is_some_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(|line| line.split_whitespace().any(|word| word == version))
        })
}

/// File of the inverse operations in the state directory.
pub const INVERSE_FILE: &str = "inverse-operations.jsonl";

const PACKAGE_PREFIX: &str = "package:";
const SERVICE_PREFIX: &str = "service:";

/// The programs that undo one journal entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InverseOperation {
    /// Number of the journal entry.
    pub op: u64,
    /// Kind of the operation that is undone.
    pub kind: String,
    /// Programs that restore the state, run one after another.
    pub programs: Vec<String>,
    /// The entry was undone; set by the last line of an entry.
    #[serde(default)]
    pub undone: bool,
    /// The change ran with the rights of the administrator, so its inverse
    /// programs do too.
    #[serde(default)]
    pub elevated: bool,
}

/// What the report of a change leaves to undo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Derived {
    /// Nothing was changed or the kind has no inverse.
    None,
    /// Programs that restore the state.
    Programs(Vec<String>),
    /// The change cannot be undone, with the reason.
    Irreversible(String),
    /// Part of the change is undone by `programs`; `irreversible` names the
    /// packages that no program restores, each with its reason.
    Partial {
        /// Programs that restore part of the state.
        programs: Vec<String>,
        /// Reasons, one per package that stays irreversible.
        irreversible: Vec<String>,
    },
}

/// Whether `text` can be written as a quoted text of the language without
/// escapes: names of packages and services, versions.
fn plain(text: &str) -> bool {
    !text.is_empty()
        && text.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | ':' | '@' | '-' | '~' | '=')
        })
}

fn quoted(text: &str) -> Option<String> {
    plain(text).then(|| format!("\"{text}\""))
}

fn manager_of(report: &Report) -> Option<String> {
    report
        .irreversible
        .iter()
        .find(|entry| entry.reason == Irreversible::PackageChanged)
        .and_then(|entry| entry.detail.clone())
        .filter(|name| plain(name))
}

fn package_program(
    command: &str,
    name: &str,
    version: Option<&str>,
    manager: Option<&str>,
) -> Option<String> {
    let mut program = format!("{command} [{}]", quoted(name)?);
    if let Some(version) = version {
        program.push_str(&format!(" version: {}", quoted(version)?));
    }
    if let Some(manager) = manager {
        program.push_str(&format!(" manager: {manager}"));
    }
    Some(program)
}

fn service_program(name: &str, command: &str) -> Option<String> {
    Some(format!("services name: {} |> {command}", quoted(name)?))
}

/// Derives the inverse of a change from its report.
///
/// The report must be the complete report of a committed change with an
/// entry number; a page of a longer result, a preview, a failed change and a
/// change without entry give [`Derived::None`].
#[must_use]
pub fn derive(report: &Report) -> Derived {
    derive_with(report, &|_, _, _| true)
}

/// Like [`derive()`], asking `offers(manager, package, version)` whether the
/// repository still offers a version that an inverse program must install;
/// a version that it does not offer makes the entry irreversible.
#[must_use]
pub fn derive_with(report: &Report, offers: &dyn Fn(&str, &str, &str) -> bool) -> Derived {
    if report.op.is_none()
        || report.status != ReportStatus::Ok
        || report.next_cursor.is_some()
        || report.changes.is_empty()
    {
        return Derived::None;
    }
    match report.kind.as_str() {
        "package.install" | "package.remove" | "package.upgrade" => derive_packages(report, offers),
        "start" | "stop" | "enable" | "disable" => derive_services(report),
        "restart" => Derived::Irreversible("a restart has no inverse".to_owned()),
        kind if admin::handles(kind) => derive_admin(report),
        _ => derive_administration(report),
    }
}

/// Inverse of an administration command (accounts, groups, mounts, kernel,
/// power, disks); the states that the report recorded decide the programs.
fn derive_administration(report: &Report) -> Derived {
    let changes: Vec<(&str, Option<&str>, Option<&str>)> = report
        .changes
        .iter()
        .map(|change| {
            (
                change.path.as_str(),
                change.before.as_deref(),
                change.after.as_deref(),
            )
        })
        .collect();
    match nurt_sys::admin::inverse(&report.kind, &changes) {
        None | Some(nurt_sys::admin::Inverse::None) => Derived::None,
        Some(nurt_sys::admin::Inverse::Programs(programs)) => Derived::Programs(programs),
        Some(nurt_sys::admin::Inverse::Irreversible(reason)) => Derived::Irreversible(reason),
    }
}

/// Inverse of a firewall, network, clock or CIM command, derived by
/// `nurt-sys` from the recorded states.
fn derive_admin(report: &Report) -> Derived {
    match admin::derive(&report.kind, &report.changes, &report.irreversible) {
        Some(admin::Undo::Programs(programs)) if !programs.is_empty() => {
            Derived::Programs(programs)
        }
        Some(admin::Undo::Irreversible(reason)) => Derived::Irreversible(reason),
        _ => Derived::None,
    }
}

/// Whether `marker` is dropped from the report when the inverse is kept.
fn is_inverse_marker(marker: &IrreversibleEntry) -> bool {
    matches!(
        marker.reason,
        Irreversible::PackageChanged | Irreversible::ServiceChanged
    ) || admin::is_marker(marker)
}

fn derive_packages(report: &Report, offers: &dyn Fn(&str, &str, &str) -> bool) -> Derived {
    let manager = manager_of(report);
    let mut programs = Vec::new();
    let mut lost: Vec<String> = Vec::new();
    for change in &report.changes {
        let Some(name) = change.path.as_str().strip_prefix(PACKAGE_PREFIX) else {
            continue;
        };
        let before = change.before.as_deref();
        let after = change.after.as_deref();
        let reinstalled = match (report.kind.as_str(), before, after) {
            ("package.install" | "package.upgrade", Some(was), Some(now)) if was != now => {
                Some(was)
            }
            ("package.remove", Some(was), None) => Some(was),
            _ => None,
        };
        if let Some(version) = reinstalled
            && !offers(manager.as_deref().unwrap_or(""), name, version)
        {
            lost.push(format!(
                "the repository no longer offers version {version} of `{name}`"
            ));
            continue;
        }
        let program = match (report.kind.as_str(), before, after) {
            ("package.install", None, Some(_)) => {
                package_program("package.remove", name, None, manager.as_deref())
            }
            ("package.install" | "package.upgrade", Some(was), Some(now)) if was != now => {
                package_program("package.install", name, Some(was), manager.as_deref())
            }
            ("package.remove", Some(was), None) => {
                package_program("package.install", name, Some(was), manager.as_deref())
            }
            _ => continue,
        };
        match program {
            Some(program) => programs.push(program),
            None => {
                return Derived::Irreversible(format!(
                    "the name or the version of `{name}` cannot be written in a program"
                ));
            }
        }
    }
    match (programs.is_empty(), lost.is_empty()) {
        (true, true) => Derived::None,
        (true, false) => Derived::Irreversible(lost.join("; ")),
        (false, true) => Derived::Programs(programs),
        (false, false) => Derived::Partial {
            programs,
            irreversible: lost,
        },
    }
}

fn derive_services(report: &Report) -> Derived {
    let mut programs = Vec::new();
    for change in &report.changes {
        let Some(name) = change.path.as_str().strip_prefix(SERVICE_PREFIX) else {
            continue;
        };
        let (Some(before), Some(after)) = (change.before.as_deref(), change.after.as_deref())
        else {
            continue;
        };
        if before == after {
            continue;
        }
        let command = match (report.kind.as_str(), before) {
            ("start", "stopped") => "stop",
            ("stop", "running") => "start",
            ("enable", "disabled") => "disable",
            ("disable", "automatic") => "enable",
            (kind, state) => {
                return Derived::Irreversible(format!(
                    "`{kind}` of `{name}` changed `{state}`, which no command restores"
                ));
            }
        };
        match service_program(name, command) {
            Some(program) => programs.push(program),
            None => {
                return Derived::Irreversible(format!(
                    "the name `{name}` cannot be written in a program"
                ));
            }
        }
    }
    if programs.is_empty() {
        Derived::None
    } else {
        Derived::Programs(programs)
    }
}

/// File of the inverse operations of one state directory.
#[derive(Debug, Clone)]
pub(crate) struct InverseLog {
    path: PathBuf,
}

impl InverseLog {
    /// The log in the state directory `root`.
    pub(crate) fn in_state(root: &Path) -> Self {
        Self {
            path: root.join(INVERSE_FILE),
        }
    }

    /// Appends a line.
    pub(crate) fn append(&self, operation: &InverseOperation) -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let mut line = serde_json::to_string(operation)?;
        line.push('\n');
        file.write_all(line.as_bytes())
    }

    /// The newest line of entry `op`, when the entry was not undone.
    pub(crate) fn find(&self, op: u64) -> Option<InverseOperation> {
        let file = std::fs::File::open(&self.path).ok()?;
        BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| serde_json::from_str::<InverseOperation>(&line).ok())
            .filter(|operation| operation.op == op)
            .last()
            .filter(|operation| !operation.undone)
    }
}

impl Engine {
    /// Keeps the programs that undo a committed change outside the file
    /// system, and removes from the report the reasons that the journal gave
    /// for the entry being irreversible.
    pub(crate) fn keep_inverse(&self, report: &mut Report) {
        let (Some(log), Some(op)) = (&self.inner.inverses, report.op) else {
            return;
        };
        let (programs, lost) = match derive_with(report, &repository_offers) {
            Derived::Programs(programs) => (programs, Vec::new()),
            Derived::Partial {
                programs,
                irreversible,
            } => (programs, irreversible),
            Derived::Irreversible(reason) => {
                for marker in &mut report.irreversible {
                    if is_inverse_marker(marker) {
                        marker.detail = Some(reason.clone());
                    }
                }
                return;
            }
            Derived::None => return,
        };
        let entry = InverseOperation {
            op,
            kind: report.kind.clone(),
            programs,
            undone: false,
            elevated: report.elevated,
        };
        if log.append(&entry).is_ok() {
            report
                .irreversible
                .retain(|entry| !is_inverse_marker(entry));
            for reason in lost {
                let mut marker = IrreversibleEntry::new(Irreversible::PackageChanged);
                marker.detail = Some(reason);
                report.irreversible.push(marker);
            }
        }
    }

    /// Runs the programs that undo the entry named by an `undo` of the
    /// program, when the engine kept them; the response is that of the last
    /// program, or of the first that failed.
    pub(crate) fn undo_by_inverse(
        &self,
        request: &Request,
        prepared: &Prepared,
        bound: &BoundDocument,
        cancel: &CancelToken,
        events: Option<Arc<dyn EventSink>>,
    ) -> Option<Response> {
        let log = self.inner.inverses.as_ref()?;
        let op = entry_id(bound)?;
        let entry = log.find(op)?;
        let mut last = None;
        for program in &entry.programs {
            let mut inner = Request::program(program.clone()).with_options(request.options.clone());
            let response = if entry.elevated && prepared.elevation.is_none() {
                inner.options.privilege = Some(nurt_contracts::envelope::Privilege::Elevated);
                self.route(&inner, cancel, events.clone())
            } else {
                self.execute_local(
                    &inner,
                    cancel,
                    events.clone(),
                    prepared.elevation.clone(),
                    None,
                )
            };
            if response.report.status != ReportStatus::Ok
                && response.report.status != ReportStatus::Planned
            {
                return Some(response);
            }
            last = Some(response);
        }
        if !prepared.options.preview() {
            let done = InverseOperation {
                undone: true,
                ..entry
            };
            let _ = log.append(&done);
        }
        last
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::record::{Change, ChangeAction};
    use nurt_contracts::report::IrreversibleEntry;

    use super::*;

    fn report(kind: &str, changes: &[(&str, Option<&str>, Option<&str>)]) -> Report {
        let mut report = Report::new(ReportStatus::Ok, kind);
        report.op = Some(4);
        for (path, before, after) in changes {
            let mut change = Change::new(*path, ChangeAction::State);
            change.before = before.map(str::to_owned);
            change.after = after.map(str::to_owned);
            report.changes.push(change);
        }
        report
    }

    fn programs(derived: Derived) -> Vec<String> {
        match derived {
            Derived::Programs(programs) => programs,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_firewall_rule_is_undone_by_removing_it() {
        let mut report = report(
            "firewall.allow",
            &[(
                "firewall:allow tcp 22 in any -",
                Some("absent"),
                Some("present"),
            )],
        );
        let mut marker = IrreversibleEntry::new(Irreversible::ExternalEffect);
        marker.detail = Some("admin:firewall:nftables:nurt".to_owned());
        assert!(is_inverse_marker(&marker));
        report.irreversible.push(marker);
        assert_eq!(
            programs(derive(&report)),
            [
                "firewall.remove \"22\" protocol: tcp direction: inbound action: allow backend: nftables table: \"nurt\""
            ]
        );
    }

    #[test]
    fn setting_the_clock_stays_irreversible_and_other_effects_are_left_alone() {
        let report_of = |kind: &str, path: &str| report(kind, &[(path, None, Some("x"))]);
        assert!(matches!(
            derive(&report_of("clock.set", "clock:time")),
            Derived::Irreversible(_)
        ));
        let plain = IrreversibleEntry::new(Irreversible::ExternalEffect);
        assert!(!is_inverse_marker(&plain));
    }

    #[test]
    fn install_of_a_new_package_is_undone_by_remove() {
        let mut report = report("package.install", &[("package:jq", None, Some("1.7"))]);
        let mut marker = IrreversibleEntry::new(Irreversible::PackageChanged);
        marker.detail = Some("apt".to_owned());
        report.irreversible.push(marker);
        assert_eq!(
            programs(derive(&report)),
            ["package.remove [\"jq\"] manager: apt"]
        );
    }

    #[test]
    fn a_version_that_the_repository_no_longer_offers_is_irreversible() {
        let mut report = report("package.remove", &[("package:jq", Some("1.7"), None)]);
        let mut marker = IrreversibleEntry::new(Irreversible::PackageChanged);
        marker.detail = Some("apt".to_owned());
        report.irreversible.push(marker);
        assert!(matches!(
            derive_with(&report, &|_, _, _| false),
            Derived::Irreversible(_)
        ));
        assert_eq!(
            programs(derive_with(&report, &|manager, name, version| {
                manager == "apt" && name == "jq" && version == "1.7"
            })),
            ["package.install [\"jq\"] version: \"1.7\" manager: apt"]
        );
    }

    #[test]
    fn an_upgrade_loses_only_the_packages_that_are_no_longer_offered() {
        let mut report = report(
            "package.upgrade",
            &[
                ("package:a", Some("1"), Some("2")),
                ("package:b", Some("5"), Some("6")),
            ],
        );
        let mut marker = IrreversibleEntry::new(Irreversible::PackageChanged);
        marker.detail = Some("apt".to_owned());
        report.irreversible.push(marker);
        match derive_with(&report, &|_, name, _| name == "a") {
            Derived::Partial {
                programs,
                irreversible,
            } => {
                assert_eq!(
                    programs,
                    ["package.install [\"a\"] version: \"1\" manager: apt"]
                );
                assert_eq!(irreversible.len(), 1);
                assert!(irreversible[0].contains("`b`"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn install_that_changed_the_version_reinstalls_the_recorded_one() {
        let report = report(
            "package.install",
            &[("package:jq", Some("1.6"), Some("1.7"))],
        );
        assert_eq!(
            programs(derive(&report)),
            ["package.install [\"jq\"] version: \"1.6\""]
        );
    }

    #[test]
    fn install_that_changed_nothing_leaves_nothing_to_undo() {
        let report = report(
            "package.install",
            &[("package:jq", Some("1.7"), Some("1.7"))],
        );
        assert_eq!(derive(&report), Derived::None);
    }

    #[test]
    fn remove_is_undone_by_install_of_the_recorded_version() {
        let report = report("package.remove", &[("package:jq", Some("1.7"), None)]);
        assert_eq!(
            programs(derive(&report)),
            ["package.install [\"jq\"] version: \"1.7\""]
        );
    }

    #[test]
    fn upgrade_restores_the_versions_of_every_package() {
        let report = report(
            "package.upgrade",
            &[
                ("package:a", Some("1"), Some("2")),
                ("package:b", Some("5"), Some("5")),
                ("package:c", Some("3.1"), Some("3.2")),
            ],
        );
        assert_eq!(
            programs(derive(&report)),
            [
                "package.install [\"a\"] version: \"1\"",
                "package.install [\"c\"] version: \"3.1\""
            ]
        );
    }

    #[test]
    fn a_name_that_needs_escapes_is_irreversible() {
        let report = report("package.remove", &[("package:we ird", Some("1"), None)]);
        assert!(matches!(derive(&report), Derived::Irreversible(_)));
    }

    #[test]
    fn services_get_the_opposite_command() {
        let started = report(
            "start",
            &[("service:nginx", Some("stopped"), Some("running"))],
        );
        assert_eq!(
            programs(derive(&started)),
            ["services name: \"nginx\" |> stop"]
        );
        let stopped = report(
            "stop",
            &[("service:nginx", Some("running"), Some("stopped"))],
        );
        assert_eq!(
            programs(derive(&stopped)),
            ["services name: \"nginx\" |> start"]
        );
        let enabled = report(
            "enable",
            &[("service:cron", Some("disabled"), Some("automatic"))],
        );
        assert_eq!(
            programs(derive(&enabled)),
            ["services name: \"cron\" |> disable"]
        );
        let disabled = report(
            "disable",
            &[("service:cron", Some("automatic"), Some("disabled"))],
        );
        assert_eq!(
            programs(derive(&disabled)),
            ["services name: \"cron\" |> enable"]
        );
    }

    #[test]
    fn a_service_that_already_held_the_state_has_nothing_to_undo() {
        let report = report(
            "start",
            &[("service:nginx", Some("running"), Some("running"))],
        );
        assert_eq!(derive(&report), Derived::None);
    }

    #[test]
    fn a_start_type_without_a_command_is_irreversible() {
        let report = report(
            "enable",
            &[("service:cron", Some("manual"), Some("automatic"))],
        );
        assert!(matches!(derive(&report), Derived::Irreversible(_)));
    }

    #[test]
    fn administration_commands_derive_their_inverse_from_the_recorded_states() {
        let created = report(
            "accounts.create",
            &[(
                "account:nurt-test-a",
                None,
                Some("{\"groups\":[],\"comment\":\"\",\"locked\":false}"),
            )],
        );
        assert_eq!(
            programs(derive(&created)),
            ["accounts.remove name: \"nurt-test-a\" keep_home: true"]
        );
        let mounted = report(
            "mount",
            &[("mount:/mnt/x", None, Some("{\"device\":\"/dev/loop1\"}"))],
        );
        assert_eq!(programs(derive(&mounted)), ["unmount target: \"/mnt/x\""]);
        let tuned = report(
            "sysctl.set",
            &[("sysctl:vm.swappiness", Some("60"), Some("10"))],
        );
        assert_eq!(
            programs(derive(&tuned)),
            ["sysctl.set key: \"vm.swappiness\" value: \"60\""]
        );
        let formatted = report(
            "disks.format",
            &[("disk:/dev/loop1", Some("[]"), Some("file system ext4"))],
        );
        assert!(matches!(derive(&formatted), Derived::Irreversible(_)));
        let unchanged = report("accounts.create", &[]);
        assert_eq!(derive(&unchanged), Derived::None);
    }

    #[test]
    fn restart_is_irreversible() {
        let report = report(
            "restart",
            &[("service:nginx", Some("running"), Some("running"))],
        );
        assert!(matches!(derive(&report), Derived::Irreversible(_)));
    }

    #[test]
    fn previews_failures_and_other_commands_derive_nothing() {
        let mut preview = report("start", &[("service:a", Some("stopped"), Some("running"))]);
        preview.op = None;
        assert_eq!(derive(&preview), Derived::None);
        let mut failed = report("start", &[("service:a", Some("stopped"), Some("running"))]);
        failed.status = ReportStatus::Failed;
        assert_eq!(derive(&failed), Derived::None);
        let other = report("write", &[("/etc/x", None, None)]);
        assert_eq!(derive(&other), Derived::None);
    }

    #[test]
    fn the_log_returns_the_newest_line_until_the_entry_is_undone() {
        let dir = tempfile::tempdir().unwrap();
        let log = InverseLog::in_state(dir.path());
        let entry = |programs: &[&str], undone| InverseOperation {
            op: 4,
            kind: "start".to_owned(),
            programs: programs.iter().map(|p| (*p).to_owned()).collect(),
            undone,
            elevated: false,
        };
        assert!(log.find(4).is_none());
        log.append(&entry(&["a"], false)).unwrap();
        assert_eq!(log.find(4).unwrap().programs, ["a"]);
        log.append(&entry(&["a"], true)).unwrap();
        assert!(log.find(4).is_none());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
