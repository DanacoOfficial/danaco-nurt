// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::accounts::tests
// Responsibility:   Tree with `passwd`, `group`, `shadow` and `login.defs`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use nurt_contracts::env::Os;
use nurt_contracts::stage::BoundArgs;
use nurt_contracts::value::Value;

use super::{Selection, is_system, same_name};

fn selection(value: Option<Value>) -> Option<Selection> {
    let args = value
        .into_iter()
        .fold(BoundArgs::new(super::COMMAND), |args, value| {
            args.with(
                super::KIND,
                nurt_contracts::stage::ArgValue::Value(value),
                false,
            )
        });
    Selection::from_args(&args).ok()
}

#[test]
fn accounts_kind_parses_user_group_all() {
    assert_eq!(selection(None), Some(Selection::All));
    assert_eq!(
        selection(Some(Value::Enum("user".to_owned()))),
        Some(Selection::Users)
    );
    assert_eq!(
        selection(Some(Value::Text("group".to_owned()))),
        Some(Selection::Groups)
    );
    assert_eq!(selection(Some(Value::Text("other".to_owned()))), None);
    assert!(Selection::All.users() && Selection::All.groups());
    assert!(!Selection::Groups.users());
    assert!(!Selection::Users.groups());
}

#[test]
fn accounts_system_flag_rules_per_system() {
    assert!(is_system(0, "root", Os::Linux, 1000));
    assert!(is_system(999, "svc", Os::Linux, 1000));
    assert!(!is_system(1000, "user", Os::Linux, 1000));
    assert!(is_system(65534, "nobody", Os::Linux, 1000));
    assert!(is_system(499, "daemon", Os::Macos, 1000));
    assert!(is_system(600, "_service", Os::Macos, 1000));
    assert!(!is_system(501, "user", Os::Macos, 1000));
    assert!(same_name("Admin", "admin", Os::Windows));
    assert!(!same_name("Admin", "admin", Os::Linux));
}

#[cfg(unix)]
mod unix_files {
    use nurt_contracts::record::{Account, AccountKind};
    use nurt_contracts::report::Notable;
    use nurt_contracts::value::{Record, Value};

    use super::super::{AccountsStage, files};
    use crate::common::SysRoot;
    use crate::host::test_support::{self, TestHost};

    /// Tree with `passwd`, `group`, `shadow` and `login.defs`.
    fn fixture_root() -> SysRoot {
        SysRoot::new(test_support::fixtures().join("sysroot"))
    }

    fn accounts(records: Vec<Record>) -> Vec<Account> {
        records
            .into_iter()
            .map(|record| Account::try_from(record).unwrap())
            .collect()
    }

    fn run(root: SysRoot, values: Vec<(&str, Value)>) -> (Vec<Account>, usize) {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_support::context(
            TestHost::new(dir.path()).with_os(nurt_contracts::env::Os::Linux),
        );
        let args = test_support::args(super::super::COMMAND, values);
        let records = test_support::run(&AccountsStage::with_root(root), &args, &ctx).unwrap();
        let notes = ctx
            .take_notes()
            .iter()
            .filter(|note| note.code == Notable::PermissionDeniedSkipped)
            .count();
        (accounts(records), notes)
    }

    fn find<'a>(accounts: &'a [Account], name: &str) -> &'a Account {
        accounts
            .iter()
            .find(|account| account.name == name)
            .unwrap_or_else(|| panic!("`{name}` is missing"))
    }

    #[test]
    fn accounts_passwd_group_parse_and_system_flag() {
        let (users, notes) = run(
            fixture_root(),
            vec![(super::super::KIND, Value::Enum("user".to_owned()))],
        );
        assert_eq!(notes, 0);
        let names: Vec<&str> = users.iter().map(|user| user.name.as_str()).collect();
        assert_eq!(names, ["root", "daemon", "nobody", "user", "build", "svc"]);
        assert!(users.iter().all(|user| user.kind == AccountKind::User));
        assert_person(find(&users, "user"));

        assert_eq!(find(&users, "root").system, Some(true));
        assert_eq!(find(&users, "root").disabled, Some(true));
        assert_eq!(find(&users, "nobody").system, Some(true));
        assert_eq!(find(&users, "svc").system, Some(true));
        assert_eq!(find(&users, "build").system, Some(false));
        assert_eq!(find(&users, "build").disabled, Some(true));
        assert_eq!(find(&users, "build").full_name, None);
    }

    /// Fields of the ordinary account `user` of the fixture.
    fn assert_person(user: &Account) {
        assert_eq!(user.id, "1000");
        assert_eq!(user.full_name.as_deref(), Some("Example User"));
        assert_eq!(
            user.home.as_ref().map(|home| home.as_str()),
            Some("/home/user")
        );
        assert_eq!(
            user.shell.as_ref().map(|shell| shell.as_str()),
            Some("/bin/bash")
        );
        assert_eq!(
            user.groups.as_deref(),
            Some(&["user".to_owned(), "sudo".to_owned(), "docker".to_owned()][..])
        );
        assert_eq!(user.system, Some(false));
        assert_eq!(user.disabled, Some(false));
        assert_eq!(
            user.os_raw.as_ref().and_then(|raw| raw.get("gid")),
            Some(&Value::Int(1000))
        );
    }

    #[test]
    fn accounts_kind_filter_groups_only() {
        let (groups, _) = run(
            fixture_root(),
            vec![(super::super::KIND, Value::Enum("group".to_owned()))],
        );
        assert_eq!(groups.len(), 7);
        assert!(groups.iter().all(|group| group.kind == AccountKind::Group));
        let docker = find(&groups, "docker");
        assert_eq!(docker.id, "999");
        assert_eq!(
            docker.groups.as_deref(),
            Some(&["user".to_owned(), "build".to_owned()][..])
        );
        assert_eq!(docker.system, Some(true));
        assert_eq!(find(&groups, "nogroup").system, Some(true));
        assert_eq!(find(&groups, "build").system, Some(false));

        let (named, _) = run(
            fixture_root(),
            vec![(super::super::NAME, Value::Text("docker".to_owned()))],
        );
        assert_eq!(named.len(), 1);
        assert_eq!(named[0].kind, AccountKind::Group);

        let (all, _) = run(fixture_root(), Vec::new());
        assert_eq!(all.len(), 13);
    }

    #[test]
    fn accounts_shadow_unreadable_warns_once() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_support::context(TestHost::new(dir.path()));
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(super::super::shadow_flags(Err(denied), &ctx).is_none());
        let notes = ctx.take_notes();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].code, Notable::PermissionDeniedSkipped);
        assert_eq!(notes[0].path.as_deref(), Some("/etc/shadow"));

        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert!(super::super::shadow_flags(Err(missing), &ctx).is_none());
        assert!(ctx.take_notes().is_empty());

        let etc = dir.path().join("etc");
        std::fs::create_dir_all(&etc).unwrap();
        std::fs::write(etc.join("passwd"), "user:x:1000:1000::/home/user:/bin/sh\n").unwrap();
        std::fs::write(etc.join("group"), "user:x:1000:\n").unwrap();
        let (users, notes) = run(SysRoot::new(dir.path()), Vec::new());
        assert_eq!(notes, 0);
        assert_eq!(find(&users, "user").disabled, None);
    }

    #[test]
    fn accounts_null_name_lists_everything() {
        let (all, _) = run(fixture_root(), vec![(super::super::NAME, Value::Null)]);
        assert_eq!(all.len(), 13);
    }

    #[test]
    fn accounts_non_utf8_full_name_does_not_abort_the_listing() {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        std::fs::create_dir_all(&etc).unwrap();
        std::fs::write(
            etc.join("passwd"),
            b"user:x:1000:1000:Ren\xE9 Dupont:/home/user:/bin/sh\n".as_slice(),
        )
        .unwrap();
        std::fs::write(etc.join("group"), "user:x:1000:\n").unwrap();
        let (users, _) = run(SysRoot::new(dir.path()), Vec::new());
        assert_eq!(
            find(&users, "user").full_name.as_deref(),
            Some("Ren\u{FFFD} Dupont")
        );
    }

    #[test]
    fn accounts_missing_passwd_is_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_support::context(TestHost::new(dir.path()));
        let args = test_support::args(super::super::COMMAND, Vec::new());
        let error = test_support::run(
            &AccountsStage::with_root(SysRoot::new(dir.path())),
            &args,
            &ctx,
        )
        .unwrap_err();
        assert_eq!(error.code(), nurt_contracts::error::ErrorCode::Io);
    }

    #[test]
    fn accounts_file_parsers_skip_incomplete_lines() {
        let users = files::parse_passwd("# comment\nshort:x:1\nuser:x:5:6:Full,Room:/h:/s\n");
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].full_name().as_deref(), Some("Full"));
        assert_eq!(
            files::parse_group("g:x:7:a,,b\nbad:x:z:\n")[0].members,
            ["a", "b"]
        );
        assert_eq!(files::parse_uid_min("UID_MIN 500\n"), Some(500));
        assert_eq!(files::parse_uid_min("UID_MAX 500\n"), None);
        let shadow = files::parse_shadow("a:!x:1\nb:hash:1\nc:*:1\n");
        assert_eq!(shadow.get("a"), Some(&true));
        assert_eq!(shadow.get("b"), Some(&false));
        assert_eq!(shadow.get("c"), Some(&true));
    }

    #[test]
    #[ignore = "requires the account database of the running system"]
    fn it_accounts_contains_current_user() {
        let uid = super::super::unix::current_uid();
        let listed = std::fs::read_to_string("/etc/passwd").is_ok_and(|text| {
            text.lines()
                .any(|line| line.split(':').nth(2) == Some(uid.to_string().as_str()))
        });
        if !listed {
            return;
        }
        let (users, _) = run(
            SysRoot::live(),
            vec![(super::super::KIND, Value::Enum("user".to_owned()))],
        );
        assert!(users.iter().any(|user| user.id == uid.to_string()));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
