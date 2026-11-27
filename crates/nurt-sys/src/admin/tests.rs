// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the administration commands on the in-memory system: the plan,
//! the programs, the recorded states and the undo of every command.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use nurt_contracts::env::Os;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::journal::Mutation;
use nurt_contracts::stage::{ArgValue, BoundArgs};
use nurt_contracts::value::{Duration, Value};

use super::testsys::FakeSys;
use super::{Inverse, Plan, inverse, plan};

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn list(values: &[&str]) -> Value {
    Value::List(values.iter().map(|v| text(v)).collect())
}

fn bound(command: &str, params: &[(&str, Value)]) -> BoundArgs {
    params
        .iter()
        .fold(BoundArgs::new(command), |args, (name, value)| {
            args.with(*name, ArgValue::Value(value.clone()), false)
        })
}

/// Runs the plan of `args`: the programs, then the file edits, as the stage
/// does; the journal is emulated by the caller through snapshots.
fn execute(sys: &FakeSys, args: &BoundArgs) -> Plan {
    let planned = plan(sys, args).unwrap_or_else(|error| panic!("{}: {error}", args.command()));
    run_plan(sys, &planned);
    planned
}

fn run_plan(sys: &FakeSys, planned: &Plan) {
    use super::sys::Sys;
    if planned.unchanged.is_some() {
        return;
    }
    for call in &planned.calls {
        let output = sys.run(call).unwrap();
        assert!(output.ok(), "{} failed: {}", call.line(), output.stderr);
    }
    for op in &planned.files {
        let nurt_contracts::journal::TargetPath::Fs(path) = &op.target.path else {
            panic!("file target expected");
        };
        let key = path.as_path().to_string_lossy().replace('\\', "/");
        let key = key.strip_prefix("C:").unwrap_or(&key).to_owned();
        let mut state = sys.state.lock().unwrap();
        match &op.mutation {
            Mutation::WriteBytes(bytes) => {
                state
                    .files
                    .insert(key, String::from_utf8(bytes.clone()).unwrap());
            }
            Mutation::Remove => {
                state.files.remove(&key);
            }
            other => panic!("unexpected mutation {other:?}"),
        }
    }
}

fn changes(planned: &Plan) -> Vec<(String, Option<String>, Option<String>)> {
    planned
        .changes
        .iter()
        .map(|c| (c.target.clone(), c.before.clone(), c.after.clone()))
        .collect()
}

fn derive(kind: &str, planned: &Plan) -> Inverse {
    let owned = changes(planned);
    let refs: Vec<(&str, Option<&str>, Option<&str>)> = owned
        .iter()
        .map(|(t, b, a)| (t.as_str(), b.as_deref(), a.as_deref()))
        .collect();
    inverse(kind, &refs).expect("an administration command")
}

/// The program text of an inverse parsed back into bound arguments.
fn parse_program(program: &str) -> BoundArgs {
    let (command, mut rest) = program
        .split_once(' ')
        .map_or((program, ""), |(c, r)| (c, r));
    let mut args = BoundArgs::new(command);
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let (name, tail) = rest.split_once(": ").expect("name: value");
        let (value, tail) = parse_value(tail);
        args.insert(name, ArgValue::Value(value), false);
        rest = tail;
    }
    args
}

fn parse_value(tail: &str) -> (Value, &str) {
    if let Some(body) = tail.strip_prefix('[') {
        let mut items = Vec::new();
        let mut rest = body;
        loop {
            rest = rest.trim_start_matches([' ', ',']);
            if let Some(after) = rest.strip_prefix(']') {
                return (Value::List(items), after);
            }
            let (item, after) = parse_value(rest);
            items.push(item);
            rest = after;
        }
    }
    if let Some(body) = tail.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = body.char_indices();
        while let Some((index, c)) = chars.next() {
            match c {
                '\\' => out.push(chars.next().unwrap().1),
                '"' => return (Value::Text(out), &body[index + 1..]),
                other => out.push(other),
            }
        }
        panic!("unterminated text");
    }
    let end = tail.find([' ', ',', ']']).unwrap_or(tail.len());
    let word = &tail[..end];
    let value = match word {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        number => Value::Int(number.parse().expect("a number")),
    };
    (value, &tail[end..])
}

/// Runs `args`, undoes it with the derived programs and checks that the
/// system is as it was.
fn round_trip(sys: &FakeSys, args: &BoundArgs) -> Plan {
    let before = sys.snapshot();
    let planned = execute(sys, args);
    assert_ne!(sys.snapshot(), before, "the command changed nothing");
    let Inverse::Programs(programs) = derive(args.command(), &planned) else {
        panic!("{} has no programs to undo it", args.command());
    };
    let after_files = before.files.clone();
    for program in &programs {
        execute(sys, &parse_program(program));
    }
    let mut now = sys.snapshot();
    now.files = after_files;
    sys.restore(now);
    assert_eq!(sys.snapshot().users, before.users, "accounts after undo");
    assert_eq!(sys.snapshot().groups, before.groups, "groups after undo");
    assert_eq!(sys.snapshot().mounts, before.mounts, "mounts after undo");
    assert_eq!(sys.snapshot().sysctl, before.sysctl, "sysctl after undo");
    assert_eq!(sys.snapshot().modules, before.modules, "modules after undo");
    assert_eq!(
        sys.snapshot().schedule,
        before.schedule,
        "schedule after undo"
    );
    planned
}

fn error_code(sys: &FakeSys, args: &BoundArgs) -> ErrorCode {
    plan(sys, args).expect_err("the plan must fail").code()
}

#[test]
fn an_account_is_created_with_useradd_and_undone_by_remove() {
    let sys = FakeSys::linux();
    let args = bound(
        "accounts.create",
        &[
            ("name", text("nurt-test-a")),
            ("shell", text("/bin/zsh")),
            ("groups", list(&["wheel"])),
            ("comment", text("Test A")),
        ],
    );
    let planned = round_trip(&sys, &args);
    assert!(
        sys.calls()
            .contains(&"useradd -s /bin/zsh -m -c Test A -G wheel nurt-test-a".to_owned()),
        "{:?}",
        sys.calls()
    );
    assert_eq!(planned.changes[0].target, "account:nurt-test-a");
    assert!(planned.changes[0].before.is_none());
    assert!(planned.irreversible.is_none());
    assert_eq!(
        derive("accounts.create", &planned),
        Inverse::Programs(vec![
            "accounts.remove name: \"nurt-test-a\" keep_home: true".to_owned()
        ])
    );
}

#[test]
fn creating_an_existing_account_fails_with_exists() {
    let sys = FakeSys::linux();
    assert_eq!(
        error_code(&sys, &bound("accounts.create", &[("name", text("root"))])),
        ErrorCode::Exists
    );
}

#[test]
fn names_that_a_program_would_read_as_options_are_refused() {
    let sys = FakeSys::linux();
    for name in ["-o", "a:b", "a b", "", "a/b"] {
        assert_eq!(
            error_code(&sys, &bound("accounts.create", &[("name", text(name))])),
            ErrorCode::InvalidValue,
            "{name:?}"
        );
    }
    assert_eq!(
        error_code(
            &sys,
            &bound(
                "accounts.create",
                &[("name", text("ok")), ("shell", text("-x"))]
            )
        ),
        ErrorCode::InvalidValue
    );
}

#[test]
fn modify_records_both_states_and_undo_restores_the_recorded_values() {
    let sys = FakeSys::linux();
    execute(
        &sys,
        &bound(
            "accounts.create",
            &[("name", text("nurt-test-m")), ("groups", list(&["docker"]))],
        ),
    );
    let args = bound(
        "accounts.modify",
        &[
            ("name", text("nurt-test-m")),
            ("shell", text("/bin/fish")),
            ("comment", text("New")),
            ("groups", list(&["wheel"])),
            ("lock", Value::Bool(true)),
        ],
    );
    let planned = round_trip(&sys, &args);
    let Inverse::Programs(programs) = derive("accounts.modify", &planned) else {
        panic!("programs expected");
    };
    assert_eq!(
        programs,
        [
            "accounts.modify name: \"nurt-test-m\" shell: \"/bin/sh\" comment: \"\" groups: [\"docker\"] lock: false"
        ]
    );
}

#[test]
fn modify_with_the_present_values_changes_nothing() {
    let sys = FakeSys::linux();
    let planned = plan(
        &sys,
        &bound(
            "accounts.modify",
            &[("name", text("root")), ("shell", text("/bin/bash"))],
        ),
    )
    .unwrap();
    assert!(planned.unchanged.is_some());
    assert!(planned.calls.is_empty());
    assert_eq!(
        error_code(&sys, &bound("accounts.modify", &[("name", text("root"))])),
        ErrorCode::MissingParam
    );
}

#[test]
fn remove_is_undone_by_create_with_the_recorded_identity() {
    let sys = FakeSys::linux();
    execute(
        &sys,
        &bound(
            "accounts.create",
            &[
                ("name", text("nurt-test-r")),
                ("uid", Value::Int(4242)),
                ("groups", list(&["docker", "wheel"])),
                ("comment", text("Removed")),
            ],
        ),
    );
    execute(
        &sys,
        &bound(
            "accounts.modify",
            &[("name", text("nurt-test-r")), ("lock", Value::Bool(true))],
        ),
    );
    let args = bound(
        "accounts.remove",
        &[
            ("name", text("nurt-test-r")),
            ("keep_home", Value::Bool(true)),
        ],
    );
    let before = sys.snapshot();
    let planned = round_trip(&sys, &args);
    assert_eq!(sys.snapshot().users, before.users);
    let Inverse::Programs(programs) = derive("accounts.remove", &planned) else {
        panic!("programs expected");
    };
    assert_eq!(programs.len(), 2);
    assert!(programs[0].contains("uid: 4242"));
    assert!(programs[0].contains("groups: [\"docker\", \"wheel\"]"));
    assert!(programs[0].contains("create_home: false"));
    assert_eq!(
        programs[1],
        "accounts.modify name: \"nurt-test-r\" lock: true"
    );
}

#[test]
fn the_home_directory_is_removed_through_the_journal_unless_it_is_kept() {
    let sys = FakeSys::linux();
    let home = tempfile::tempdir().unwrap();
    let path = home.path().to_string_lossy().into_owned();
    execute(
        &sys,
        &bound(
            "accounts.create",
            &[("name", text("nurt-test-h")), ("home", text(&path))],
        ),
    );
    let planned = plan(
        &sys,
        &bound("accounts.remove", &[("name", text("nurt-test-h"))]),
    )
    .unwrap();
    assert_eq!(planned.files.len(), 1);
    assert_eq!(planned.files[0].mutation, Mutation::Remove);
    let kept = plan(
        &sys,
        &bound(
            "accounts.remove",
            &[
                ("name", text("nurt-test-h")),
                ("keep_home", Value::Bool(true)),
            ],
        ),
    )
    .unwrap();
    assert!(kept.files.is_empty());
    assert_eq!(planned.calls[0].line(), "userdel nurt-test-h");
}

#[test]
fn the_order_of_groups_does_not_make_a_change() {
    let sys = FakeSys::linux();
    execute(
        &sys,
        &bound(
            "accounts.create",
            &[
                ("name", text("nurt-test-o")),
                ("groups", list(&["wheel", "docker"])),
            ],
        ),
    );
    let same = plan(
        &sys,
        &bound(
            "accounts.modify",
            &[
                ("name", text("nurt-test-o")),
                ("groups", list(&["wheel", "docker", "wheel"])),
            ],
        ),
    )
    .unwrap();
    assert!(same.unchanged.is_some());
}

#[test]
fn mount_options_may_hold_values_but_one_option_is_one_element() {
    let sys = FakeSys::linux();
    let ok = bound(
        "mount",
        &[
            ("device", text("/dev/loop7")),
            ("target", text("/mnt/o")),
            ("options", list(&["uid=1000", "ro"])),
        ],
    );
    assert_eq!(
        plan(&sys, &ok).unwrap().calls[0].line(),
        "mount -o uid=1000,ro /dev/loop7 /mnt/o"
    );
    let joined = bound(
        "mount",
        &[
            ("device", text("/dev/loop7")),
            ("target", text("/mnt/o")),
            ("options", list(&["ro,noexec"])),
        ],
    );
    assert_eq!(error_code(&sys, &joined), ErrorCode::InvalidValue);
}

#[test]
fn the_superuser_is_never_removed() {
    let sys = FakeSys::linux();
    assert_eq!(
        error_code(&sys, &bound("accounts.remove", &[("name", text("root"))])),
        ErrorCode::InvalidValue
    );
    assert_eq!(
        error_code(
            &sys,
            &bound("accounts.remove", &[("name", text("nurt-test-none"))])
        ),
        ErrorCode::NotFound
    );
}

#[test]
fn a_password_goes_to_standard_input_and_is_irreversible() {
    let sys = FakeSys::linux();
    let args = bound(
        "accounts.password",
        &[("name", text("root")), ("value", text("s3cr:et"))],
    );
    let planned = execute(&sys, &args);
    assert_eq!(sys.snapshot().passwords["root"], "s3cr:et");
    let calls = sys.calls();
    assert!(calls.iter().all(|line| !line.contains("s3cr")), "{calls:?}");
    assert!(planned.irreversible.is_some());
    assert!(matches!(
        derive("accounts.password", &planned),
        Inverse::Irreversible(_)
    ));
    let dump = format!("{:?}", changes(&planned));
    assert!(!dump.contains("s3cr"));
}

#[test]
fn groups_are_created_changed_and_removed_pairwise() {
    let sys = FakeSys::linux();
    round_trip(
        &sys,
        &bound(
            "groups.create",
            &[("name", text("nurt-test-g")), ("gid", Value::Int(5151))],
        ),
    );
    execute(
        &sys,
        &bound("groups.create", &[("name", text("nurt-test-g"))]),
    );
    execute(
        &sys,
        &bound("accounts.create", &[("name", text("nurt-test-u"))]),
    );
    let add = bound(
        "groups.add_member",
        &[
            ("name", text("nurt-test-g")),
            ("member", text("nurt-test-u")),
        ],
    );
    let planned = round_trip(&sys, &add);
    assert_eq!(
        derive("groups.add_member", &planned),
        Inverse::Programs(vec![
            "groups.remove_member name: \"nurt-test-g\" member: \"nurt-test-u\"".to_owned()
        ])
    );
    execute(&sys, &add);
    let remove = bound(
        "groups.remove_member",
        &[
            ("name", text("nurt-test-g")),
            ("member", text("nurt-test-u")),
        ],
    );
    round_trip(&sys, &remove);
    let drop = bound("groups.remove", &[("name", text("nurt-test-g"))]);
    let planned = round_trip(&sys, &drop);
    let Inverse::Programs(programs) = derive("groups.remove", &planned) else {
        panic!("programs expected");
    };
    assert!(programs[0].starts_with("groups.create name: \"nurt-test-g\" gid: "));
    assert_eq!(
        programs[1],
        "groups.add_member name: \"nurt-test-g\" member: \"nurt-test-u\""
    );
}

#[test]
fn membership_that_already_holds_is_unchanged() {
    let sys = FakeSys::linux();
    let planned = plan(
        &sys,
        &bound(
            "groups.remove_member",
            &[("name", text("wheel")), ("member", text("nobody"))],
        ),
    )
    .unwrap();
    assert!(planned.unchanged.is_some());
    assert_eq!(
        error_code(&sys, &bound("groups.create", &[("name", text("wheel"))])),
        ErrorCode::Exists
    );
    assert_eq!(
        error_code(&sys, &bound("groups.remove", &[("name", text("root"))])),
        ErrorCode::InvalidValue
    );
}

#[test]
fn a_mount_is_undone_by_unmount_and_back() {
    let sys = FakeSys::linux();
    let mount = bound(
        "mount",
        &[
            ("device", text("/dev/loop7")),
            ("target", text("/mnt/nurt-test")),
            ("type", text("ext4")),
            ("options", list(&["ro", "noatime"])),
        ],
    );
    let planned = round_trip(&sys, &mount);
    assert_eq!(
        derive("mount", &planned),
        Inverse::Programs(vec!["unmount target: \"/mnt/nurt-test\"".to_owned()])
    );
    execute(&sys, &mount);
    let unmount = bound("unmount", &[("target", text("/mnt/nurt-test"))]);
    let planned = round_trip(&sys, &unmount);
    assert_eq!(
        derive("unmount", &planned),
        Inverse::Programs(vec![
            "mount device: \"/dev/loop7\" target: \"/mnt/nurt-test\" type: \"ext4\" options: [\"ro\", \"noatime\"]"
                .to_owned()
        ])
    );
}

#[test]
fn mount_states_are_checked_before_any_change() {
    let sys = FakeSys::linux();
    let mount = bound(
        "mount",
        &[("device", text("/dev/loop7")), ("target", text("/mnt/a"))],
    );
    execute(&sys, &mount);
    assert!(plan(&sys, &mount).unwrap().unchanged.is_some());
    let other = bound(
        "mount",
        &[("device", text("/dev/loop8")), ("target", text("/mnt/a"))],
    );
    assert_eq!(error_code(&sys, &other), ErrorCode::InUse);
    assert_eq!(
        error_code(&sys, &bound("unmount", &[("target", text("/mnt/none"))])),
        ErrorCode::NotFound
    );
}

#[test]
fn a_persistent_mount_edits_fstab_and_the_edit_is_a_journaled_file() {
    let sys = FakeSys::linux();
    sys.set_file("/etc/fstab", "# comment\nUUID=1 / ext4 defaults 0 1\n");
    let mount = bound(
        "mount",
        &[
            ("device", text("/dev/loop7")),
            ("target", text("/mnt/p")),
            ("type", text("ext4")),
            ("persistent", Value::Bool(true)),
        ],
    );
    let planned = execute(&sys, &mount);
    assert_eq!(planned.files.len(), 1);
    assert_eq!(
        sys.file("/etc/fstab").unwrap(),
        "# comment\nUUID=1 / ext4 defaults 0 1\n/dev/loop7 /mnt/p ext4 defaults 0 0\n"
    );
    let unmount = bound(
        "unmount",
        &[
            ("target", text("/mnt/p")),
            ("persistent", Value::Bool(true)),
        ],
    );
    execute(&sys, &unmount);
    assert_eq!(
        sys.file("/etc/fstab").unwrap(),
        "# comment\nUUID=1 / ext4 defaults 0 1\n"
    );
}

#[test]
fn fstab_lines_are_replaced_by_mount_point_and_never_duplicated() {
    use super::mounts::{MountState, fstab_remove, fstab_set};
    let state = MountState {
        device: "/dev/sdb1".to_owned(),
        fstype: String::new(),
        options: vec!["ro".to_owned()],
        persistent: true,
    };
    let text = "/dev/old /srv ext4 defaults 0 2\n#/dev/x /srv x\n";
    let set = fstab_set(text, &state, "/srv").unwrap();
    assert_eq!(set, "/dev/sdb1 /srv auto ro 0 0\n#/dev/x /srv x\n");
    assert_eq!(fstab_set(&set, &state, "/srv").unwrap(), set);
    assert_eq!(fstab_remove(&set, "/srv"), "#/dev/x /srv x\n");
    assert_eq!(fstab_remove("", "/srv"), "");
    let spaced = MountState {
        device: "a b".to_owned(),
        ..state
    };
    assert!(fstab_set("", &spaced, "/srv").is_err());
}

#[test]
fn a_kernel_parameter_is_undone_to_its_previous_value() {
    let sys = FakeSys::linux();
    let set = bound(
        "sysctl.set",
        &[("key", text("vm.swappiness")), ("value", text("10"))],
    );
    let planned = round_trip(&sys, &set);
    assert_eq!(
        derive("sysctl.set", &planned),
        Inverse::Programs(vec![
            "sysctl.set key: \"vm.swappiness\" value: \"60\"".to_owned()
        ])
    );
    assert_eq!(sys.calls()[1], "sysctl -w vm.swappiness=10");
}

#[test]
fn a_kernel_parameter_is_checked_and_compared() {
    let sys = FakeSys::linux();
    assert!(
        plan(
            &sys,
            &bound(
                "sysctl.set",
                &[("key", text("vm.swappiness")), ("value", text(" 60"))]
            )
        )
        .unwrap()
        .unchanged
        .is_some()
    );
    assert_eq!(
        error_code(
            &sys,
            &bound(
                "sysctl.set",
                &[("key", text("no.such")), ("value", text("1"))]
            )
        ),
        ErrorCode::NotFound
    );
    assert_eq!(
        error_code(
            &sys,
            &bound("sysctl.set", &[("key", text("-w")), ("value", text("1"))])
        ),
        ErrorCode::InvalidValue
    );
    let windows = FakeSys::linux().with_os(Os::Windows);
    assert_eq!(
        error_code(
            &windows,
            &bound(
                "sysctl.set",
                &[
                    ("key", text("a")),
                    ("value", text("1")),
                    ("persistent", Value::Bool(true))
                ]
            )
        ),
        ErrorCode::Unsupported
    );
}

#[test]
fn persistent_values_go_to_the_file_of_nurt() {
    use super::kernel::sysctl_file_set;
    let sys = FakeSys::linux();
    let set = bound(
        "sysctl.set",
        &[
            ("key", text("vm.swappiness")),
            ("value", text("10")),
            ("persistent", Value::Bool(true)),
        ],
    );
    let planned = execute(&sys, &set);
    assert_eq!(planned.files.len(), 1);
    assert_eq!(
        sys.file("/etc/sysctl.d/90-nurt.conf").unwrap(),
        "vm.swappiness = 10\n"
    );
    assert_eq!(
        sysctl_file_set("# x\nvm.swappiness=3\nnet.a = 1\n", "vm.swappiness", "7").unwrap(),
        "# x\nvm.swappiness = 7\nnet.a = 1\n"
    );
    let again = plan(&sys, &set).unwrap();
    assert!(again.unchanged.is_some());
}

#[test]
fn modules_are_loaded_and_unloaded_pairwise() {
    let sys = FakeSys::linux();
    let planned = round_trip(
        &sys,
        &bound("modules.load", &[("name", text("br-netfilter"))]),
    );
    assert_eq!(
        derive("modules.load", &planned),
        Inverse::Programs(vec!["modules.unload name: \"br-netfilter\"".to_owned()])
    );
    round_trip(&sys, &bound("modules.unload", &[("name", text("loop"))]));
    assert!(
        plan(&sys, &bound("modules.load", &[("name", text("loop"))]))
            .unwrap()
            .unchanged
            .is_some()
    );
    let windows = FakeSys::linux().with_os(Os::Windows);
    assert_eq!(
        error_code(&windows, &bound("modules.load", &[("name", text("x"))])),
        ErrorCode::OsOnly
    );
}

#[test]
fn a_scheduled_restart_is_undone_by_cancel_and_an_immediate_one_is_irreversible() {
    let sys = FakeSys::linux();
    let scheduled = bound(
        "power.restart",
        &[
            ("delay", Value::Duration(Duration::from_millis(90_000))),
            ("message", text("Maintenance")),
        ],
    );
    let planned = round_trip(&sys, &scheduled);
    assert_eq!(sys.calls()[0], "shutdown -r +2 Maintenance");
    assert_eq!(planned.changes[0].target, "power:schedule");
    assert_eq!(
        derive("power.restart", &planned),
        Inverse::Programs(vec!["power.cancel".to_owned()])
    );
    let now = plan(&sys, &bound("power.shutdown", &[])).unwrap();
    assert_eq!(now.calls[0].line(), "shutdown -P now");
    assert!(now.irreversible.is_some());
    assert!(matches!(
        derive("power.shutdown", &now),
        Inverse::Irreversible(_)
    ));
}

#[test]
fn at_is_converted_with_the_clock_and_the_past_is_refused() {
    let sys = FakeSys::linux();
    let ahead = nurt_contracts::value::Instant::from_unix_nanos(
        1_700_000_000 * 1_000_000_000 + 600 * 1_000_000_000,
    );
    let planned = plan(
        &sys,
        &bound("power.shutdown", &[("at", Value::Instant(ahead))]),
    )
    .unwrap();
    assert_eq!(planned.calls[0].line(), "shutdown -P +10");
    let past = nurt_contracts::value::Instant::from_unix_nanos(1_600_000_000 * 1_000_000_000);
    assert_eq!(
        error_code(
            &sys,
            &bound("power.restart", &[("at", Value::Instant(past))])
        ),
        ErrorCode::InvalidValue
    );
    assert_eq!(
        error_code(
            &sys,
            &bound(
                "power.restart",
                &[
                    ("at", Value::Instant(ahead)),
                    ("delay", Value::Duration(Duration::from_millis(1000)))
                ]
            )
        ),
        ErrorCode::ParamConflict
    );
}

#[test]
fn cancel_needs_a_schedule_and_a_second_schedule_needs_a_cancel() {
    let sys = FakeSys::linux();
    assert!(
        plan(&sys, &bound("power.cancel", &[]))
            .unwrap()
            .unchanged
            .is_some()
    );
    let scheduled = bound(
        "power.shutdown",
        &[("delay", Value::Duration(Duration::from_millis(60_000)))],
    );
    execute(&sys, &scheduled);
    assert_eq!(error_code(&sys, &scheduled), ErrorCode::InUse);
    let cancel = execute(&sys, &bound("power.cancel", &[]));
    assert_eq!(sys.snapshot().schedule, None);
    assert!(matches!(
        derive("power.cancel", &cancel),
        Inverse::Irreversible(_)
    ));
    assert_eq!(
        plan(&sys, &bound("power.cancel", &[]))
            .unwrap()
            .unchanged
            .is_some(),
        true
    );
}

#[test]
fn power_programs_of_windows_and_macos() {
    let windows = FakeSys::linux().with_os(Os::Windows);
    let planned = plan(
        &windows,
        &bound(
            "power.restart",
            &[
                ("delay", Value::Duration(Duration::from_millis(30_000))),
                ("message", text("Hi")),
            ],
        ),
    )
    .unwrap();
    assert_eq!(planned.calls[0].line(), "shutdown /r /t 30 /c Hi");
    assert_eq!(
        plan(&windows, &bound("power.cancel", &[])).unwrap().calls[0].line(),
        "shutdown /a"
    );
    let mac = FakeSys::linux().with_os(Os::Macos);
    let planned = plan(
        &mac,
        &bound(
            "power.shutdown",
            &[("delay", Value::Duration(Duration::from_millis(300_000)))],
        ),
    )
    .unwrap();
    assert_eq!(planned.calls[0].line(), "shutdown -h +5");
}

const DISK: &str = r#"{"blockdevices":[{"name":"/dev/loop9","size":"104857600","fstype":null,"label":null,"mountpoint":null,"fsused":null,
 "children":[{"name":"/dev/loop9p1","size":52428800,"fstype":"ext4","label":"data","mountpoint":null,"fsused":"1048576"},
 {"name":"/dev/loop9p2","size":52428800,"fstype":"ext4","label":null,"mountpoint":"/srv","fsused":"2097152"}]}]}"#;

#[test]
fn the_preview_of_a_disk_change_lists_the_layout_and_the_data_lost() {
    let sys = FakeSys::linux();
    sys.state.lock().unwrap().lsblk = DISK.to_owned();
    let planned = plan(
        &sys,
        &bound(
            "disks.partition",
            &[
                ("device", text("/dev/loop9")),
                ("action", text("delete")),
                ("number", Value::Int(1)),
            ],
        ),
    )
    .unwrap();
    assert_eq!(
        planned.calls[0].line(),
        "parted -s -a optimal /dev/loop9 rm 1"
    );
    assert!(planned.notes[0].contains("/dev/loop9p1 52428800 bytes fs=ext4 label=data mount=-"));
    assert!(planned.notes[0].contains("/dev/loop9p2"));
    assert_eq!(planned.notes[1], "data that will be lost: 1048576 bytes");
    assert!(
        planned
            .irreversible
            .as_deref()
            .unwrap()
            .contains("1048576 bytes")
    );
    assert!(matches!(
        derive("disks.partition", &planned),
        Inverse::Irreversible(_)
    ));
}

#[test]
fn a_mounted_partition_or_device_is_refused() {
    let sys = FakeSys::linux();
    sys.state.lock().unwrap().lsblk = DISK.to_owned();
    assert_eq!(
        error_code(
            &sys,
            &bound(
                "disks.partition",
                &[
                    ("device", text("/dev/loop9")),
                    ("action", text("resize")),
                    ("number", Value::Int(2)),
                    (
                        "size",
                        Value::Size(nurt_contracts::value::Size::from_bytes(1 << 20))
                    )
                ]
            )
        ),
        ErrorCode::InUse
    );
    assert_eq!(
        error_code(
            &sys,
            &bound(
                "disks.format",
                &[("device", text("/dev/loop9")), ("fs", text("ext4"))]
            )
        ),
        ErrorCode::InUse
    );
    assert_eq!(
        error_code(
            &sys,
            &bound(
                "disks.format",
                &[("device", text("sda")), ("fs", text("ext4"))]
            )
        ),
        ErrorCode::InvalidValue
    );
}

#[test]
fn a_format_names_the_file_system_label_and_device() {
    let sys = FakeSys::linux();
    sys.state.lock().unwrap().lsblk = r#"{"blockdevices":[{"name":"/dev/loop9p1","size":1000,"fstype":"ext4","label":"old","mountpoint":null,"fsused":"500"}]}"#.to_owned();
    let planned = plan(
        &sys,
        &bound(
            "disks.format",
            &[
                ("device", text("/dev/loop9p1")),
                ("fs", text("xfs")),
                ("label", text("new")),
            ],
        ),
    )
    .unwrap();
    assert_eq!(planned.calls[0].line(), "mkfs.xfs -f -L new /dev/loop9p1");
    assert_eq!(planned.notes[1], "data that will be lost: 500 bytes");
    assert!(planned.irreversible.is_some());
    assert_eq!(
        error_code(
            &sys,
            &bound(
                "disks.format",
                &[("device", text("/dev/loop9p1")), ("fs", text("zfs"))]
            )
        ),
        ErrorCode::InvalidValue
    );
}

#[test]
fn partition_paths_follow_the_naming_of_the_device() {
    use super::disks::{parse_parted_start, partition_path};
    assert_eq!(partition_path("/dev/sda", 1), "/dev/sda1");
    assert_eq!(partition_path("/dev/nvme0n1", 2), "/dev/nvme0n1p2");
    assert_eq!(partition_path("/dev/loop3", 1), "/dev/loop3p1");
    assert_eq!(
        parse_parted_start(
            "BYT;\n/dev/sdb:100B:scsi:512:512:gpt:x:;\n1:1048576B:5242879B:4194304B:ext4::;\n",
            1
        ),
        Some(1_048_576)
    );
    assert_eq!(parse_parted_start("1:1B:2B:2B:::;", 2), None);
}

#[test]
fn windows_and_macos_programs_are_built_without_a_shell() {
    let windows = FakeSys::linux().with_os(Os::Windows);
    let planned = plan(
        &windows,
        &bound(
            "accounts.create",
            &[
                ("name", text("nurt-test-w")),
                ("comment", text("A B")),
                ("groups", list(&["Administrators"])),
            ],
        ),
    );
    let planned = planned.unwrap();
    assert_eq!(
        planned.calls[0].line(),
        "net user nurt-test-w /add /fullname:A B"
    );
    assert_eq!(
        planned.calls[1].line(),
        "net localgroup Administrators nurt-test-w /add"
    );
    let password = plan(
        &windows,
        &bound(
            "accounts.password",
            &[("name", text("root")), ("value", text("pw"))],
        ),
    );
    assert_eq!(password.unwrap_err().code(), ErrorCode::NotFound);
    let mac = FakeSys::linux().with_os(Os::Macos);
    let planned = plan(
        &mac,
        &bound(
            "accounts.create",
            &[
                ("name", text("nurt-test-x")),
                ("uid", Value::Int(600)),
                ("shell", text("/bin/zsh")),
            ],
        ),
    )
    .unwrap();
    assert_eq!(
        planned.calls[0].line(),
        "sysadminctl -addUser nurt-test-x -UID 600 -shell /bin/zsh"
    );
    let volume = plan(
        &windows,
        &bound(
            "mount",
            &[
                ("device", text("\\\\?\\Volume{1}\\")),
                ("target", text("X:\\")),
            ],
        ),
    )
    .unwrap();
    assert_eq!(volume.calls[0].line(), "mountvol X:\\ \\\\?\\Volume{1}\\");
    assert_eq!(
        error_code(
            &mac,
            &bound(
                "mount",
                &[
                    ("device", text("/dev/x")),
                    ("target", text("/m")),
                    ("persistent", Value::Bool(true))
                ]
            )
        ),
        ErrorCode::Unsupported
    );
}

#[test]
fn outputs_of_the_system_programs_are_parsed() {
    use super::accounts::{parse_dscl, parse_locked, parse_net_user, parse_passwd};
    use super::groups::{parse_dscl_group, parse_group_line, parse_net_localgroup};
    use super::mounts::{parse_findmnt, parse_mount_list};
    let passwd = parse_passwd("ann:x:1001:1001:Ann,,,:/home/ann:/bin/bash").unwrap();
    assert_eq!((passwd.uid, passwd.comment.as_str()), (Some(1001), "Ann"));
    assert!(parse_locked("ann L 2020-01-01 0 99999 7 -1"));
    assert!(!parse_locked("ann P 2020-01-01"));
    let dscl = parse_dscl(
        "NFSHomeDirectory: /Users/ann\nRealName:\n Ann Smith\nUniqueID: 501\nUserShell: /bin/zsh\n",
    );
    assert_eq!(
        (dscl.uid, dscl.comment.as_str(), dscl.home.as_deref()),
        (Some(501), "Ann Smith", Some("/Users/ann"))
    );
    let net = parse_net_user(
        "User name  ann\nFull Name  Ann Smith\nHome directory  C:\\Users\\ann\nAccount active  No\nLocal Group Memberships  *Administrators  *Users\nGlobal Group memberships  *None\nThe command completed successfully.\n",
    );
    assert_eq!(net.comment, "Ann Smith");
    assert!(net.locked);
    assert_eq!(net.groups, ["Administrators"]);
    assert_eq!(
        parse_group_line("ops:x:30:b,a").unwrap().members,
        ["a", "b"]
    );
    assert_eq!(
        parse_dscl_group("PrimaryGroupID: 80\nGroupMembership: root ann\n").gid,
        Some(80)
    );
    assert_eq!(
        parse_net_localgroup(
            "Alias name  ops\n\nMembers\n\n-----\nann\nbob\nThe command completed successfully.\n"
        )
        .members,
        ["ann", "bob"]
    );
    let mount = parse_findmnt("/dev/sdb1 ext4 rw,relatime").unwrap();
    assert_eq!((mount.fstype.as_str(), mount.options.len()), ("ext4", 2));
    let listed = parse_mount_list(
        "/dev/disk3s1 on /Volumes/X (apfs, local, nobrowse)\n",
        "/Volumes/X",
    )
    .unwrap();
    assert_eq!(
        (listed.device.as_str(), listed.options.len()),
        ("/dev/disk3s1", 2)
    );
}

#[test]
fn texts_that_cannot_be_written_in_a_program_make_the_undo_irreversible() {
    let state = "{\"gid\":7,\"members\":[\"a\\nb\"]}";
    let result = inverse("groups.remove", &[("group:g", Some(state), None)]).unwrap();
    assert!(matches!(result, Inverse::Irreversible(_)));
    assert_eq!(inverse("write", &[]), None);
    assert_eq!(inverse("accounts.create", &[]), Some(Inverse::None));
}

#[test]
fn every_command_has_a_stage_and_a_plan() {
    let stages = super::stages();
    assert_eq!(stages.len(), super::COMMANDS.len());
    for (stage, command) in stages.iter().zip(super::COMMANDS) {
        let descriptor = stage.descriptor();
        assert_eq!(descriptor.command, command);
        assert!(descriptor.source);
        assert!(stage.as_effect().is_some());
    }
}

/// Tests on a real Linux machine as root, run by the coordinator on the pool
/// machine. Every test removes what it created: accounts and groups carry the
/// prefix `nurt-test-`, loop devices are detached and files are deleted.
mod system {
    use std::path::PathBuf;

    use super::super::accounts;
    use super::super::sys::{Call, ProcessSys, Sys};
    use super::*;

    const ROOT: &str = "needs root on Linux; run on the pool machine";

    fn real(args: &BoundArgs) -> Plan {
        let sys = ProcessSys;
        let planned = plan(&sys, args).unwrap_or_else(|e| panic!("{}: {e}", args.command()));
        if planned.unchanged.is_some() {
            return planned;
        }
        for call in &planned.calls {
            let output = sys.run(call).unwrap();
            assert!(output.ok(), "{} failed: {}", call.line(), output.stderr);
        }
        for op in &planned.files {
            let nurt_contracts::journal::TargetPath::Fs(path) = &op.target.path else {
                continue;
            };
            match &op.mutation {
                Mutation::WriteBytes(bytes) => std::fs::write(path.as_path(), bytes).unwrap(),
                Mutation::Remove => {
                    let _ = std::fs::remove_dir_all(path.as_path());
                }
                other => panic!("unexpected mutation {other:?}"),
            }
        }
        planned
    }

    fn undo(kind: &str, planned: &Plan) {
        match derive(kind, planned) {
            Inverse::Programs(programs) => {
                for program in programs {
                    real(&parse_program(&program));
                }
            }
            other => panic!("{kind}: {other:?}"),
        }
    }

    fn shell(program: &str, args: &[&str]) -> String {
        let output = ProcessSys
            .run(&Call::new(program, args.iter().copied()))
            .unwrap();
        output.stdout.trim().to_owned()
    }

    struct Cleanup(Vec<Call>);

    impl Drop for Cleanup {
        fn drop(&mut self) {
            for call in self.0.iter().rev() {
                let _ = ProcessSys.run(call);
            }
        }
    }

    #[test]
    #[ignore = "needs root on Linux; run on the pool machine"]
    fn accounts_and_groups_change_and_undo_on_the_system() {
        let _ = ROOT;
        let _cleanup = Cleanup(vec![
            Call::new("userdel", ["-r", "nurt-test-acct"]),
            Call::new("groupdel", ["nurt-test-grp"]),
            Call::new("groupdel", ["nurt-test-acct"]),
        ]);
        let group = bound("groups.create", &[("name", text("nurt-test-grp"))]);
        real(&group);
        let create = bound(
            "accounts.create",
            &[
                ("name", text("nurt-test-acct")),
                ("shell", text("/bin/sh")),
                ("groups", list(&["nurt-test-grp"])),
                ("comment", text("Nurt test")),
            ],
        );
        let created = real(&create);
        let state = accounts::read(&ProcessSys, "nurt-test-acct")
            .unwrap()
            .unwrap();
        assert_eq!(state.groups, ["nurt-test-grp"]);
        assert_eq!(state.comment, "Nurt test");
        let modified = real(&bound(
            "accounts.modify",
            &[
                ("name", text("nurt-test-acct")),
                ("shell", text("/bin/bash")),
                ("lock", Value::Bool(true)),
            ],
        ));
        let state = accounts::read(&ProcessSys, "nurt-test-acct")
            .unwrap()
            .unwrap();
        assert_eq!(state.shell.as_deref(), Some("/bin/bash"));
        assert!(state.locked);
        undo("accounts.modify", &modified);
        let state = accounts::read(&ProcessSys, "nurt-test-acct")
            .unwrap()
            .unwrap();
        assert_eq!(state.shell.as_deref(), Some("/bin/sh"));
        assert!(!state.locked);
        real(&bound(
            "accounts.password",
            &[
                ("name", text("nurt-test-acct")),
                ("value", text("Nurt-test-1")),
            ],
        ));
        let removed = real(&bound(
            "accounts.remove",
            &[
                ("name", text("nurt-test-acct")),
                ("keep_home", Value::Bool(true)),
            ],
        ));
        assert!(
            accounts::read(&ProcessSys, "nurt-test-acct")
                .unwrap()
                .is_none()
        );
        undo("accounts.remove", &removed);
        assert!(
            accounts::read(&ProcessSys, "nurt-test-acct")
                .unwrap()
                .is_some()
        );
        undo("accounts.create", &created);
        assert!(
            accounts::read(&ProcessSys, "nurt-test-acct")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    #[ignore = "needs root on Linux; run on the pool machine"]
    fn a_loop_device_is_mounted_unmounted_and_remounted_on_the_system() {
        let dir = tempfile::tempdir().unwrap();
        let image: PathBuf = dir.path().join("nurt-test.img");
        let target = dir.path().join("mnt");
        std::fs::create_dir(&target).unwrap();
        let file = std::fs::File::create(&image).unwrap();
        file.set_len(16 * 1024 * 1024).unwrap();
        let device = shell("losetup", &["-f", "--show", image.to_str().unwrap()]);
        assert!(device.starts_with("/dev/loop"), "{device}");
        let _cleanup = Cleanup(vec![
            Call::new("losetup", ["-d", device.as_str()]),
            Call::new("umount", [target.to_str().unwrap()]),
        ]);
        assert!(
            ProcessSys
                .run(&Call::new("mkfs.ext4", ["-q", "-F", device.as_str()]))
                .unwrap()
                .ok()
        );
        let target_text = target.to_str().unwrap();
        let mount = bound(
            "mount",
            &[
                ("device", text(&device)),
                ("target", text(target_text)),
                ("type", text("ext4")),
                ("options", list(&["noatime"])),
            ],
        );
        let mounted = real(&mount);
        assert!(!shell("findmnt", &["-n", "-M", target_text]).is_empty());
        let unmounted = real(&bound("unmount", &[("target", text(target_text))]));
        assert!(shell("findmnt", &["-n", "-M", target_text]).is_empty());
        undo("unmount", &unmounted);
        assert!(!shell("findmnt", &["-n", "-M", target_text]).is_empty());
        undo("mount", &mounted);
        assert!(shell("findmnt", &["-n", "-M", target_text]).is_empty());
    }

    #[test]
    #[ignore = "needs root on Linux; run on the pool machine"]
    fn a_kernel_parameter_and_a_module_change_and_undo_on_the_system() {
        let before = shell("sysctl", &["-n", "vm.swappiness"]);
        let wanted = if before == "11" { "12" } else { "11" };
        let set = real(&bound(
            "sysctl.set",
            &[("key", text("vm.swappiness")), ("value", text(wanted))],
        ));
        assert_eq!(shell("sysctl", &["-n", "vm.swappiness"]), wanted);
        undo("sysctl.set", &set);
        assert_eq!(shell("sysctl", &["-n", "vm.swappiness"]), before);
        if ProcessSys
            .run(&Call::new("modprobe", ["-n", "dummy"]))
            .map_or(false, |o| o.ok())
        {
            let _cleanup = Cleanup(vec![Call::new("modprobe", ["-r", "dummy"])]);
            let loaded = real(&bound("modules.load", &[("name", text("dummy"))]));
            assert!(super::super::kernel::is_loaded(&ProcessSys, "dummy").unwrap());
            undo("modules.load", &loaded);
            assert!(!super::super::kernel::is_loaded(&ProcessSys, "dummy").unwrap());
        }
    }

    #[test]
    #[ignore = "needs root and systemd on Linux; run on the pool machine"]
    fn a_scheduled_restart_is_cancelled_on_the_system() {
        let _cleanup = Cleanup(vec![Call::new("shutdown", ["-c"])]);
        let scheduled = real(&bound(
            "power.restart",
            &[("delay", Value::Duration(Duration::from_millis(3_600_000)))],
        ));
        assert!(std::path::Path::new("/run/systemd/shutdown/scheduled").exists());
        undo("power.restart", &scheduled);
        assert!(!std::path::Path::new("/run/systemd/shutdown/scheduled").exists());
    }

    #[test]
    #[ignore = "needs root on Linux with parted and mkfs; run on the pool machine"]
    fn a_loop_device_is_partitioned_and_formatted_on_the_system() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("nurt-test-disk.img");
        std::fs::File::create(&image)
            .unwrap()
            .set_len(64 * 1024 * 1024)
            .unwrap();
        let device = shell("losetup", &["-f", "--show", "-P", image.to_str().unwrap()]);
        let _cleanup = Cleanup(vec![Call::new("losetup", ["-d", device.as_str()])]);
        assert!(
            ProcessSys
                .run(&Call::new(
                    "parted",
                    ["-s", device.as_str(), "mklabel", "gpt"]
                ))
                .unwrap()
                .ok()
        );
        real(&bound(
            "disks.partition",
            &[
                ("device", text(&device)),
                ("action", text("create")),
                (
                    "size",
                    Value::Size(nurt_contracts::value::Size::from_bytes(32 << 20)),
                ),
            ],
        ));
        let part = format!("{device}p1");
        shell("partprobe", &[&device]);
        let formatted = real(&bound(
            "disks.format",
            &[
                ("device", text(&part)),
                ("fs", text("ext4")),
                ("label", text("nurt-test")),
            ],
        ));
        assert_eq!(shell("lsblk", &["-n", "-o", "FSTYPE", &part]), "ext4");
        assert!(matches!(
            derive("disks.format", &formatted),
            Inverse::Irreversible(_)
        ));
        real(&bound(
            "disks.partition",
            &[
                ("device", text(&device)),
                ("action", text("delete")),
                ("number", Value::Int(1)),
            ],
        ));
    }
}
