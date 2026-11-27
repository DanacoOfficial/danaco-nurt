// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::accounts
// Responsibility:   User and group accounts: the `accounts` command.
// Dependencies:     indexmap, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! User and group accounts: the `accounts` command.
//!
//! | System | Users and groups | `disabled` | `system` |
//! |---|---|---|---|
//! | Linux | `getpwent`, `getgrent` | password field of `/etc/shadow` starts with `!` or is `*` | identifier below `UID_MIN` of `/etc/login.defs`, 1000 when unset |
//! | macOS | `getpwent`, `getgrent`; the directory service supplies the local accounts | not reported | identifier below 500 or name starting with `_` |
//! | Windows | `NetUserEnum`, `NetLocalGroupEnum` | `UF_ACCOUNTDISABLE` | relative identifier below 1000; built-in groups |
//!
//! `nobody`, `nogroup` and `nfsnobody` are system accounts on every system.
//! The groups of a user are its primary group followed by the groups that
//! list it as a member; the groups of a group record are its members. When
//! `/etc/shadow` exists but cannot be read, `disabled` is left empty and the
//! report carries one `permission_denied_skipped` note. Domain accounts on
//! Windows are not listed.

#[cfg(unix)]
mod files;
#[cfg(test)]
mod tests;
#[cfg(unix)]
pub(crate) mod unix;
#[cfg(windows)]
pub(crate) mod windows;

#[cfg(unix)]
use std::collections::HashMap;
#[cfg(unix)]
use std::io;

#[cfg(unix)]
use indexmap::IndexMap;
use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::Account;
#[cfg(unix)]
use nurt_contracts::record::AccountKind;
#[cfg(unix)]
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Record, Value};

use crate::common::SysRoot;
#[cfg(unix)]
use crate::common::canonical_path;

/// Name of the command.
pub(crate) const COMMAND: &str = "accounts";
/// Parameter selecting users, groups or both.
pub(crate) const KIND: &str = "kind";
/// Parameter with the account name.
pub(crate) const NAME: &str = "name";
/// Identifiers of macOS accounts below this value belong to the system.
#[cfg(any(unix, test))]
const MACOS_UID_MIN: u32 = 500;
/// Accounts without privileges that are system accounts regardless of their
/// identifier.
#[cfg(any(unix, test))]
const UNPRIVILEGED_NAMES: [&str; 3] = ["nobody", "nogroup", "nfsnobody"];
/// Shadow password database.
#[cfg(unix)]
const SHADOW_FILE: &str = "/etc/shadow";

/// Accounts selected by the `kind` parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Selection {
    /// User accounts only.
    Users,
    /// Groups only.
    Groups,
    /// Users and groups.
    All,
}

impl Selection {
    /// Selection from the `kind` argument; `All` when it is absent.
    ///
    /// # Errors
    ///
    /// Fails with `E_TYPE` for a value other than `user`, `group` or `all`.
    pub(crate) fn from_args(args: &BoundArgs) -> Result<Self> {
        match args.value(KIND).map(|value| value.as_str()) {
            None | Some(Some("all")) => Ok(Self::All),
            Some(Some("user")) => Ok(Self::Users),
            Some(Some("group")) => Ok(Self::Groups),
            Some(_) => Err(Error::new(
                ErrorCode::Type,
                "`accounts` expects `user`, `group` or `all` for `kind`",
            )
            .with_fix("accounts kind: all")),
        }
    }

    /// User accounts are listed.
    pub(crate) fn users(self) -> bool {
        self != Self::Groups
    }

    /// Groups are listed.
    pub(crate) fn groups(self) -> bool {
        self != Self::Users
    }
}

/// The `accounts` command.
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_sys::accounts::AccountsStage;
///
/// assert_eq!(AccountsStage::new().descriptor().command, "accounts");
/// ```
#[derive(Debug, Clone, Default)]
pub struct AccountsStage {
    root: SysRoot,
}

impl AccountsStage {
    /// The command reading the account databases of the running system.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The command reading the account files under `root`.
    #[must_use]
    pub fn with_root(root: SysRoot) -> Self {
        Self { root }
    }
}

impl StageImpl for AccountsStage {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for AccountsStage {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let selection = Selection::from_args(args)?;
        let name = name_filter(args)?;
        let os = ctx.host().info().os;
        let records: Vec<Record> = list(&self.root, selection, os, ctx)?
            .into_iter()
            .filter(|account| name.is_none_or(|wanted| same_name(wanted, &account.name, os)))
            .map(Record::from)
            .collect();
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

fn name_filter(args: &BoundArgs) -> Result<Option<&str>> {
    match args.value(NAME) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Text(name)) => Ok(Some(name.as_str())),
        Some(_) => Err(Error::new(
            ErrorCode::Type,
            "`accounts` expects text for `name`",
        )),
    }
}

/// Account names are compared regardless of case on Windows only.
fn same_name(wanted: &str, name: &str, os: Os) -> bool {
    if os == Os::Windows {
        wanted.eq_ignore_ascii_case(name)
    } else {
        wanted == name
    }
}

/// Whether the account `name` with the identifier `id` belongs to the
/// operating system rather than to a person.
#[cfg(any(unix, test))]
pub(crate) fn is_system(id: u32, name: &str, os: Os, uid_min: u32) -> bool {
    if UNPRIVILEGED_NAMES.contains(&name) {
        return true;
    }
    match os {
        Os::Macos => id < MACOS_UID_MIN || name.starts_with('_'),
        _ => id < uid_min,
    }
}

#[cfg(unix)]
fn list(root: &SysRoot, selection: Selection, os: Os, ctx: &ExecCtx) -> Result<Vec<Account>> {
    let users =
        unix::users(root).map_err(|error| crate::common::source_error(&error, "/etc/passwd"))?;
    let groups =
        unix::groups(root).map_err(|error| crate::common::source_error(&error, "/etc/group"))?;
    let uid_min = root
        .read_to_string("/etc/login.defs")
        .ok()
        .and_then(|text| files::parse_uid_min(&text))
        .unwrap_or(files::DEFAULT_UID_MIN);
    let mut accounts = Vec::new();
    if selection.users() {
        let shadow = shadow_flags(root.read_to_string(SHADOW_FILE), ctx);
        accounts.extend(
            users
                .iter()
                .map(|user| user_account(user, &groups, shadow.as_ref(), os, uid_min)),
        );
    }
    if selection.groups() {
        accounts.extend(groups.iter().map(|group| group_account(group, os, uid_min)));
    }
    Ok(accounts)
}

#[cfg(windows)]
fn list(_root: &SysRoot, selection: Selection, _os: Os, _ctx: &ExecCtx) -> Result<Vec<Account>> {
    windows::accounts(selection)
}

/// Locked state of each account of the shadow database `content`.
///
/// A database that does not exist yields `None` silently; one that cannot be
/// read for lack of permission yields `None` and one
/// `permission_denied_skipped` note.
#[cfg(unix)]
pub(crate) fn shadow_flags(
    content: io::Result<String>,
    ctx: &ExecCtx,
) -> Option<HashMap<String, bool>> {
    match content {
        Ok(text) => Some(files::parse_shadow(&text)),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            ctx.note(NotableEntry {
                path: Some(SHADOW_FILE.to_owned()),
                count: Some(1),
                detail: Some(
                    "the shadow database is not readable; `disabled` is not reported".to_owned(),
                ),
                ..NotableEntry::new(Notable::PermissionDeniedSkipped)
            });
            None
        }
        Err(_) => None,
    }
}

#[cfg(unix)]
fn user_account(
    user: &files::PasswdEntry,
    groups: &[files::GroupEntry],
    shadow: Option<&HashMap<String, bool>>,
    os: Os,
    uid_min: u32,
) -> Account {
    let mut names: Vec<String> = groups
        .iter()
        .filter(|group| group.gid == user.gid)
        .map(|group| group.name.clone())
        .take(1)
        .collect();
    for group in groups {
        if group.members.iter().any(|member| member == &user.name) && !names.contains(&group.name) {
            names.push(group.name.clone());
        }
    }
    let mut raw = IndexMap::new();
    raw.insert("gid".to_owned(), Value::Int(i64::from(user.gid)));
    Account {
        name: user.name.clone(),
        kind: AccountKind::User,
        id: user.uid.to_string(),
        home: non_empty(&user.home).map(|home| canonical_path(std::path::Path::new(home))),
        shell: non_empty(&user.shell).map(|shell| canonical_path(std::path::Path::new(shell))),
        groups: Some(names),
        disabled: shadow.and_then(|flags| flags.get(&user.name).copied()),
        full_name: user.full_name(),
        system: Some(is_system(user.uid, &user.name, os, uid_min)),
        last_login: None,
        os_raw: Some(raw),
    }
}

#[cfg(unix)]
fn group_account(group: &files::GroupEntry, os: Os, uid_min: u32) -> Account {
    Account {
        name: group.name.clone(),
        kind: AccountKind::Group,
        id: group.gid.to_string(),
        groups: Some(group.members.clone()),
        system: Some(is_system(group.gid, &group.name, os, uid_min)),
        ..Account::default()
    }
}

#[cfg(unix)]
fn non_empty(text: &str) -> Option<&str> {
    (!text.is_empty()).then_some(text)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
