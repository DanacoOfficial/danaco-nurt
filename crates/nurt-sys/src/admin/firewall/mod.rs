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

pub use self::backend::{Backend, Listed, NAMES, select};
pub use self::model::{
    Direction, LABEL_PREFIX, Protocol, Rule, TARGET_PREFIX, Verdict, label_of, parse_ports,
    valid_source, valid_zone,
};
pub use self::nftables::DEFAULT_TABLE;
pub use self::nftables::{Nftables, parse_ruleset};

use super::Tool;
use super::effects::{effect_record, marker, state_change, unchanged};

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
        None | Some("inbound" | "in") => Ok(Direction::In),
        Some("outbound" | "out") => Ok(Direction::Out),
        Some(other) => Err(invalid(PARAM_DIRECTION, other, "inbound or outbound")),
    }
}

fn source_of(args: &BoundArgs) -> Result<Option<String>> {
    match args.text(PARAM_SOURCE)? {
        None => Ok(None),
        Some(text) if valid_source(text) => Ok(Some(text.to_owned())),
        Some(text) => Err(invalid(
            PARAM_SOURCE,
            text,
            "an IP address or a network in CIDR form",
        )),
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
        let protocol = word(args, PARAM_PROTOCOL)?
            .map(|_| protocol_of(args))
            .transpose()?;
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
enum Verb {
    Allow,
    Deny,
    Remove,
}

/// The commands `firewall.allow`, `firewall.deny` and `firewall.remove`.
#[derive(Debug, Clone, Copy)]
pub struct FirewallChange {
    change: Verb,
}

impl FirewallChange {
    /// The command `firewall.allow`.
    pub const ALLOW: Self = Self {
        change: Verb::Allow,
    };
    /// The command `firewall.deny`.
    pub const DENY: Self = Self { change: Verb::Deny };
    /// The command `firewall.remove`.
    pub const REMOVE: Self = Self {
        change: Verb::Remove,
    };

    fn command(self) -> &'static str {
        match self.change {
            Verb::Allow => "firewall.allow",
            Verb::Deny => "firewall.deny",
            Verb::Remove => "firewall.remove",
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
            (Verb::Allow, _) => (Verdict::Allow, false),
            (Verb::Deny, _) => (Verdict::Deny, false),
            (Verb::Remove, None | Some("any")) => (Verdict::Allow, true),
            (Verb::Remove, Some("allow")) => (Verdict::Allow, false),
            (Verb::Remove, Some("deny")) => (Verdict::Deny, false),
            (Verb::Remove, Some(other)) => {
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

/// Kind of the marker of a firewall change: `firewall:<backend>:<table>`; the
/// inverse of the operation reads the backend and the table from it.
fn marker_kind(backend: &str, table: &str) -> String {
    format!("firewall:{backend}:{table}")
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
        if self.change == Verb::Remove {
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
            plan.irreversible.push(marker(
                &marker_kind(backend.name(), &request.table),
                &step.rule.target(),
            ));
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
            let entry = marker(&marker_kind(backend.name(), &state.table), &target);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::System;
    use crate::admin::testing::{FakeProcess, RecordingTx, args, ctx, input, records, text, word};

    const EMPTY_NFT: &str = r#"{"nftables":[{"metainfo":{"version":"1.0.9"}}]}"#;

    fn allow_22() -> BoundArgs {
        args(
            "firewall.allow",
            &[
                ("port", Value::Int(22)),
                ("backend", word("nftables")),
                ("table", text("nurt_test_a")),
            ],
        )
    }

    #[test]
    fn allow_adds_a_rule_with_its_key_as_comment() {
        let process = FakeProcess::new();
        process.reply("nft -j list ruleset", EMPTY_NFT);
        process.fail("nft list chain", 1, "Error: No such file or directory");
        let ctx = ctx(System::Linux, &process);
        let request = allow_22();
        let plan = FirewallChange::ALLOW.plan(&request, input(), &ctx).unwrap();
        assert_eq!(plan.changes.len(), 1);
        assert_eq!(
            plan.changes[0].path.as_str(),
            "firewall:allow tcp 22 in any -"
        );
        assert_eq!(plan.changes[0].before.as_deref(), Some("absent"));
        let mut tx = RecordingTx::default();
        let outcome = FirewallChange::ALLOW
            .apply(&request, plan, &mut tx, &ctx)
            .unwrap();
        let lines = process.lines();
        assert!(lines.contains(&"nft add table inet nurt_test_a".to_owned()));
        assert_eq!(
            lines.last().unwrap(),
            "nft add rule inet nurt_test_a input tcp dport 22 accept comment \"nurt:allow tcp 22 in any -\""
        );
        assert_eq!(tx.markers.len(), 1);
        assert_eq!(
            tx.markers[0].detail.as_deref(),
            Some("admin:firewall:nftables:nurt_test_a")
        );
        assert_eq!(records(outcome.output).len(), 1);
    }

    #[test]
    fn allow_of_an_existing_rule_changes_nothing() {
        let process = FakeProcess::new();
        process.reply(
            "nft -j list ruleset",
            r#"{"nftables":[{"chain":{"family":"inet","table":"nurt_test_a","name":"input","handle":1,"type":"filter","hook":"input","prio":0,"policy":"accept"}},{"rule":{"family":"inet","table":"nurt_test_a","chain":"input","handle":2,"comment":"nurt:allow tcp 22 in any -","expr":[]}}]}"#,
        );
        let ctx = ctx(System::Linux, &process);
        let plan = FirewallChange::ALLOW
            .plan(&allow_22(), input(), &ctx)
            .unwrap();
        assert!(plan.changes.is_empty());
        assert_eq!(plan.notable.len(), 1);
        let mut tx = RecordingTx::default();
        FirewallChange::ALLOW
            .apply(&allow_22(), plan, &mut tx, &ctx)
            .unwrap();
        assert!(tx.markers.is_empty());
        assert_eq!(process.lines().len(), 1);
    }

    #[test]
    fn remove_deletes_by_handle_and_records_the_removed_rule() {
        let process = FakeProcess::new();
        process.reply(
            "nft -j list ruleset",
            r#"{"nftables":[{"chain":{"family":"inet","table":"nurt_test_a","name":"input","handle":1,"type":"filter","hook":"input","prio":0,"policy":"accept"}},{"rule":{"family":"inet","table":"nurt_test_a","chain":"input","handle":7,"comment":"nurt:deny tcp 22 in any -","expr":[]}}]}"#,
        );
        let ctx = ctx(System::Linux, &process);
        let request = args(
            "firewall.remove",
            &[
                ("port", Value::Int(22)),
                ("backend", word("nftables")),
                ("table", text("nurt_test_a")),
            ],
        );
        let plan = FirewallChange::REMOVE
            .plan(&request, input(), &ctx)
            .unwrap();
        assert_eq!(
            plan.changes[0].path.as_str(),
            "firewall:deny tcp 22 in any -"
        );
        assert_eq!(plan.changes[0].before.as_deref(), Some("present"));
        let mut tx = RecordingTx::default();
        FirewallChange::REMOVE
            .apply(&request, plan, &mut tx, &ctx)
            .unwrap();
        assert_eq!(
            process.lines().last().unwrap(),
            "nft delete rule inet nurt_test_a input handle 7"
        );
    }

    #[test]
    fn remove_of_a_missing_rule_is_not_found() {
        let process = FakeProcess::new();
        process.reply("nft -j list ruleset", EMPTY_NFT);
        let ctx = ctx(System::Linux, &process);
        let request = args(
            "firewall.remove",
            &[("port", Value::Int(22)), ("backend", word("nftables"))],
        );
        let error = FirewallChange::REMOVE
            .plan(&request, input(), &ctx)
            .err()
            .unwrap();
        assert_eq!(error.code(), ErrorCode::NotFound);
    }

    #[test]
    fn invalid_values_are_rejected_before_any_program_runs() {
        let process = FakeProcess::new();
        let ctx = ctx(System::Linux, &process);
        for request in [
            args("firewall.allow", &[("port", Value::Int(70000))]),
            args("firewall.allow", &[("port", text("80; reboot"))]),
            args(
                "firewall.allow",
                &[("port", Value::Int(80)), ("source", text("not-an-address"))],
            ),
            args(
                "firewall.allow",
                &[("port", Value::Int(80)), ("zone", text("a b"))],
            ),
            args(
                "firewall.allow",
                &[("port", Value::Int(80)), ("protocol", word("icmp"))],
            ),
            args("firewall.allow", &[("source", text("10.0.0.1"))]),
        ] {
            assert!(FirewallChange::ALLOW.plan(&request, input(), &ctx).is_err());
        }
        assert!(process.lines().is_empty());
    }

    #[test]
    fn auto_prefers_firewalld_then_ufw_then_nftables() {
        let process = FakeProcess::new();
        process.reply("firewall-cmd --state", "running\n");
        let chosen = |process: &std::sync::Arc<FakeProcess>| {
            let cancel = nurt_contracts::stage::CancelToken::new();
            let tool = Tool::new(process.as_ref(), System::Linux, &cancel);
            select(&tool, "auto", "nurt").map(|backend| backend.name())
        };
        assert_eq!(chosen(&process).unwrap(), "firewalld");
        let process = FakeProcess::new();
        process.without("firewall-cmd");
        assert_eq!(chosen(&process).unwrap(), "ufw");
        process.without("ufw");
        assert_eq!(chosen(&process).unwrap(), "nftables");
        process.without("nft");
        assert_eq!(
            chosen(&process).unwrap_err().code(),
            ErrorCode::ExternalMissing
        );
    }

    #[test]
    fn a_backend_of_another_system_is_os_only() {
        let process = FakeProcess::new();
        let cancel = nurt_contracts::stage::CancelToken::new();
        let tool = Tool::new(process.as_ref(), System::Linux, &cancel);
        assert_eq!(
            select(&tool, "pf", "nurt").err().unwrap().code(),
            ErrorCode::OsOnly
        );
        assert_eq!(
            select(&tool, "windows", "nurt").err().unwrap().code(),
            ErrorCode::OsOnly
        );
    }

    #[test]
    fn firewalld_normalizes_the_default_zone_and_adds_both_runtime_and_permanent() {
        let process = FakeProcess::new();
        process.reply("firewall-cmd --get-default-zone", "public\n");
        process.reply(
            "firewall-cmd --get-active-zones",
            "public\n  interfaces: eth0\n",
        );
        process.reply("firewall-cmd --zone=public --list-ports", "80/tcp\n");
        let ctx = ctx(System::Linux, &process);
        let request = args(
            "firewall.allow",
            &[("port", Value::Int(22)), ("backend", word("firewalld"))],
        );
        let plan = FirewallChange::ALLOW.plan(&request, input(), &ctx).unwrap();
        assert_eq!(
            plan.changes[0].path.as_str(),
            "firewall:allow tcp 22 in any public"
        );
        let mut tx = RecordingTx::default();
        FirewallChange::ALLOW
            .apply(&request, plan, &mut tx, &ctx)
            .unwrap();
        let lines = process.lines();
        assert!(lines.contains(&"firewall-cmd --zone=public --add-port=22/tcp".to_owned()));
        assert_eq!(
            lines.last().unwrap(),
            "firewall-cmd --permanent --zone=public --add-port=22/tcp"
        );
    }

    #[test]
    fn a_failure_of_the_permanent_change_removes_the_runtime_rule() {
        let process = FakeProcess::new();
        process.reply("firewall-cmd --get-default-zone", "public\n");
        process.reply("firewall-cmd --get-active-zones", "public\n");
        process.fail("firewall-cmd --permanent", 1, "Error: ALREADY_ENABLED");
        let ctx = ctx(System::Linux, &process);
        let request = args(
            "firewall.allow",
            &[("port", Value::Int(22)), ("backend", word("firewalld"))],
        );
        let plan = FirewallChange::ALLOW.plan(&request, input(), &ctx).unwrap();
        let mut tx = RecordingTx::default();
        assert!(
            FirewallChange::ALLOW
                .apply(&request, plan, &mut tx, &ctx)
                .is_err()
        );
        assert_eq!(
            process.lines().last().unwrap(),
            "firewall-cmd --zone=public --remove-port=22/tcp"
        );
    }

    #[test]
    fn ufw_adds_and_deletes_with_force() {
        let process = FakeProcess::new();
        process.reply(
            "ufw status",
            "Status: active\n\nTo Action From\n-- ------ ----\n",
        );
        let ctx = ctx(System::Linux, &process);
        let request = args(
            "firewall.deny",
            &[
                ("port", text("8000-8100")),
                ("protocol", word("udp")),
                ("source", text("10.0.0.0/8")),
                ("backend", word("ufw")),
            ],
        );
        let plan = FirewallChange::DENY.plan(&request, input(), &ctx).unwrap();
        let mut tx = RecordingTx::default();
        FirewallChange::DENY
            .apply(&request, plan, &mut tx, &ctx)
            .unwrap();
        assert_eq!(
            process.lines().last().unwrap(),
            "ufw --force deny in proto udp from 10.0.0.0/8 to any port 8000:8100"
        );
    }

    #[test]
    fn pf_loads_the_whole_anchor_through_standard_input() {
        let process = FakeProcess::new();
        process.reply(
            "pfctl -a nurt -sr",
            "pass in quick proto tcp from any to any port = 80 label \"nurt:allow tcp 80 in any -\"\n",
        );
        let ctx = ctx(System::Macos, &process);
        let request = args(
            "firewall.allow",
            &[("port", Value::Int(22)), ("backend", word("pf"))],
        );
        let plan = FirewallChange::ALLOW.plan(&request, input(), &ctx).unwrap();
        let mut tx = RecordingTx::default();
        FirewallChange::ALLOW
            .apply(&request, plan, &mut tx, &ctx)
            .unwrap();
        let loaded = process.stdin();
        assert_eq!(loaded.len(), 1);
        assert!(loaded[0].contains("label \"nurt:allow tcp 80 in any -\""));
        assert!(loaded[0].contains("label \"nurt:allow tcp 22 in any -\""));
    }

    #[test]
    fn windows_rules_are_created_and_removed_by_label() {
        let process = FakeProcess::new();
        process.reply(
            "powershell -NoProfile -NonInteractive -Command Get-NetFirewallRule",
            "",
        );
        let ctx = ctx(System::Windows, &process);
        let request = args("firewall.allow", &[("port", Value::Int(3389))]);
        let plan = FirewallChange::ALLOW.plan(&request, input(), &ctx).unwrap();
        let mut tx = RecordingTx::default();
        FirewallChange::ALLOW
            .apply(&request, plan, &mut tx, &ctx)
            .unwrap();
        assert!(
            process
                .lines()
                .last()
                .unwrap()
                .contains("New-NetFirewallRule -DisplayName 'nurt:allow tcp 3389 in any -'")
        );
    }

    #[test]
    fn rules_lists_records_with_the_fields_of_the_rule() {
        let process = FakeProcess::new();
        process.reply(
            "ufw status",
            "Status: active\n\nTo Action From\n-- ------ ----\n22/tcp ALLOW IN Anywhere\n",
        );
        let ctx = ctx(System::Linux, &process);
        let request = args(
            "firewall.rules",
            &[("backend", word("ufw")), ("port", Value::Int(22))],
        );
        let rows = records(FirewallRules.execute(&request, input(), &ctx).unwrap());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("action"), Some(&text("allow")));
        assert_eq!(rows[0].get("backend"), Some(&text("ufw")));
        let none = args(
            "firewall.rules",
            &[("backend", word("ufw")), ("port", Value::Int(80))],
        );
        assert!(records(FirewallRules.execute(&none, input(), &ctx).unwrap()).is_empty());
    }
}
