// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-correct/tests/bash_constructs.rs
// Responsibility:   Translation of the bash forms of the language 1.1 and of the.
// Dependencies:     nurt_contracts, nurt_syntax
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//
// Module: nurt-correct tests: bash constructs
// Responsibility: Tests of the translation of every row of section 11 of the language brief from the bash form.
// Dependencies: nurt-correct, nurt-syntax, nurt-contracts
// Author: Danaco Official
// Version: 0.1.1, 2026-11-27
// License: see the LICENSE file or https://www.danaco-group.pl
//! Translation of the bash forms of the language 1.1 and of the
//! administration programs: every row of section 11 of the brief, from the
//! bash form to the text of the program. Every translation is also read by
//! the parser of the language, printed again and read again.

// Test helpers report failures by panicking.
#![allow(clippy::panic, clippy::expect_used)]

use nurt_contracts::error::ErrorCode;
use nurt_correct::bash::{FunctionSignature, Mode, TranslateContext, translate_with};
use nurt_syntax::parse::{ParseOptions, Variant, parse};
use nurt_syntax::print::print;

const SCRIPT: TranslateContext<'static> = TranslateContext {
    base: None,
    lets: &[],
    functions: &[],
    mode: Mode::Script,
};

const LIVE: TranslateContext<'static> = TranslateContext {
    base: None,
    lets: &[],
    functions: &[],
    mode: Mode::Live,
};

/// Translates `input` and returns the text of the program, after checking that
/// the language 1.1 reads it and prints it unchanged.
fn translated(input: &str, context: &TranslateContext<'_>) -> String {
    let translation = translate_with(input, context)
        .unwrap_or_else(|| panic!("not a bash habit: {input}"))
        .unwrap_or_else(|error| panic!("{input}: {error:?}"));
    let text = translation.text();
    let tree = parse(&text, Variant::A, &ParseOptions::default())
        .unwrap_or_else(|error| panic!("{input}\n=> {text}\n{error:?}"));
    let printed = print(&tree, Variant::A);
    assert_eq!(
        printed.trim_end(),
        text,
        "{input}: the printed program differs"
    );
    let again = parse(&printed, Variant::A, &ParseOptions::default())
        .unwrap_or_else(|error| panic!("{input}\n=> {printed}\n{error:?}"));
    assert_eq!(print(&again, Variant::A), printed, "{input}: not stable");
    text
}

fn error_of(input: &str, context: &TranslateContext<'_>) -> nurt_contracts::error::Error {
    translate_with(input, context)
        .unwrap_or_else(|| panic!("not a bash habit: {input}"))
        .expect_err("an error")
}

fn check(cases: &[(&str, &str)]) {
    for (input, expected) in cases {
        assert_eq!(translated(input, &SCRIPT), *expected, "{input}");
    }
}

#[test]
fn conditions_translate() {
    check(&[
        (
            "if [ -f /srv/a ]; then rm /srv/a; fi",
            "when exists(\"/srv/a\") and ((info \"/srv/a\" |> first 1)).type == \"file\" {\n    remove \"/srv/a\" on_missing: error\n}",
        ),
        (
            "if [ -d /srv/x ]; then rm -rf /srv/x; elif [ -f /srv/y ]; then rm /srv/y; else mkdir -p /srv/z; fi",
            "when exists(\"/srv/x\") and ((info \"/srv/x\" |> first 1)).type == \"directory\" {\n    remove \"/srv/x\"\n} otherwise when exists(\"/srv/y\") and ((info \"/srv/y\" |> first 1)).type == \"file\" {\n    remove \"/srv/y\" on_missing: error\n} otherwise {\n    create \"/srv/z\" type: directory\n}",
        ),
        (
            "if [[ -f /srv/a && ! -d /srv/b ]]; then touch /srv/c; fi",
            "when exists(\"/srv/a\") and ((info \"/srv/a\" |> first 1)).type == \"file\" and not (exists \"/srv/b\" and ((info \"/srv/b\" |> first 1)).type == \"directory\") {\n    create \"/srv/c\" type: file on_exists: touch\n}",
        ),
        (
            "if test -d /srv/a; then rm -rf /srv/a; fi",
            "when exists(\"/srv/a\") and ((info \"/srv/a\" |> first 1)).type == \"directory\" {\n    remove \"/srv/a\"\n}",
        ),
        (
            "case \"$1\" in\n  start) mkdir -p /srv/a;;\n  stop|halt) rm -rf /srv/a;;\n  *) touch /srv/c;;\nesac",
            "params {\n    arg1: text\n}\nwhen @arg1 == \"start\" {\n    create \"/srv/a\" type: directory\n} otherwise when @arg1 == \"stop\" or @arg1 == \"halt\" {\n    remove \"/srv/a\"\n} otherwise {\n    create \"/srv/c\" type: file on_exists: touch\n}",
        ),
        (
            "if [ \"$1\" = \"prod\" ]; then touch /srv/p; fi",
            "params {\n    arg1: text\n}\nwhen @arg1 == \"prod\" {\n    create \"/srv/p\" type: file on_exists: touch\n}",
        ),
        (
            "if [ \"$1\" -gt 3 ]; then touch /srv/big; fi",
            "params {\n    arg1: int\n}\nwhen @arg1 > 3 {\n    create \"/srv/big\" type: file on_exists: touch\n}",
        ),
        (
            "case \"$1\" in\n  rel-*) mkdir -p /srv/rel;;\nesac",
            "params {\n    arg1: text\n}\nwhen @arg1 ~ /^rel-.*$/ {\n    create \"/srv/rel\" type: directory\n}",
        ),
    ]);
}

#[test]
fn alternative_translate() {
    check(&[
        (
            "test -f /srv/app/.env || cp /srv/app/.env.example /srv/app/.env",
            "when not (exists \"/srv/app/.env\" and ((info \"/srv/app/.env\" |> first 1)).type == \"file\") {\n    copy \"/srv/app/.env.example\" to: \"/srv/app/.env\" preserve: false\n}",
        ),
        ("rm -f /srv/a.lock || true", "remove \"/srv/a.lock\""),
        (
            "[ -d /srv/out ] && rm -rf /srv/out",
            "when exists(\"/srv/out\") and ((info \"/srv/out\" |> first 1)).type == \"directory\" {\n    remove \"/srv/out\"\n}",
        ),
    ]);
}

#[test]
fn loops_translate() {
    check(&[
        (
            "for f in /srv/in/*.csv; do gzip \"$f\"; done",
            "each (find \"/srv/in\" glob: \"*.csv\" depth: 1 hidden: false) as f {\n    compress @f.path keep: false\n}",
        ),
        (
            "for f in /srv/in/*.csv; do\n  gzip \"$f\"\n  rm -f /srv/in/skip\ndone",
            "each (find \"/srv/in\" glob: \"*.csv\" depth: 1 hidden: false) as f {\n    compress @f.path keep: false\n    remove \"/srv/in/skip\"\n}",
        ),
        (
            "for d in /srv/a /srv/b; do rm -rf \"$d\"; done",
            "each [\"/srv/a\", \"/srv/b\"] as d {\n    remove @d\n}",
        ),
        (
            "for f in $(find /srv/in -name \"*.csv\"); do gzip \"$f\"; done",
            "each (find \"/srv/in\" glob: \"**/*.csv\" type: any) as f {\n    compress @f.path keep: false\n}",
        ),
        (
            "while [ -f /srv/lock ]; do rm -f /srv/lock; done",
            "while exists(\"/srv/lock\") and ((info \"/srv/lock\" |> first 1)).type == \"file\" {\n    remove \"/srv/lock\"\n}",
        ),
        (
            "until [ -f /srv/ready ]; do touch /srv/probe; sleep 5; done",
            "repeat until: exists(\"/srv/ready\") and ((info \"/srv/ready\" |> first 1)).type == \"file\" every: 5s {\n    create \"/srv/probe\" type: file on_exists: touch\n}",
        ),
        (
            "for f in \"$@\"; do rm -f \"$f\"; done",
            "params {\n    args: list<text> = []\n}\neach @args as f {\n    remove @f\n}",
        ),
    ]);
}

#[test]
fn functions_translate() {
    check(&[
        (
            "rotate() {\n  find \"$1\" -name \"*.log\" -delete\n}\nrotate /var/log/app1\nrotate /var/log/app2",
            "define rotate(arg1: path) {\n    find @arg1 glob: \"**/*.log\" type: any\n    |> remove\n}\nrotate \"/var/log/app1\"\nrotate \"/var/log/app2\"",
        ),
        (
            "function clean {\n  rm -rf \"$1\"\n}\nclean /srv/tmp",
            "define clean(arg1: path) {\n    remove @arg1\n}\nclean \"/srv/tmp\"",
        ),
    ]);
}

#[test]
fn params_translate() {
    check(&[
        (
            "dir=\"$1\"\nkeep=${2:-7}\nrm -rf \"$dir\"",
            "params {\n    dir: path\n    keep: text = \"7\"\n}\nremove @dir",
        ),
        (
            "rm -rf \"$1\" \"$2\"",
            "params {\n    arg1: path\n    arg2: path\n}\nremove @arg1 @arg2",
        ),
        (
            "mode=$1\nshift\nrm -rf \"$1\"",
            "params {\n    mode: text\n    arg2: path\n}\nremove @arg2",
        ),
        (
            "while getopts \"d:v\" opt; do\n  case $opt in\n    d) dir=$OPTARG;;\n    v) verbose=1;;\n  esac\ndone\nrm -rf \"$dir\"",
            "params {\n    dir: text\n    verbose: bool = false\n}\nremove @dir",
        ),
    ]);
}

#[test]
fn steps_translate() {
    check(&[
        (
            "mkdir -p /srv/out && cp -a /srv/in/data.csv /srv/out/",
            "steps {\n    create \"/srv/out\" type: directory\n    copy \"/srv/in/data.csv\" into: \"/srv/out\"\n}",
        ),
        (
            "cd /srv/app && make && make install",
            "steps {\n    run [\"make\"] cwd: \"/srv/app\"\n    run [\"make\", \"install\"] cwd: \"/srv/app\"\n}",
        ),
        ("cd /srv/app && make", "run [\"make\"] cwd: \"/srv/app\""),
    ]);
}

#[test]
fn fifo_translate() {
    check(&[
        (
            "mkfifo -m 0660 /srv/run/q",
            "create \"/srv/run/q\" type: fifo mode: \"0660\"",
        ),
        ("mknod /srv/run/q p", "create \"/srv/run/q\" type: fifo"),
        (
            "mknod /srv/dev/null2 c 1 3",
            "create \"/srv/dev/null2\" type: device kind: char major: 1 minor: 3",
        ),
        (
            "mknod /srv/dev/loop9 b 7 9",
            "create \"/srv/dev/loop9\" type: device kind: block major: 7 minor: 9",
        ),
    ]);
}

#[test]
fn accounts_translate() {
    check(&[
        (
            "useradd -m -s /bin/bash -G docker,wheel -c \"Deploy\" deploy",
            "accounts.create name: \"deploy\" shell: \"/bin/bash\" comment: \"Deploy\" groups: [\"docker\", \"wheel\"] create_home: true",
        ),
        (
            "useradd -r -u 990 svc",
            "accounts.create name: \"svc\" uid: 990 system: true",
        ),
        (
            "usermod -s /bin/zsh -L alice",
            "accounts.modify name: \"alice\" shell: \"/bin/zsh\" lock: true",
        ),
        (
            "usermod -aG docker deploy",
            "groups.add_member name: \"docker\" member: \"deploy\"",
        ),
        (
            "userdel bob",
            "accounts.remove name: \"bob\" keep_home: true",
        ),
        (
            "userdel -r bob",
            "accounts.remove name: \"bob\" keep_home: false",
        ),
        (
            "groupadd -g 1500 ops",
            "groups.create name: \"ops\" gid: 1500",
        ),
        ("groupdel ops", "groups.remove name: \"ops\""),
        (
            "gpasswd -a alice ops",
            "groups.add_member name: \"ops\" member: \"alice\"",
        ),
        (
            "gpasswd -d alice ops",
            "groups.remove_member name: \"ops\" member: \"alice\"",
        ),
        ("passwd alice", "accounts.password name: \"alice\""),
        (
            "sudo useradd -m eve",
            "accounts.create name: \"eve\" create_home: true privilege: elevated",
        ),
    ]);
}

#[test]
fn mounts_translate() {
    check(&[
        (
            "mount -t ext4 -o noatime,ro /dev/sdb1 /srv/data",
            "mount device: \"/dev/sdb1\" target: \"/srv/data\" type: \"ext4\" options: [\"noatime\", \"ro\"]",
        ),
        ("umount /srv/data", "unmount target: \"/srv/data\""),
    ]);
}

#[test]
fn kernel_translate() {
    check(&[
        (
            "sysctl -w net.ipv4.ip_forward=1",
            "sysctl.set key: \"net.ipv4.ip_forward\" value: \"1\"",
        ),
        ("modprobe overlay", "modules.load name: \"overlay\""),
        ("modprobe -r nfs", "modules.unload name: \"nfs\""),
        ("rmmod nfs", "modules.unload name: \"nfs\""),
    ]);
}

#[test]
fn firewall_translate() {
    check(&[
        (
            "ufw allow 443/tcp",
            "firewall.allow port: 443 protocol: \"tcp\"",
        ),
        ("ufw deny 23", "firewall.deny port: 23"),
        (
            "ufw allow from 10.0.0.0/8 to any port 22 proto tcp",
            "firewall.allow port: 22 protocol: \"tcp\" source: \"10.0.0.0/8\"",
        ),
        (
            "firewall-cmd --permanent --zone=public --add-port=8080/tcp",
            "firewall.allow port: 8080 protocol: \"tcp\" zone: \"public\"",
        ),
        (
            "nft add rule inet filter input tcp dport 22 accept",
            "firewall.allow port: 22 protocol: \"tcp\" direction: \"in\"",
        ),
        (
            "iptables -A INPUT -p tcp --dport 80 -j ACCEPT",
            "firewall.allow port: 80 protocol: \"tcp\" direction: \"in\"",
        ),
    ]);
}

#[test]
fn network_translate() {
    check(&[
        (
            "ip route add 10.1.0.0/16 via 10.0.0.1 dev eth0 metric 100",
            "network.route_add destination: \"10.1.0.0/16\" gateway: \"10.0.0.1\" interface: \"eth0\" metric: 100",
        ),
        (
            "ip route del 10.1.0.0/16",
            "network.route_remove destination: \"10.1.0.0/16\"",
        ),
        (
            "resolvectl dns eth0 1.1.1.1 9.9.9.9",
            "network.dns_set interface: \"eth0\" servers: [\"1.1.1.1\", \"9.9.9.9\"]",
        ),
    ]);
}

#[test]
fn clock_translate() {
    check(&[
        (
            "timedatectl set-timezone Europe/Warsaw",
            "clock.timezone_set \"Europe/Warsaw\"",
        ),
        (
            "date -s \"2026-11-27 10:00:00\"",
            "clock.set \"2026-11-27 10:00:00\"",
        ),
    ]);
}

#[test]
fn power_translate() {
    check(&[
        (
            "shutdown -r +5 \"maintenance\"",
            "power.restart delay: 5m message: \"maintenance\"",
        ),
        ("reboot", "power.restart"),
        ("poweroff", "power.shutdown"),
    ]);
}

#[test]
fn disks_translate() {
    check(&[
        (
            "parted -s /dev/sdb rm 2",
            "disks.partition device: \"/dev/sdb\" action: \"delete\" number: 2",
        ),
        (
            "mkfs.ext4 -L data /dev/sdb1",
            "disks.format device: \"/dev/sdb1\" fs: \"ext4\" label: \"data\"",
        ),
    ]);
}

#[test]
fn complex_firewall_rules_run_as_programs() {
    check(&[
        (
            "iptables -t nat -A PREROUTING -p tcp --dport 80 -j DNAT --to 10.0.0.2",
            "run [\n  \"iptables\",\n  \"-t\",\n  \"nat\",\n  \"-A\",\n  \"PREROUTING\",\n  \"-p\",\n  \"tcp\",\n  \"--dport\",\n  \"80\",\n  \"-j\",\n  \"DNAT\",\n  \"--to\",\n  \"10.0.0.2\"\n] cwd: \"/\"",
        ),
        (
            "sudo nft add table inet filter",
            "run [\"nft\", \"add\", \"table\", \"inet\", \"filter\"] cwd: \"/\" privilege: elevated",
        ),
    ]);
}

#[test]
fn sql_translate() {
    check(&[(
        "sqlite3 /srv/app.db \"DELETE FROM t\"",
        "sql \"/srv/app.db\" \"DELETE FROM t\"",
    )]);
}

#[test]
fn functions_with_functions_translate() {
    let functions = [FunctionSignature {
        name: "rotate".to_owned(),
        params: 1,
    }];
    let context = TranslateContext {
        functions: &functions,
        ..SCRIPT
    };
    assert_eq!(
        translated("rotate /var/log/app3", &context),
        "rotate \"/var/log/app3\""
    );
}

#[test]
fn a_list_with_several_changes_in_one_call_proposes_steps_first() {
    let error = error_of(
        "mkdir -p /srv/out && cp -a /srv/in/data.csv /srv/out/",
        &LIVE,
    );
    assert_eq!(error.code(), ErrorCode::OneEffect);
    let data = error.data();
    assert_eq!(data.candidates.len(), 1);
    let steps = "steps {\n    create \"/srv/out\" type: directory\n    copy \"/srv/in/data.csv\" into: \"/srv/out\"\n}";
    assert_eq!(data.candidates[0].program, steps);
    parse(steps, Variant::A, &ParseOptions::default()).expect("the language reads the steps");
    assert_eq!(
        data.next_steps,
        [
            "create \"/srv/out\" type: directory",
            "copy \"/srv/in/data.csv\" into: \"/srv/out\""
        ]
    );
}

#[test]
fn a_single_call_translates_constructs_and_administration_programs() {
    for (input, expected) in [
        (
            "if [ -f /srv/a ]; then rm /srv/a; fi",
            "when exists(\"/srv/a\") and ((info \"/srv/a\" |> first 1)).type == \"file\" {\n    remove \"/srv/a\" on_missing: error\n}",
        ),
        ("mkfifo /srv/run/q", "create \"/srv/run/q\" type: fifo"),
        (
            "useradd -m eve",
            "accounts.create name: \"eve\" create_home: true",
        ),
        (
            "sudo ufw allow 443/tcp",
            "firewall.allow port: 443 protocol: \"tcp\" privilege: elevated",
        ),
    ] {
        assert_eq!(translated(input, &LIVE), expected, "{input}");
    }
}

#[test]
fn forms_that_depend_on_the_shell_have_no_translation() {
    for input in [
        "(cd /srv && make)",
        "for ((i = 0; i < 3; i++)); do rm /srv/a; done",
        "ls /srv/a || mkdir -p /srv/a",
        "usermod -aG docker,wheel deploy",
        "sysctl -w net.ipv4.ip_forward=1 vm.swappiness=10",
    ] {
        let error = error_of(input, &SCRIPT);
        assert_eq!(error.code(), ErrorCode::Uncorrectable, "{input}");
    }
}

#[test]
fn a_failure_of_a_command_that_is_ignored_is_an_assumption() {
    let translation = translate_with("rm -f /srv/a.lock || true", &SCRIPT)
        .expect("a bash habit")
        .expect("a translation");
    assert_eq!(translation.text(), "remove \"/srv/a.lock\"");
    assert!(
        translation
            .assumptions
            .iter()
            .any(|assumption| assumption.text.contains("is ignored by the shell"))
    );
}

#[test]
fn parameters_get_the_type_of_their_use_and_a_note_when_unnamed() {
    let translation = translate_with("rm -rf \"$1\" \"$2\"", &SCRIPT)
        .expect("a bash habit")
        .expect("a translation");
    assert_eq!(
        translation.text(),
        "params {\n    arg1: path\n    arg2: path\n}\nremove @arg1 @arg2"
    );
    assert!(
        translation
            .assumptions
            .iter()
            .any(|assumption| assumption.rule == "B42" && assumption.text.contains("arg1"))
    );
}

/// Names of the commands of the catalog files with the names of their
/// parameters.
fn catalog_parameters() -> std::collections::BTreeMap<String, Vec<String>> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../catalog");
    let mut commands = std::collections::BTreeMap::new();
    for file in std::fs::read_dir(dir).expect("catalog directory") {
        let path = file.expect("entry").path();
        if path.extension().is_none_or(|extension| extension != "toml")
            || path.file_name().is_some_and(|name| name == "_index.toml")
        {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("catalog file");
        let document: toml_edit::DocumentMut = text.parse().expect("catalog syntax");
        let Some(list) = document
            .get("command")
            .and_then(toml_edit::Item::as_array_of_tables)
        else {
            continue;
        };
        for command in list {
            let name = command["name"].as_str().expect("name").to_owned();
            let params = command
                .get("param")
                .and_then(toml_edit::Item::as_array_of_tables)
                .map(|params| {
                    params
                        .iter()
                        .map(|param| param["name"].as_str().expect("param name").to_owned())
                        .collect()
                })
                .unwrap_or_default();
            commands.insert(name, params);
        }
    }
    commands
}

#[test]
fn translations_to_administration_commands_use_the_catalog() {
    let catalog = catalog_parameters();
    let inputs = [
        "useradd -m -s /bin/bash -c 'Deploy user' -G docker,www deploy",
        "useradd -r -u 990 svc",
        "usermod -s /bin/zsh -L alice",
        "usermod -aG docker deploy",
        "userdel bob",
        "userdel -r bob",
        "passwd alice",
        "groupadd -g 1500 ops",
        "groupdel ops",
        "gpasswd -a alice ops",
        "gpasswd -d alice ops",
        "mount -t ext4 -o noatime,ro /dev/sdb1 /srv/data",
        "umount /srv/data",
        "sysctl -w net.ipv4.ip_forward=1",
        "modprobe overlay",
        "modprobe -r nfs",
        "rmmod nfs",
        "ufw allow 443/tcp",
        "ufw deny 23",
        "ufw allow from 10.0.0.0/8 to any port 22 proto tcp",
        "firewall-cmd --permanent --zone=public --add-port=8080/tcp",
        "nft add rule inet filter input tcp dport 22 accept",
        "iptables -A INPUT -p tcp --dport 80 -j ACCEPT",
        "ip route add 10.1.0.0/16 via 10.0.0.1 dev eth0 metric 100",
        "ip route del 10.1.0.0/16",
        "resolvectl dns eth0 1.1.1.1 9.9.9.9",
        "timedatectl set-timezone Europe/Warsaw",
        "reboot",
        "poweroff",
        "parted -s /dev/sdb rm 2",
        "mkfs.ext4 -L data /dev/sdb1",
    ];
    let mut problems = Vec::new();
    for input in inputs {
        let text = translated(input, &SCRIPT);
        for line in text.lines() {
            let words: Vec<&str> = line.split_whitespace().collect();
            let Some(&command) = words.first() else {
                continue;
            };
            if command == "run" {
                continue;
            }
            let Some(params) = catalog.get(command) else {
                problems.push(format!("{input}: unknown command {command}"));
                continue;
            };
            for word in &words[1..] {
                if let Some(name) = word.strip_suffix(':')
                    && !params.iter().any(|param| param == name)
                {
                    problems.push(format!("{input}: {command} has no parameter {name}"));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
