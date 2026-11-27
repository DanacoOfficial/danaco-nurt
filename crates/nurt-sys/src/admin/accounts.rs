// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::admin::accounts
// Responsibility:   `accounts.create`, `accounts.modify`, `accounts.remove` and.
// Dependencies:     nurt_contracts, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `accounts.create`, `accounts.modify`, `accounts.remove` and
//! `accounts.password`.
//!
//! | System | Read | Change |
//! |---|---|---|
//! | Linux | `getent passwd`, `id`, `passwd -S` | `useradd`, `usermod`, `userdel`, `chpasswd` |
//! | macOS | `dscl`, `id` | `sysadminctl`, `dseditgroup`, `pwpolicy`, `dscl` |
//! | Windows | `net user` | `net user`, `net localgroup`, `Set-LocalUser` |
//!
//! The home directory of a removed account is not deleted by the system
//! program: it is removed through the journal, so that `undo` restores it.

use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::FileOp;
use nurt_contracts::stage::BoundArgs;
use serde::{Deserialize, Serialize};

use super::sys::{Call, Sys};
use super::{Plan, StateChange, args};

/// State of an account as recorded in the journal.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) struct AccountState {
    /// Numeric identifier; absent on Windows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<u32>,
    /// Login shell; absent on Windows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// Home directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    /// Groups other than the primary one.
    #[serde(default)]
    pub groups: Vec<String>,
    /// Comment or full name.
    #[serde(default)]
    pub comment: String,
    /// The account is locked.
    #[serde(default)]
    pub locked: bool,
}

impl AccountState {
    /// The JSON document kept as `before` or `after`.
    pub(crate) fn to_text(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// The state from its JSON document.
    pub(crate) fn from_text(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }
}

/// Target of the change of an account.
pub(crate) fn target(name: &str) -> String {
    format!("account:{name}")
}

pub(crate) fn plan(sys: &dyn Sys, args: &BoundArgs) -> Result<Plan> {
    let name = args::required(args, "name")?;
    args::name("account name", &name, sys.os() == Os::Windows)?;
    match args.command() {
        "accounts.create" => create(sys, args, &name),
        "accounts.modify" => modify(sys, args, &name),
        "accounts.remove" => remove(sys, args, &name),
        _ => password(sys, args, &name),
    }
}

fn texts(args: &BoundArgs) -> Result<(Option<String>, Option<String>, Option<String>)> {
    let shell = args::text(args, "shell")?;
    let home = args::text(args, "home")?;
    let comment = args::text(args, "comment")?;
    if let Some(shell) = &shell {
        args::operand("shell", shell)?;
    }
    if let Some(home) = &home {
        args::operand("home directory", home)?;
    }
    if let Some(comment) = &comment
        && (comment.chars().any(char::is_control) || comment.contains(':'))
    {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "`comment` must not hold a control character or `:`",
        ));
    }
    Ok((shell, home, comment))
}

fn groups_arg(args: &BoundArgs) -> Result<Option<Vec<String>>> {
    let groups = args::list(args, "groups")?;
    if let Some(groups) = &groups {
        for group in groups {
            args::name("group name", group, true)?;
        }
    }
    Ok(groups)
}

/// The values of an account to create.
struct Wanted<'a> {
    name: &'a str,
    uid: Option<i64>,
    shell: Option<&'a str>,
    home: Option<&'a str>,
    comment: Option<&'a str>,
    groups: &'a [String],
    system: bool,
    create_home: bool,
}

fn create_windows(w: &Wanted<'_>) -> Vec<Call> {
    let mut call = Call::new("net", ["user", w.name, "/add"]);
    if let Some(comment) = w.comment {
        call.args.push(format!("/fullname:{comment}"));
    }
    if let Some(home) = w.home {
        call.args.push(format!("/homedir:{home}"));
    }
    let mut calls = vec![call];
    for group in w.groups {
        calls.push(Call::new("net", ["localgroup", group, w.name, "/add"]));
    }
    calls
}

fn create_macos(w: &Wanted<'_>) -> Vec<Call> {
    let mut call = Call::new("sysadminctl", ["-addUser", w.name]);
    if let Some(uid) = w.uid {
        call.args.extend(["-UID".to_owned(), uid.to_string()]);
    }
    for (flag, value) in [
        ("-shell", w.shell),
        ("-home", w.home),
        ("-fullName", w.comment),
    ] {
        if let Some(value) = value {
            call.args.extend([flag.to_owned(), value.to_owned()]);
        }
    }
    let mut calls = vec![call];
    for group in w.groups {
        calls.push(Call::new(
            "dseditgroup",
            ["-o", "edit", "-a", w.name, "-t", "user", group],
        ));
    }
    calls
}

fn create_linux(w: &Wanted<'_>) -> Vec<Call> {
    let mut call = Call::new("useradd", Vec::<String>::new());
    if let Some(uid) = w.uid {
        call.args.extend(["-u".to_owned(), uid.to_string()]);
    }
    for (flag, value) in [("-s", w.shell), ("-d", w.home)] {
        if let Some(value) = value {
            call.args.extend([flag.to_owned(), value.to_owned()]);
        }
    }
    call.args.push(
        if w.create_home && !w.system {
            "-m"
        } else {
            "-M"
        }
        .to_owned(),
    );
    if let Some(comment) = w.comment {
        call.args.extend(["-c".to_owned(), comment.to_owned()]);
    }
    if !w.groups.is_empty() {
        call.args.extend(["-G".to_owned(), w.groups.join(",")]);
    }
    if w.system {
        call.args.push("-r".to_owned());
    }
    call.args.push(w.name.to_owned());
    vec![call]
}

fn create(sys: &dyn Sys, args: &BoundArgs, name: &str) -> Result<Plan> {
    let (shell, home, comment) = texts(args)?;
    let groups = groups_arg(args)?.unwrap_or_default();
    let uid = args::number(args, "uid", i64::from(u32::MAX) - 2)?;
    let system = args::flag(args, "system", false)?;
    let create_home = args::flag(args, "create_home", true)?;
    if read(sys, name)?.is_some() {
        return Err(
            Error::new(ErrorCode::Exists, format!("the account `{name}` exists"))
                .with_fix(format!("accounts.modify name: \"{name}\"")),
        );
    }
    let wanted = Wanted {
        name,
        uid,
        shell: shell.as_deref(),
        home: home.as_deref(),
        comment: comment.as_deref(),
        groups: &groups,
        system,
        create_home,
    };
    let calls = match sys.os() {
        Os::Windows => create_windows(&wanted),
        Os::Macos => create_macos(&wanted),
        _ => create_linux(&wanted),
    };
    let after = AccountState {
        uid: uid.and_then(|uid| u32::try_from(uid).ok()),
        shell,
        home,
        groups,
        comment: comment.unwrap_or_default(),
        locked: false,
    };
    Ok(Plan {
        calls,
        changes: vec![StateChange::new(target(name), None, Some(after.to_text()))],
        ..Plan::default()
    })
}

/// The fields of an account to change.
struct Change<'a> {
    name: &'a str,
    shell: Option<&'a str>,
    home: Option<&'a str>,
    comment: Option<&'a str>,
    groups: Option<&'a [String]>,
    lock: Option<bool>,
    before: &'a AccountState,
}

fn modify_windows(c: &Change<'_>) -> Vec<Call> {
    let mut calls = Vec::new();
    let mut call = Call::new("net", ["user", c.name]);
    if let Some(comment) = c.comment {
        call.args.push(format!("/fullname:{comment}"));
    }
    if let Some(home) = c.home {
        call.args.push(format!("/homedir:{home}"));
    }
    if let Some(lock) = c.lock {
        call.args
            .push(format!("/active:{}", if lock { "no" } else { "yes" }));
    }
    if call.args.len() > 2 {
        calls.push(call);
    }
    if let Some(wanted) = c.groups {
        for group in c.before.groups.iter().filter(|g| !wanted.contains(g)) {
            calls.push(Call::new("net", ["localgroup", group, c.name, "/delete"]));
        }
        for group in wanted.iter().filter(|g| !c.before.groups.contains(g)) {
            calls.push(Call::new("net", ["localgroup", group, c.name, "/add"]));
        }
    }
    calls
}

fn modify_macos(c: &Change<'_>) -> Vec<Call> {
    let mut calls = Vec::new();
    let record = format!("/Users/{}", c.name);
    for (key, value) in [
        ("UserShell", c.shell),
        ("NFSHomeDirectory", c.home),
        ("RealName", c.comment),
    ] {
        if let Some(value) = value {
            calls.push(Call::new("dscl", [".", "-create", &record, key, value]));
        }
    }
    if let Some(wanted) = c.groups {
        for group in c.before.groups.iter().filter(|g| !wanted.contains(g)) {
            calls.push(Call::new(
                "dseditgroup",
                ["-o", "edit", "-d", c.name, "-t", "user", group],
            ));
        }
        for group in wanted.iter().filter(|g| !c.before.groups.contains(g)) {
            calls.push(Call::new(
                "dseditgroup",
                ["-o", "edit", "-a", c.name, "-t", "user", group],
            ));
        }
    }
    if let Some(lock) = c.lock {
        let flag = if lock { "-disableuser" } else { "-enableuser" };
        calls.push(Call::new("pwpolicy", ["-u", c.name, flag]));
    }
    calls
}

fn modify_linux(c: &Change<'_>) -> Vec<Call> {
    let mut call = Call::new("usermod", Vec::<String>::new());
    for (flag, value) in [("-s", c.shell), ("-d", c.home), ("-c", c.comment)] {
        if let Some(value) = value {
            call.args.extend([flag.to_owned(), value.to_owned()]);
        }
    }
    if let Some(groups) = c.groups {
        call.args.extend(["-G".to_owned(), groups.join(",")]);
    }
    if let Some(lock) = c.lock {
        call.args.push(if lock { "-L" } else { "-U" }.to_owned());
    }
    call.args.push(c.name.to_owned());
    vec![call]
}

fn modify(sys: &dyn Sys, args: &BoundArgs, name: &str) -> Result<Plan> {
    let (shell, home, comment) = texts(args)?;
    let groups = groups_arg(args)?;
    let lock = args.bool("lock")?;
    if shell.is_none() && home.is_none() && comment.is_none() && groups.is_none() && lock.is_none()
    {
        return Err(Error::new(
            ErrorCode::MissingParam,
            "`accounts.modify` needs at least one of `shell`, `home`, `groups`, `comment`, `lock`",
        ));
    }
    let before = read(sys, name)?.ok_or_else(|| not_found(name))?;
    let mut after = before.clone();
    if let Some(shell) = shell.clone() {
        after.shell = Some(shell);
    }
    if let Some(home) = home.clone() {
        after.home = Some(home);
    }
    if let Some(comment) = comment.clone() {
        after.comment = comment;
    }
    if let Some(mut groups) = groups.clone() {
        groups.sort();
        groups.dedup();
        after.groups = groups;
    }
    if let Some(lock) = lock {
        after.locked = lock;
    }
    if after == before {
        return Ok(Plan {
            unchanged: Some(format!("the account `{name}` already has these values")),
            ..Plan::default()
        });
    }
    let change = Change {
        name,
        shell: shell.as_deref(),
        home: home.as_deref(),
        comment: comment.as_deref(),
        groups: groups.as_deref(),
        lock,
        before: &before,
    };
    let calls = match sys.os() {
        Os::Windows => modify_windows(&change),
        Os::Macos => modify_macos(&change),
        _ => modify_linux(&change),
    };
    Ok(Plan {
        calls,
        changes: vec![StateChange::new(
            target(name),
            Some(before.to_text()),
            Some(after.to_text()),
        )],
        ..Plan::default()
    })
}

fn remove(sys: &dyn Sys, args: &BoundArgs, name: &str) -> Result<Plan> {
    let keep_home = args::flag(args, "keep_home", false)?;
    let before = read(sys, name)?.ok_or_else(|| not_found(name))?;
    if before.uid == Some(0) || name == "root" {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "the account with identifier 0 is never removed",
        ));
    }
    let calls = match sys.os() {
        Os::Windows => vec![Call::new("net", ["user", name, "/delete"])],
        Os::Macos => vec![Call::new("sysadminctl", ["-deleteUser", name, "-keepHome"])],
        _ => vec![Call::new("userdel", [name])],
    };
    let mut files = Vec::new();
    if !keep_home && let Some(home) = removable_home(before.home.as_deref(), sys) {
        files.push(FileOp::remove(home));
    }
    Ok(Plan {
        calls,
        files,
        changes: vec![StateChange::new(target(name), Some(before.to_text()), None)],
        ..Plan::default()
    })
}

/// The home directory when it may be removed with the account: absolute, at
/// least two components deep and existing.
fn removable_home(home: Option<&str>, sys: &dyn Sys) -> Option<AbsPath> {
    let home = home?;
    let path = AbsPath::new(home).ok()?;
    let depth = path.as_path().components().count();
    let floor = if sys.os() == Os::Windows { 4 } else { 3 };
    (depth >= floor && path.as_path().is_dir()).then_some(path)
}

fn password(sys: &dyn Sys, args: &BoundArgs, name: &str) -> Result<Plan> {
    let value = args::required(args, "value")?;
    if value.chars().any(char::is_control) {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "the password must not hold a control character",
        ));
    }
    let before = read(sys, name)?.ok_or_else(|| not_found(name))?;
    let call = match sys.os() {
        Os::Windows => Call::new(
            "powershell",
            [
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Set-LocalUser -Name $env:NURT_ACCOUNT -Password (ConvertTo-SecureString $env:NURT_SECRET -AsPlainText -Force)",
            ],
        )
        .with_env("NURT_ACCOUNT", name)
        .with_env("NURT_SECRET", &value),
        Os::Macos => Call::new("dscl", [".", "-passwd", &format!("/Users/{name}"), &value]),
        _ => Call::new("chpasswd", Vec::<String>::new()).with_stdin(format!("{name}:{value}\n")),
    };
    Ok(Plan {
        calls: vec![call],
        changes: vec![StateChange::new(
            target(name),
            Some(before.to_text()),
            Some(before.to_text()),
        )],
        irreversible: Some("the previous password is not recorded".to_owned()),
        ..Plan::default()
    })
}

fn not_found(name: &str) -> Error {
    Error::new(ErrorCode::NotFound, format!("there is no account `{name}`"))
}

/// Reads the account `name`; `None` when it does not exist.
pub(crate) fn read(sys: &dyn Sys, name: &str) -> Result<Option<AccountState>> {
    match sys.os() {
        Os::Windows => read_windows(sys, name),
        Os::Macos => read_macos(sys, name),
        _ => read_unix(sys, name),
    }
}

fn read_unix(sys: &dyn Sys, name: &str) -> Result<Option<AccountState>> {
    let passwd = sys.run(&Call::new("getent", ["passwd", name]))?;
    if !passwd.ok() {
        return Ok(None);
    }
    let Some(mut state) = parse_passwd(passwd.stdout.trim()) else {
        return Ok(None);
    };
    let primary = sys.run(&Call::new("id", ["-gn", name]))?;
    let all = sys.run(&Call::new("id", ["-Gn", name]))?;
    if all.ok() {
        let primary = primary.stdout.trim().to_owned();
        state.groups = all
            .stdout
            .split_whitespace()
            .filter(|group| *group != primary)
            .map(str::to_owned)
            .collect();
        state.groups.sort();
    }
    let status = sys.run(&Call::new("passwd", ["-S", name]))?;
    state.locked = status.ok() && parse_locked(&status.stdout);
    Ok(Some(state))
}

/// Fields of a line of `getent passwd`.
pub(crate) fn parse_passwd(line: &str) -> Option<AccountState> {
    let fields: Vec<&str> = line.split(':').collect();
    if fields.len() < 7 {
        return None;
    }
    Some(AccountState {
        uid: fields[2].parse().ok(),
        comment: fields[4].split(',').next().unwrap_or_default().to_owned(),
        home: Some(fields[5].to_owned()),
        shell: Some(fields[6].to_owned()),
        groups: Vec::new(),
        locked: false,
    })
}

/// The account is locked according to the output of `passwd -S`.
pub(crate) fn parse_locked(output: &str) -> bool {
    output
        .split_whitespace()
        .nth(1)
        .is_some_and(|state| state == "L" || state == "LK")
}

fn read_macos(sys: &dyn Sys, name: &str) -> Result<Option<AccountState>> {
    let record = format!("/Users/{name}");
    let output = sys.run(&Call::new(
        "dscl",
        [
            ".",
            "-read",
            &record,
            "UniqueID",
            "UserShell",
            "NFSHomeDirectory",
            "RealName",
        ],
    ))?;
    if !output.ok() {
        return Ok(None);
    }
    let mut state = parse_dscl(&output.stdout);
    let all = sys.run(&Call::new("id", ["-Gn", name]))?;
    let primary = sys.run(&Call::new("id", ["-gn", name]))?;
    if all.ok() {
        let primary = primary.stdout.trim().to_owned();
        state.groups = all
            .stdout
            .split_whitespace()
            .filter(|group| *group != primary)
            .map(str::to_owned)
            .collect();
        state.groups.sort();
    }
    let policy = sys.run(&Call::new(
        "dscl",
        [".", "-read", &record, "AuthenticationAuthority"],
    ))?;
    state.locked = policy.ok() && policy.stdout.contains(";DisabledUser;");
    Ok(Some(state))
}

/// Fields of the output of `dscl . -read`: `Key: value`, with the value of
/// `RealName` on the next line.
pub(crate) fn parse_dscl(output: &str) -> AccountState {
    let mut state = AccountState::default();
    let mut lines = output.lines().peekable();
    while let Some(line) = lines.next() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let mut value = value.trim().to_owned();
        if value.is_empty()
            && let Some(next) = lines.peek()
            && next.starts_with(' ')
        {
            value = next.trim().to_owned();
            lines.next();
        }
        match key {
            "UniqueID" => state.uid = value.parse().ok(),
            "UserShell" => state.shell = Some(value),
            "NFSHomeDirectory" => state.home = Some(value),
            "RealName" => state.comment = value,
            _ => {}
        }
    }
    state
}

fn read_windows(sys: &dyn Sys, name: &str) -> Result<Option<AccountState>> {
    let output = sys.run(&Call::new("net", ["user", name]))?;
    if !output.ok() {
        return Ok(None);
    }
    Ok(Some(parse_net_user(&output.stdout)))
}

/// Fields of the output of `net user NAME`.
pub(crate) fn parse_net_user(output: &str) -> AccountState {
    let mut state = AccountState::default();
    let mut in_groups = false;
    for line in output.lines() {
        if in_groups && (line.trim().is_empty() || line.starts_with("The command")) {
            in_groups = false;
            continue;
        }
        let field = |label: &str| line.strip_prefix(label).map(|rest| rest.trim().to_owned());
        if let Some(value) = field("Full Name") {
            state.comment = value;
        } else if let Some(value) = field("Home directory") {
            if !value.is_empty() {
                state.home = Some(value);
            }
        } else if let Some(value) = field("Account active") {
            state.locked = value.eq_ignore_ascii_case("No");
        } else if let Some(rest) = line.strip_prefix("Local Group Memberships") {
            in_groups = true;
            state.groups = star_names(rest);
        } else if in_groups && line.starts_with(' ') {
            state.groups.extend(star_names(line));
        }
    }
    state.groups.retain(|group| group != "Users");
    state.groups.sort();
    state
}

fn star_names(text: &str) -> Vec<String> {
    text.split('*')
        .skip(1)
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect()
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
