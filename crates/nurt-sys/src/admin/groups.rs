// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::admin::groups
// Responsibility:   `groups.create`, `groups.remove`, `groups.add_member` and `groups.remove_member`.
// Dependencies:     nurt_contracts, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `groups.create`, `groups.remove`, `groups.add_member` and
//! `groups.remove_member`.
//!
//! | System | Read | Change |
//! |---|---|---|
//! | Linux | `getent group` | `groupadd`, `groupdel`, `gpasswd` |
//! | macOS | `dscl` | `dseditgroup` |
//! | Windows | `net localgroup` | `net localgroup` |

use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::BoundArgs;
use serde::{Deserialize, Serialize};

use super::sys::{Call, Sys};
use super::{Plan, StateChange, args};

/// State of a group as recorded in the journal.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) struct GroupState {
    /// Numeric identifier; absent on Windows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gid: Option<u32>,
    /// Members, sorted.
    #[serde(default)]
    pub members: Vec<String>,
}

impl GroupState {
    /// The JSON document kept as `before` or `after`.
    pub(crate) fn to_text(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// The state from its JSON document.
    pub(crate) fn from_text(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }
}

/// Target of the change of a group.
pub(crate) fn target(name: &str) -> String {
    format!("group:{name}")
}

pub(crate) fn plan(sys: &dyn Sys, args: &BoundArgs) -> Result<Plan> {
    let name = args::required(args, "name")?;
    let windows = sys.os() == Os::Windows;
    args::name("group name", &name, windows)?;
    match args.command() {
        "groups.create" => create(sys, args, &name),
        "groups.remove" => remove(sys, &name),
        command => {
            let member = args::required(args, "member")?;
            args::name("account name", &member, windows)?;
            membership(sys, &name, &member, command == "groups.add_member")
        }
    }
}

fn create(sys: &dyn Sys, args: &BoundArgs, name: &str) -> Result<Plan> {
    let gid = args::number(args, "gid", i64::from(u32::MAX) - 2)?;
    let system = args::flag(args, "system", false)?;
    if read(sys, name)?.is_some() {
        return Err(Error::new(
            ErrorCode::Exists,
            format!("the group `{name}` exists"),
        ));
    }
    let call = match sys.os() {
        Os::Windows => Call::new("net", ["localgroup", name, "/add"]),
        Os::Macos => {
            let mut call = Call::new("dseditgroup", ["-o", "create"]);
            if let Some(gid) = gid {
                call.args.extend(["-i".to_owned(), gid.to_string()]);
            }
            call.args.push(name.to_owned());
            call
        }
        _ => {
            let mut call = Call::new("groupadd", Vec::<String>::new());
            if let Some(gid) = gid {
                call.args.extend(["-g".to_owned(), gid.to_string()]);
            }
            if system {
                call.args.push("-r".to_owned());
            }
            call.args.push(name.to_owned());
            call
        }
    };
    let after = GroupState {
        gid: gid.and_then(|gid| u32::try_from(gid).ok()),
        members: Vec::new(),
    };
    Ok(Plan {
        calls: vec![call],
        changes: vec![StateChange::new(target(name), None, Some(after.to_text()))],
        ..Plan::default()
    })
}

fn remove(sys: &dyn Sys, name: &str) -> Result<Plan> {
    let before = read(sys, name)?.ok_or_else(|| not_found(name))?;
    if before.gid == Some(0) {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "the group with identifier 0 is never removed",
        ));
    }
    let call = match sys.os() {
        Os::Windows => Call::new("net", ["localgroup", name, "/delete"]),
        Os::Macos => Call::new("dseditgroup", ["-o", "delete", name]),
        _ => Call::new("groupdel", [name]),
    };
    Ok(Plan {
        calls: vec![call],
        changes: vec![StateChange::new(target(name), Some(before.to_text()), None)],
        ..Plan::default()
    })
}

fn membership(sys: &dyn Sys, name: &str, member: &str, add: bool) -> Result<Plan> {
    let before = read(sys, name)?.ok_or_else(|| not_found(name))?;
    let present = before.members.iter().any(|m| m == member);
    if present == add {
        let state = if add {
            "already a member"
        } else {
            "not a member"
        };
        return Ok(Plan {
            unchanged: Some(format!("`{member}` is {state} of `{name}`")),
            ..Plan::default()
        });
    }
    let mut after = before.clone();
    if add {
        after.members.push(member.to_owned());
        after.members.sort();
    } else {
        after.members.retain(|m| m != member);
    }
    let flag = if add { "-a" } else { "-d" };
    let call = match sys.os() {
        Os::Windows => Call::new(
            "net",
            [
                "localgroup",
                name,
                member,
                if add { "/add" } else { "/delete" },
            ],
        ),
        Os::Macos => Call::new(
            "dseditgroup",
            ["-o", "edit", flag, member, "-t", "user", name],
        ),
        _ => Call::new("gpasswd", [flag, member, name]),
    };
    Ok(Plan {
        calls: vec![call],
        changes: vec![StateChange::new(
            target(name),
            Some(before.to_text()),
            Some(after.to_text()),
        )],
        ..Plan::default()
    })
}

fn not_found(name: &str) -> Error {
    Error::new(ErrorCode::NotFound, format!("there is no group `{name}`"))
}

/// Reads the group `name`; `None` when it does not exist.
pub(crate) fn read(sys: &dyn Sys, name: &str) -> Result<Option<GroupState>> {
    match sys.os() {
        Os::Windows => {
            let output = sys.run(&Call::new("net", ["localgroup", name]))?;
            Ok(output.ok().then(|| parse_net_localgroup(&output.stdout)))
        }
        Os::Macos => {
            let output = sys.run(&Call::new(
                "dscl",
                [
                    ".",
                    "-read",
                    &format!("/Groups/{name}"),
                    "PrimaryGroupID",
                    "GroupMembership",
                ],
            ))?;
            Ok(output.ok().then(|| parse_dscl_group(&output.stdout)))
        }
        _ => {
            let output = sys.run(&Call::new("getent", ["group", name]))?;
            if !output.ok() {
                return Ok(None);
            }
            Ok(parse_group_line(output.stdout.trim()))
        }
    }
}

/// A line of `getent group`: `name:x:gid:member,member`.
pub(crate) fn parse_group_line(line: &str) -> Option<GroupState> {
    let fields: Vec<&str> = line.split(':').collect();
    if fields.len() < 4 {
        return None;
    }
    let mut members: Vec<String> = fields[3]
        .split(',')
        .filter(|member| !member.is_empty())
        .map(str::to_owned)
        .collect();
    members.sort();
    Some(GroupState {
        gid: fields[2].parse().ok(),
        members,
    })
}

/// Output of `dscl . -read /Groups/NAME PrimaryGroupID GroupMembership`.
pub(crate) fn parse_dscl_group(output: &str) -> GroupState {
    let mut state = GroupState::default();
    for line in output.lines() {
        if let Some(value) = line.strip_prefix("PrimaryGroupID:") {
            state.gid = value.trim().parse().ok();
        } else if let Some(value) = line.strip_prefix("GroupMembership:") {
            state.members = value.split_whitespace().map(str::to_owned).collect();
        }
    }
    state.members.sort();
    state
}

/// Output of `net localgroup NAME`: the members are listed between the line
/// of dashes and the line `The command completed successfully.`.
pub(crate) fn parse_net_localgroup(output: &str) -> GroupState {
    let mut members = Vec::new();
    let mut listing = false;
    for line in output.lines() {
        if line.starts_with("---") {
            listing = true;
        } else if listing {
            if line.starts_with("The command") || line.trim().is_empty() {
                break;
            }
            members.push(line.trim().to_owned());
        }
    }
    members.sort();
    GroupState { gid: None, members }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
