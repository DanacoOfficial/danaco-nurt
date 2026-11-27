// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `power.restart`, `power.shutdown` and `power.cancel`.
//!
//! | System | Immediate | Scheduled | Cancel |
//! |---|---|---|---|
//! | Linux | `shutdown -r now`, `shutdown -P now` | `shutdown -r +N`, `shutdown -P +N` | `shutdown -c` |
//! | macOS | `shutdown -r now`, `shutdown -h now` | `shutdown -r +N`, `shutdown -h +N` | `pkill -x shutdown` |
//! | Windows | `shutdown /r /t 0`, `shutdown /s /t 0` | `shutdown /r /t S`, `shutdown /s /t S` | `shutdown /a` |
//!
//! `at` is converted to a delay with the clock of the engine, so no time zone
//! is involved. A delay is rounded up to whole minutes on Linux and macOS.

use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::BoundArgs;
use nurt_contracts::value::Value;
use serde::{Deserialize, Serialize};

use super::sys::{Call, Sys};
use super::{Plan, StateChange, args};

/// Target of a scheduled restart or shutdown.
pub(crate) const SCHEDULE: &str = "power:schedule";
/// Target of an immediate restart or shutdown.
pub(crate) const NOW: &str = "power:now";
/// File that systemd keeps while a shutdown is scheduled.
const SCHEDULED_FILE: &str = "/run/systemd/shutdown/scheduled";
const NANOS_PER_SECOND: i128 = 1_000_000_000;

/// State of the power schedule as recorded in the journal.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) struct PowerState {
    /// `restart` or `shutdown`.
    pub action: String,
    /// Minutes until the action; absent when it is immediate or unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes: Option<u64>,
    /// Message shown to the users.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl PowerState {
    /// The JSON document kept as `before` or `after`.
    pub(crate) fn to_text(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

pub(crate) fn plan(sys: &dyn Sys, args: &BoundArgs) -> Result<Plan> {
    match args.command() {
        "power.cancel" => cancel(sys),
        "power.restart" => change(sys, args, "restart"),
        _ => change(sys, args, "shutdown"),
    }
}

/// The delay in seconds that `at` or `delay` ask for, `None` for an
/// immediate action.
fn delay_seconds(sys: &dyn Sys, args: &BoundArgs) -> Result<Option<u64>> {
    let at = args.value("at");
    let delay = args.value("delay");
    match (at, delay) {
        (Some(Value::Null) | None, Some(Value::Null) | None) => Ok(None),
        (Some(Value::Null) | None, Some(Value::Duration(duration))) => {
            let millis = duration.as_millis();
            if millis < 0 {
                return Err(Error::new(
                    ErrorCode::InvalidValue,
                    "`delay` must not be negative",
                ));
            }
            Ok(Some(u64::try_from(millis).unwrap_or(0).div_ceil(1000)))
        }
        (Some(Value::Instant(instant)), Some(Value::Null) | None) => {
            let ahead = (i128::from(instant.as_unix_nanos()) - sys.now_nanos()) / NANOS_PER_SECOND;
            if ahead <= 0 {
                return Err(Error::new(
                    ErrorCode::InvalidValue,
                    "`at` is not in the future",
                ));
            }
            Ok(Some(u64::try_from(ahead).unwrap_or(u64::MAX)))
        }
        (Some(Value::Instant(_)), Some(_)) => Err(Error::new(
            ErrorCode::ParamConflict,
            "`at` and `delay` exclude each other",
        )),
        (Some(other), _) | (_, Some(other)) => Err(Error::new(
            ErrorCode::Type,
            format!(
                "`at` expects an instant and `delay` a duration, found {}",
                other.type_name()
            ),
        )),
    }
}

fn change(sys: &dyn Sys, args: &BoundArgs, action: &str) -> Result<Plan> {
    let message = args::text(args, "message")?;
    if let Some(message) = &message
        && message.chars().any(char::is_control)
    {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "`message` must not hold a control character",
        ));
    }
    let seconds = delay_seconds(sys, args)?;
    let os = sys.os();
    if seconds.is_some()
        && let Some(existing) = scheduled(sys)?
    {
        return Err(Error::new(
            ErrorCode::InUse,
            format!("a {existing} is already scheduled; cancel it first"),
        )
        .with_fix("power.cancel"));
    }
    let minutes = seconds.map(|seconds| seconds.div_ceil(60));
    let reboot = action == "restart";
    let call = match os {
        Os::Windows => {
            let mut call = Call::new(
                "shutdown",
                [
                    if reboot { "/r" } else { "/s" }.to_owned(),
                    "/t".to_owned(),
                    seconds.unwrap_or(0).to_string(),
                ],
            );
            if let Some(message) = &message {
                call.args.extend(["/c".to_owned(), message.clone()]);
            }
            call
        }
        _ => {
            let flag = match (os, reboot) {
                (_, true) => "-r",
                (Os::Macos, false) => "-h",
                (_, false) => "-P",
            };
            let when = minutes.map_or_else(|| "now".to_owned(), |minutes| format!("+{minutes}"));
            let mut call = Call::new("shutdown", [flag.to_owned(), when]);
            if let Some(message) = &message {
                call.args.push(message.clone());
            }
            call
        }
    };
    let after = PowerState {
        action: action.to_owned(),
        minutes: if os == Os::Windows {
            seconds.map(|s| s.div_ceil(60))
        } else {
            minutes
        },
        message,
    };
    let scheduled = seconds.is_some();
    Ok(Plan {
        calls: vec![call],
        changes: vec![StateChange::new(
            if scheduled { SCHEDULE } else { NOW },
            None,
            Some(after.to_text()),
        )],
        irreversible: (!scheduled)
            .then(|| format!("the {action} runs at once and cannot be taken back")),
        ..Plan::default()
    })
}

fn cancel(sys: &dyn Sys) -> Result<Plan> {
    let os = sys.os();
    let before = if os == Os::Linux {
        let Some(existing) = scheduled(sys)? else {
            return Ok(Plan {
                unchanged: Some("no restart or shutdown is scheduled".to_owned()),
                ..Plan::default()
            });
        };
        PowerState {
            action: existing,
            ..PowerState::default()
        }
    } else {
        PowerState {
            action: "shutdown".to_owned(),
            ..PowerState::default()
        }
    };
    let call = match os {
        Os::Windows => Call::new("shutdown", ["/a"]),
        Os::Macos => Call::new("pkill", ["-x", "shutdown"]),
        _ => Call::new("shutdown", ["-c"]),
    };
    Ok(Plan {
        calls: vec![call],
        changes: vec![StateChange::new(SCHEDULE, Some(before.to_text()), None)],
        irreversible: Some("a cancelled schedule is not restored".to_owned()),
        ..Plan::default()
    })
}

/// The scheduled action on Linux, `None` when nothing is scheduled or the
/// system cannot tell.
fn scheduled(sys: &dyn Sys) -> Result<Option<String>> {
    if sys.os() != Os::Linux {
        return Ok(None);
    }
    Ok(sys
        .read_file(SCHEDULED_FILE)?
        .map(|text| parse_scheduled(&text)))
}

/// The action named by `MODE=` of the file of systemd.
pub(crate) fn parse_scheduled(text: &str) -> String {
    let mode = text
        .lines()
        .find_map(|line| line.strip_prefix("MODE="))
        .unwrap_or("");
    match mode {
        "reboot" | "kexec" => "restart".to_owned(),
        _ => "shutdown".to_owned(),
    }
}
