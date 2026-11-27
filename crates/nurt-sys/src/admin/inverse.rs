// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::admin::inverse
// Responsibility:   The programs that undo an administration command, derived from the.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The programs that undo an administration command, derived from the
//! states that its report recorded.
//!
//! | Command | Recorded state | Undo |
//! |---|---|---|
//! | `accounts.create` | absent before | `accounts.remove` with `keep_home` |
//! | `accounts.remove` | account before | `accounts.create` with the recorded values and `accounts.modify lock` |
//! | `accounts.modify` | account before and after | `accounts.modify` with the recorded values of the fields that changed |
//! | `accounts.password` | none | irreversible |
//! | `groups.create` | absent before | `groups.remove` |
//! | `groups.remove` | group before | `groups.create` and `groups.add_member` of every member |
//! | `groups.add_member`, `groups.remove_member` | members before and after | the opposite command |
//! | `mount` | absent before | `unmount` |
//! | `unmount` | mount before | `mount` with the recorded device, type and options |
//! | `sysctl.set` | value before | `sysctl.set` with that value |
//! | `modules.load`, `modules.unload` | state before | the opposite command |
//! | `power.restart`, `power.shutdown` | schedule | `power.cancel`; an immediate action is irreversible |
//! | `power.cancel`, `disks.partition`, `disks.format` | none | irreversible |

use super::COMMANDS;
use super::accounts::AccountState;
use super::args::Program;
use super::groups::GroupState;
use super::kernel::{LOADED, UNLOADED};
use super::mounts::MountState;
use super::power::{NOW, SCHEDULE};

/// The undo of one report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inverse {
    /// The command changed nothing that has to be undone.
    None,
    /// Programs that restore the state, run one after another.
    Programs(Vec<String>),
    /// The change cannot be undone, with the reason.
    Irreversible(String),
}

/// The undo of the command `kind` that made `changes`, each given as target,
/// state before and state after; `None` when `kind` is not an administration
/// command.
#[must_use]
pub fn inverse(kind: &str, changes: &[(&str, Option<&str>, Option<&str>)]) -> Option<Inverse> {
    if !COMMANDS.contains(&kind) {
        return None;
    }
    if matches!(
        kind,
        "accounts.password" | "power.cancel" | "disks.partition" | "disks.format"
    ) {
        return Some(if changes.is_empty() {
            Inverse::None
        } else {
            Inverse::Irreversible(format!("`{kind}` cannot be undone"))
        });
    }
    let mut programs = Vec::new();
    for (target, before, after) in changes {
        let Some((area, name)) = target.split_once(':') else {
            continue;
        };
        let derived = match (area, kind) {
            ("account", _) => account(kind, name, *before, *after),
            ("group", _) => group(kind, name, *before, *after),
            ("mount", _) => mount(name, *before, *after),
            ("sysctl", _) => sysctl(name, *before, *after),
            ("module", _) => module(name, *before, *after),
            ("power", _) => power(target, *before),
            _ => continue,
        };
        match derived {
            Derived::Programs(found) => programs.extend(found),
            Derived::Skip => {}
            Derived::Irreversible(reason) => return Some(Inverse::Irreversible(reason)),
        }
    }
    Some(if programs.is_empty() {
        Inverse::None
    } else {
        Inverse::Programs(programs)
    })
}

enum Derived {
    Skip,
    Programs(Vec<String>),
    Irreversible(String),
}

fn unwritable(what: &str) -> Derived {
    Derived::Irreversible(format!("`{what}` cannot be written in a program"))
}

fn programs(items: impl IntoIterator<Item = Option<String>>, what: &str) -> Derived {
    let mut out = Vec::new();
    for item in items {
        match item {
            Some(text) => out.push(text),
            None => return unwritable(what),
        }
    }
    Derived::Programs(out)
}

fn account(kind: &str, name: &str, before: Option<&str>, after: Option<&str>) -> Derived {
    match (before, after) {
        (None, Some(_)) => programs(
            [Program::new("accounts.remove")
                .text("name", name)
                .flag("keep_home", true)
                .finish()],
            name,
        ),
        (Some(before), None) => {
            let Some(state) = AccountState::from_text(before) else {
                return Derived::Irreversible(format!("the state of `{name}` is not readable"));
            };
            let mut create = Program::new("accounts.create")
                .text("name", name)
                .text_opt("shell", state.shell.as_deref())
                .text_opt("home", state.home.as_deref())
                .flag("create_home", false);
            if let Some(uid) = state.uid {
                create = create.int("uid", i64::from(uid));
            }
            if !state.groups.is_empty() {
                create = create.list("groups", &state.groups);
            }
            if !state.comment.is_empty() {
                create = create.text("comment", &state.comment);
            }
            let mut items = vec![create.finish()];
            if state.locked {
                items.push(
                    Program::new("accounts.modify")
                        .text("name", name)
                        .flag("lock", true)
                        .finish(),
                );
            }
            programs(items, name)
        }
        (Some(before), Some(after)) if kind == "accounts.modify" => {
            let (Some(old), Some(new)) = (
                AccountState::from_text(before),
                AccountState::from_text(after),
            ) else {
                return Derived::Irreversible(format!("the state of `{name}` is not readable"));
            };
            if old == new {
                return Derived::Skip;
            }
            let mut modify = Program::new("accounts.modify").text("name", name);
            if old.shell != new.shell {
                modify = modify.text_opt("shell", old.shell.as_deref());
            }
            if old.home != new.home {
                modify = modify.text_opt("home", old.home.as_deref());
            }
            if old.comment != new.comment {
                modify = modify.text("comment", &old.comment);
            }
            if old.groups != new.groups {
                modify = modify.list("groups", &old.groups);
            }
            if old.locked != new.locked {
                modify = modify.flag("lock", old.locked);
            }
            programs([modify.finish()], name)
        }
        _ => Derived::Skip,
    }
}

fn group(kind: &str, name: &str, before: Option<&str>, after: Option<&str>) -> Derived {
    match (before, after) {
        (None, Some(_)) => programs(
            [Program::new("groups.remove").text("name", name).finish()],
            name,
        ),
        (Some(before), None) => {
            let Some(state) = GroupState::from_text(before) else {
                return Derived::Irreversible(format!("the state of `{name}` is not readable"));
            };
            let mut create = Program::new("groups.create").text("name", name);
            if let Some(gid) = state.gid {
                create = create.int("gid", i64::from(gid));
            }
            let mut items = vec![create.finish()];
            for member in &state.members {
                items.push(
                    Program::new("groups.add_member")
                        .text("name", name)
                        .text("member", member)
                        .finish(),
                );
            }
            programs(items, name)
        }
        (Some(before), Some(after))
            if matches!(kind, "groups.add_member" | "groups.remove_member") =>
        {
            let (Some(old), Some(new)) =
                (GroupState::from_text(before), GroupState::from_text(after))
            else {
                return Derived::Irreversible(format!("the state of `{name}` is not readable"));
            };
            let mut items = Vec::new();
            for member in new.members.iter().filter(|m| !old.members.contains(m)) {
                items.push(
                    Program::new("groups.remove_member")
                        .text("name", name)
                        .text("member", member)
                        .finish(),
                );
            }
            for member in old.members.iter().filter(|m| !new.members.contains(m)) {
                items.push(
                    Program::new("groups.add_member")
                        .text("name", name)
                        .text("member", member)
                        .finish(),
                );
            }
            programs(items, name)
        }
        _ => Derived::Skip,
    }
}

fn mount(target: &str, before: Option<&str>, after: Option<&str>) -> Derived {
    match (before, after) {
        (None, Some(_)) => programs(
            [Program::new("unmount").text("target", target).finish()],
            target,
        ),
        (Some(before), None) => {
            let Some(state) = MountState::from_text(before) else {
                return Derived::Irreversible(format!("the state of `{target}` is not readable"));
            };
            let mut mount = Program::new("mount")
                .text("device", &state.device)
                .text("target", target);
            if !state.fstype.is_empty() {
                mount = mount.text("type", &state.fstype);
            }
            if !state.options.is_empty() {
                mount = mount.list("options", &state.options);
            }
            programs([mount.finish()], target)
        }
        _ => Derived::Skip,
    }
}

fn sysctl(key: &str, before: Option<&str>, after: Option<&str>) -> Derived {
    match (before, after) {
        (Some(before), Some(after)) if before != after => programs(
            [Program::new("sysctl.set")
                .text("key", key)
                .text("value", before)
                .finish()],
            key,
        ),
        _ => Derived::Skip,
    }
}

fn module(name: &str, before: Option<&str>, after: Option<&str>) -> Derived {
    let command = match (before, after) {
        (Some(UNLOADED), Some(LOADED)) => "modules.unload",
        (Some(LOADED), Some(UNLOADED)) => "modules.load",
        _ => return Derived::Skip,
    };
    programs([Program::new(command).text("name", name).finish()], name)
}

fn power(target: &str, before: Option<&str>) -> Derived {
    if target == NOW {
        return Derived::Irreversible(
            "an immediate restart or shutdown cannot be undone".to_owned(),
        );
    }
    if target == SCHEDULE && before.is_none() {
        return Derived::Programs(vec![
            Program::new("power.cancel").finish().unwrap_or_default(),
        ]);
    }
    Derived::Skip
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
