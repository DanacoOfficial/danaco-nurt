// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::details::accounts
// Responsibility:   Names of accounts and groups.
// Dependencies:     nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Names of accounts and groups.
//!
//! On Linux and macOS the names come from `/etc/passwd` and `/etc/group`,
//! read through the file system of the execution context. Accounts kept only
//! in a directory service have no entry there and are shown by their numeric
//! identifier. Windows identifies accounts by security identifiers, which
//! need no table, so the tables stay empty there.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;

use nurt_contracts::env::Os;
use nurt_contracts::fs::{AbsPath, Fs};

/// Account table of Unix systems.
const PASSWD_FILE: &str = "/etc/passwd";

/// Group table of Unix systems.
const GROUP_FILE: &str = "/etc/group";

/// Largest account table read, in bytes.
const TABLE_LIMIT: u64 = 1 << 22;

/// Separator of the fields of a table line.
const FIELD_SEPARATOR: char = ':';

/// Field holding the name.
const NAME_FIELD: usize = 0;

/// Field holding the numeric identifier.
const ID_FIELD: usize = 2;

/// Lines starting with this character are comments.
const COMMENT: char = '#';

/// Start of the identifier in the form `name (id)`.
const ID_OPEN: &str = " (";

/// End of the identifier in the form `name (id)`.
const ID_CLOSE: char = ')';

/// User id of the superuser.
pub(crate) const SUPERUSER_UID: u32 = 0;

/// Names of accounts and groups by numeric identifier.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Accounts {
    users: Table,
    groups: Table,
}

impl Accounts {
    /// Reads the account and group tables of the host; empty tables on
    /// Windows and when a table cannot be read.
    pub(crate) fn load(fs: &dyn Fs, os: Os) -> Self {
        if os == Os::Windows {
            return Self::default();
        }
        Self {
            users: read_table(fs, PASSWD_FILE),
            groups: read_table(fs, GROUP_FILE),
        }
    }

    /// Name of an account, when the table knows it.
    pub(crate) fn user_name(&self, uid: u32) -> Option<&str> {
        self.users.name(uid)
    }

    /// Name of a group, when the table knows it.
    pub(crate) fn group_name(&self, gid: u32) -> Option<&str> {
        self.groups.name(gid)
    }

    /// Account as shown in `entry.owner`: `name (id)`, or the identifier
    /// alone when the name is not known.
    pub(crate) fn describe_user(&self, uid: u32) -> String {
        self.users.describe(uid)
    }

    /// Group as shown in `entry.group`: `name (id)`, or the identifier alone
    /// when the name is not known.
    pub(crate) fn describe_group(&self, gid: u32) -> String {
        self.groups.describe(gid)
    }

    /// Account identifier of a name, of a decimal identifier or of the form
    /// `name (id)`.
    pub(crate) fn resolve_user(&self, text: &str) -> Option<u32> {
        self.users.resolve(text)
    }

    /// Group identifier of a name, of a decimal identifier or of the form
    /// `name (id)`.
    pub(crate) fn resolve_group(&self, text: &str) -> Option<u32> {
        self.groups.resolve(text)
    }
}

/// One table: names by identifier and identifiers by name; the first line
/// of a name or an identifier wins, as in the system lookup.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Table {
    names: BTreeMap<u32, String>,
    ids: HashMap<String, u32>,
}

impl Table {
    fn parse(text: &str) -> Self {
        let mut table = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(COMMENT) {
                continue;
            }
            let fields: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
            let name = fields.get(NAME_FIELD).copied().unwrap_or_default();
            let id = fields.get(ID_FIELD).and_then(|id| id.parse::<u32>().ok());
            let Some(id) = id.filter(|_| !name.is_empty()) else {
                continue;
            };
            table.names.entry(id).or_insert_with(|| name.to_owned());
            table.ids.entry(name.to_owned()).or_insert(id);
        }
        table
    }

    fn name(&self, id: u32) -> Option<&str> {
        self.names.get(&id).map(String::as_str)
    }

    fn describe(&self, id: u32) -> String {
        match self.name(id) {
            Some(name) => format!("{name}{ID_OPEN}{id}{ID_CLOSE}"),
            None => id.to_string(),
        }
    }

    fn resolve(&self, text: &str) -> Option<u32> {
        let text = text.trim();
        if let Ok(id) = text.parse::<u32>() {
            return Some(id);
        }
        if let Some((_, rest)) = text.rsplit_once(ID_OPEN)
            && let Some(id) = rest.strip_suffix(ID_CLOSE)
        {
            return id.parse().ok();
        }
        self.ids.get(text).copied()
    }
}

fn read_table(fs: &dyn Fs, file: &str) -> Table {
    let Ok(path) = AbsPath::new(file) else {
        return Table::default();
    };
    let Ok(handle) = fs.open_read(&path) else {
        return Table::default();
    };
    let mut bytes = Vec::new();
    if handle.take(TABLE_LIMIT).read_to_end(&mut bytes).is_err() {
        return Table::default();
    }
    Table::parse(&String::from_utf8_lossy(&bytes))
}

#[cfg(test)]
mod tests {
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::vfs::test_path;

    use super::*;

    const PASSWD: &str = "# local accounts\nroot:x:0:0:root:/root:/bin/sh\nuser:x:1000:1000::/home/user:/bin/sh\nshadow:x:1000:1000::/:/bin/false\nbroken:x:none:1\n\n";
    const GROUP: &str = "root:x:0:\nstaff:x:50:user\n";

    fn with_tables() -> TestContext {
        let test = TestContext::new();
        test.fs()
            .write_file(&test_path(PASSWD_FILE).unwrap(), PASSWD.as_bytes())
            .unwrap();
        test.fs()
            .write_file(&test_path(GROUP_FILE).unwrap(), GROUP.as_bytes())
            .unwrap();
        test
    }

    #[test]
    fn tables_give_names_and_identifiers() {
        let test = with_tables();
        let accounts = Accounts::load(test.fs().as_ref(), Os::Linux);
        if cfg!(windows) {
            assert_eq!(accounts, Accounts::default());
            return;
        }
        assert_eq!(accounts.user_name(1000), Some("user"));
        assert_eq!(accounts.group_name(50), Some("staff"));
        assert_eq!(accounts.describe_user(0), "root (0)");
        assert_eq!(accounts.describe_group(50), "staff (50)");
        assert_eq!(accounts.describe_user(4242), "4242");
        assert_eq!(accounts.resolve_user("user"), Some(1000));
        assert_eq!(accounts.resolve_user("shadow"), Some(1000));
        assert_eq!(accounts.resolve_user("broken"), None);
        assert_eq!(accounts.resolve_group("staff"), Some(50));
    }

    #[test]
    fn identifiers_resolve_without_tables() {
        let accounts = Accounts::default();
        assert_eq!(accounts.resolve_user("1001"), Some(1001));
        assert_eq!(accounts.resolve_user("alice (1002)"), Some(1002));
        assert_eq!(accounts.resolve_group(" 7 "), Some(7));
        assert_eq!(accounts.resolve_user("alice"), None);
        assert_eq!(accounts.resolve_user("alice (x)"), None);
        assert_eq!(accounts.user_name(0), None);
    }

    #[test]
    fn windows_hosts_use_no_tables() {
        let test = with_tables();
        let accounts = Accounts::load(test.fs().as_ref(), Os::Windows);
        assert_eq!(accounts, Accounts::default());
    }

    #[test]
    fn missing_tables_leave_names_unknown() {
        let test = TestContext::new();
        let accounts = Accounts::load(test.fs().as_ref(), Os::Macos);
        assert_eq!(accounts.describe_user(501), "501");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
