// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! pf backend (macOS): the rules of Nurt live in the anchor `nurt`.
//!
//! The anchor holds only rules of Nurt. A change loads the complete new
//! anchor with `pfctl -a nurt -f -`; the label of every rule holds its key.
//! The anchor takes effect when the main ruleset refers to it with
//! `anchor "nurt"`.

use nurt_contracts::error::{Error, ErrorCode, Result};

use super::backend::{Backend, Listed};
use super::model::{Direction, LABEL_PREFIX, Rule, Verdict, label_of};
use crate::admin::{Call, Tool};

/// Name of the anchor.
pub(crate) const ANCHOR: &str = "nurt";

/// The pf firewall.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pf;

/// Line of the anchor for `rule`.
#[must_use]
pub(crate) fn anchor_line(rule: &Rule) -> String {
    let action = match rule.verdict {
        Verdict::Allow => "pass",
        Verdict::Deny => "block",
    };
    let remote = rule.source.as_deref().unwrap_or("any");
    let (from, to) = match rule.direction {
        Direction::In => (remote, "any"),
        Direction::Out => ("any", remote),
    };
    let ports = if rule.from == rule.to {
        rule.from.to_string()
    } else {
        format!("{}:{}", rule.from, rule.to)
    };
    format!(
        "{action} {} quick proto {} from {from} to {to} port {ports} label \"{}\"",
        rule.direction.as_str(),
        rule.protocol.as_str(),
        label_of(rule)
    )
}

/// Rules of Nurt in the output of `pfctl -a nurt -sr`.
#[must_use]
pub(crate) fn parse_anchor(text: &str) -> Vec<Listed> {
    text.lines()
        .filter_map(|line| {
            let start = line.find(&format!("\"{LABEL_PREFIX}"))? + 1;
            let end = line[start..].find('"')? + start;
            Some(Listed {
                rule: Rule::parse_key(&line[start..end]),
                raw: line.trim().to_owned(),
                place: ANCHOR.to_owned(),
                managed: true,
            })
        })
        .collect()
}

impl Pf {
    fn current(tool: &Tool<'_>) -> Result<Vec<Rule>> {
        let output = tool.ok(&Call::new("pfctl", &["-a", ANCHOR, "-sr"]))?;
        Ok(parse_anchor(&output.stdout)
            .into_iter()
            .filter_map(|listed| listed.rule)
            .collect())
    }

    fn load(tool: &Tool<'_>, rules: &[Rule]) -> Result<()> {
        let mut text = rules.iter().map(anchor_line).collect::<Vec<_>>().join("\n");
        text.push('\n');
        tool.ok(&Call::new("pfctl", &["-a", ANCHOR, "-f", "-"]).with_stdin(text))?;
        Ok(())
    }
}

impl Backend for Pf {
    fn name(&self) -> &'static str {
        "pf"
    }

    fn normalize(&self, _tool: &Tool<'_>, rule: &mut Rule) -> Result<()> {
        if rule.zone.is_some() {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "pf has no zones; leave `zone` out",
            ));
        }
        Ok(())
    }

    fn list(&self, tool: &Tool<'_>) -> Result<Vec<Listed>> {
        let output = tool.ok(&Call::new("pfctl", &["-a", ANCHOR, "-sr"]))?;
        Ok(parse_anchor(&output.stdout))
    }

    fn add(&self, tool: &Tool<'_>, rule: &Rule) -> Result<()> {
        let mut rules = Self::current(tool)?;
        rules.push(rule.clone());
        Self::load(tool, &rules)
    }

    fn remove(&self, tool: &Tool<'_>, rule: &Rule) -> Result<()> {
        let mut rules = Self::current(tool)?;
        let before = rules.len();
        rules.retain(|existing| existing != rule);
        if rules.len() == before {
            return Err(Error::new(
                ErrorCode::NotFound,
                format!("no rule `{}` in the anchor `{ANCHOR}`", rule.key()),
            ));
        }
        Self::load(tool, &rules)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_and_listing_agree() {
        let rule = Rule::parse_key("deny tcp 8000-8100 out 10.0.0.0/8 -").unwrap();
        let line = anchor_line(&rule);
        assert_eq!(
            line,
            "block out quick proto tcp from any to 10.0.0.0/8 port 8000:8100 label \"nurt:deny tcp 8000-8100 out 10.0.0.0/8 -\""
        );
        let listed = parse_anchor(&format!("{line}\npass in all\n"));
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].rule, Some(rule));
    }
}
