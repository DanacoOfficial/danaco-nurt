// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Routes: `network.route_add` and `network.route_remove`.

use crate::admin::System;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::kinds::Implementation;
use nurt_contracts::record::EffectStatus;
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, ExternalProgram,
    InputKinds, RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::Value;
use serde_json::Value as Json;

use super::valid_interface;
use crate::admin::effects::{effect_record, marker, state_change, unchanged};
use crate::admin::firewall::valid_source;
use crate::admin::{Call, Tool};

/// Prefix of the target of a route change.
pub const ROUTE_PREFIX: &str = "route:";

/// A route of the routing table.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Route {
    /// Destination network, a host address or `default`.
    pub destination: String,
    /// Next hop; `None` for a route on the link.
    pub gateway: Option<String>,
    /// Interface of the route.
    pub interface: Option<String>,
    /// Metric of the route.
    pub metric: Option<u32>,
}

impl Route {
    /// Target of the change in reports.
    #[must_use]
    pub fn target(&self) -> String {
        format!("{ROUTE_PREFIX}{}", self.destination)
    }

    /// State text of the route: `via=<gateway> dev=<interface> metric=<n>`
    /// with `-` for a missing part. The interface is written with `_` for
    /// spaces.
    #[must_use]
    pub fn state(&self) -> String {
        format!(
            "via={} dev={} metric={}",
            self.gateway.as_deref().unwrap_or("-"),
            self.interface
                .as_deref()
                .map_or_else(|| "-".to_owned(), |name| name.replace(' ', "\u{a0}")),
            self.metric
                .map_or_else(|| "-".to_owned(), |n| n.to_string()),
        )
    }

    /// Route of a destination and a state written by [`Route::state`].
    #[must_use]
    pub fn from_state(destination: &str, state: &str) -> Option<Self> {
        let mut words = state.split(' ');
        let gateway = words.next()?.strip_prefix("via=")?;
        let interface = words.next()?.strip_prefix("dev=")?;
        let metric = words.next()?.strip_prefix("metric=")?;
        words.next().is_none().then_some(())?;
        Some(Self {
            destination: destination.to_owned(),
            gateway: (gateway != "-").then(|| gateway.to_owned()),
            interface: (interface != "-").then(|| interface.replace('\u{a0}', " ")),
            metric: if metric == "-" {
                None
            } else {
                Some(metric.parse().ok()?)
            },
        })
    }

    /// Program that adds (`network.route_add`) or removes
    /// (`network.route_remove`) the route.
    #[must_use]
    pub fn program(&self, command: &str) -> String {
        let mut text = format!("{command} \"{}\"", self.destination);
        if let Some(gateway) = &self.gateway {
            text.push_str(&format!(" gateway: \"{gateway}\""));
        }
        if let Some(interface) = &self.interface {
            text.push_str(&format!(" interface: \"{interface}\""));
        }
        if let Some(metric) = self.metric {
            text.push_str(&format!(" metric: {metric}"));
        }
        text
    }

    fn is_v6(&self) -> bool {
        self.destination.contains(':')
    }

    /// Whether `self`, as a request, selects `route` of the table.
    #[must_use]
    pub fn selects(&self, route: &Self) -> bool {
        self.destination == route.destination
            && self
                .gateway
                .as_ref()
                .is_none_or(|gw| route.gateway.as_ref() == Some(gw))
            && self
                .interface
                .as_ref()
                .is_none_or(|name| route.interface.as_ref() == Some(name))
            && self
                .metric
                .is_none_or(|metric| route.metric == Some(metric))
    }
}

/// Routes in the JSON of `ip -j route show`.
///
/// # Errors
///
/// `E_STAGE_FAILED` when the text is not JSON.
pub fn parse_ip_json(text: &str) -> Result<Vec<Route>> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let document: Json = serde_json::from_str(text).map_err(|error| {
        Error::new(
            ErrorCode::StageFailed,
            format!("`ip -j` printed text that is not JSON: {error}"),
        )
    })?;
    let items = document.as_array().cloned().unwrap_or_default();
    Ok(items
        .iter()
        .filter_map(|item| {
            Some(Route {
                destination: item.get("dst")?.as_str()?.to_owned(),
                gateway: item
                    .get("gateway")
                    .and_then(Json::as_str)
                    .map(str::to_owned),
                interface: item.get("dev").and_then(Json::as_str).map(str::to_owned),
                metric: item
                    .get("metric")
                    .and_then(Json::as_u64)
                    .and_then(|metric| u32::try_from(metric).ok()),
            })
        })
        .collect())
}

/// Routes in the output of `netstat -rn` on macOS.
#[must_use]
pub fn parse_netstat(text: &str) -> Vec<Route> {
    text.lines()
        .filter_map(|line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            if words.len() < 4
                || !(words[0] == "default" || words[0].chars().next()?.is_ascii_digit())
            {
                return None;
            }
            let gateway = words[1]
                .parse::<std::net::IpAddr>()
                .ok()
                .map(|ip| ip.to_string());
            Some(Route {
                destination: words[0].to_owned(),
                gateway,
                interface: Some(words[3].to_owned()),
                metric: None,
            })
        })
        .collect()
}

/// Routes in the JSON of `Get-NetRoute` through `ConvertTo-Json`.
///
/// # Errors
///
/// `E_STAGE_FAILED` when the text is not JSON.
pub fn parse_windows(text: &str) -> Result<Vec<Route>> {
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
    Ok(items
        .iter()
        .filter_map(|item| {
            let prefix = item.get("DestinationPrefix")?.as_str()?;
            let destination = match prefix {
                "0.0.0.0/0" | "::/0" => "default",
                other => other,
            };
            let hop = item
                .get("NextHop")
                .and_then(Json::as_str)
                .unwrap_or_default();
            Some(Route {
                destination: destination.to_owned(),
                gateway: (!matches!(hop, "" | "0.0.0.0" | "::")).then(|| hop.to_owned()),
                interface: item
                    .get("InterfaceAlias")
                    .and_then(Json::as_str)
                    .map(str::to_owned),
                metric: item
                    .get("RouteMetric")
                    .and_then(Json::as_u64)
                    .and_then(|metric| u32::try_from(metric).ok()),
            })
        })
        .collect())
}

fn quote_ps(text: &str) -> String {
    format!("'{text}'")
}

fn windows_prefix(route: &Route) -> String {
    match route.destination.as_str() {
        "default" => "0.0.0.0/0".to_owned(),
        other if other.contains('/') => other.to_owned(),
        other if other.contains(':') => format!("{other}/128"),
        other => format!("{other}/32"),
    }
}

/// Calls that add `route`.
#[must_use]
pub(crate) fn add_call(os: System, route: &Route) -> Call {
    match os {
        System::Linux => {
            let mut args: Vec<String> = Vec::new();
            if route.is_v6() {
                args.push("-6".to_owned());
            }
            args.extend([
                "route".to_owned(),
                "add".to_owned(),
                route.destination.clone(),
            ]);
            args.extend(detail_args(route));
            Call::owned("ip", args)
        }
        System::Macos => {
            let mut args = vec!["-n".to_owned(), "add".to_owned()];
            if route.is_v6() {
                args.push("-inet6".to_owned());
            }
            if route.destination != "default" {
                args.push("-net".to_owned());
            }
            args.push(route.destination.clone());
            match (&route.gateway, &route.interface) {
                (Some(gateway), _) => args.push(gateway.clone()),
                (None, Some(interface)) => {
                    args.extend(["-interface".to_owned(), interface.clone()])
                }
                (None, None) => {}
            }
            Call::owned("route", args)
        }
        System::Windows => {
            let mut script = format!(
                "New-NetRoute -DestinationPrefix {} -Confirm:$false",
                quote_ps(&windows_prefix(route))
            );
            script.push_str(&windows_selectors(route));
            script.push_str(" | Out-Null");
            Call::powershell(&script)
        }
    }
}

/// Call that removes `route`.
#[must_use]
pub(crate) fn remove_call(os: System, route: &Route) -> Call {
    match os {
        System::Linux => {
            let mut args: Vec<String> = Vec::new();
            if route.is_v6() {
                args.push("-6".to_owned());
            }
            args.extend([
                "route".to_owned(),
                "del".to_owned(),
                route.destination.clone(),
            ]);
            args.extend(detail_args(route));
            Call::owned("ip", args)
        }
        System::Macos => {
            let mut args = vec!["-n".to_owned(), "delete".to_owned()];
            if route.is_v6() {
                args.push("-inet6".to_owned());
            }
            if route.destination != "default" {
                args.push("-net".to_owned());
            }
            args.push(route.destination.clone());
            if let Some(gateway) = &route.gateway {
                args.push(gateway.clone());
            }
            Call::owned("route", args)
        }
        System::Windows => {
            let mut script = format!(
                "Remove-NetRoute -DestinationPrefix {} -Confirm:$false",
                quote_ps(&windows_prefix(route))
            );
            script.push_str(&windows_selectors(route));
            Call::powershell(&script)
        }
    }
}

fn detail_args(route: &Route) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(gateway) = &route.gateway {
        args.extend(["via".to_owned(), gateway.clone()]);
    }
    if let Some(interface) = &route.interface {
        args.extend(["dev".to_owned(), interface.clone()]);
    }
    if let Some(metric) = route.metric {
        args.extend(["metric".to_owned(), metric.to_string()]);
    }
    args
}

fn windows_selectors(route: &Route) -> String {
    let mut text = String::new();
    if let Some(gateway) = &route.gateway {
        text.push_str(&format!(" -NextHop {}", quote_ps(gateway)));
    }
    if let Some(interface) = &route.interface {
        text.push_str(&format!(" -InterfaceAlias {}", quote_ps(interface)));
    }
    if let Some(metric) = route.metric {
        text.push_str(&format!(" -RouteMetric {metric}"));
    }
    text
}

fn list_call(os: System, v6: bool) -> Call {
    match os {
        System::Linux if v6 => Call::new("ip", &["-j", "-6", "route", "show"]),
        System::Linux => Call::new("ip", &["-j", "route", "show"]),
        System::Macos => Call::new("netstat", &["-rn", "-f", if v6 { "inet6" } else { "inet" }]),
        System::Windows => Call::powershell(&format!(
            "Get-NetRoute -AddressFamily {} | Select-Object DestinationPrefix,NextHop,InterfaceAlias,RouteMetric | ConvertTo-Json -Compress",
            if v6 { "IPv6" } else { "IPv4" }
        )),
    }
}

/// Routes of the routing table of the family of `destination`.
///
/// # Errors
///
/// The error of the program that failed.
pub(crate) fn read_routes(tool: &Tool<'_>, v6: bool) -> Result<Vec<Route>> {
    let output = tool.ok(&list_call(tool.os(), v6))?;
    match tool.os() {
        System::Linux => parse_ip_json(&output.stdout),
        System::Macos => Ok(parse_netstat(&output.stdout)),
        System::Windows => parse_windows(&output.stdout),
    }
}

/// Which command a [`RouteChange`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    Add,
    Remove,
}

/// The commands `network.route_add` and `network.route_remove`.
#[derive(Debug, Clone, Copy)]
pub struct RouteChange {
    verb: Verb,
}

impl RouteChange {
    /// The command `network.route_add`.
    pub const ADD: Self = Self { verb: Verb::Add };
    /// The command `network.route_remove`.
    pub const REMOVE: Self = Self { verb: Verb::Remove };

    fn command(self) -> &'static str {
        match self.verb {
            Verb::Add => "network.route_add",
            Verb::Remove => "network.route_remove",
        }
    }

    fn request(self, args: &BoundArgs) -> Result<Route> {
        let destination = args.require("destination").and_then(|value| match value {
            Value::Text(text) => Ok(text.clone()),
            _ => args.text("destination").map(|_| String::new()),
        })?;
        if destination != "default" && !valid_source(&destination) {
            return Err(invalid(
                "destination",
                &destination,
                "an address, a network in CIDR form or `default`",
            ));
        }
        let gateway = match args.text("gateway")? {
            None => None,
            Some(text) if valid_source(text) && !text.contains('/') => Some(text.to_owned()),
            Some(text) => return Err(invalid("gateway", text, "an IP address")),
        };
        let interface = match args.text("interface")? {
            None => None,
            Some(text) if valid_interface(text) => Some(text.to_owned()),
            Some(text) => return Err(invalid("interface", text, "an interface name")),
        };
        let metric = match args.int("metric")? {
            None => None,
            Some(number) => Some(u32::try_from(number).map_err(|_| {
                invalid(
                    "metric",
                    &number.to_string(),
                    "a number from 0 to 4294967295",
                )
            })?),
        };
        if self.verb == Verb::Add && gateway.is_none() && interface.is_none() {
            return Err(Error::new(
                ErrorCode::MissingParam,
                "`network.route_add` needs a `gateway` or an `interface`",
            ));
        }
        Ok(Route {
            destination,
            gateway,
            interface,
            metric,
        })
    }
}

fn invalid(name: &str, text: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("`{text}` is not valid for `{name}`: {expected}"),
    )
    .with_found(text)
}

struct State {
    routes: Vec<Route>,
    add: bool,
}

impl StageImpl for RouteChange {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: self.command().to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        self.request(args)?;
        let mut info = StaticInfo::default();
        let program = match System::of(ctx.host().info().os) {
            System::Linux => "ip",
            System::Macos => "route",
            System::Windows => "powershell",
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

impl EffectStage for RouteChange {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let wanted = self.request(args)?;
        let tool = Tool::from_ctx(ctx)?;
        let table = read_routes(&tool, wanted.destination.contains(':'))?;
        let mut plan = EffectPlan::new();
        let add = self.verb == Verb::Add;
        let routes: Vec<Route> = if add {
            if table.iter().any(|route| wanted.selects(route)) {
                plan.notable
                    .push(unchanged(&wanted.target(), "the route already exists"));
                Vec::new()
            } else {
                vec![wanted]
            }
        } else {
            let mut found: Vec<Route> = table
                .into_iter()
                .filter(|route| wanted.selects(route))
                .collect();
            found.sort();
            found.dedup();
            if found.is_empty() {
                return Err(Error::new(
                    ErrorCode::NotFound,
                    format!("no route to `{}` matches the request", wanted.destination),
                )
                .with_found(wanted.destination.clone()));
            }
            found
        };
        plan.bulk = routes.len() > 1;
        for route in &routes {
            let (before, after) = states(route, add);
            plan.changes.push(state_change(
                &route.target(),
                before.as_deref(),
                after.as_deref(),
            ));
            plan.irreversible.push(marker("route", &route.target()));
            plan.preview.push(effect_record(
                self.command(),
                &route.target(),
                EffectStatus::Planned,
                before.as_deref(),
                after.as_deref(),
            ));
        }
        Ok(plan.with_state(State { routes, add }))
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
            .ok_or_else(|| Error::new(ErrorCode::StageFailed, "the route change has no plan"))?;
        let tool = Tool::from_ctx(ctx)?;
        let os = tool.os();
        let mut outcome = EffectOutcome::new(StageOutput::none());
        let mut records = Vec::new();
        let mut done: Vec<&Route> = Vec::new();
        for route in &state.routes {
            let entry = marker("route", &route.target());
            tx.mark_irreversible(entry.clone());
            let call = if state.add {
                add_call(os, route)
            } else {
                remove_call(os, route)
            };
            if let Err(error) = ctx.checkpoint().and_then(|()| tool.ok(&call)) {
                for earlier in done.iter().rev() {
                    let undo = if state.add {
                        remove_call(os, earlier)
                    } else {
                        add_call(os, earlier)
                    };
                    let _ = tool.run(&undo);
                }
                return Err(error.with_effect_applied(false));
            }
            let (before, after) = states(route, state.add);
            outcome.changes.push(state_change(
                &route.target(),
                before.as_deref(),
                after.as_deref(),
            ));
            outcome.irreversible.push(entry);
            records.push(effect_record(
                self.command(),
                &route.target(),
                EffectStatus::Ok,
                before.as_deref(),
                after.as_deref(),
            ));
            done.push(route);
        }
        outcome.output = StageOutput::records(RecordStream::from_records(records));
        Ok(outcome)
    }
}

fn states(route: &Route, add: bool) -> (Option<String>, Option<String>) {
    if add {
        (None, Some(route.state()))
    } else {
        (Some(route.state()), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::testing::{FakeProcess, RecordingTx, args, ctx, input, records, text};
    use nurt_contracts::value::Value;

    const TABLE: &str = r#"[{"dst":"default","gateway":"192.168.1.1","dev":"eth0","metric":100},{"dst":"10.9.0.0/16","gateway":"192.168.1.5","dev":"eth0"},{"dst":"192.168.1.0/24","dev":"eth0","scope":"link"}]"#;

    #[test]
    fn state_round_trips() {
        let route = Route {
            destination: "10.9.0.0/16".to_owned(),
            gateway: Some("192.168.1.5".to_owned()),
            interface: Some("Ethernet 2".to_owned()),
            metric: Some(50),
        };
        assert_eq!(
            Route::from_state("10.9.0.0/16", &route.state()),
            Some(route)
        );
        assert_eq!(Route::from_state("x", "via=1.1.1.1"), None);
    }

    #[test]
    fn parses_the_three_listing_formats() {
        let routes = parse_ip_json(TABLE).unwrap();
        assert_eq!(routes.len(), 3);
        assert_eq!(routes[0].metric, Some(100));
        assert_eq!(routes[2].gateway, None);
        let netstat = "Routing tables\n\nInternet:\nDestination        Gateway            Flags               Netif Expire\ndefault            192.168.1.1        UGScg                 en0\n10.9.0.0/16        link#6             UCS                   en1\n";
        let routes = parse_netstat(netstat);
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].gateway.as_deref(), Some("192.168.1.1"));
        assert_eq!(routes[1].gateway, None);
        let windows = r#"[{"DestinationPrefix":"0.0.0.0/0","NextHop":"192.168.1.1","InterfaceAlias":"Ethernet","RouteMetric":25}]"#;
        let routes = parse_windows(windows).unwrap();
        assert_eq!(routes[0].destination, "default");
        assert_eq!(routes[0].metric, Some(25));
    }

    #[test]
    fn calls_per_system() {
        let route = Route {
            destination: "10.9.0.0/16".to_owned(),
            gateway: Some("192.168.1.5".to_owned()),
            interface: Some("eth0".to_owned()),
            metric: Some(20),
        };
        assert_eq!(
            add_call(System::Linux, &route).line(),
            "ip route add 10.9.0.0/16 via 192.168.1.5 dev eth0 metric 20"
        );
        assert_eq!(
            remove_call(System::Macos, &route).line(),
            "route -n delete -net 10.9.0.0/16 192.168.1.5"
        );
        assert!(add_call(System::Windows, &route)
            .line()
            .contains("New-NetRoute -DestinationPrefix '10.9.0.0/16' -Confirm:$false -NextHop '192.168.1.5' -InterfaceAlias 'eth0' -RouteMetric 20"));
    }

    #[test]
    fn adding_a_present_route_changes_nothing() {
        let process = FakeProcess::new();
        process.reply("ip -j route show", TABLE);
        let ctx = ctx(System::Linux, &process);
        let request = args(
            "network.route_add",
            &[
                ("destination", text("10.9.0.0/16")),
                ("gateway", text("192.168.1.5")),
            ],
        );
        let plan = RouteChange::ADD.plan(&request, input(), &ctx).unwrap();
        assert!(plan.changes.is_empty());
        assert_eq!(plan.notable.len(), 1);
    }

    #[test]
    fn adding_a_route_records_the_state_and_runs_ip() {
        let process = FakeProcess::new();
        process.reply("ip -j route show", TABLE);
        let ctx = ctx(System::Linux, &process);
        let request = args(
            "network.route_add",
            &[
                ("destination", text("10.20.0.0/16")),
                ("gateway", text("192.168.1.9")),
                ("metric", Value::Int(7)),
            ],
        );
        let plan = RouteChange::ADD.plan(&request, input(), &ctx).unwrap();
        assert_eq!(plan.changes[0].before, None);
        let mut tx = RecordingTx::default();
        let outcome = RouteChange::ADD
            .apply(&request, plan, &mut tx, &ctx)
            .unwrap();
        assert_eq!(
            outcome.changes[0].after.as_deref(),
            Some("via=192.168.1.9 dev=- metric=7")
        );
        assert_eq!(tx.markers.len(), 1);
        assert_eq!(
            process.lines().last().unwrap(),
            "ip route add 10.20.0.0/16 via 192.168.1.9 metric 7"
        );
        assert_eq!(records(outcome.output).len(), 1);
    }

    #[test]
    fn removing_a_route_records_what_was_removed() {
        let process = FakeProcess::new();
        process.reply("ip -j route show", TABLE);
        let ctx = ctx(System::Linux, &process);
        let request = args(
            "network.route_remove",
            &[("destination", text("10.9.0.0/16"))],
        );
        let plan = RouteChange::REMOVE.plan(&request, input(), &ctx).unwrap();
        assert_eq!(
            plan.changes[0].before.as_deref(),
            Some("via=192.168.1.5 dev=eth0 metric=-")
        );
        let mut tx = RecordingTx::default();
        RouteChange::REMOVE
            .apply(&request, plan, &mut tx, &ctx)
            .unwrap();
        assert_eq!(
            process.lines().last().unwrap(),
            "ip route del 10.9.0.0/16 via 192.168.1.5 dev eth0"
        );
    }

    #[test]
    fn removing_a_missing_route_is_not_found() {
        let process = FakeProcess::new();
        process.reply("ip -j route show", TABLE);
        let ctx = ctx(System::Linux, &process);
        let request = args(
            "network.route_remove",
            &[("destination", text("172.16.0.0/12"))],
        );
        let error = RouteChange::REMOVE
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
            args(
                "network.route_add",
                &[
                    ("destination", text("10.0.0.0/8; reboot")),
                    ("gateway", text("1.1.1.1")),
                ],
            ),
            args("network.route_add", &[("destination", text("10.0.0.0/8"))]),
            args(
                "network.route_add",
                &[
                    ("destination", text("10.0.0.0/8")),
                    ("interface", text("-x")),
                ],
            ),
        ] {
            assert!(RouteChange::ADD.plan(&request, input(), &ctx).is_err());
        }
        assert!(process.lines().is_empty());
    }

    #[test]
    fn a_failing_program_reports_missing_rights() {
        let process = FakeProcess::new();
        process.reply("ip -j route show", TABLE);
        process.fail(
            "ip route add",
            2,
            "RTNETLINK answers: Operation not permitted",
        );
        let ctx = ctx(System::Linux, &process);
        let request = args(
            "network.route_add",
            &[
                ("destination", text("10.20.0.0/16")),
                ("gateway", text("192.168.1.9")),
            ],
        );
        let plan = RouteChange::ADD.plan(&request, input(), &ctx).unwrap();
        let mut tx = RecordingTx::default();
        let error = RouteChange::ADD
            .apply(&request, plan, &mut tx, &ctx)
            .err()
            .unwrap();
        assert_eq!(error.code(), ErrorCode::Privilege);
    }
}
