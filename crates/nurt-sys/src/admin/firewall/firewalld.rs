// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! firewalld backend: ports and rich rules of a zone, runtime and permanent.

use nurt_contracts::error::{Error, ErrorCode, Result};

use super::backend::{Backend, Listed};
use super::model::{is_v6, Direction, Protocol, Rule, Verdict};
use crate::admin::{Call, Tool};

/// The firewalld firewall.
#[derive(Debug, Clone, Copy)]
pub struct Firewalld;

fn zone_args(rule: &Rule) -> Vec<String> {
    rule.zone
        .as_ref()
        .map(|zone| vec![format!("--zone={zone}")])
        .unwrap_or_default()
}

fn simple(rule: &Rule) -> bool {
    rule.verdict == Verdict::Allow && rule.source.is_none()
}

/// Rich rule of `rule` in the syntax of `firewall-cmd`.
#[must_use]
pub fn rich_rule(rule: &Rule) -> String {
    let mut text = String::from("rule");
    if let Some(source) = &rule.source {
        let family = if is_v6(source) { "ipv6" } else { "ipv4" };
        text.push_str(&format!(" family=\"{family}\" source address=\"{source}\""));
    }
    text.push_str(&format!(
        " port port=\"{}\" protocol=\"{}\" {}",
        rule.port_text(),
        rule.protocol.as_str(),
        match rule.verdict {
            Verdict::Allow => "accept",
            Verdict::Deny => "drop",
        }
    ));
    text
}

fn spec_args(rule: &Rule, verb: &str) -> Vec<String> {
    if simple(rule) {
        vec![format!(
            "--{verb}-port={}/{}",
            rule.port_text(),
            rule.protocol.as_str()
        )]
    } else {
        vec![format!("--{verb}-rich-rule={}", rich_rule(rule))]
    }
}

fn calls(rule: &Rule, verb: &str) -> Vec<Call> {
    [false, true]
        .into_iter()
        .map(|permanent| {
            let mut args = Vec::new();
            if permanent {
                args.push("--permanent".to_owned());
            }
            args.extend(zone_args(rule));
            args.extend(spec_args(rule, verb));
            Call::owned("firewall-cmd", args)
        })
        .collect()
}

/// Rule of one entry of `--list-ports` such as `8000-8100/tcp`.
#[must_use]
pub fn parse_port_entry(entry: &str, zone: &str) -> Option<Rule> {
    let (ports, protocol) = entry.split_once('/')?;
    let protocol = match protocol {
        "tcp" => Protocol::Tcp,
        "udp" => Protocol::Udp,
        _ => return None,
    };
    let (from, to) = super::model::parse_ports(ports).ok()?;
    Some(Rule {
        verdict: Verdict::Allow,
        protocol,
        from,
        to,
        source: None,
        direction: Direction::In,
        zone: Some(zone.to_owned()),
    })
}

fn quoted_after<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let start = text.find(&format!("{name}=\""))? + name.len() + 2;
    let end = text[start..].find('"')? + start;
    Some(&text[start..end])
}

/// Rule of one rich rule such as
/// `rule family="ipv4" source address="10.0.0.0/8" port port="22" protocol="tcp" accept`.
#[must_use]
pub fn parse_rich_rule(text: &str, zone: &str) -> Option<Rule> {
    let verdict = match text.split_whitespace().last()? {
        "accept" => Verdict::Allow,
        "drop" | "reject" => Verdict::Deny,
        _ => return None,
    };
    let protocol = match quoted_after(text, "protocol")? {
        "tcp" => Protocol::Tcp,
        "udp" => Protocol::Udp,
        _ => return None,
    };
    let (from, to) = super::model::parse_ports(quoted_after(text, "port")?).ok()?;
    let source = match text.find("source address=") {
        Some(_) => Some(quoted_after(text, "address")?.to_owned()),
        None => None,
    };
    Some(Rule {
        verdict,
        protocol,
        from,
        to,
        source,
        direction: Direction::In,
        zone: Some(zone.to_owned()),
    })
}

impl Firewalld {
    fn zones(tool: &Tool<'_>) -> Result<Vec<String>> {
        let active = tool.ok(&Call::new("firewall-cmd", &["--get-active-zones"]))?;
        let mut zones: Vec<String> = active
            .stdout
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with(char::is_whitespace))
            .map(|line| line.trim().to_owned())
            .collect();
        if zones.is_empty() {
            zones.push(Self::default_zone(tool)?);
        }
        Ok(zones)
    }

    fn default_zone(tool: &Tool<'_>) -> Result<String> {
        let output = tool.ok(&Call::new("firewall-cmd", &["--get-default-zone"]))?;
        Ok(output.stdout.trim().to_owned())
    }
}

impl Backend for Firewalld {
    fn name(&self) -> &'static str {
        "firewalld"
    }

    fn normalize(&self, tool: &Tool<'_>, rule: &mut Rule) -> Result<()> {
        if rule.direction == Direction::Out {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "firewalld filters incoming traffic only; use `direction: in` or the backend `nftables`",
            ));
        }
        if rule.zone.is_none() {
            rule.zone = Some(Self::default_zone(tool)?);
        }
        Ok(())
    }

    fn list(&self, tool: &Tool<'_>) -> Result<Vec<Listed>> {
        let mut found = Vec::new();
        for zone in Self::zones(tool)? {
            let zone_arg = format!("--zone={zone}");
            let ports = tool.ok(&Call::new("firewall-cmd", &[&zone_arg, "--list-ports"]))?;
            for entry in ports.stdout.split_whitespace() {
                found.push(Listed {
                    rule: parse_port_entry(entry, &zone),
                    raw: entry.to_owned(),
                    place: zone.clone(),
                    managed: false,
                });
            }
            let rich = tool.ok(&Call::new(
                "firewall-cmd",
                &[&zone_arg, "--list-rich-rules"],
            ))?;
            for line in rich.stdout.lines().map(str::trim).filter(|l| !l.is_empty()) {
                found.push(Listed {
                    rule: parse_rich_rule(line, &zone),
                    raw: line.to_owned(),
                    place: zone.clone(),
                    managed: false,
                });
            }
        }
        Ok(found)
    }

    fn add(&self, tool: &Tool<'_>, rule: &Rule) -> Result<()> {
        let calls = calls(rule, "add");
        tool.ok(&calls[0])?;
        if let Err(error) = tool.ok(&calls[1]) {
            let _ = tool.run(&Call::owned(
                "firewall-cmd",
                calls_remove_runtime(rule),
            ));
            return Err(error);
        }
        Ok(())
    }

    fn remove(&self, tool: &Tool<'_>, rule: &Rule) -> Result<()> {
        let calls = calls(rule, "remove");
        let runtime = tool.run(&calls[0])?;
        let permanent = tool.run(&calls[1])?;
        if runtime.success() || permanent.success() {
            Ok(())
        } else {
            Err(crate::admin::tool::failure(&calls[0], &runtime))
        }
    }
}

fn calls_remove_runtime(rule: &Rule) -> Vec<String> {
    let mut args = zone_args(rule);
    args.extend(spec_args(rule, "remove"));
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_allow_uses_a_port() {
        let rule = Rule::parse_key("allow tcp 8000-8100 in any public").unwrap();
        let calls = calls(&rule, "add");
        assert_eq!(
            calls[0].line(),
            "firewall-cmd --zone=public --add-port=8000-8100/tcp"
        );
        assert_eq!(
            calls[1].line(),
            "firewall-cmd --permanent --zone=public --add-port=8000-8100/tcp"
        );
    }

    #[test]
    fn source_or_deny_uses_a_rich_rule() {
        let rule = Rule::parse_key("deny udp 53 in 10.0.0.0/8 public").unwrap();
        assert_eq!(
            rich_rule(&rule),
            "rule family=\"ipv4\" source address=\"10.0.0.0/8\" port port=\"53\" protocol=\"udp\" drop"
        );
        assert_eq!(parse_rich_rule(&rich_rule(&rule), "public"), Some(rule));
        let any = Rule::parse_key("deny tcp 23 in any public").unwrap();
        assert_eq!(parse_rich_rule(&rich_rule(&any), "public"), Some(any));
    }

    #[test]
    fn parses_port_entries() {
        assert_eq!(
            parse_port_entry("22/tcp", "home").unwrap().key(),
            "allow tcp 22 in any home"
        );
        assert_eq!(parse_port_entry("22/sctp", "home"), None);
    }
}
