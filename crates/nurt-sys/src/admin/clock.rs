// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::admin::clock
// Responsibility:   The commands `clock.timezone_set`, `clock.ntp` and `clock.set`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The commands `clock.timezone_set`, `clock.ntp` and `clock.set`.
//!
//! The time zone and the state of network time are read before the change
//! and recorded as the targets `clock:timezone` and `clock:ntp`, so `undo`
//! restores them. Setting the time itself is irreversible: the previous
//! reading is lost the moment the clock moves.

use crate::admin::System;
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::kinds::Implementation;
use nurt_contracts::record::EffectStatus;
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, ExternalProgram,
    InputKinds, RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::Value;

use super::effects::{effect_record, marker, state_change, unchanged};
use super::{Call, Tool};

/// Target of the time zone.
pub const TIMEZONE_TARGET: &str = "clock:timezone";
/// Target of the state of network time.
pub const NTP_TARGET: &str = "clock:ntp";
/// Target of the time of the clock.
pub const TIME_TARGET: &str = "clock:time";

const ON: &str = "on";
const OFF: &str = "off";

/// Every stage of the clock commands, in catalog order.
#[must_use]
pub fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![
        Arc::new(ClockCommand::TIMEZONE),
        Arc::new(ClockCommand::SET),
        Arc::new(ClockCommand::NTP),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    Timezone,
    Set,
    Ntp,
}

/// The three clock commands.
#[derive(Debug, Clone, Copy)]
pub struct ClockCommand {
    verb: Verb,
}

impl ClockCommand {
    /// The command `clock.timezone_set`.
    pub const TIMEZONE: Self = Self {
        verb: Verb::Timezone,
    };
    /// The command `clock.set`.
    pub const SET: Self = Self { verb: Verb::Set };
    /// The command `clock.ntp`.
    pub const NTP: Self = Self { verb: Verb::Ntp };

    fn command(self) -> &'static str {
        match self.verb {
            Verb::Timezone => "clock.timezone_set",
            Verb::Set => "clock.set",
            Verb::Ntp => "clock.ntp",
        }
    }

    fn target(self) -> &'static str {
        match self.verb {
            Verb::Timezone => TIMEZONE_TARGET,
            Verb::Set => TIME_TARGET,
            Verb::Ntp => NTP_TARGET,
        }
    }
}

/// Whether `text` can be a time zone: IANA names and Windows identifiers.
#[must_use]
pub fn valid_zone_name(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && !text.starts_with(['-', '/'])
        && !text.contains("..")
        && text.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '_' | '+' | '-' | '/' | ' ' | '.' | '(' | ')' | ',')
        })
}

/// Date and time of an RFC 3339 instant as `YYYY-MM-DD` and `HH:MM:SS`.
#[must_use]
pub fn date_and_time(instant: &str) -> Option<(String, String)> {
    let (date, time) = instant.split_once('T')?;
    let time = time.split(['.', 'Z', '+']).next()?;
    (date.len() == 10 && time.len() == 8).then(|| (date.to_owned(), time.to_owned()))
}

enum Request {
    Timezone(String),
    Ntp(bool),
    Set(String),
}

fn request(verb: Verb, args: &BoundArgs) -> Result<Request> {
    match verb {
        Verb::Timezone => {
            let zone = match args.require("zone")? {
                Value::Text(text) => text.clone(),
                _ => return args.text("zone").map(|_| Request::Timezone(String::new())),
            };
            if valid_zone_name(&zone) {
                Ok(Request::Timezone(zone))
            } else {
                Err(Error::new(
                    ErrorCode::InvalidValue,
                    format!("`{zone}` is not a time zone name"),
                )
                .with_found(zone))
            }
        }
        Verb::Ntp => match args.require("state")? {
            Value::Enum(text) | Value::Text(text) if text == ON => Ok(Request::Ntp(true)),
            Value::Enum(text) | Value::Text(text) if text == OFF => Ok(Request::Ntp(false)),
            Value::Bool(flag) => Ok(Request::Ntp(*flag)),
            other => Err(Error::new(
                ErrorCode::InvalidValue,
                format!("`state` is `on` or `off`, not {}", other.type_name()),
            )),
        },
        Verb::Set => match args.require("time")? {
            Value::Instant(instant) => {
                let text = instant.to_string();
                date_and_time(&text)
                    .map(|(date, time)| Request::Set(format!("{date}T{time}Z")))
                    .ok_or_else(|| {
                        Error::new(
                            ErrorCode::InvalidValue,
                            format!("`{text}` is not a time of day"),
                        )
                    })
            }
            other => Err(Error::new(
                ErrorCode::Type,
                format!("`clock.set` expects an instant, not {}", other.type_name()),
            )),
        },
    }
}

fn single_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().trim().to_owned()
}

/// Time zone of the system.
///
/// # Errors
///
/// The error of the program that failed.
pub fn read_timezone(tool: &Tool<'_>) -> Result<String> {
    match tool.os() {
        System::Linux => Ok(single_line(
            &tool
                .ok(&Call::new(
                    "timedatectl",
                    &["show", "-p", "Timezone", "--value"],
                ))?
                .stdout,
        )),
        System::Macos => {
            let output = tool.ok(&Call::new("systemsetup", &["-gettimezone"]))?;
            Ok(single_line(&output.stdout)
                .rsplit_once(": ")
                .map_or_else(String::new, |(_, zone)| zone.to_owned()))
        }
        System::Windows => Ok(single_line(&tool.ok(&Call::new("tzutil", &["/g"]))?.stdout)),
    }
}

/// State of network time: `on` or `off`.
///
/// # Errors
///
/// The error of the program that failed.
pub fn read_ntp(tool: &Tool<'_>) -> Result<String> {
    let on = match tool.os() {
        System::Linux => {
            single_line(
                &tool
                    .ok(&Call::new("timedatectl", &["show", "-p", "NTP", "--value"]))?
                    .stdout,
            ) == "yes"
        }
        System::Macos => single_line(
            &tool
                .ok(&Call::new("systemsetup", &["-getusingnetworktime"]))?
                .stdout,
        )
        .ends_with("On"),
        System::Windows => {
            let output = tool.ok(&Call::powershell(
                "(Get-Service w32time).StartType.ToString()",
            ))?;
            single_line(&output.stdout) != "Disabled"
        }
    };
    Ok(if on { ON } else { OFF }.to_owned())
}

/// Calls that set the time zone.
#[must_use]
pub fn timezone_calls(os: System, zone: &str) -> Vec<Call> {
    vec![match os {
        System::Linux => Call::new("timedatectl", &["set-timezone", zone]),
        System::Macos => Call::new("systemsetup", &["-settimezone", zone]),
        System::Windows => Call::new("tzutil", &["/s", zone]),
    }]
}

/// Calls that switch network time.
#[must_use]
pub fn ntp_calls(os: System, on: bool) -> Vec<Call> {
    match os {
        System::Linux => vec![Call::new(
            "timedatectl",
            &["set-ntp", if on { "true" } else { "false" }],
        )],
        System::Macos => vec![Call::new(
            "systemsetup",
            &["-setusingnetworktime", if on { ON } else { OFF }],
        )],
        System::Windows => vec![Call::powershell(if on {
            "Set-Service w32time -StartupType Automatic; Start-Service w32time"
        } else {
            "Stop-Service w32time; Set-Service w32time -StartupType Disabled"
        })],
    }
}

/// Calls that set the clock to `instant` (`YYYY-MM-DDTHH:MM:SSZ`).
#[must_use]
pub fn set_calls(os: System, instant: &str) -> Vec<Call> {
    let Some((date, time)) = date_and_time(instant) else {
        return Vec::new();
    };
    vec![match os {
        System::Linux => Call::owned(
            "date",
            vec!["-u".to_owned(), "-s".to_owned(), format!("{date} {time}")],
        ),
        System::Macos => Call::owned(
            "date",
            vec![
                "-u".to_owned(),
                format!(
                    "{}{}{}{}{}.{}",
                    &date[0..4],
                    &date[5..7],
                    &date[8..10],
                    &time[0..2],
                    &time[3..5],
                    &time[6..8]
                ),
            ],
        ),
        System::Windows => Call::powershell(&format!(
            "Set-Date -Date ([DateTimeOffset]::Parse('{instant}').LocalDateTime) | Out-Null"
        )),
    }]
}

struct State {
    request: Request,
    before: Option<String>,
}

impl StageImpl for ClockCommand {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: self.command().to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        request(self.verb, args)?;
        let mut info = StaticInfo::default();
        let program = match (System::of(ctx.host().info().os), self.verb) {
            (System::Linux, Verb::Set) => "date",
            (System::Linux, _) => "timedatectl",
            (System::Macos, Verb::Set) => "date",
            (System::Macos, _) => "systemsetup",
            (System::Windows, Verb::Timezone) => "tzutil",
            (System::Windows, _) => "powershell",
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

impl EffectStage for ClockCommand {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let wanted = request(self.verb, args)?;
        let tool = Tool::from_ctx(ctx)?;
        let (before, after) = match &wanted {
            Request::Timezone(zone) => (Some(read_timezone(&tool)?), zone.clone()),
            Request::Ntp(on) => (
                Some(read_ntp(&tool)?),
                if *on { ON } else { OFF }.to_owned(),
            ),
            Request::Set(time) => (None, time.clone()),
        };
        let mut plan = EffectPlan::new();
        if before.as_deref() == Some(after.as_str()) {
            plan.notable.push(unchanged(
                self.target(),
                "the system already holds this value",
            ));
            return Ok(plan.with_state(State {
                request: wanted,
                before,
            }));
        }
        plan.changes
            .push(state_change(self.target(), before.as_deref(), Some(&after)));
        plan.irreversible
            .push(marker(self.command(), self.target()));
        plan.preview.push(effect_record(
            self.command(),
            self.target(),
            EffectStatus::Planned,
            before.as_deref(),
            Some(&after),
        ));
        Ok(plan.with_state(State {
            request: wanted,
            before,
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
            .ok_or_else(|| Error::new(ErrorCode::StageFailed, "the clock change has no plan"))?;
        let mut outcome = EffectOutcome::new(StageOutput::none());
        if plan.changes.is_empty() {
            return Ok(outcome);
        }
        let tool = Tool::from_ctx(ctx)?;
        let (calls, after) = match &state.request {
            Request::Timezone(zone) => (timezone_calls(tool.os(), zone), zone.clone()),
            Request::Ntp(on) => (
                ntp_calls(tool.os(), *on),
                if *on { ON } else { OFF }.to_owned(),
            ),
            Request::Set(time) => (set_calls(tool.os(), time), time.clone()),
        };
        let entry = marker(self.command(), self.target());
        tx.mark_irreversible(entry.clone());
        for call in &calls {
            ctx.checkpoint()?;
            tool.ok(call)
                .map_err(|error| error.with_effect_applied(false))?;
        }
        outcome.changes.push(state_change(
            self.target(),
            state.before.as_deref(),
            Some(&after),
        ));
        outcome.irreversible.push(entry);
        outcome.output = StageOutput::records(RecordStream::from_records(vec![effect_record(
            self.command(),
            self.target(),
            EffectStatus::Ok,
            state.before.as_deref(),
            Some(&after),
        )]));
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::testing::{FakeProcess, RecordingTx, args, ctx, input, text, word};

    #[test]
    fn zone_names_are_checked() {
        assert!(valid_zone_name("Europe/Warsaw"));
        assert!(valid_zone_name("W. Europe Standard Time"));
        assert!(!valid_zone_name("../etc/passwd"));
        assert!(!valid_zone_name("-x"));
        assert!(!valid_zone_name("a;b"));
        assert!(!valid_zone_name(""));
    }

    #[test]
    fn instants_are_split_into_date_and_time() {
        assert_eq!(
            date_and_time("2026-11-21T09:12:00Z"),
            Some(("2026-11-21".to_owned(), "09:12:00".to_owned()))
        );
        assert_eq!(date_and_time("nonsense"), None);
    }

    #[test]
    fn set_calls_per_system() {
        let at = "2026-11-21T09:12:05Z";
        assert_eq!(
            set_calls(System::Linux, at)[0].line(),
            "date -u -s 2026-11-21 09:12:05"
        );
        assert_eq!(
            set_calls(System::Macos, at)[0].line(),
            "date -u 202611210912.05"
        );
        assert!(
            set_calls(System::Windows, at)[0]
                .line()
                .contains("Parse('2026-11-21T09:12:05Z')")
        );
    }

    #[test]
    fn timezone_change_is_recorded_and_applied() {
        let process = FakeProcess::new();
        process.reply("timedatectl show -p Timezone", "Europe/Warsaw\n");
        let ctx = ctx(System::Linux, &process);
        let request = args("clock.timezone_set", &[("zone", text("UTC"))]);
        let plan = ClockCommand::TIMEZONE
            .plan(&request, input(), &ctx)
            .unwrap();
        assert_eq!(plan.changes[0].before.as_deref(), Some("Europe/Warsaw"));
        let mut tx = RecordingTx::default();
        let outcome = ClockCommand::TIMEZONE
            .apply(&request, plan, &mut tx, &ctx)
            .unwrap();
        assert_eq!(outcome.changes[0].after.as_deref(), Some("UTC"));
        assert_eq!(
            process.lines().last().unwrap(),
            "timedatectl set-timezone UTC"
        );
        assert_eq!(tx.markers.len(), 1);
    }

    #[test]
    fn the_same_zone_changes_nothing() {
        let process = FakeProcess::new();
        process.reply("timedatectl show -p Timezone", "UTC\n");
        let ctx = ctx(System::Linux, &process);
        let request = args("clock.timezone_set", &[("zone", text("UTC"))]);
        let plan = ClockCommand::TIMEZONE
            .plan(&request, input(), &ctx)
            .unwrap();
        assert!(plan.changes.is_empty());
        let mut tx = RecordingTx::default();
        ClockCommand::TIMEZONE
            .apply(&request, plan, &mut tx, &ctx)
            .unwrap();
        assert!(tx.markers.is_empty());
        assert_eq!(process.lines().len(), 1);
    }

    #[test]
    fn ntp_state_is_read_per_system() {
        let process = FakeProcess::new();
        process.reply("systemsetup -getusingnetworktime", "Network Time: On\n");
        let tool_ctx = ctx(System::Macos, &process);
        let request = args("clock.ntp", &[("state", word("off"))]);
        let plan = ClockCommand::NTP
            .plan(&request, input(), &tool_ctx)
            .unwrap();
        assert_eq!(plan.changes[0].before.as_deref(), Some("on"));
        assert_eq!(plan.changes[0].after.as_deref(), Some("off"));
    }

    #[test]
    fn a_failing_program_is_an_error_and_missing_rights_are_reported() {
        let process = FakeProcess::new();
        process.reply("timedatectl show -p NTP", "yes\n");
        process.fail(
            "timedatectl set-ntp",
            1,
            "Failed to set ntp: Interactive authentication required. Permission denied",
        );
        let ctx = ctx(System::Linux, &process);
        let request = args("clock.ntp", &[("state", word("off"))]);
        let plan = ClockCommand::NTP.plan(&request, input(), &ctx).unwrap();
        let mut tx = RecordingTx::default();
        let error = ClockCommand::NTP
            .apply(&request, plan, &mut tx, &ctx)
            .err()
            .unwrap();
        assert_eq!(error.code(), ErrorCode::Privilege);
    }

    #[test]
    fn setting_the_time_has_no_previous_value() {
        let process = FakeProcess::new();
        let ctx = ctx(System::Linux, &process);
        let instant = "2026-11-21T09:12:00Z".parse().unwrap();
        let request = args("clock.set", &[("time", Value::Instant(instant))]);
        let plan = ClockCommand::SET.plan(&request, input(), &ctx).unwrap();
        assert_eq!(plan.changes[0].before, None);
        assert_eq!(plan.irreversible.len(), 1);
        assert!(process.lines().is_empty());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
