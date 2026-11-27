// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-migrate/tests/migrate/constructs.rs
// Responsibility:   Tests of the migration of the bash constructs and administration programs of section 11 of the language brief.
// Dependencies:     nurt-migrate, nurt-syntax
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Migration of the bash forms of section 11 of the language brief: the
//! conditions, loops, functions, script parameters, chains of changes and the
//! administration programs of a whole script. Every program that the
//! migration writes is read by the parser of the language 1.1.

use nurt_migrate::{MigrateOptions, Migration, ScriptLanguage, Status, migrate};
use nurt_syntax::parse::{ParseOptions, Variant, parse};

/// Migrates `source` as a bash script and checks that nothing is left
/// untranslated and that the language reads the whole program.
fn translated(source: &str) -> Migration {
    let options = MigrateOptions::new().with_script("script.sh");
    let migration = match migrate(source, ScriptLanguage::Bash, &options) {
        Ok(migration) => migration,
        Err(error) => panic!("migration failed: {error:?}"),
    };
    for item in &migration.report.items {
        assert!(
            matches!(item.status, Status::Translated | Status::Assumption),
            "{} -> {:?} {:?}",
            item.source,
            item.status,
            item.problem
        );
    }
    if let Err(error) = parse(&migration.nurt, Variant::A, &ParseOptions::default()) {
        panic!("the language does not read\n{}\n{error:?}", migration.nurt);
    }
    migration
}

/// Checks that every fragment is in the program, in the given order.
fn contains_in_order(program: &str, fragments: &[&str]) {
    let mut rest = program;
    for fragment in fragments {
        match rest.find(fragment) {
            Some(at) => rest = &rest[at + fragment.len()..],
            None => panic!("`{fragment}` is missing or out of order in\n{program}"),
        }
    }
}

#[test]
fn conditions_become_when() {
    let migration = translated(
        "if [ -f /srv/a ]; then rm /srv/a; elif [ -d /srv/b ]; then rm -rf /srv/b; else mkdir -p /srv/c; fi\n",
    );
    contains_in_order(
        &migration.nurt,
        &[
            "when exists(\"/srv/a\")",
            "remove \"/srv/a\"",
            "} otherwise when exists(\"/srv/b\")",
            "} otherwise {",
            "create \"/srv/c\" type: directory",
        ],
    );
}

#[test]
fn case_becomes_a_chain_on_one_value() {
    let migration = translated(
        "case \"$1\" in\n  start) mkdir -p /srv/a;;\n  stop|halt) rm -rf /srv/a;;\n  *) touch /srv/c;;\nesac\n",
    );
    contains_in_order(
        &migration.nurt,
        &[
            "params {",
            "arg1: text",
            "when @arg1 == \"start\" {",
            "} otherwise when @arg1 == \"stop\" or @arg1 == \"halt\" {",
            "} otherwise {",
        ],
    );
}

#[test]
fn loops_become_each_while_and_repeat() {
    let migration = translated(
        "for f in /srv/in/*.csv; do gzip \"$f\"; done\n\
         while [ -f /srv/lock ]; do rm -f /srv/lock; done\n\
         until [ -f /srv/ready ]; do touch /srv/probe; sleep 5; done\n",
    );
    contains_in_order(
        &migration.nurt,
        &[
            "each (find \"/srv/in\" glob: \"*.csv\"",
            ") as f {",
            "compress @f.path",
            "while exists(\"/srv/lock\")",
            "repeat until: exists(\"/srv/ready\")",
            "every: 5s",
        ],
    );
}

#[test]
fn functions_become_define_and_calls_stay_calls() {
    let migration = translated(
        "rotate() {\n  find \"$1\" -name \"*.log\" -delete\n}\nrotate /var/log/app1\nrotate /var/log/app2\n",
    );
    contains_in_order(
        &migration.nurt,
        &[
            "define rotate(arg1: path) {",
            "|> remove",
            "rotate \"/var/log/app1\"",
            "rotate \"/var/log/app2\"",
        ],
    );
}

#[test]
fn positional_parameters_become_params() {
    let migration = translated("dir=\"$1\"\nkeep=${2:-7}\nrm -rf \"$dir\"\n");
    contains_in_order(
        &migration.nurt,
        &["params {", "dir: path", "keep: text = \"7\"", "remove @dir"],
    );
}

#[test]
fn the_rest_of_the_arguments_becomes_a_list_parameter() {
    let migration = translated("for f in \"$@\"; do rm -f \"$f\"; done\n");
    contains_in_order(
        &migration.nurt,
        &["args: list<text> = []", "each @args as f {", "remove @f"],
    );
}

#[test]
fn shift_moves_the_numbering_of_the_parameters() {
    let migration = translated("mode=$1\nshift\nrm -rf \"$1\"\n");
    contains_in_order(
        &migration.nurt,
        &["mode: text", "arg2: path", "remove @arg2"],
    );
}

#[test]
fn getopts_declares_one_parameter_per_option() {
    let migration = translated(
        "while getopts \"d:v\" opt; do\n  case $opt in\n    d) dir=$OPTARG;;\n    v) verbose=1;;\n  esac\ndone\nrm -rf \"$dir\"\n",
    );
    contains_in_order(
        &migration.nurt,
        &["dir: text", "verbose: bool = false", "remove @dir"],
    );
}

#[test]
fn a_parameter_without_a_name_is_an_assumption() {
    let migration = translated("rm -rf \"$1\"\n");
    assert_eq!(migration.report.items[0].status, Status::Assumption);
    assert!(
        migration.report.items[0]
            .assumptions
            .iter()
            .any(|note| note.contains("arg1"))
    );
}

#[test]
fn a_chain_of_changes_becomes_steps() {
    let migration = translated("mkdir -p /srv/out && cp -a /srv/in/data.csv /srv/out/\n");
    contains_in_order(
        &migration.nurt,
        &[
            "steps {",
            "create \"/srv/out\" type: directory",
            "copy \"/srv/in/data.csv\" into: \"/srv/out\"",
        ],
    );
}

#[test]
fn an_alternative_after_a_test_becomes_when_not() {
    let migration = translated("test -f /srv/app/.env || cp /srv/app/.env.example /srv/app/.env\n");
    contains_in_order(
        &migration.nurt,
        &[
            "when not (exists \"/srv/app/.env\"",
            "copy \"/srv/app/.env.example\"",
        ],
    );
}

#[test]
fn administration_programs_become_native_commands() {
    let source = "\
mkfifo /srv/run/q
mknod /srv/dev/n c 1 3
useradd -m -s /bin/bash deploy
usermod -aG docker deploy
userdel -r bob
groupadd -g 1500 ops
gpasswd -a alice ops
passwd alice
mount -t ext4 /dev/sdb1 /srv/data
umount /srv/data
sysctl -w net.ipv4.ip_forward=1
modprobe overlay
rmmod nfs
ufw allow 443/tcp
firewall-cmd --add-port=8080/tcp
nft add rule inet filter input tcp dport 22 accept
iptables -A INPUT -p tcp --dport 80 -j ACCEPT
ip route add 10.1.0.0/16 via 198.51.100.1
resolvectl dns eth0 1.1.1.1
timedatectl set-timezone Europe/Warsaw
date -s \"2026-11-27 10:00:00\"
shutdown -r +5
reboot
poweroff
parted -s /dev/sdb rm 2
mkfs.ext4 /dev/sdb1
sqlite3 /srv/app.db \"DELETE FROM t\"
";
    let migration = translated(source);
    contains_in_order(
        &migration.nurt,
        &[
            "create \"/srv/run/q\" type: fifo",
            "create \"/srv/dev/n\" type: device",
            "accounts.create name: \"deploy\"",
            "groups.add_member name: \"docker\"",
            "accounts.remove name: \"bob\"",
            "groups.create name: \"ops\"",
            "groups.add_member name: \"ops\"",
            "accounts.password name: \"alice\"",
            "mount device: \"/dev/sdb1\"",
            "unmount target: \"/srv/data\"",
            "sysctl.set key: \"net.ipv4.ip_forward\"",
            "modules.load name: \"overlay\"",
            "modules.unload name: \"nfs\"",
            "firewall.allow port: 443",
            "firewall.allow port: 8080",
            "firewall.allow port: 22",
            "firewall.allow port: 80",
            "network.route_add destination: \"10.1.0.0/16\"",
            "network.dns_set interface: \"eth0\"",
            "clock.timezone_set \"Europe/Warsaw\"",
            "clock.set \"2026-11-27 10:00:00\"",
            "power.restart delay: 5m",
            "power.restart",
            "power.shutdown",
            "disks.partition device: \"/dev/sdb\"",
            "disks.format device: \"/dev/sdb1\"",
            "sql \"/srv/app.db\" \"DELETE FROM t\"",
        ],
    );
}

#[test]
fn elevation_of_an_administration_program_is_a_privilege_option() {
    let migration = translated("sudo ufw allow 443/tcp\nsudo useradd -m eve\n");
    assert_eq!(
        migration.nurt.matches("privilege: elevated").count(),
        2,
        "{}",
        migration.nurt
    );
}

#[test]
fn a_complex_firewall_rule_runs_with_the_elevated_privilege() {
    let migration = translated(
        "sudo iptables -t nat -A PREROUTING -p tcp --dport 80 -j DNAT --to 198.51.100.2\n",
    );
    contains_in_order(
        &migration.nurt,
        &["run [", "\"iptables\"", "] cwd: \"/\" privilege: elevated"],
    );
}

#[test]
fn eval_and_subshells_stay_without_translation() {
    let options = MigrateOptions::new().with_script("script.sh");
    let migration = match migrate(
        "eval \"$CMD\"\n(cd /a && make)\nfor ((i = 0; i < 3; i++)); do echo; done\n",
        ScriptLanguage::Bash,
        &options,
    ) {
        Ok(migration) => migration,
        Err(error) => panic!("migration failed: {error:?}"),
    };
    assert!(
        migration
            .report
            .items
            .iter()
            .all(|item| item.status == Status::Impossible)
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
