// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The commands `firewall.rules`, `firewall.allow`, `firewall.deny` and
//! `firewall.remove`.
//!
//! The rules of the system are read and changed through a backend: nftables,
//! firewalld or ufw on Linux, pf on macOS and the Windows Defender Firewall.
//! `backend: auto` picks the first one that is installed and running. A
//! rule is described by `port`, `protocol`, `source`, `direction` and `zone`;
//! `source` is the remote address of the traffic: the sender of incoming and
//! the receiver of outgoing traffic.
//!
//! The effect of `allow`, `deny` and `remove` is a change of the target
//! `firewall:<rule>` from `absent` to `present` or back. `undo` runs the
//! opposite command (see [`crate::admin::inverse`]).

mod backend;
mod firewalld;
mod model;
mod nftables;
mod pf;
mod ufw;
mod windows;

use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::kinds::Implementation;
use nurt_contracts::record::EffectStatus;
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, ExternalProgram,
    InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput,
    StaticInfo,
};
use nurt_contracts::value::{Record, RecordKind, Value};

pub use self::backend::{select, Backend, Listed, NAMES};
pub use self::model::{
    label_of, parse_ports, valid_source, valid_zone, Direction, Protocol, Rule, Verdict,
    LABEL_PREFIX, TARGET_PREFIX,
};
pub use self::nftables::DEFAULT_TABLE;
pub use self::nftables::{parse_ruleset, Nftables};

use super::effects::{effect_record, marker, state_change, unchanged};
use super::Tool;

const PARAM_PORT: &str = "port";
const PARAM_PROTOCOL: &str = "protocol";
const PARAM_SOURCE: &str = "source";
const PARAM_DIRECTION: &str = "direction";
const PARAM_ZONE: &str = "zone";
const PARAM_BACKEND: &str = "backend";
const PARAM_TABLE: &str = "table";
const PARAM_ACTION: &str = "action";

const PRESENT: &str = "present";
const ABSENT: &str = "absent";

/// Every stage of the firewall commands, in catalog order.
#[must_use]
pub fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![
        Arc::new(FirewallRules),
        Arc::new(FirewallChange::ALLOW),
        Arc::new(FirewallChange::DENY),
        Arc::new(FirewallChange::REMOVE),
    ]
}

fn word<'a>(args: &'a BoundArgs, name: &str) -> Result<Option<&'a str>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Enum(text) | Value::Text(text)) => Ok(Some(text.as_str())),
        Some(_) => args.text(name),
    }
}

fn port_of(args: &BoundArgs) -> Result<Option<(u16, u16)>> {
    match args.value(PARAM_PORT) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Int(number)) => parse_ports(&number.to_string()).map(Some),
        Some(Value::Text(text)) => parse_ports(text).map(Some),
        Some(_) => args.text(PARAM_PORT).map(|_| None),
    }
}

fn invalid(name: &str, text: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("`{text}` is not valid for `{name}`: {expected}"),
    )
    .with_found(text)
}

fn protocol_of(args: &BoundArgs) -> Result<Protocol> {
    match word(args, PARAM_PROTOCOL)? {
        None | Some("tcp") => Ok(Protocol::Tcp),
        Some("udp") => Ok(Protocol::Udp),
        Some(other) => Err(invalid(PARAM_PROTOCOL, other, "tcp or udp")),
    }
}

fn direction_of(args: &BoundArgs) -> Result<Direction> {
    match word(args, PARAM_DIRECTION)? {
        None | Some("in") => Ok(Direction::In),
        Some("out") => Ok(Direction::Out),
        Some(other) => Err(invalid(PARAM_DIRECTION, other, "in or out")),
    }
}

fn source_of(args: &BoundArgs) -> Result<Option<String>> {
    match args.text(PARAM_SOURCE)? {
        None => Ok(None),
        Some(text) if valid_source(text) => Ok(Some(text.to_owned())),
        Some(text) => Err(invalid(PARAM_SOURCE, text, "an IP address or a network in CIDR form")),
    }
}

fn zone_of(args: &BoundArgs) -> Result<Option<String>> {
    match args.text(PARAM_ZONE)? {
        None => Ok(None),
        Some(text) if valid_zone(text) => Ok(Some(text.to_owned())),
        Some(text) => Err(invalid(PARAM_ZONE, text, "letters, digits, `_` and `-`")),
    }
}

fn backend_name(args: &BoundArgs) -> Result<String> {
    match word(args, PARAM_BACKEND)? {
        None => Ok("auto".to_owned()),
        Some(name) if NAMES.contains(&name) => Ok(name.to_owned()),
        Some(other) => Err(invalid(PARAM_BACKEND, other, &NAMES.join(", "))),
    }
}

fn table_of(args: &BoundArgs) -> Result<String> {
    match args.text(PARAM_TABLE)? {
        None => Ok(DEFAULT_TABLE.to_owned()),
        Some(text) if valid_zone(text) => Ok(text.to_owned()),
        Some(text) => Err(invalid(PARAM_TABLE, text, "letters, digits, `_` and `-`")),
    }
}

fn external_info(args: &BoundArgs) -> Result<StaticInfo> {
    let mut info = StaticInfo::default();
    let programs: &[&str] = match backend_name(args)?.as_str() {
        "nftables" => &["nft"],
        "firewalld" => &["firewall-cmd"],
        "ufw" => &["ufw"],
        "pf" => &["pfctl"],
        _ => &[],
    };
    for program in programs {
        info.programs.push(ExternalProgram {
            program: (*program).to_owned(),
            implementation: Implementation::SystemProgram,
        });
    }
    Ok(info)
}

/// The command `firewall.rules`: the rules of the firewall as records.
#[derive(Debug, Clone, Copy)]
pub struct FirewallRules;

impl StageImpl for FirewallRules {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "firewall.rules".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        port_of(args)?;
        protocol_of(args)?;
        table_of(args)?;
        external_info(args)
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

/// Record of a listed rule.
#[must_use]
pub fn rule_record(backend: &str, listed: &Listed) -> Record {
    let mut record = Record::new(RecordKind::Map)
        .with("backend", Value::Text(backend.to_owned()))
        .with("place", Value::Text(listed.place.clone()))
        .with("managed", Value::Bool(listed.managed));
    if let Some(rule) = &listed.rule {
        record = record
            .with("action", Value::Text(rule.verdict.as_str().to_owned()))
            .with("protocol", Value::Text(rule.protocol.as_str().to_owned()))
            .with("port", Value::Text(rule.port_text()))
            .with("direction", Value::Text(rule.direction.as_str().to_owned()));
        if let Some(source) = &rule.source {
            record = record.with("source", Value::Text(source.clone()));
        }
        if let Some(zone) = &rule.zone {
            record = record.with("zone", Value::Text(zone.clone()));
        }
    }
    record.with("rule", Value::Text(listed.raw.clone()))
}

impl ReadStage for FirewallRules {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let tool = Tool::from_ctx(ctx)?;
        let backend = select(&tool, &backend_name(args)?, &table_of(args)?)?;
        let ports = port_of(args)?;
        let protocol = word(args, PARAM_PROTOCOL)?.map(|_| protocol_of(args)).transpose()?;
        let records: Vec<Record> = backend
            .list(&tool)?
            .iter()
            .filter(|listed| match (&listed.rule, ports, protocol) {
                (Some(rule), Some(range), _) if (rule.from, rule.to) != range => false,
                (Some(rule), _, Some(wanted)) if rule.protocol != wanted => false,
                (None, Some(_), _) | (None, _, Some(_)) => false,
                _ => true,
            })
            .map(|listed| rule_record(backend.name(), listed))
            .collect();
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

/// Which of the three changing commands a [`FirewallChange`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    Allow,
    Deny,
    Remove,
}

/// The commands `firewall.allow`, `firewall.deny` and `firewall.remove`.
#[derive(Debug, Clone, Copy)]
pub struct FirewallChange {
    change: Change,
}

impl FirewallChange {
    /// The command `firewall.allow`.
    pub const ALLOW: Self = Self {
        change: Change::Allow,
    };
    /// The command `firewall.deny`.
    pub const DENY: Self = Self {
        change: Change::Deny,
    };
    /// The command `firewall.remove`.
    pub const REMOVE: Self = Self {
        change: Change::Remove,
    };

    fn command(self) -> &'static str {
        match self.change {
            Change::Allow => "firewall.allow",
            Change::Deny => "firewall.deny",
            Change::Remove => "firewall.remove",
        }
    }

    fn request(self, args: &BoundArgs) -> Result<Request> {
        let (from, to) = port_of(args)?.ok_or_else(|| {
            Error::new(
                ErrorCode::MissingParam,
                format!("`{}` needs a `port`", self.command()),
            )
        })?;
        let (verdict, any_verdict) = match (self.change, word(args, PARAM_ACTION)?) {
            (Change::Allow, _) => (Verdict::Allow, false),
            (Change::Deny, _) => (Verdict::Deny, false),
            (Change::Remove, None | Some("any")) => (Verdict::Allow, true),
            (Change::Remove, Some("allow")) => (Verdict::Allow, false),
            (Change::Remove, Some("deny")) => (Verdict::Deny, false),
            (Change::Remove, Some(other)) => {
                return Err(invalid(PARAM_ACTION, other, "any, allow or deny"));
            }
        };
        Ok(Request {
            rule: Rule {
                verdict,
                protocol: protocol_of(args)?,
                from,
                to,
                source: source_of(args)?,
                direction: direction_of(args)?,
                zone: zone_of(args)?,
            },
            any_verdict,
            backend: backend_name(args)?,
            table: table_of(args)?,
        })
    }
}

struct Request {
    rule: Rule,
    any_verdict: bool,
    backend: String,
    table: String,
}

/// One step of an applied change.
#[derive(Debug, Clone)]
struct Step {
    rule: Rule,
    add: bool,
}

struct State {
    backend: String,
    table: String,
    steps: Vec<Step>,
}

impl StageImpl for FirewallChange {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: self.command().to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        self.request(args)?;
        external_info(args)
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

fn effects_of(kind: &str, steps: &[Step], status: EffectStatus) -> Vec<Record> {
    steps
        .iter()
        .map(|step| {
            let (before, after) = if step.add {
                (ABSENT, PRESENT)
            } else {
                (PRESENT, ABSENT)
            };
            effect_record(kind, &step.rule.target(), status, Some(before), Some(after))
        })
        .collect()
}

impl EffectStage for FirewallChange {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let mut request = self.request(args)?;
        let tool = Tool::from_ctx(ctx)?;
        let backend = select(&tool, &request.backend, &request.table)?;
        backend.normalize(&tool, &mut request.rule)?;
        let listed = backend.list(&tool)?;
        let mut plan = EffectPlan::new();
        let mut steps = Vec::new();
        if self.change == Change::Remove {
            let mut found: Vec<Rule> = listed
                .iter()
                .filter(|item| backend.owns(item))
                .filter_map(|item| item.rule.clone())
                .filter(|rule| request.rule.selects(rule, request.any_verdict))
                .collect();
            found.sort();
            found.dedup();
            if found.is_empty() {
                return Err(Error::new(
                    ErrorCode::NotFound,
                    format!("no firewall rule matches `{}`", request.rule.key()),
                )
                .with_found(request.rule.key())
                .with_fix("firewall.rules"));
            }
            steps.extend(found.into_iter().map(|rule| Step { rule, add: false }));
        } else if listed
            .iter()
            .any(|item| backend.owns(item) && item.rule.as_ref() == Some(&request.rule))
        {
            plan.notable.push(unchanged(
                &request.rule.target(),
                "the rule is already in the firewall",
            ));
        } else {
            steps.push(Step {
                rule: request.rule.clone(),
                add: true,
            });
        }
        plan.bulk = steps.len() > 1;
        for step in &steps {
            let (before, after) = if step.add {
                (ABSENT, PRESENT)
            } else {
                (PRESENT, ABSENT)
            };
            plan.changes
                .push(state_change(&step.rule.target(), Some(before), Some(after)));
            plan.irreversible
                .push(marker(backend.name(), &step.rule.target()));
        }
        plan.preview = effects_of(self.command(), &steps, EffectStatus::Planned);
        Ok(plan.with_state(State {
            backend: backend.name().to_owned(),
            table: request.table,
            steps,
        }))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let state = plan
            .take_state::<State>()
            .ok_or_else(|| Error::new(ErrorCode::StageFailed, "the firewall change has no plan"))?;
        let tool = Tool::from_ctx(ctx)?;
        let backend = select(&tool, &state.backend, &state.table)?;
        let mut outcome = EffectOutcome::new(StageOutput::none());
        let mut done: Vec<Step> = Vec::new();
        for step in &state.steps {
            let target = step.rule.target();
            let entry = marker(backend.name(), &target);
            tx.mark_irreversible(entry.clone());
            let result = ctx.checkpoint().and_then(|()| {
                if step.add {
                    backend.add(&tool, &step.rule)
                } else {
                    backend.remove(&tool, &step.rule)
                }
            });
            if let Err(error) = result {
                for earlier in done.iter().rev() {
                    let _ = if earlier.add {
                        backend.remove(&tool, &earlier.rule)
                    } else {
                        backend.add(&tool, &earlier.rule)
                    };
                }
                return Err(error.with_effect_applied(false));
            }
            let (before, after) = if step.add {
                (ABSENT, PRESENT)
            } else {
                (PRESENT, ABSENT)
            };
            outcome
                .changes
                .push(state_change(&target, Some(before), Some(after)));
            outcome.irreversible.push(entry);
            done.push(step.clone());
        }
        outcome.output = StageOutput::records(RecordStream::from_records(effects_of(
            self.command(),
            &done,
            EffectStatus::Ok,
        )));
        Ok(outcome)
    }
}
