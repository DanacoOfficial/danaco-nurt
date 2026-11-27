// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! DNS servers and search domains: `network.dns_set`.
//!
//! systemd-resolved (`resolvectl`) is used when it runs; otherwise the file
//! `/etc/resolv.conf` is rewritten through the journal, so the journal
//! restores it. macOS uses `networksetup`, Windows the DnsClient cmdlets.
//! An empty list of `servers` returns the interface to automatic servers.

use std::net::IpAddr;

use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::{apply_ops, FileOp, MutationTx};
use nurt_contracts::kinds::Implementation;
use nurt_contracts::path::AbsPath;
use nurt_contracts::record::{Change, ChangeAction, EffectStatus};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, ExternalProgram,
    InputKinds, RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::Value;

use super::valid_interface;
use crate::admin::effects::{effect_record, marker, state_change, unchanged};
use crate::admin::{Call, Tool};

/// Prefix of the target of a DNS change.
pub const DNS_PREFIX: &str = "dns:";

const RESOLV_CONF: &str = "/etc/resolv.conf";

/// Servers and search domains of an interface.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DnsState {
    /// Name servers in order; empty for automatic.
    pub servers: Vec<String>,
    /// Search domains in order.
    pub search: Vec<String>,
}

impl DnsState {
    /// State text: `servers=a,b;search=x,y`.
    #[must_use]
    pub fn text(&self) -> String {
        format!(
            "servers={};search={}",
            self.servers.join(","),
            self.search.join(",")
        )
    }

    /// Program that restores the state on `interface`.
    #[must_use]
    pub fn program(&self, interface: &str) -> String {
        let list = |items: &[String]| {
            items
                .iter()
                .map(|item| format!("\"{item}\""))
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!(
            "network.dns_set [{}] search: [{}] interface: \"{interface}\"",
            list(&self.servers),
            list(&self.search)
        )
    }
}

/// State of the text written by [`DnsState::text`].
#[must_use]
pub fn parse_dns_text(text: &str) -> Option<DnsState> {
    let (servers, search) = text.split_once(";search=")?;
    let servers = servers.strip_prefix("servers=")?;
    let split = |list: &str| -> Vec<String> {
        list.split(',')
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect()
    };
    Some(DnsState {
        servers: split(servers),
        search: split(search),
    })
}

/// State in the content of a `resolv.conf`.
#[must_use]
pub fn parse_resolvconf(text: &str) -> DnsState {
    let mut state = DnsState::default();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("nameserver") => state.servers.extend(words.next().map(str::to_owned)),
            Some("search" | "domain") => state.search = words.map(str::to_owned).collect(),
            _ => {}
        }
    }
    state
}

fn resolvconf_text(old: &str, wanted: &DnsState) -> String {
    let mut text: String = old
        .lines()
        .filter(|line| {
            let first = line.split_whitespace().next();
            !matches!(first, Some("nameserver" | "search" | "domain"))
        })
        .map(|line| format!("{line}\n"))
        .collect();
    if !wanted.search.is_empty() {
        text.push_str(&format!("search {}\n", wanted.search.join(" ")));
    }
    for server in &wanted.servers {
        text.push_str(&format!("nameserver {server}\n"));
    }
    text
}

/// Mechanism that sets the DNS of the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsBackendKind {
    /// `resolvectl` of systemd-resolved.
    Resolved,
    /// The file `/etc/resolv.conf`.
    ResolvConf,
    /// `networksetup` of macOS.
    Networksetup,
    /// DnsClient cmdlets of Windows.
    Windows,
}

impl DnsBackendKind {
    /// Name used in the marker of the operation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Resolved => "resolved",
            Self::ResolvConf => "resolv.conf",
            Self::Networksetup => "networksetup",
            Self::Windows => "windows",
        }
    }
}

fn detect(tool: &Tool<'_>) -> DnsBackendKind {
    match tool.os() {
        Os::Macos => DnsBackendKind::Networksetup,
        Os::Windows => DnsBackendKind::Windows,
        Os::Linux => {
            if tool.available("resolvectl")
                && tool
                    .run(&Call::new("resolvectl", &["status", "--no-pager"]))
                    .is_ok_and(|output| output.success())
            {
                DnsBackendKind::Resolved
            } else {
                DnsBackendKind::ResolvConf
            }
        }
    }
}

fn default_interface(tool: &Tool<'_>) -> Result<String> {
    let output = tool.ok(&Call::new("ip", &["-j", "route", "show", "default"]))?;
    super::route::parse_ip_json(&output.stdout)?
        .into_iter()
        .find_map(|route| route.interface)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::MissingParam,
                "`network.dns_set` needs `interface`: the system has no default route",
            )
        })
}

fn after_colon(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.split_once(':').map(|(_, rest)| rest))
        .flat_map(str::split_whitespace)
        .map(str::to_owned)
        .collect()
}

fn lines_of(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && line.parse::<IpAddr>().is_ok() || line.contains('.') && !line.contains(' '))
        .map(str::to_owned)
        .collect()
}

fn ps_list(items: &[String]) -> String {
    format!(
        "@({})",
        items
            .iter()
            .map(|item| format!("'{item}'"))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn read_state(tool: &Tool<'_>, kind: DnsBackendKind, interface: &str) -> Result<DnsState> {
    match kind {
        DnsBackendKind::Resolved => {
            let servers = tool.ok(&Call::new("resolvectl", &["dns", interface]))?;
            let search = tool.ok(&Call::new("resolvectl", &["domain", interface]))?;
            Ok(DnsState {
                servers: after_colon(&servers.stdout),
                search: after_colon(&search.stdout),
            })
        }
        DnsBackendKind::ResolvConf => {
            let text = std::fs::read_to_string(RESOLV_CONF).unwrap_or_default();
            Ok(parse_resolvconf(&text))
        }
        DnsBackendKind::Networksetup => {
            let servers = tool.ok(&Call::new("networksetup", &["-getdnsservers", interface]))?;
            let search = tool.ok(&Call::new("networksetup", &["-getsearchdomains", interface]))?;
            let real = |text: &str| -> Vec<String> {
                if text.contains("aren't any") {
                    Vec::new()
                } else {
                    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()
                }
            };
            Ok(DnsState {
                servers: real(&servers.stdout),
                search: real(&search.stdout),
            })
        }
        DnsBackendKind::Windows => {
            let servers = tool.ok(&Call::powershell(&format!(
                "Get-DnsClientServerAddress -InterfaceAlias '{interface}' -AddressFamily IPv4 | Select-Object -ExpandProperty ServerAddresses"
            )))?;
            let search = tool.ok(&Call::powershell(
                "(Get-DnsClientGlobalSetting).SuffixSearchList",
            ))?;
            Ok(DnsState {
                servers: lines_of(&servers.stdout),
                search: search
                    .stdout
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned)
                    .collect(),
            })
        }
    }
}

/// Calls that bring the interface to `wanted`; `search` of `None` leaves the
/// search domains alone.
#[must_use]
pub fn set_calls(
    kind: DnsBackendKind,
    interface: &str,
    servers: &[String],
    search: Option<&[String]>,
) -> Vec<Call> {
    let mut calls = Vec::new();
    match kind {
        DnsBackendKind::Resolved => {
            if servers.is_empty() {
                calls.push(Call::new("resolvectl", &["revert", interface]));
            } else {
                let mut args = vec!["dns".to_owned(), interface.to_owned()];
                args.extend(servers.iter().cloned());
                calls.push(Call::owned("resolvectl", args));
            }
            if let Some(search) = search {
                let mut args = vec!["domain".to_owned(), interface.to_owned()];
                args.extend(search.iter().cloned());
                calls.push(Call::owned("resolvectl", args));
            }
        }
        DnsBackendKind::Networksetup => {
            let mut args = vec!["-setdnsservers".to_owned(), interface.to_owned()];
            if servers.is_empty() {
                args.push("Empty".to_owned());
            } else {
                args.extend(servers.iter().cloned());
            }
            calls.push(Call::owned("networksetup", args));
            if let Some(search) = search {
                let mut args = vec!["-setsearchdomains".to_owned(), interface.to_owned()];
                if search.is_empty() {
                    args.push("Empty".to_owned());
                } else {
                    args.extend(search.iter().cloned());
                }
                calls.push(Call::owned("networksetup", args));
            }
        }
        DnsBackendKind::Windows => {
            let script = if servers.is_empty() {
                format!("Set-DnsClientServerAddress -InterfaceAlias '{interface}' -ResetServerAddresses")
            } else {
                format!(
                    "Set-DnsClientServerAddress -InterfaceAlias '{interface}' -ServerAddresses {}",
                    ps_list(servers)
                )
            };
            calls.push(Call::powershell(&script));
            if let Some(search) = search {
                calls.push(Call::powershell(&format!(
                    "Set-DnsClientGlobalSetting -SuffixSearchList {}",
                    ps_list(search)
                )));
            }
        }
        DnsBackendKind::ResolvConf => {}
    }
    calls
}

fn texts_of(args: &BoundArgs, name: &str) -> Result<Option<Vec<String>>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::List(items)) => items
            .iter()
            .map(|item| match item {
                Value::Text(text) => Ok(text.clone()),
                other => Err(Error::new(
                    ErrorCode::Type,
                    format!("`network.dns_set` expects text items in `{name}`, not {}", other.type_name()),
                )),
            })
            .collect::<Result<Vec<_>>>()
            .map(Some),
        Some(Value::Text(text)) => Ok(Some(vec![text.clone()])),
        Some(_) => args.text(name).map(|_| None),
    }
}

fn valid_domain(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 253
        && !text.starts_with('-')
        && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

struct Request {
    servers: Vec<String>,
    search: Option<Vec<String>>,
    interface: Option<String>,
}

fn request(args: &BoundArgs) -> Result<Request> {
    let servers = texts_of(args, "servers")?.ok_or_else(|| {
        Error::new(
            ErrorCode::MissingParam,
            "`network.dns_set` needs `servers`; an empty list returns to automatic servers",
        )
    })?;
    for server in &servers {
        if server.parse::<IpAddr>().is_err() {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!("`{server}` is not an IP address of a name server"),
            )
            .with_found(server.clone()));
        }
    }
    let search = texts_of(args, "search")?;
    for domain in search.iter().flatten() {
        if !valid_domain(domain) {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!("`{domain}` is not a search domain"),
            )
            .with_found(domain.clone()));
        }
    }
    let interface = match args.text("interface")? {
        None => None,
        Some(text) if valid_interface(text) => Some(text.to_owned()),
        Some(text) => {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!("`{text}` is not an interface name"),
            )
            .with_found(text));
        }
    };
    Ok(Request {
        servers,
        search,
        interface,
    })
}

/// The command `network.dns_set`.
#[derive(Debug, Clone, Copy)]
pub struct DnsSet;

struct State {
    kind: DnsBackendKind,
    interface: String,
    servers: Vec<String>,
    search: Option<Vec<String>>,
    content: Option<String>,
}

impl StageImpl for DnsSet {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "network.dns_set".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        request(args)?;
        let mut info = StaticInfo::default();
        let program = match ctx.host().info().os {
            Os::Linux => "resolvectl",
            Os::Macos => "networksetup",
            Os::Windows => "powershell",
        };
        info.programs.push(ExternalProgram {
            program: program.to_owned(),
            implementation: Implementation::SystemProgram,
        });
        Ok(info)
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for DnsSet {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let wanted = request(args)?;
        let tool = Tool::from_ctx(ctx)?;
        let kind = detect(&tool);
        let interface = match (&wanted.interface, kind) {
            (Some(name), _) => name.clone(),
            (None, DnsBackendKind::ResolvConf) => String::new(),
            (None, DnsBackendKind::Resolved) => default_interface(&tool)?,
            (None, _) => {
                return Err(Error::new(
                    ErrorCode::MissingParam,
                    "`network.dns_set` needs `interface` on this system",
                ));
            }
        };
        let before = read_state(&tool, kind, &interface)?;
        let after = DnsState {
            servers: wanted.servers.clone(),
            search: wanted.search.clone().unwrap_or_else(|| before.search.clone()),
        };
        let mut plan = EffectPlan::new();
        let target = if kind == DnsBackendKind::ResolvConf {
            RESOLV_CONF.to_owned()
        } else {
            format!("{DNS_PREFIX}{interface}")
        };
        if before == after {
            plan.notable.push(unchanged(&target, "the servers and search domains are already set"));
            return Ok(plan.with_state(State {
                kind,
                interface,
                servers: wanted.servers,
                search: wanted.search,
                content: None,
            }));
        }
        let mut content = None;
        if kind == DnsBackendKind::ResolvConf {
            if let Ok(link) = std::fs::read_link(RESOLV_CONF) {
                return Err(Error::new(
                    ErrorCode::Unsupported,
                    format!(
                        "`{RESOLV_CONF}` is a link to `{}` that another service manages; enable systemd-resolved or set the DNS in that service",
                        link.display()
                    ),
                ));
            }
            let old = std::fs::read_to_string(RESOLV_CONF).unwrap_or_default();
            content = Some(resolvconf_text(&old, &after));
            plan.changes.push(Change::new(RESOLV_CONF.to_owned(), ChangeAction::Modify));
        } else {
            plan.changes.push(state_change(&target, Some(&before.text()), Some(&after.text())));
            plan.irreversible.push(marker(&format!("dns:{}", kind.as_str()), &target));
        }
        plan.preview.push(effect_record(
            "network.dns_set",
            &target,
            EffectStatus::Planned,
            Some(&before.text()),
            Some(&after.text()),
        ));
        Ok(plan.with_state(State {
            kind,
            interface,
            servers: wanted.servers,
            search: wanted.search,
            content,
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
            .ok_or_else(|| Error::new(ErrorCode::StageFailed, "the DNS change has no plan"))?;
        let mut outcome = EffectOutcome::new(StageOutput::none());
        if plan.changes.is_empty() {
            return Ok(outcome);
        }
        let tool = Tool::from_ctx(ctx)?;
        let before = read_state(&tool, state.kind, &state.interface)?;
        let after = DnsState {
            servers: state.servers.clone(),
            search: state.search.clone().unwrap_or_else(|| before.search.clone()),
        };
        let target;
        if let Some(content) = &state.content {
            target = RESOLV_CONF.to_owned();
            let path = AbsPath::new(RESOLV_CONF)
                .map_err(|error| Error::new(ErrorCode::StageFailed, error.to_string()))?;
            apply_ops(tx, vec![FileOp::write(path, content.clone().into_bytes())])?;
            outcome
                .changes
                .push(Change::new(RESOLV_CONF.to_owned(), ChangeAction::Modify));
        } else {
            target = format!("{DNS_PREFIX}{}", state.interface);
            let entry = marker(&format!("dns:{}", state.kind.as_str()), &target);
            tx.mark_irreversible(entry.clone());
            let calls = set_calls(state.kind, &state.interface, &state.servers, state.search.as_deref());
            for call in &calls {
                if let Err(error) = ctx.checkpoint().and_then(|()| tool.ok(call)) {
                    for restore in set_calls(state.kind, &state.interface, &before.servers, Some(&before.search)) {
                        let _ = tool.run(&restore);
                    }
                    return Err(error.with_effect_applied(false));
                }
            }
            outcome.changes.push(state_change(&target, Some(&before.text()), Some(&after.text())));
            outcome.irreversible.push(entry);
        }
        outcome.output = StageOutput::records(RecordStream::from_records(vec![effect_record(
            "network.dns_set",
            &target,
            EffectStatus::Ok,
            Some(&before.text()),
            Some(&after.text()),
        )]));
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::testing::{args, ctx, input, records, text, texts, FakeProcess, RecordingTx};

    #[test]
    fn state_text_round_trips() {
        let state = DnsState {
            servers: vec!["1.1.1.1".to_owned(), "8.8.8.8".to_owned()],
            search: vec!["example.org".to_owned()],
        };
        assert_eq!(state.text(), "servers=1.1.1.1,8.8.8.8;search=example.org");
        assert_eq!(parse_dns_text(&state.text()), Some(state.clone()));
        assert_eq!(
            state.program("eth0"),
            "network.dns_set [\"1.1.1.1\", \"8.8.8.8\"] search: [\"example.org\"] interface: \"eth0\""
        );
        assert_eq!(parse_dns_text("servers=;search="), Some(DnsState::default()));
    }

    #[test]
    fn resolvconf_is_read_and_rewritten_keeping_other_lines() {
        let old = "# managed\noptions edns0\nsearch a.org\nnameserver 10.0.0.1\nnameserver 10.0.0.2\n";
        let state = parse_resolvconf(old);
        assert_eq!(state.servers, ["10.0.0.1", "10.0.0.2"]);
        assert_eq!(state.search, ["a.org"]);
        let wanted = DnsState {
            servers: vec!["9.9.9.9".to_owned()],
            search: vec!["b.org".to_owned(), "c.org".to_owned()],
        };
        assert_eq!(
            resolvconf_text(old, &wanted),
            "# managed\noptions edns0\nsearch b.org c.org\nnameserver 9.9.9.9\n"
        );
    }

    #[test]
    fn calls_per_backend() {
        let servers = vec!["1.1.1.1".to_owned(), "8.8.8.8".to_owned()];
        let search = vec!["example.org".to_owned()];
        let lines = |kind| -> Vec<String> {
            set_calls(kind, "eth0", &servers, Some(&search))
                .iter()
                .map(Call::line)
                .collect()
        };
        assert_eq!(
            lines(DnsBackendKind::Resolved),
            ["resolvectl dns eth0 1.1.1.1 8.8.8.8", "resolvectl domain eth0 example.org"]
        );
        assert_eq!(
            lines(DnsBackendKind::Networksetup),
            ["networksetup -setdnsservers eth0 1.1.1.1 8.8.8.8", "networksetup -setsearchdomains eth0 example.org"]
        );
        assert_eq!(
            set_calls(DnsBackendKind::Resolved, "eth0", &[], None)[0].line(),
            "resolvectl revert eth0"
        );
        assert!(set_calls(DnsBackendKind::Windows, "Ethernet", &[], None)[0]
            .line()
            .ends_with("-ResetServerAddresses"));
    }

    fn resolved() -> std::sync::Arc<FakeProcess> {
        let process = FakeProcess::new();
        process.reply("resolvectl dns eth0", "Link 2 (eth0): 10.0.0.1 10.0.0.2\n");
        process.reply("resolvectl domain eth0", "Link 2 (eth0): corp.example\n");
        process
    }

    #[test]
    fn resolved_change_records_the_previous_state() {
        let process = resolved();
        let ctx = ctx(Os::Linux, &process);
        let request = args(
            "network.dns_set",
            &[("servers", texts(&["1.1.1.1"])), ("interface", text("eth0"))],
        );
        let plan = DnsSet.plan(&request, input(), &ctx).unwrap();
        assert_eq!(
            plan.changes[0].before.as_deref(),
            Some("servers=10.0.0.1,10.0.0.2;search=corp.example")
        );
        assert_eq!(
            plan.changes[0].after.as_deref(),
            Some("servers=1.1.1.1;search=corp.example")
        );
        let mut tx = RecordingTx::default();
        let outcome = DnsSet.apply(&request, plan, &mut tx, &ctx).unwrap();
        assert_eq!(tx.markers.len(), 1);
        assert!(process.lines().contains(&"resolvectl dns eth0 1.1.1.1".to_owned()));
        assert_eq!(records(outcome.output).len(), 1);
    }

    #[test]
    fn same_servers_change_nothing() {
        let process = resolved();
        let ctx = ctx(Os::Linux, &process);
        let request = args(
            "network.dns_set",
            &[("servers", texts(&["10.0.0.1", "10.0.0.2"])), ("interface", text("eth0"))],
        );
        let plan = DnsSet.plan(&request, input(), &ctx).unwrap();
        assert!(plan.changes.is_empty());
        assert_eq!(plan.notable.len(), 1);
    }

    #[test]
    fn a_failing_second_call_restores_the_first() {
        let process = resolved();
        process.fail("resolvectl domain eth0 x.org", 1, "Failed to set domain");
        let ctx = ctx(Os::Linux, &process);
        let request = args(
            "network.dns_set",
            &[
                ("servers", texts(&["1.1.1.1"])),
                ("search", texts(&["x.org"])),
                ("interface", text("eth0")),
            ],
        );
        let plan = DnsSet.plan(&request, input(), &ctx).unwrap();
        let mut tx = RecordingTx::default();
        assert!(DnsSet.apply(&request, plan, &mut tx, &ctx).is_err());
        let lines = process.lines();
        assert_eq!(lines.last().unwrap(), "resolvectl domain eth0 corp.example");
    }

    #[test]
    fn rejects_values_that_are_not_addresses_or_domains() {
        let process = FakeProcess::new();
        let ctx = ctx(Os::Linux, &process);
        for request in [
            args("network.dns_set", &[("servers", texts(&["dns.example"]))]),
            args("network.dns_set", &[("servers", texts(&["1.1.1.1"])), ("search", texts(&["a b"]))]),
            args("network.dns_set", &[("search", texts(&["a.org"]))]),
        ] {
            assert!(DnsSet.plan(&request, input(), &ctx).is_err());
        }
        assert!(process.lines().is_empty());
    }
}
