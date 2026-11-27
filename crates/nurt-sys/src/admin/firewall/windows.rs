// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Windows Defender Firewall backend through the NetSecurity cmdlets.

use nurt_contracts::error::{Error, ErrorCode, Result};
use serde_json::Value as Json;

use super::backend::{Backend, Listed};
use super::model::{Direction, LABEL_PREFIX, Protocol, Rule, Verdict, label_of};
use crate::admin::{Call, Tool};

/// The Windows Defender Firewall.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WindowsFirewall;

const PROFILES: [&str; 3] = ["Domain", "Private", "Public"];

const LIST_SCRIPT: &str = "Get-NetFirewallRule -Enabled True | ForEach-Object { $port = $_ | Get-NetFirewallPortFilter; $address = $_ | Get-NetFirewallAddressFilter; [pscustomobject]@{ name = $_.DisplayName; direction = [string]$_.Direction; action = [string]$_.Action; profile = [string]$_.Profile; protocol = [string]$port.Protocol; port = (@($port.LocalPort) -join ','); remote = (@($address.RemoteAddress) -join ',') } } | ConvertTo-Json -Compress";

fn profile_of(rule: &Rule) -> Result<Option<&'static str>> {
    let Some(zone) = &rule.zone else {
        return Ok(None);
    };
    PROFILES
        .iter()
        .find(|name| name.eq_ignore_ascii_case(zone))
        .map(|name| Some(*name))
        .ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("`{zone}` is not a profile; use domain, private or public"),
            )
        })
}

/// PowerShell command that creates the rule.
///
/// # Errors
///
/// `E_INVALID_VALUE` for a zone that is not a profile.
pub(crate) fn add_script(rule: &Rule) -> Result<String> {
    let mut script = format!(
        "New-NetFirewallRule -DisplayName '{}' -Direction {} -Action {} -Protocol {} -LocalPort {}",
        label_of(rule),
        match rule.direction {
            Direction::In => "Inbound",
            Direction::Out => "Outbound",
        },
        match rule.verdict {
            Verdict::Allow => "Allow",
            Verdict::Deny => "Block",
        },
        rule.protocol.as_str().to_uppercase(),
        rule.port_text(),
    );
    if let Some(source) = &rule.source {
        script.push_str(&format!(" -RemoteAddress {source}"));
    }
    if let Some(profile) = profile_of(rule)? {
        script.push_str(&format!(" -Profile {profile}"));
    }
    script.push_str(" | Out-Null");
    Ok(script)
}

/// PowerShell command that removes the rule.
#[must_use]
pub(crate) fn remove_script(rule: &Rule) -> String {
    format!(
        "Remove-NetFirewallRule -DisplayName '{}' -ErrorAction Stop",
        label_of(rule)
    )
}

/// Rules in the JSON printed by [`LIST_SCRIPT`]: an object for one rule, an
/// array for several, nothing for none.
///
/// # Errors
///
/// `E_STAGE_FAILED` when the text is not JSON.
pub(crate) fn parse_list(text: &str) -> Result<Vec<Listed>> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let document: Json = serde_json::from_str(text.trim()).map_err(|error| {
        Error::new(
            ErrorCode::StageFailed,
            format!("PowerShell printed text that is not JSON: {error}"),
        )
    })?;
    let items = match document {
        Json::Array(items) => items,
        single => vec![single],
    };
    Ok(items.iter().map(listed_of).collect())
}

fn listed_of(item: &Json) -> Listed {
    let text = |name: &str| item.get(name).and_then(Json::as_str).unwrap_or_default();
    let name = text("name");
    let managed = name.starts_with(LABEL_PREFIX);
    let rule = if managed {
        Rule::parse_key(name)
    } else {
        foreign_rule(item)
    };
    Listed {
        rule,
        raw: item.to_string(),
        place: text("profile").to_owned(),
        managed,
    }
}

fn foreign_rule(item: &Json) -> Option<Rule> {
    let text = |name: &str| item.get(name).and_then(Json::as_str).unwrap_or_default();
    let protocol = match text("protocol").to_ascii_lowercase().as_str() {
        "tcp" => Protocol::Tcp,
        "udp" => Protocol::Udp,
        _ => return None,
    };
    let (from, to) = super::model::parse_ports(text("port")).ok()?;
    let direction = match text("direction") {
        "Inbound" => Direction::In,
        "Outbound" => Direction::Out,
        _ => return None,
    };
    let verdict = match text("action") {
        "Allow" => Verdict::Allow,
        "Block" => Verdict::Deny,
        _ => return None,
    };
    let source = match text("remote") {
        "Any" | "" => None,
        remote if super::model::valid_source(remote) => Some(remote.to_owned()),
        _ => return None,
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

impl Backend for WindowsFirewall {
    fn name(&self) -> &'static str {
        "windows"
    }

    fn normalize(&self, _tool: &Tool<'_>, rule: &mut Rule) -> Result<()> {
        profile_of(rule).map(|_| ())
    }

    fn list(&self, tool: &Tool<'_>) -> Result<Vec<Listed>> {
        let output = tool.ok(&Call::powershell(LIST_SCRIPT))?;
        parse_list(&output.stdout)
    }

    fn add(&self, tool: &Tool<'_>, rule: &Rule) -> Result<()> {
        tool.ok(&Call::powershell(&add_script(rule)?))?;
        Ok(())
    }

    fn remove(&self, tool: &Tool<'_>, rule: &Rule) -> Result<()> {
        tool.ok(&Call::powershell(&remove_script(rule)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_script_describes_the_rule() {
        let rule = Rule::parse_key("deny tcp 8000-8100 out 10.0.0.0/8 public").unwrap();
        assert_eq!(
            add_script(&rule).unwrap(),
            "New-NetFirewallRule -DisplayName 'nurt:deny tcp 8000-8100 out 10.0.0.0/8 public' -Direction Outbound -Action Block -Protocol TCP -LocalPort 8000-8100 -RemoteAddress 10.0.0.0/8 -Profile Public | Out-Null"
        );
    }

    #[test]
    fn unknown_profile_is_rejected() {
        let rule = Rule::parse_key("allow tcp 22 in any dmz").unwrap();
        assert_eq!(
            add_script(&rule).unwrap_err().code(),
            ErrorCode::InvalidValue
        );
    }

    #[test]
    fn list_accepts_one_object_or_an_array() {
        let one = r#"{"name":"nurt:allow tcp 22 in any -","direction":"Inbound","action":"Allow","profile":"Any","protocol":"TCP","port":"22","remote":"Any"}"#;
        let rules = parse_list(one).unwrap();
        assert_eq!(rules.len(), 1);
        assert!(rules[0].managed);
        let many = format!(
            "[{one},{{\"name\":\"Web\",\"direction\":\"Inbound\",\"action\":\"Allow\",\"profile\":\"Any\",\"protocol\":\"UDP\",\"port\":\"5000-5010\",\"remote\":\"10.0.0.1\"}},{{\"name\":\"Ping\",\"direction\":\"Inbound\",\"action\":\"Allow\",\"profile\":\"Any\",\"protocol\":\"ICMPv4\",\"port\":\"\",\"remote\":\"Any\"}}]"
        );
        let rules = parse_list(&many).unwrap();
        assert_eq!(rules.len(), 3);
        assert_eq!(
            rules[1].rule.as_ref().unwrap().key(),
            "allow udp 5000-5010 in 10.0.0.1 -"
        );
        assert_eq!(rules[2].rule, None);
        assert!(parse_list("").unwrap().is_empty());
    }
}
