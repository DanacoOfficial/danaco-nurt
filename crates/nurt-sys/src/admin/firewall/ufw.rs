// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::admin::firewall::ufw
// Responsibility:   Ufw backend: rules of `ufw allow` and `ufw deny`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! ufw backend: rules of `ufw allow` and `ufw deny`.

use nurt_contracts::error::{Error, ErrorCode, Result};

use super::backend::{Backend, Listed};
use super::model::{Direction, Protocol, Rule, Verdict};
use crate::admin::{Call, Tool};

/// The ufw firewall.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ufw;

/// Arguments after `ufw allow` or `ufw deny` that describe `rule`.
#[must_use]
pub(crate) fn rule_args(rule: &Rule) -> Vec<String> {
    let remote = rule.source.clone().unwrap_or_else(|| "any".to_owned());
    let ports = rule.port_text().replace('-', ":");
    let mut args = vec![
        rule.direction.as_str().to_owned(),
        "proto".to_owned(),
        rule.protocol.as_str().to_owned(),
    ];
    match rule.direction {
        Direction::In => {
            args.extend(["from".to_owned(), remote, "to".to_owned(), "any".to_owned()])
        }
        Direction::Out => {
            args.extend(["from".to_owned(), "any".to_owned(), "to".to_owned(), remote])
        }
    }
    args.extend(["port".to_owned(), ports]);
    args
}

fn verb(rule: &Rule) -> &'static str {
    match rule.verdict {
        Verdict::Allow => "allow",
        Verdict::Deny => "deny",
    }
}

/// Parses the table printed by `ufw status`.
#[must_use]
pub(crate) fn parse_status(text: &str) -> Vec<Listed> {
    let mut found = Vec::new();
    for line in text
        .lines()
        .skip_while(|line| !line.starts_with("--"))
        .skip(1)
    {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.len() < 3 || line.contains("(v6)") {
            continue;
        }
        found.push(Listed {
            rule: parse_line(&words),
            raw: line.trim().to_owned(),
            place: "ufw".to_owned(),
            managed: false,
        });
    }
    found
}

fn parse_line(words: &[&str]) -> Option<Rule> {
    let (ports, protocol) = words[0].split_once('/')?;
    let protocol = match protocol {
        "tcp" => Protocol::Tcp,
        "udp" => Protocol::Udp,
        _ => return None,
    };
    let (from, to) = super::model::parse_ports(&ports.replace(':', "-")).ok()?;
    let verdict = match words[1] {
        "ALLOW" => Verdict::Allow,
        "DENY" | "REJECT" => Verdict::Deny,
        _ => return None,
    };
    let (direction, rest) = match words.get(2).copied() {
        Some("IN") => (Direction::In, &words[3..]),
        Some("OUT") => (Direction::Out, &words[3..]),
        _ => (Direction::In, &words[2..]),
    };
    let source = match rest.first().copied() {
        Some("Anywhere") | None => None,
        Some(text) if super::model::valid_source(text) => Some(text.to_owned()),
        Some(_) => return None,
    };
    Some(Rule {
        verdict,
        protocol,
        from,
        to,
        source,
        direction,
        zone: None,
    })
}

impl Backend for Ufw {
    fn name(&self) -> &'static str {
        "ufw"
    }

    fn normalize(&self, _tool: &Tool<'_>, rule: &mut Rule) -> Result<()> {
        if rule.zone.is_some() {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "ufw has no zones; leave `zone` out or use the backend `firewalld`",
            ));
        }
        Ok(())
    }

    fn list(&self, tool: &Tool<'_>) -> Result<Vec<Listed>> {
        let output = tool.ok(&Call::new("ufw", &["status"]))?;
        Ok(parse_status(&output.stdout))
    }

    fn add(&self, tool: &Tool<'_>, rule: &Rule) -> Result<()> {
        let mut args = vec!["--force".to_owned(), verb(rule).to_owned()];
        args.extend(rule_args(rule));
        tool.ok(&Call::owned("ufw", args))?;
        Ok(())
    }

    fn remove(&self, tool: &Tool<'_>, rule: &Rule) -> Result<()> {
        let mut args = vec![
            "--force".to_owned(),
            "delete".to_owned(),
            verb(rule).to_owned(),
        ];
        args.extend(rule_args(rule));
        tool.ok(&Call::owned("ufw", args))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "Status: active\n\nTo                         Action      From\n--                         ------      ----\n22/tcp                     ALLOW IN    Anywhere\n8000:8100/tcp              ALLOW       10.0.0.0/8\n53/udp                     DENY OUT    Anywhere\n22/tcp (v6)                ALLOW IN    Anywhere (v6)\nAnywhere                   ALLOW IN    192.168.1.5\n";

    #[test]
    fn parses_the_status_table() {
        let rules = parse_status(STATUS);
        let keys: Vec<Option<String>> = rules
            .iter()
            .map(|listed| listed.rule.as_ref().map(Rule::key))
            .collect();
        assert_eq!(
            keys,
            vec![
                Some("allow tcp 22 in any -".to_owned()),
                Some("allow tcp 8000-8100 in 10.0.0.0/8 -".to_owned()),
                Some("deny udp 53 out any -".to_owned()),
                None,
            ]
        );
    }

    #[test]
    fn arguments_follow_the_direction() {
        let incoming = Rule::parse_key("allow tcp 8000-8100 in 10.0.0.0/8 -").unwrap();
        assert_eq!(
            rule_args(&incoming).join(" "),
            "in proto tcp from 10.0.0.0/8 to any port 8000:8100"
        );
        let outgoing = Rule::parse_key("deny udp 53 out 10.0.0.1 -").unwrap();
        assert_eq!(
            rule_args(&outgoing).join(" "),
            "out proto udp from any to 10.0.0.1 port 53"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
