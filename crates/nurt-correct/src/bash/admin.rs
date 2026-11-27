// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-correct::bash::admin
// Responsibility:   Translation of the administration programs of a shell (`useradd`.
// Dependencies:     nurt_contracts, nurt_syntax
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//
// Module: nurt-correct bash admin
// Responsibility: Translates the administration programs of bash into the native administration commands.
// Dependencies: nurt-contracts, nurt-correct bash session
// Author: Danaco Official
// Version: 0.1.1, 2026-11-27
// License: see the LICENSE file or https://www.danaco-group.pl
//! Translation of the administration programs of a shell (`useradd`,
//! `mount`, `sysctl -w`, `ufw`, `ip route`, `shutdown`, `mkfs.*`, …) into the
//! native administration commands of Nurt.
//!
//! Only forms whose meaning the native command keeps are translated. A form
//! with an option that has no counterpart is an error that names the option;
//! a firewall rule that is not a simple port rule is run with `run`.

use nurt_contracts::ast::{Expr, Head, Stage, Statement};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::report::CorrectionClass;
use nurt_syntax::parse::{ParseOptions, Variant, parse};

use super::lex::Word;
use super::session::{Session, Variants};
use super::syntax::{Located, Simple};
use super::table::CommandEntry;
use super::tree::print_text;

/// Problem of a firewall rule that is not a simple rule on a port.
const COMPLEX_RULE: &str = "complex_firewall_rule";

/// Translates one administration program.
pub(crate) fn translate(
    session: &Session<'_>,
    entry: &CommandEntry,
    simple: &Simple,
    words: &[Located<Word>],
) -> Result<Variants, Error> {
    let name = entry.words.join(" ");
    let mut args = Vec::new();
    for word in words {
        match word.value.literal() {
            Some(text) => args.push(text),
            None => {
                return Err(session.error(
                    ErrorCode::Uncorrectable,
                    "admin_value",
                    word.span,
                    &[("command", &name)],
                ));
            }
        }
    }
    let program = Program {
        session,
        simple,
        name: &name,
    };
    let outcome = match name.as_str() {
        "useradd" => program.useradd(&args),
        "usermod" => program.usermod(&args),
        "userdel" => program.userdel(&args),
        "groupadd" => program.groupadd(&args),
        "groupdel" => program.single("groups.remove", &args),
        "gpasswd" => program.gpasswd(&args),
        "passwd" => program.passwd(&args),
        "mount" => program.mount(&args),
        "umount" => program.umount(&args),
        "sysctl" => program.sysctl(&args),
        "modprobe" => program.modprobe(&args),
        "rmmod" => program.single_named("modules.unload", "name", &args),
        "ufw" => program.ufw(&args),
        "firewall-cmd" => program.firewall_cmd(&args),
        "nft" => program.nft(&args),
        "iptables" => program.iptables(&args),
        "ip" => program.ip(&args),
        "resolvectl" => program.resolvectl(&args),
        "timedatectl" => program.timedatectl(&args),
        "shutdown" => program.shutdown(&args),
        "reboot" => program.power("power.restart", &args),
        "poweroff" | "halt" => program.power("power.shutdown", &args),
        "parted" => program.parted(&args),
        "mknod" => program.mknod(&args),
        _ if name.starts_with("mkfs") => program.mkfs(&args),
        _ => Err(program.unsupported("this program")),
    };
    match outcome {
        Ok(text) => program.finish(&text),
        Err(error) if error.problem.as_deref() == Some(COMPLEX_RULE) => {
            // An administration program does not depend on the directory of
            // the script, so the program runs from the root of the file system.
            let start = simple
                .words
                .iter()
                .position(|word| {
                    word.value.literal().as_deref() == entry.words.first().map(String::as_str)
                })
                .unwrap_or(0);
            session.run_program(simple, &simple.words[start..], Some("/".to_owned()))
        }
        Err(error) => Err(error),
    }
}

struct Program<'a> {
    session: &'a Session<'a>,
    simple: &'a Simple,
    name: &'a str,
}

/// Options of a program: flags with and without values, and operands.
#[derive(Debug, Default)]
struct Parsed {
    flags: Vec<(String, Option<String>)>,
    operands: Vec<String>,
}

impl Parsed {
    fn has(&self, flag: &str) -> bool {
        self.flags.iter().any(|(name, _)| name == flag)
    }

    fn value(&self, flag: &str) -> Option<&str> {
        self.flags
            .iter()
            .find(|(name, _)| name == flag)
            .and_then(|(_, value)| value.as_deref())
    }
}

fn quote_all(items: &[String]) -> String {
    let texts: Vec<String> = items.iter().map(|item| print_text(item)).collect();
    format!("[{}]", texts.join(", "))
}

fn list(value: &str) -> String {
    let items: Vec<String> = value
        .split(',')
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect();
    quote_all(&items)
}

/// `22/tcp` or `443` as a port and a protocol.
fn port_and_protocol(text: &str) -> Option<(String, Option<String>)> {
    let (port, protocol) = match text.split_once('/') {
        Some((port, protocol)) => (port, Some(protocol)),
        None => (text, None),
    };
    let valid = !port.is_empty()
        && port
            .split(':')
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()));
    let protocol_ok = protocol.is_none_or(|p| matches!(p, "tcp" | "udp"));
    (valid && protocol_ok).then(|| (port.replace(':', "-"), protocol.map(str::to_owned)))
}

fn port_argument(port: &str) -> String {
    if port.chars().all(|c| c.is_ascii_digit()) {
        port.to_owned()
    } else {
        print_text(port)
    }
}

impl Program<'_> {
    fn unsupported(&self, what: &str) -> Error {
        self.session.error(
            ErrorCode::Uncorrectable,
            "admin_form",
            self.simple.span,
            &[("command", self.name), ("what", what)],
        )
    }

    fn parse(
        &self,
        args: &[String],
        with_value: &[&str],
        without_value: &[&str],
    ) -> Result<Parsed, Error> {
        let mut parsed = Parsed::default();
        let mut index = 0;
        while index < args.len() {
            let arg = &args[index];
            index += 1;
            if arg == "--" {
                parsed.operands.extend(args[index..].iter().cloned());
                break;
            }
            if let Some(long) = arg.strip_prefix("--") {
                let (name, inline) = match long.split_once('=') {
                    Some((name, value)) => (name, Some(value.to_owned())),
                    None => (long, None),
                };
                let flag = format!("--{name}");
                if with_value.contains(&flag.as_str()) {
                    let value = match inline {
                        Some(value) => value,
                        None => {
                            index += 1;
                            args.get(index - 1)
                                .cloned()
                                .ok_or_else(|| self.unsupported(&flag))?
                        }
                    };
                    parsed.flags.push((flag, Some(value)));
                } else if without_value.contains(&flag.as_str()) {
                    parsed.flags.push((flag, None));
                } else {
                    return Err(self.unsupported(&format!("the option {flag}")));
                }
                continue;
            }
            if arg.len() > 1
                && arg.starts_with('-')
                && !arg[1..].chars().all(|c| c.is_ascii_digit())
            {
                let letters: Vec<char> = arg[1..].chars().collect();
                for (position, letter) in letters.iter().enumerate() {
                    let flag = format!("-{letter}");
                    if with_value.contains(&flag.as_str()) {
                        let attached: String = letters[position + 1..].iter().collect();
                        let value = if attached.is_empty() {
                            index += 1;
                            args.get(index - 1)
                                .cloned()
                                .ok_or_else(|| self.unsupported(&flag))?
                        } else {
                            attached
                        };
                        parsed.flags.push((flag, Some(value)));
                        break;
                    } else if without_value.contains(&flag.as_str()) {
                        parsed.flags.push((flag, None));
                    } else {
                        return Err(self.unsupported(&format!("the option {flag}")));
                    }
                }
                continue;
            }
            parsed.operands.push(arg.clone());
        }
        Ok(parsed)
    }

    fn operand<'p>(&self, parsed: &'p Parsed, count: usize) -> Result<&'p [String], Error> {
        if parsed.operands.len() == count {
            Ok(&parsed.operands)
        } else {
            Err(self.unsupported(&format!("{} operands", parsed.operands.len())))
        }
    }

    // ----- accounts and groups ---------------------------------------------

    fn useradd(&self, args: &[String]) -> Result<String, Error> {
        let parsed = self.parse(
            args,
            &[
                "-s",
                "--shell",
                "-d",
                "--home-dir",
                "-G",
                "--groups",
                "-c",
                "--comment",
                "-u",
                "--uid",
            ],
            &["-m", "--create-home", "-r", "--system"],
        )?;
        let [name] = self.operand(&parsed, 1)? else {
            return Err(self.unsupported("operands"));
        };
        let mut text = format!("accounts.create name: {}", print_text(name));
        for (flags, key) in [
            (["-s", "--shell"], "shell"),
            (["-d", "--home-dir"], "home"),
            (["-c", "--comment"], "comment"),
        ] {
            if let Some(value) = flags.iter().find_map(|flag| parsed.value(flag)) {
                text.push_str(&format!(" {key}: {}", print_text(value)));
            }
        }
        if let Some(groups) = ["-G", "--groups"]
            .iter()
            .find_map(|flag| parsed.value(flag))
        {
            text.push_str(&format!(" groups: {}", list(groups)));
        }
        if let Some(uid) = ["-u", "--uid"].iter().find_map(|flag| parsed.value(flag)) {
            let number: u32 = uid
                .parse()
                .map_err(|_| self.unsupported("a user id that is not a number"))?;
            text.push_str(&format!(" uid: {number}"));
        }
        if parsed.has("-m") || parsed.has("--create-home") {
            text.push_str(" create_home: true");
        }
        if parsed.has("-r") || parsed.has("--system") {
            text.push_str(" system: true");
        }
        Ok(text)
    }

    fn usermod(&self, args: &[String]) -> Result<String, Error> {
        let parsed = self.parse(
            args,
            &[
                "-s",
                "--shell",
                "-d",
                "--home",
                "-G",
                "--groups",
                "-c",
                "--comment",
            ],
            &["-a", "--append", "-L", "--lock", "-U", "--unlock"],
        )?;
        let [name] = self.operand(&parsed, 1)? else {
            return Err(self.unsupported("operands"));
        };
        let groups = ["-G", "--groups"]
            .iter()
            .find_map(|flag| parsed.value(flag));
        let append = parsed.has("-a") || parsed.has("--append");
        if append && groups.is_some() {
            if parsed.flags.len() > 2 {
                return Err(self.unsupported("appending to groups together with other changes"));
            }
            let groups = groups.unwrap_or_default();
            let lines: Vec<String> = groups
                .split(',')
                .filter(|group| !group.is_empty())
                .map(|group| {
                    format!(
                        "groups.add_member name: {} member: {}",
                        print_text(group),
                        print_text(name)
                    )
                })
                .collect();
            if lines.len() != 1 {
                return Err(self.unsupported("several groups to append; add them one by one"));
            }
            return Ok(lines.join("\n"));
        }
        if append {
            return Err(self.unsupported("the option -a without -G"));
        }
        let mut text = format!("accounts.modify name: {}", print_text(name));
        for (flags, key) in [
            (["-s", "--shell"], "shell"),
            (["-d", "--home"], "home"),
            (["-c", "--comment"], "comment"),
        ] {
            if let Some(value) = flags.iter().find_map(|flag| parsed.value(flag)) {
                text.push_str(&format!(" {key}: {}", print_text(value)));
            }
        }
        if let Some(groups) = groups {
            text.push_str(&format!(" groups: {}", list(groups)));
        }
        if parsed.has("-L") || parsed.has("--lock") {
            text.push_str(" lock: true");
        }
        if parsed.has("-U") || parsed.has("--unlock") {
            text.push_str(" lock: false");
        }
        if text.ends_with(&print_text(name)) {
            return Err(self.unsupported("a call without a change"));
        }
        Ok(text)
    }

    fn userdel(&self, args: &[String]) -> Result<String, Error> {
        let parsed = self.parse(args, &[], &["-r", "--remove", "-f", "--force"])?;
        let [name] = self.operand(&parsed, 1)? else {
            return Err(self.unsupported("operands"));
        };
        let keep = !(parsed.has("-r") || parsed.has("--remove"));
        Ok(format!(
            "accounts.remove name: {} keep_home: {keep}",
            print_text(name)
        ))
    }

    fn groupadd(&self, args: &[String]) -> Result<String, Error> {
        let parsed = self.parse(args, &["-g", "--gid"], &["-r", "--system"])?;
        let [name] = self.operand(&parsed, 1)? else {
            return Err(self.unsupported("operands"));
        };
        let mut text = format!("groups.create name: {}", print_text(name));
        if let Some(gid) = ["-g", "--gid"].iter().find_map(|flag| parsed.value(flag)) {
            let number: u32 = gid
                .parse()
                .map_err(|_| self.unsupported("a group id that is not a number"))?;
            text.push_str(&format!(" gid: {number}"));
        }
        if parsed.has("-r") || parsed.has("--system") {
            text.push_str(" system: true");
        }
        Ok(text)
    }

    fn gpasswd(&self, args: &[String]) -> Result<String, Error> {
        let parsed = self.parse(args, &["-a", "--add", "-d", "--delete"], &[])?;
        let [group] = self.operand(&parsed, 1)? else {
            return Err(self.unsupported("operands"));
        };
        let (command, member) = match (
            ["-a", "--add"].iter().find_map(|flag| parsed.value(flag)),
            ["-d", "--delete"]
                .iter()
                .find_map(|flag| parsed.value(flag)),
        ) {
            (Some(member), None) => ("groups.add_member", member),
            (None, Some(member)) => ("groups.remove_member", member),
            _ => return Err(self.unsupported("a call that is not -a or -d")),
        };
        Ok(format!(
            "{command} name: {} member: {}",
            print_text(group),
            print_text(member)
        ))
    }

    fn passwd(&self, args: &[String]) -> Result<String, Error> {
        let parsed = self.parse(args, &[], &[])?;
        let [name] = self.operand(&parsed, 1)? else {
            return Err(self.unsupported("operands"));
        };
        self.session.assume("B43", "password_secret");
        Ok(format!("accounts.password name: {}", print_text(name)))
    }

    fn single(&self, command: &str, args: &[String]) -> Result<String, Error> {
        self.single_named(command, "name", args)
    }

    fn single_named(&self, command: &str, key: &str, args: &[String]) -> Result<String, Error> {
        let parsed = self.parse(args, &[], &[])?;
        let [name] = self.operand(&parsed, 1)? else {
            return Err(self.unsupported("operands"));
        };
        Ok(format!("{command} {key}: {}", print_text(name)))
    }

    // ----- mounts and kernel -----------------------------------------------

    fn mount(&self, args: &[String]) -> Result<String, Error> {
        let parsed = self.parse(args, &["-t", "--types", "-o", "--options"], &[])?;
        let [device, target] = self.operand(&parsed, 2)? else {
            return Err(self.unsupported("operands"));
        };
        let mut text = format!(
            "mount device: {} target: {}",
            print_text(device),
            print_text(target)
        );
        if let Some(kind) = ["-t", "--types"].iter().find_map(|flag| parsed.value(flag)) {
            text.push_str(&format!(" type: {}", print_text(kind)));
        }
        if let Some(options) = ["-o", "--options"]
            .iter()
            .find_map(|flag| parsed.value(flag))
        {
            text.push_str(&format!(" options: {}", list(options)));
        }
        Ok(text)
    }

    fn umount(&self, args: &[String]) -> Result<String, Error> {
        self.single_named("unmount", "target", args)
    }

    fn sysctl(&self, args: &[String]) -> Result<String, Error> {
        let parsed = self.parse(args, &[], &["-w", "--write"])?;
        let [pair] = self.operand(&parsed, 1)? else {
            return Err(self
                .unsupported("a call that does not set exactly one key; write one call per key"));
        };
        let Some((key, value)) = pair.split_once('=') else {
            return Err(self.unsupported("reading a key"));
        };
        Ok(format!(
            "sysctl.set key: {} value: {}",
            print_text(key),
            print_text(value)
        ))
    }

    fn modprobe(&self, args: &[String]) -> Result<String, Error> {
        let parsed = self.parse(args, &[], &["-r", "--remove"])?;
        let Some((module, params)) = parsed.operands.split_first() else {
            return Err(self.unsupported("operands"));
        };
        if parsed.has("-r") || parsed.has("--remove") {
            if !params.is_empty() {
                return Err(self.unsupported("parameters when removing a module"));
            }
            return Ok(format!("modules.unload name: {}", print_text(module)));
        }
        let mut text = format!("modules.load name: {}", print_text(module));
        if !params.is_empty() {
            text.push_str(&format!(" params: {}", quote_all(params)));
        }
        Ok(text)
    }

    // ----- firewall ------------------------------------------------------------

    fn rule(
        &self,
        command: &str,
        port: &str,
        protocol: Option<&str>,
        extra: &[(&str, String)],
    ) -> String {
        let mut text = format!("{command} port: {}", port_argument(port));
        if let Some(protocol) = protocol {
            text.push_str(&format!(" protocol: {}", print_text(protocol)));
        }
        for (key, value) in extra {
            text.push_str(&format!(" {key}: {}", print_text(value)));
        }
        text
    }

    fn ufw(&self, args: &[String]) -> Result<String, Error> {
        let Some((action, rest)) = args.split_first() else {
            return Err(self.unsupported("a call without an action"));
        };
        let command = match action.as_str() {
            "allow" => "firewall.allow",
            "deny" | "reject" => "firewall.deny",
            "delete" => "firewall.remove",
            _ => return Err(self.unsupported(&format!("the action {action}"))),
        };
        let rest = if command == "firewall.remove" {
            match rest.split_first() {
                Some((word, tail)) if word == "allow" || word == "deny" => tail,
                _ => rest,
            }
        } else {
            rest
        };
        let mut port = None;
        let mut protocol = None;
        let mut source = None;
        let mut direction = None;
        let mut index = 0;
        while index < rest.len() {
            match rest[index].as_str() {
                "from" => {
                    source = rest.get(index + 1).filter(|s| s.as_str() != "any").cloned();
                    index += 2;
                }
                "to" => index += 2,
                "port" => {
                    let (p, proto) = rest
                        .get(index + 1)
                        .and_then(|text| port_and_protocol(text))
                        .ok_or_else(|| self.unsupported("a port that is not a number"))?;
                    port = Some(p);
                    protocol = protocol.or(proto);
                    index += 2;
                }
                "proto" => {
                    protocol = rest.get(index + 1).cloned();
                    index += 2;
                }
                "in" | "out" => {
                    direction = Some(rest[index].clone());
                    index += 1;
                }
                other => {
                    let (p, proto) = port_and_protocol(other)
                        .ok_or_else(|| self.unsupported(&format!("the rule word {other}")))?;
                    port = Some(p);
                    protocol = protocol.or(proto);
                    index += 1;
                }
            }
        }
        let Some(port) = port else {
            return Err(self.unsupported("a rule without a port"));
        };
        let mut extra = Vec::new();
        if let Some(source) = source {
            extra.push(("source", source));
        }
        if let Some(direction) = direction {
            extra.push(("direction", direction));
        }
        Ok(self.rule(command, &port, protocol.as_deref(), &extra))
    }

    fn firewall_cmd(&self, args: &[String]) -> Result<String, Error> {
        let parsed = self.parse(
            args,
            &["--add-port", "--remove-port", "--zone"],
            &["--permanent"],
        )?;
        if !parsed.operands.is_empty() {
            return Err(self.unsupported("operands"));
        }
        if parsed.has("--permanent") {
            self.session.assume("B44", "firewall_permanent");
        }
        let (command, spec) = match (parsed.value("--add-port"), parsed.value("--remove-port")) {
            (Some(spec), None) => ("firewall.allow", spec),
            (None, Some(spec)) => ("firewall.remove", spec),
            _ => return Err(self.unsupported("a call that does not add or remove one port")),
        };
        let (port, protocol) = port_and_protocol(spec)
            .ok_or_else(|| self.unsupported("a port that is not a number"))?;
        let mut extra = Vec::new();
        if let Some(zone) = parsed.value("--zone") {
            extra.push(("zone", zone.to_owned()));
        }
        Ok(self.rule(command, &port, protocol.as_deref(), &extra))
    }

    /// `nft add rule inet filter input tcp dport 22 accept`.
    fn nft(&self, args: &[String]) -> Result<String, Error> {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        let tail = match words.as_slice() {
            ["add", "rule", _, _, chain, rest @ ..] | ["add", "rule", _, chain, rest @ ..]
                if matches!(*chain, "input" | "output") =>
            {
                Some((*chain, rest))
            }
            _ => None,
        };
        let Some((chain, rest)) = tail else {
            return Err(self.complex_rule());
        };
        let direction = if chain == "input" { "in" } else { "out" };
        match rest {
            [protocol @ ("tcp" | "udp"), "dport", port, verdict] => {
                let command = match *verdict {
                    "accept" => "firewall.allow",
                    "drop" | "reject" => "firewall.deny",
                    _ => return Err(self.complex_rule()),
                };
                let (port, _) = port_and_protocol(port).ok_or_else(|| self.complex_rule())?;
                Ok(self.rule(
                    command,
                    &port,
                    Some(protocol),
                    &[("direction", direction.to_owned())],
                ))
            }
            _ => Err(self.complex_rule()),
        }
    }

    /// `iptables -A INPUT -p tcp --dport 22 -j ACCEPT`.
    fn iptables(&self, args: &[String]) -> Result<String, Error> {
        let parsed = self
            .parse(args, &["-A", "-p", "--dport", "-j", "-s"], &[])
            .map_err(|_| self.complex_rule())?;
        if !parsed.operands.is_empty() {
            return Err(self.complex_rule());
        }
        let direction = match parsed.value("-A") {
            Some("INPUT") => "in",
            Some("OUTPUT") => "out",
            _ => return Err(self.complex_rule()),
        };
        let command = match parsed.value("-j") {
            Some("ACCEPT") => "firewall.allow",
            Some("DROP" | "REJECT") => "firewall.deny",
            _ => return Err(self.complex_rule()),
        };
        let (Some(protocol @ ("tcp" | "udp")), Some(port)) =
            (parsed.value("-p"), parsed.value("--dport"))
        else {
            return Err(self.complex_rule());
        };
        let (port, _) = port_and_protocol(port).ok_or_else(|| self.complex_rule())?;
        let mut extra = vec![("direction", direction.to_owned())];
        if let Some(source) = parsed.value("-s") {
            extra.push(("source", source.to_owned()));
        }
        Ok(self.rule(command, &port, Some(protocol), &extra))
    }

    /// A rule that is not a simple rule on a port runs as a program.
    fn complex_rule(&self) -> Error {
        self.session.error(
            ErrorCode::Uncorrectable,
            COMPLEX_RULE,
            self.simple.span,
            &[("command", self.name)],
        )
    }

    // ----- network, time, power, disks ------------------------------------------

    fn ip(&self, args: &[String]) -> Result<String, Error> {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        let (command, rest) = match words.as_slice() {
            ["route" | "r", "add", rest @ ..] => ("network.route_add", rest),
            ["route" | "r", "del" | "delete", rest @ ..] => ("network.route_remove", rest),
            _ => return Err(self.unsupported("this form")),
        };
        let Some((destination, mut rest)) = rest.split_first() else {
            return Err(self.unsupported("a route without a destination"));
        };
        let mut text = format!("{command} destination: {}", print_text(destination));
        while let Some((key, tail)) = rest.split_first() {
            let (value, tail) = tail
                .split_first()
                .ok_or_else(|| self.unsupported(&format!("the word {key} without a value")))?;
            match *key {
                "via" => text.push_str(&format!(" gateway: {}", print_text(value))),
                "dev" => text.push_str(&format!(" interface: {}", print_text(value))),
                "metric" => {
                    let number: u32 = value
                        .parse()
                        .map_err(|_| self.unsupported("a metric that is not a number"))?;
                    text.push_str(&format!(" metric: {number}"));
                }
                other => return Err(self.unsupported(&format!("the route word {other}"))),
            }
            rest = tail;
        }
        Ok(text)
    }

    fn resolvectl(&self, args: &[String]) -> Result<String, Error> {
        match args.split_first() {
            Some((verb, rest)) if verb == "dns" && rest.len() >= 2 => Ok(format!(
                "network.dns_set interface: {} servers: {}",
                print_text(&rest[0]),
                quote_all(&rest[1..])
            )),
            _ => Err(self.unsupported("this form; use resolvectl dns INTERFACE SERVER...")),
        }
    }

    fn timedatectl(&self, args: &[String]) -> Result<String, Error> {
        match args {
            [verb, zone] if verb == "set-timezone" => {
                Ok(format!("clock.timezone_set {}", print_text(zone)))
            }
            [verb, state]
                if verb == "set-ntp" && matches!(state.as_str(), "true" | "yes" | "1" | "on") =>
            {
                Ok("clock.ntp on".to_owned())
            }
            [verb, state]
                if verb == "set-ntp" && matches!(state.as_str(), "false" | "no" | "0" | "off") =>
            {
                Ok("clock.ntp off".to_owned())
            }
            _ => Err(self.unsupported("this form")),
        }
    }

    fn shutdown(&self, args: &[String]) -> Result<String, Error> {
        let parsed = self.parse(args, &[], &["-r", "-h", "-P", "-H", "-c"])?;
        if parsed.has("-c") {
            return Ok("power.cancel".to_owned());
        }
        let command = if parsed.has("-r") {
            "power.restart"
        } else {
            "power.shutdown"
        };
        let mut text = command.to_owned();
        let mut operands = parsed.operands.iter();
        match operands.next().map(String::as_str) {
            None | Some("now") => {}
            Some(delay) if delay.starts_with('+') => {
                let minutes: u32 = delay[1..]
                    .parse()
                    .map_err(|_| self.unsupported("a delay that is not a number of minutes"))?;
                text.push_str(&format!(" delay: {minutes}m"));
            }
            Some(_) => return Err(self.unsupported("a time of day; use +MINUTES or now")),
        }
        let message: Vec<&str> = operands.map(String::as_str).collect();
        if !message.is_empty() {
            text.push_str(&format!(" message: {}", print_text(&message.join(" "))));
        }
        Ok(text)
    }

    fn power(&self, command: &str, args: &[String]) -> Result<String, Error> {
        let parsed = self.parse(args, &[], &["-f", "--force"])?;
        if !parsed.operands.is_empty() {
            return Err(self.unsupported("operands"));
        }
        Ok(command.to_owned())
    }

    fn parted(&self, args: &[String]) -> Result<String, Error> {
        let parsed = self.parse(args, &[], &["-s", "--script"])?;
        let words: Vec<&str> = parsed.operands.iter().map(String::as_str).collect();
        match words.as_slice() {
            [device, "rm", number] => {
                let number: u32 = number
                    .parse()
                    .map_err(|_| self.unsupported("a partition number"))?;
                Ok(format!(
                    "disks.partition device: {} action: \"delete\" number: {number}",
                    print_text(device)
                ))
            }
            [device, "mkpart", _, _, start, end] | [device, "mkpart", _, start, end] => {
                Ok(format!(
                    "disks.partition device: {} action: \"create\" start: {} end: {}",
                    print_text(device),
                    print_text(start),
                    print_text(end)
                ))
            }
            _ => Err(self.unsupported("this form; use rm N or mkpart")),
        }
    }

    fn mknod(&self, args: &[String]) -> Result<String, Error> {
        let parsed = self.parse(args, &["-m", "--mode"], &[])?;
        let words: Vec<&str> = parsed.operands.iter().map(String::as_str).collect();
        let (path, kind) = match words.as_slice() {
            [path, "p"] => (*path, None),
            [path, kind @ ("c" | "u" | "b"), major, minor] => {
                let major: u32 = major
                    .parse()
                    .map_err(|_| self.unsupported("a major number that is not a number"))?;
                let minor: u32 = minor
                    .parse()
                    .map_err(|_| self.unsupported("a minor number that is not a number"))?;
                (
                    *path,
                    Some((if *kind == "b" { "block" } else { "char" }, major, minor)),
                )
            }
            _ => return Err(self.unsupported("this form; use p or c|b MAJOR MINOR")),
        };
        if !path.starts_with('/') {
            return Err(self.session.error(
                ErrorCode::PathRelative,
                "relative_path",
                self.simple.span,
                &[("path", path)],
            ));
        }
        let mut text = format!("create {}", print_text(path));
        match kind {
            None => text.push_str(" type: fifo"),
            Some((kind, major, minor)) => text.push_str(&format!(
                " type: device kind: {kind} major: {major} minor: {minor}"
            )),
        }
        if let Some(mode) = parsed.value("-m").or_else(|| parsed.value("--mode")) {
            text.push_str(&format!(" mode: {}", print_text(mode)));
        }
        Ok(text)
    }

    fn mkfs(&self, args: &[String]) -> Result<String, Error> {
        let fs = if let Some(suffix) = self.name.strip_prefix("mkfs.") {
            suffix.to_owned()
        } else {
            let parsed = self.parse(args, &["-t", "--type"], &["-F", "-f"])?;
            return self.mkfs_with(
                parsed
                    .value("-t")
                    .or_else(|| parsed.value("--type"))
                    .map(str::to_owned),
                &parsed,
            );
        };
        let parsed = self.parse(args, &["-L", "-n", "-l"], &["-F", "-f", "-q"])?;
        self.mkfs_with(Some(fs), &parsed)
    }

    fn mkfs_with(&self, fs: Option<String>, parsed: &Parsed) -> Result<String, Error> {
        let Some(fs) = fs else {
            return Err(self.unsupported("a call without a file system type"));
        };
        let [device] = self.operand(parsed, 1)? else {
            return Err(self.unsupported("operands"));
        };
        let mut text = format!(
            "disks.format device: {} fs: {}",
            print_text(device),
            print_text(&fs)
        );
        if let Some(label) = parsed
            .value("-L")
            .or_else(|| parsed.value("-n"))
            .or_else(|| parsed.value("-l"))
        {
            text.push_str(&format!(" label: {}", print_text(label)));
        }
        Ok(text)
    }

    /// Parses the text of one or more commands into the stages of a piece.
    fn finish(&self, text: &str) -> Result<Variants, Error> {
        let document = parse(text, Variant::A, &ParseOptions::default()).map_err(|error| {
            Error::new(
                ErrorCode::InternalCorrection,
                format!("the administration translation does not parse: {error}\n{text}"),
            )
        })?;
        let mut pieces = Vec::new();
        for statement in document.program.statements {
            let Statement::Pipeline(pipeline) = statement else {
                continue;
            };
            let Head::Stage(stage) = pipeline.head else {
                continue;
            };
            pieces.push(stage_without_spans(stage));
        }
        let [stage] = pieces.as_slice() else {
            return Err(self.unsupported("a call that changes more than one thing"));
        };
        let stage = stage.clone();
        self.session.record(
            "B43",
            CorrectionClass::Certain,
            self.simple.span,
            super::tree::print_stage(&stage),
            "habit",
        );
        Ok(Variants::one(vec![stage], "effect"))
    }
}

fn stage_without_spans(mut stage: Stage) -> Stage {
    stage.span = None;
    for arg in &mut stage.args {
        arg.span = None;
        clear(&mut arg.value);
    }
    stage
}

fn clear(expr: &mut Expr) {
    match expr {
        Expr::Text { span, .. } | Expr::Int { span, .. } => *span = None,
        Expr::List { items, span } => {
            *span = None;
            items.iter_mut().for_each(clear);
        }
        _ => {}
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
