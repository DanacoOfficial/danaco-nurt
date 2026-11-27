// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::admin::firewall::model
// Responsibility:   Firewall rule model shared by every backend.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Firewall rule model shared by every backend.
//!
//! A rule is described by its verdict, protocol, port range, remote address,
//! direction and zone. The canonical text of a rule (its key) is stored in
//! the comment or label of the rule in the firewall, in the report and in the
//! journal, and parsed back by the inverse of the operation.

use std::fmt;
use std::net::IpAddr;

use nurt_contracts::error::{Error, ErrorCode, Result};

/// What the rule does with matching traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Verdict {
    /// The traffic passes.
    Allow,
    /// The traffic is dropped.
    Deny,
}

impl Verdict {
    /// Name used in keys and records.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

/// Direction of the traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Direction {
    /// Traffic that arrives at this host.
    In,
    /// Traffic that leaves this host.
    Out,
}

impl Direction {
    /// Name used in keys and records.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
        }
    }

    /// Value of the `direction` parameter: `in` is a keyword of the language,
    /// so the parameter names the directions `inbound` and `outbound`.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::In => "inbound",
            Self::Out => "outbound",
        }
    }
}

/// Transport protocol of a port rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Protocol {
    /// TCP.
    Tcp,
    /// UDP.
    Udp,
}

impl Protocol {
    /// Name used in keys and records.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

/// A firewall rule of the supported shape.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Rule {
    /// Allow or deny.
    pub verdict: Verdict,
    /// Transport protocol.
    pub protocol: Protocol,
    /// First port.
    pub from: u16,
    /// Last port; equal to `from` for a single port.
    pub to: u16,
    /// Address or network of the remote side; `None` for any.
    pub source: Option<String>,
    /// Direction of the traffic.
    pub direction: Direction,
    /// Zone (firewalld) or profile (Windows); `None` for the default.
    pub zone: Option<String>,
}

impl Rule {
    /// Port or range as text: `22` or `8000-8100`.
    #[must_use]
    pub fn port_text(&self) -> String {
        if self.from == self.to {
            self.from.to_string()
        } else {
            format!("{}-{}", self.from, self.to)
        }
    }

    /// Canonical key of the rule.
    #[must_use]
    pub fn key(&self) -> String {
        format!(
            "{} {} {} {} {} {}",
            self.verdict.as_str(),
            self.protocol.as_str(),
            self.port_text(),
            self.direction.as_str(),
            self.source.as_deref().unwrap_or("any"),
            self.zone.as_deref().unwrap_or("-"),
        )
    }

    /// Target of the change in reports: `firewall:<key>`.
    #[must_use]
    pub fn target(&self) -> String {
        format!("{TARGET_PREFIX}{}", self.key())
    }

    /// Rule of a key written by [`Rule::key`].
    #[must_use]
    pub fn parse_key(key: &str) -> Option<Self> {
        let key = key.strip_prefix(LABEL_PREFIX).unwrap_or(key);
        let mut words = key.split_whitespace();
        let verdict = match words.next()? {
            "allow" => Verdict::Allow,
            "deny" => Verdict::Deny,
            _ => return None,
        };
        let protocol = match words.next()? {
            "tcp" => Protocol::Tcp,
            "udp" => Protocol::Udp,
            _ => return None,
        };
        let (from, to) = parse_ports(words.next()?).ok()?;
        let direction = match words.next()? {
            "in" => Direction::In,
            "out" => Direction::Out,
            _ => return None,
        };
        let source = match words.next()? {
            "any" => None,
            text if valid_source(text) => Some(text.to_owned()),
            _ => return None,
        };
        let zone = match words.next()? {
            "-" => None,
            text if valid_zone(text) => Some(text.to_owned()),
            _ => return None,
        };
        words.next().is_none().then_some(Self {
            verdict,
            protocol,
            from,
            to,
            source,
            direction,
            zone,
        })
    }

    /// Whether `self` selects `other`: every field given in `self` equals
    /// the field of `other`; `verdict` of `None` selects both verdicts.
    #[must_use]
    pub fn selects(&self, other: &Self, any_verdict: bool) -> bool {
        (any_verdict || self.verdict == other.verdict)
            && self.protocol == other.protocol
            && self.from == other.from
            && self.to == other.to
            && self.direction == other.direction
            && self.source == other.source
            && self.zone == other.zone
    }

    /// Words of the key written as a program: `firewall.<verb> <port> …`.
    ///
    /// Used by the engine to undo the change; `verb` is `allow`, `deny` or
    /// `remove`.
    #[must_use]
    pub fn program(&self, verb: &str, backend: Option<&str>, table: Option<&str>) -> String {
        let mut text = format!(
            "firewall.{verb} \"{}\" protocol: {} direction: {}",
            self.port_text(),
            self.protocol.as_str(),
            self.direction.word()
        );
        if let Some(source) = &self.source {
            text.push_str(&format!(" source: \"{source}\""));
        }
        if let Some(zone) = &self.zone {
            text.push_str(&format!(" zone: \"{zone}\""));
        }
        if verb == "remove" {
            text.push_str(&format!(" action: {}", self.verdict.as_str()));
        }
        if let Some(backend) = backend {
            text.push_str(&format!(" backend: {backend}"));
        }
        if let Some(table) = table {
            text.push_str(&format!(" table: \"{table}\""));
        }
        text
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.key())
    }
}

/// Prefix of the target of a firewall change.
pub const TARGET_PREFIX: &str = "firewall:";

/// Prefix of the comment or label that marks a rule created by Nurt.
pub const LABEL_PREFIX: &str = "nurt:";

/// Label of the rule in the firewall: the key with the Nurt prefix.
#[must_use]
pub fn label_of(rule: &Rule) -> String {
    format!("{LABEL_PREFIX}{}", rule.key())
}

/// Parses `22` or `8000-8100` into an ordered pair of ports.
///
/// # Errors
///
/// `E_INVALID_VALUE` for text that is not a port or a range of ports.
pub fn parse_ports(text: &str) -> Result<(u16, u16)> {
    let invalid = || {
        Error::new(
            ErrorCode::InvalidValue,
            format!("`{text}` is not a port (1-65535) or a range `first-last`"),
        )
    };
    let port = |part: &str| -> Result<u16> {
        part.trim()
            .parse::<u16>()
            .ok()
            .filter(|number| *number != 0)
            .ok_or_else(invalid)
    };
    match text.split_once('-') {
        None => port(text).map(|number| (number, number)),
        Some((first, last)) => {
            let (first, last) = (port(first)?, port(last)?);
            if first > last {
                return Err(invalid());
            }
            Ok((first, last))
        }
    }
}

/// Whether `text` is an address or a network in CIDR notation.
#[must_use]
pub fn valid_source(text: &str) -> bool {
    if text.is_empty() || text.len() > 49 {
        return false;
    }
    let (address, prefix) = match text.split_once('/') {
        Some((address, prefix)) => (address, Some(prefix)),
        None => (text, None),
    };
    let Ok(parsed) = address.parse::<IpAddr>() else {
        return false;
    };
    let limit = if parsed.is_ipv4() { 32 } else { 128 };
    prefix.is_none_or(|prefix| prefix.parse::<u8>().is_ok_and(|bits| bits <= limit))
}

/// Whether `text` is usable as a zone or profile name.
#[must_use]
pub fn valid_zone(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 32
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

/// Whether `text` is an IPv6 address or network.
#[must_use]
pub(crate) fn is_v6(text: &str) -> bool {
    text.contains(':')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule() -> Rule {
        Rule {
            verdict: Verdict::Allow,
            protocol: Protocol::Tcp,
            from: 8000,
            to: 8100,
            source: Some("10.0.0.0/8".to_owned()),
            direction: Direction::In,
            zone: Some("public".to_owned()),
        }
    }

    #[test]
    fn key_round_trips() {
        let rule = rule();
        assert_eq!(rule.key(), "allow tcp 8000-8100 in 10.0.0.0/8 public");
        assert_eq!(Rule::parse_key(&rule.key()), Some(rule.clone()));
        assert_eq!(Rule::parse_key(&label_of(&rule)), Some(rule));
    }

    #[test]
    fn key_without_optional_fields() {
        let rule = Rule {
            source: None,
            zone: None,
            from: 22,
            to: 22,
            ..rule()
        };
        assert_eq!(rule.key(), "allow tcp 22 in any -");
        assert_eq!(Rule::parse_key("allow tcp 22 in any -"), Some(rule));
    }

    #[test]
    fn rejects_malformed_keys() {
        for key in [
            "",
            "allow tcp 22 in any",
            "allow icmp 22 in any -",
            "allow tcp 0 in any -",
            "allow tcp 9-1 in any -",
            "allow tcp 22 in 10.0.0.0/33 -",
            "allow tcp 22 in any - extra",
            "permit tcp 22 in any -",
        ] {
            assert_eq!(Rule::parse_key(key), None, "{key}");
        }
    }

    #[test]
    fn ports_are_validated() {
        assert_eq!(parse_ports("22").unwrap(), (22, 22));
        assert_eq!(parse_ports("1-65535").unwrap(), (1, 65535));
        for text in ["0", "65536", "9-1", "a", "", "1-", "-1", "1-2-3"] {
            assert!(parse_ports(text).is_err(), "{text}");
        }
    }

    #[test]
    fn sources_and_zones_are_validated() {
        assert!(valid_source("192.168.1.0/24"));
        assert!(valid_source("fe80::1"));
        assert!(valid_source("2001:db8::/32"));
        assert!(!valid_source("10.0.0.0/33"));
        assert!(!valid_source("example.org"));
        assert!(!valid_source("198.51.100.1; reboot"));
        assert!(valid_zone("public"));
        assert!(!valid_zone("pub lic"));
        assert!(!valid_zone("a\"b"));
    }

    #[test]
    fn program_of_a_rule_names_every_field() {
        assert_eq!(
            rule().program("remove", Some("ufw"), None),
            "firewall.remove \"8000-8100\" protocol: tcp direction: inbound source: \"10.0.0.0/8\" zone: \"public\" action: allow backend: ufw"
        );
        assert!(
            rule()
                .program("allow", Some("nftables"), Some("nurt"))
                .ends_with("backend: nftables table: \"nurt\"")
        );
        assert_eq!(
            Rule {
                source: None,
                zone: None,
                verdict: Verdict::Deny,
                from: 22,
                to: 22,
                ..rule()
            }
            .program("deny", None, None),
            "firewall.deny \"22\" protocol: tcp direction: inbound"
        );
    }

    #[test]
    fn selection_ignores_verdict_on_request() {
        let allow = rule();
        let deny = Rule {
            verdict: Verdict::Deny,
            ..rule()
        };
        assert!(!allow.selects(&deny, false));
        assert!(allow.selects(&deny, true));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
