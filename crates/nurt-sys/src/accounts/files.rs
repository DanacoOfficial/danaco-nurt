// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::accounts::files
// Responsibility:   Parsers of the account files `/etc/passwd`, `/etc/group`, `/etc/shadow`.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parsers of the account files `/etc/passwd`, `/etc/group`, `/etc/shadow`
//! and `/etc/login.defs`.

use std::collections::HashMap;

/// Lowest identifier of ordinary users when `/etc/login.defs` does not set
/// `UID_MIN`.
pub(crate) const DEFAULT_UID_MIN: u32 = 1000;

/// One entry of the user database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PasswdEntry {
    /// Login name.
    pub(crate) name: String,
    /// User identifier.
    pub(crate) uid: u32,
    /// Identifier of the primary group.
    pub(crate) gid: u32,
    /// Comment field; the first comma-separated part is the full name.
    pub(crate) gecos: String,
    /// Home directory.
    pub(crate) home: String,
    /// Login shell.
    pub(crate) shell: String,
}

impl PasswdEntry {
    /// Full name: the first comma-separated part of the comment field.
    pub(crate) fn full_name(&self) -> Option<String> {
        self.gecos
            .split(',')
            .next()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
    }
}

/// One entry of the group database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GroupEntry {
    /// Group name.
    pub(crate) name: String,
    /// Group identifier.
    pub(crate) gid: u32,
    /// Names of the supplementary members.
    pub(crate) members: Vec<String>,
}

/// Parses `/etc/passwd`; lines that are not complete entries are skipped.
pub(crate) fn parse_passwd(text: &str) -> Vec<PasswdEntry> {
    entries(text)
        .filter_map(|fields| {
            let [name, _, uid, gid, gecos, home, shell] = fields.as_slice() else {
                return None;
            };
            Some(PasswdEntry {
                name: (*name).to_owned(),
                uid: uid.parse().ok()?,
                gid: gid.parse().ok()?,
                gecos: (*gecos).to_owned(),
                home: (*home).to_owned(),
                shell: (*shell).to_owned(),
            })
        })
        .collect()
}

/// Parses `/etc/group`; lines that are not complete entries are skipped.
pub(crate) fn parse_group(text: &str) -> Vec<GroupEntry> {
    entries(text)
        .filter_map(|fields| {
            let [name, _, gid, members] = fields.as_slice() else {
                return None;
            };
            Some(GroupEntry {
                name: (*name).to_owned(),
                gid: gid.parse().ok()?,
                members: members
                    .split(',')
                    .filter(|member| !member.is_empty())
                    .map(str::to_owned)
                    .collect(),
            })
        })
        .collect()
}

/// Accounts of `/etc/shadow` and whether each is locked: the password field
/// starts with `!` or is `*`.
pub(crate) fn parse_shadow(text: &str) -> HashMap<String, bool> {
    entries(text)
        .filter_map(|fields| {
            let name = fields.first()?;
            let password = fields.get(1)?;
            let locked = password.starts_with('!') || password.starts_with('*');
            Some(((*name).to_owned(), locked))
        })
        .collect()
}

/// `UID_MIN` of `/etc/login.defs`.
pub(crate) fn parse_uid_min(text: &str) -> Option<u32> {
    text.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next() == Some("UID_MIN"))
            .then(|| words.next()?.parse().ok())
            .flatten()
    })
}

fn entries(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split(':').collect())
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
