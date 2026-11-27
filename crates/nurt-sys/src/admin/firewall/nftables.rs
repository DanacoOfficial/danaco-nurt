// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::admin::firewall::nftables
// Responsibility:   Nftables backend: rules in a table of the family `inet`.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! nftables backend: rules in a table of the family `inet`.
//!
//! Rules of Nurt live in the chains `input` and `output` of one table
//! (`nurt` by default). The comment of the rule holds its key, so a rule is
//! found again without comparing expressions.

use nurt_contracts::error::{Error, ErrorCode, Result};
use serde_json::Value as Json;

use super::backend::{Backend, Listed};
use super::model::{Direction, LABEL_PREFIX, Protocol, Rule, Verdict, label_of};
use crate::admin::{Call, Tool};

/// Default table of the rules of Nurt.
pub const DEFAULT_TABLE: &str = "nurt";

/// The nftables firewall.
#[derive(Debug, Clone)]
pub struct Nftables {
    table: String,
}

/// A rule of the ruleset with its address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftRule {
    /// Table of the rule.
    pub table: String,
    /// Chain of the rule.
    pub chain: String,
    /// Handle of the rule in the chain.
    pub handle: u64,
    /// Rule in the listing format.
    pub listed: Listed,
}

impl Nftables {
    /// Backend that manages the table `table` of the family `inet`.
    #[must_use]
    pub fn new(table: &str) -> Self {
        Self {
            table: table.to_owned(),
        }
    }

    fn chain_of(direction: Direction) -> &'static str {
        match direction {
            Direction::In => "input",
            Direction::Out => "output",
        }
    }

    /// Calls that create the table and the chain of `direction`.
    fn ensure(&self, tool: &Tool<'_>, direction: Direction) -> Result<()> {
        let chain = Self::chain_of(direction);
        let present = tool.run(&Call::new(
            "nft",
            &["list", "chain", "inet", &self.table, chain],
        ))?;
        if present.success() {
            return Ok(());
        }
        tool.ok(&Call::new("nft", &["add", "table", "inet", &self.table]))?;
        let spec = format!("{{ type filter hook {chain} priority 0 ; policy accept ; }}");
        tool.ok(&Call::new(
            "nft",
            &["add", "chain", "inet", &self.table, chain, &spec],
        ))?;
        Ok(())
    }

    /// Arguments of the rule expression of `rule`.
    #[must_use]
    pub fn expression(rule: &Rule) -> Vec<String> {
        let mut words: Vec<String> = Vec::new();
        if let Some(source) = &rule.source {
            let family = if super::model::is_v6(source) {
                "ip6"
            } else {
                "ip"
            };
            let side = match rule.direction {
                Direction::In => "saddr",
                Direction::Out => "daddr",
            };
            words.extend([family.to_owned(), side.to_owned(), source.clone()]);
        }
        words.extend([
            rule.protocol.as_str().to_owned(),
            "dport".to_owned(),
            rule.port_text(),
            match rule.verdict {
                Verdict::Allow => "accept",
                Verdict::Deny => "drop",
            }
            .to_owned(),
            "comment".to_owned(),
            format!("\"{}\"", label_of(rule)),
        ]);
        words
    }

    /// Rules of the whole ruleset with their handles.
    ///
    /// # Errors
    ///
    /// The error of `nft`, or `E_STAGE_FAILED` when its output is not JSON.
    pub fn rules(tool: &Tool<'_>) -> Result<Vec<NftRule>> {
        let output = tool.ok(&Call::new("nft", &["-j", "list", "ruleset"]))?;
        parse_ruleset(&output.stdout)
    }
}

/// Parses the JSON of `nft -j list ruleset`.
///
/// # Errors
///
/// `E_STAGE_FAILED` when the text is not the JSON of a ruleset.
pub fn parse_ruleset(text: &str) -> Result<Vec<NftRule>> {
    let document: Json = serde_json::from_str(text).map_err(|error| {
        Error::new(
            ErrorCode::StageFailed,
            format!("`nft -j` printed text that is not JSON: {error}"),
        )
    })?;
    let items = document
        .get("nftables")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();
    let hooks: Vec<(String, String, String)> = items
        .iter()
        .filter_map(|item| item.get("chain"))
        .filter_map(|chain| {
            Some((
                chain.get("table")?.as_str()?.to_owned(),
                chain.get("name")?.as_str()?.to_owned(),
                chain.get("hook")?.as_str()?.to_owned(),
            ))
        })
        .collect();
    let mut found = Vec::new();
    for item in &items {
        let Some(rule) = item.get("rule") else {
            continue;
        };
        let (Some(table), Some(chain), Some(handle)) = (
            rule.get("table").and_then(Json::as_str),
            rule.get("chain").and_then(Json::as_str),
            rule.get("handle").and_then(Json::as_u64),
        ) else {
            continue;
        };
        let hook = hooks
            .iter()
            .find(|(t, c, _)| t == table && c == chain)
            .map(|(_, _, hook)| hook.as_str());
        let comment = rule.get("comment").and_then(Json::as_str);
        let managed = comment.is_some_and(|text| text.starts_with(LABEL_PREFIX));
        let parsed = comment
            .filter(|_| managed)
            .and_then(Rule::parse_key)
            .or_else(|| from_expression(rule, hook));
        found.push(NftRule {
            table: table.to_owned(),
            chain: chain.to_owned(),
            handle,
            listed: Listed {
                rule: parsed,
                raw: rule.to_string(),
                place: format!("{table} {chain}"),
                managed,
            },
        });
    }
    Ok(found)
}

fn from_expression(rule: &Json, hook: Option<&str>) -> Option<Rule> {
    let direction = match hook? {
        "input" => Direction::In,
        "output" => Direction::Out,
        _ => return None,
    };
    let mut verdict = None;
    let mut ports = None;
    let mut protocol = None;
    let mut source = None;
    for expr in rule.get("expr")?.as_array()? {
        if expr.get("accept").is_some() {
            verdict = Some(Verdict::Allow);
        } else if expr.get("drop").is_some() || expr.get("reject").is_some() {
            verdict = Some(Verdict::Deny);
        }
        let Some(matched) = expr.get("match") else {
            continue;
        };
        let payload = matched.get("left").and_then(|left| left.get("payload"));
        let field = payload.and_then(|payload| payload.get("field")?.as_str());
        let proto = payload.and_then(|payload| payload.get("protocol")?.as_str());
        match (proto, field) {
            (Some("tcp" | "udp"), Some("dport")) => {
                protocol = Some(if proto == Some("tcp") {
                    Protocol::Tcp
                } else {
                    Protocol::Udp
                });
                ports = port_range(matched.get("right")?);
            }
            (Some("ip" | "ip6"), Some("saddr" | "daddr")) => {
                source = address_of(matched.get("right")?);
            }
            _ => {}
        }
    }
    let (from, to) = ports?;
    Some(Rule {
        verdict: verdict?,
        protocol: protocol?,
        from,
        to,
        source,
        direction,
        zone: None,
    })
}

fn port_range(right: &Json) -> Option<(u16, u16)> {
    if let Some(port) = right.as_u64() {
        let port = u16::try_from(port).ok()?;
        return Some((port, port));
    }
    let range = right.get("range")?.as_array()?;
    let first = u16::try_from(range.first()?.as_u64()?).ok()?;
    let last = u16::try_from(range.get(1)?.as_u64()?).ok()?;
    Some((first, last))
}

fn address_of(right: &Json) -> Option<String> {
    if let Some(text) = right.as_str() {
        return Some(text.to_owned());
    }
    let prefix = right.get("prefix")?;
    Some(format!(
        "{}/{}",
        prefix.get("addr")?.as_str()?,
        prefix.get("len")?.as_u64()?
    ))
}

impl Backend for Nftables {
    fn name(&self) -> &'static str {
        "nftables"
    }

    fn normalize(&self, _tool: &Tool<'_>, rule: &mut Rule) -> Result<()> {
        if rule.zone.is_some() {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "nftables has no zones; leave `zone` out or use the backend `firewalld`",
            ));
        }
        Ok(())
    }

    fn owns(&self, listed: &Listed) -> bool {
        listed.place.split(' ').next() == Some(self.table.as_str())
    }

    fn list(&self, tool: &Tool<'_>) -> Result<Vec<Listed>> {
        Ok(Self::rules(tool)?
            .into_iter()
            .map(|rule| rule.listed)
            .collect())
    }

    fn add(&self, tool: &Tool<'_>, rule: &Rule) -> Result<()> {
        self.ensure(tool, rule.direction)?;
        let mut args: Vec<String> = ["add", "rule", "inet", &self.table]
            .iter()
            .map(|word| (*word).to_owned())
            .collect();
        args.push(Self::chain_of(rule.direction).to_owned());
        args.extend(Self::expression(rule));
        tool.ok(&Call::owned("nft", args))?;
        Ok(())
    }

    fn remove(&self, tool: &Tool<'_>, rule: &Rule) -> Result<()> {
        let chain = Self::chain_of(rule.direction);
        let handles: Vec<u64> = Self::rules(tool)?
            .into_iter()
            .filter(|found| {
                found.table == self.table
                    && found.chain == chain
                    && found.listed.rule.as_ref() == Some(rule)
            })
            .map(|found| found.handle)
            .collect();
        if handles.is_empty() {
            return Err(Error::new(
                ErrorCode::NotFound,
                format!("no rule `{}` in the table `{}`", rule.key(), self.table),
            ));
        }
        for handle in handles {
            tool.ok(&Call::owned(
                "nft",
                vec![
                    "delete".to_owned(),
                    "rule".to_owned(),
                    "inet".to_owned(),
                    self.table.clone(),
                    chain.to_owned(),
                    "handle".to_owned(),
                    handle.to_string(),
                ],
            ))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULESET: &str = r#"{"nftables":[
      {"metainfo":{"version":"1.0.9"}},
      {"table":{"family":"inet","name":"nurt","handle":1}},
      {"chain":{"family":"inet","table":"nurt","name":"input","handle":1,"type":"filter","hook":"input","prio":0,"policy":"accept"}},
      {"chain":{"family":"inet","table":"filter","name":"output","handle":2,"type":"filter","hook":"output","prio":0,"policy":"accept"}},
      {"rule":{"family":"inet","table":"nurt","chain":"input","handle":4,"comment":"nurt:allow tcp 22 in any -","expr":[{"match":{"op":"==","left":{"payload":{"protocol":"tcp","field":"dport"}},"right":22}},{"accept":null}]}},
      {"rule":{"family":"inet","table":"filter","chain":"output","handle":7,"expr":[{"match":{"op":"==","left":{"payload":{"protocol":"ip","field":"daddr"}},"right":{"prefix":{"addr":"198.51.100.0","len":24}}}},{"match":{"op":"==","left":{"payload":{"protocol":"udp","field":"dport"}},"right":{"range":[5000,5010]}}},{"drop":null}]}},
      {"rule":{"family":"inet","table":"filter","chain":"output","handle":8,"expr":[{"counter":{"packets":0,"bytes":0}},{"accept":null}]}}
    ]}"#;

    #[test]
    fn parses_managed_and_foreign_rules() {
        let rules = parse_ruleset(RULESET).unwrap();
        assert_eq!(rules.len(), 3);
        assert!(rules[0].listed.managed);
        assert_eq!(rules[0].handle, 4);
        assert_eq!(
            rules[0].listed.rule.as_ref().unwrap().key(),
            "allow tcp 22 in any -"
        );
        let foreign = rules[1].listed.rule.as_ref().unwrap();
        assert_eq!(foreign.key(), "deny udp 5000-5010 out 198.51.100.0/24 -");
        assert!(!rules[1].listed.managed);
        assert_eq!(rules[2].listed.rule, None);
    }

    #[test]
    fn rejects_text_that_is_not_json() {
        assert_eq!(
            parse_ruleset("table inet nurt {").unwrap_err().code(),
            ErrorCode::StageFailed
        );
    }

    #[test]
    fn expression_carries_the_key_as_comment() {
        let rule = Rule::parse_key("deny udp 53 out 2001:db8::/32 -").unwrap();
        assert_eq!(
            Nftables::expression(&rule).join(" "),
            "ip6 daddr 2001:db8::/32 udp dport 53 drop comment \"nurt:deny udp 53 out 2001:db8::/32 -\""
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
