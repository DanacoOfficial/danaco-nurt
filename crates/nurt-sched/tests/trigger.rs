// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Cron expressions, intervals, single times, time zones and changes to and
//! from daylight saving time.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::time::Duration;

use nurt_contracts::env::WallTime;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::TriggerKind;
use nurt_contracts::value::Instant;
use nurt_sched::settings::ScheduleSettings;
use nurt_sched::trigger::cron::CronSpec;
use nurt_sched::trigger::time::{add, instant_from_wall, since};
use nurt_sched::trigger::{Trigger, TriggerDef, format_duration, format_local};
use support::{at, minutes};

const T0: &str = "2026-11-22T14:30:00Z";

/// The first `count` run times of a cron expression after `from`.
fn runs(expression: &str, timezone: &str, from: &str, count: usize) -> Vec<Instant> {
    let from = at(from);
    let def = TriggerDef::cron(expression, timezone, "UTC", from).unwrap();
    let trigger = Trigger::compile(&def).unwrap();
    let mut after = from;
    let mut found = Vec::with_capacity(count);
    for _ in 0..count {
        after = trigger.next_after(after).unwrap();
        found.push(after);
    }
    found
}

fn instants(texts: &[&str]) -> Vec<Instant> {
    texts.iter().map(|text| at(text)).collect()
}

fn every(interval: Duration) -> Trigger {
    let def = TriggerDef::every(interval, "UTC", "UTC", &ScheduleSettings::default()).unwrap();
    Trigger::compile(&def).unwrap()
}

#[test]
fn tc_02_every_below_minimum_rejected_unless_minimum_lowered() {
    let settings = ScheduleSettings::default();
    let error = TriggerDef::every(Duration::from_secs(10), "UTC", "UTC", &settings).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert!(
        error.fix.iter().any(|fix| fix == "every: 30s"),
        "{:?}",
        error.fix
    );
    assert_eq!(error.found.as_deref(), Some("10s"));

    let lowered = ScheduleSettings {
        min_interval: Duration::from_secs(5),
        ..ScheduleSettings::default()
    };
    let trigger = TriggerDef::every(Duration::from_secs(10), "UTC", "UTC", &lowered).unwrap();
    assert_eq!(trigger.kind, TriggerKind::Every);
    assert_eq!(trigger.normalized(), "every 10s");
}

#[test]
fn tc_03_every_above_366_days_rejected() {
    let settings = ScheduleSettings::default();
    let day = Duration::from_secs(86_400);
    let error = TriggerDef::every(day * 367, "UTC", "UTC", &settings).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert!(error.message.contains("366d"), "{}", error.message);
    assert!(TriggerDef::every(day * 366, "UTC", "UTC", &settings).is_ok());
}

#[test]
fn tc_04_cron_steps_ranges_lists_and_names() {
    let expression = "*/15 9-17 * JAN,jun MON-fri";
    assert_eq!(
        runs(expression, "UTC", "2026-11-22T12:00:00Z", 3),
        instants(&[
            "2027-01-01T09:00:00Z",
            "2027-01-01T09:15:00Z",
            "2027-01-01T09:30:00Z"
        ])
    );
    assert_eq!(
        runs(expression, "UTC", "2027-01-01T17:40:00Z", 2),
        instants(&["2027-01-01T17:45:00Z", "2027-01-04T09:00:00Z"])
    );
    assert_eq!(
        runs(expression, "UTC", "2027-01-29T17:50:00Z", 1),
        instants(&["2027-06-01T09:00:00Z"])
    );
}

#[test]
fn tc_05_day_of_month_and_day_of_week_are_alternatives() {
    assert_eq!(
        runs("0 12 13 * fri", "UTC", "2026-11-22T00:00:00Z", 1),
        instants(&["2026-11-27T12:00:00Z"])
    );
    assert_eq!(
        runs("0 12 13 * *", "UTC", "2026-11-22T00:00:00Z", 1),
        instants(&["2026-12-13T12:00:00Z"])
    );
    assert_eq!(
        runs("0 12 * * fri", "UTC", "2026-11-22T00:00:00Z", 1),
        instants(&["2026-11-27T12:00:00Z"])
    );
}

#[test]
fn tc_06_step_from_value_and_sunday_as_seven() {
    assert_eq!(
        runs("5/20 * * * *", "UTC", "2026-11-22T12:00:00Z", 4),
        instants(&[
            "2026-11-22T12:05:00Z",
            "2026-11-22T12:25:00Z",
            "2026-11-22T12:45:00Z",
            "2026-11-22T13:05:00Z"
        ])
    );
    let sundays = instants(&["2026-11-29T00:00:00Z", "2026-12-06T00:00:00Z"]);
    assert_eq!(runs("0 0 * * 7", "UTC", "2026-11-22T12:00:00Z", 2), sundays);
    assert_eq!(runs("0 0 * * 0", "UTC", "2026-11-22T12:00:00Z", 2), sundays);
    assert_eq!(
        runs("0 0 * * SUN", "UTC", "2026-11-22T12:00:00Z", 2),
        sundays
    );
}

#[test]
fn tc_07_invalid_cron_expressions_are_explained() {
    let out_of_range = CronSpec::parse("61 * * * *").unwrap_err();
    assert_eq!(out_of_range.code(), ErrorCode::InvalidValue);
    assert!(
        out_of_range.message.contains("0-59"),
        "{}",
        out_of_range.message
    );

    let four_fields = CronSpec::parse("* * * *").unwrap_err();
    assert_eq!(four_fields.code(), ErrorCode::InvalidValue);
    assert!(
        four_fields.message.contains("five fields"),
        "{}",
        four_fields.message
    );
    assert!(
        four_fields.message.contains("*/10 * * * *"),
        "{}",
        four_fields.message
    );

    let never = TriggerDef::cron("0 0 31 2 *", "UTC", "UTC", at(T0)).unwrap_err();
    assert_eq!(never.code(), ErrorCode::InvalidValue);
    assert!(never.message.contains("five years"), "{}", never.message);
}

#[test]
fn tc_08_next_run_is_strictly_later() {
    assert_eq!(
        runs("*/10 * * * *", "UTC", "2026-11-22T14:40:00Z", 1),
        instants(&["2026-11-22T14:50:00Z"])
    );
}

#[test]
fn tc_29_cron_in_warsaw_runs_once_on_both_changes_of_time() {
    assert_eq!(
        runs("30 2 * * *", "Europe/Warsaw", "2027-03-27T12:00:00Z", 2),
        instants(&["2027-03-28T01:30:00Z", "2027-03-29T00:30:00Z"])
    );
    assert_eq!(
        runs("30 2 * * *", "Europe/Warsaw", "2027-10-30T12:00:00Z", 2),
        instants(&["2027-10-31T00:30:00Z", "2027-11-01T01:30:00Z"])
    );
}

#[test]
fn cron_dst_gap_shifted() {
    assert_eq!(
        runs("15 2 * * *", "Europe/Warsaw", "2027-03-28T00:00:00Z", 1),
        instants(&["2027-03-28T01:15:00Z"])
    );
    assert_eq!(
        runs("30 2 * * *", "America/New_York", "2027-03-14T00:00:00Z", 1),
        instants(&["2027-03-14T07:30:00Z"])
    );
}

#[test]
fn cron_dst_overlap_once() {
    assert_eq!(
        runs("*/30 2 * * *", "Europe/Warsaw", "2027-10-30T23:00:00Z", 3),
        instants(&[
            "2027-10-31T00:00:00Z",
            "2027-10-31T00:30:00Z",
            "2027-11-01T01:00:00Z"
        ])
    );
}

#[test]
fn every_min_interval_rejected() {
    let settings = ScheduleSettings::default();
    let error = TriggerDef::every(Duration::from_secs(29), "UTC", "UTC", &settings).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert!(error.message.contains("30s"), "{}", error.message);
    assert!(TriggerDef::every(Duration::from_secs(30), "UTC", "UTC", &settings).is_ok());
}

#[test]
fn at_past_rejected() {
    let now = at(T0);
    let past = TriggerDef::at(at("2026-11-01T00:00:00Z"), "UTC", "UTC", now).unwrap_err();
    assert_eq!(past.code(), ErrorCode::InvalidValue);
    assert!(past.message.contains("already passed"), "{}", past.message);
    assert!(TriggerDef::at(now, "UTC", "UTC", now).is_err());
    assert!(TriggerDef::at(add(now, minutes(10)), "UTC", "UTC", now).is_ok());
}

#[test]
fn cron_shortcuts_expand_and_reboot_is_rejected() {
    assert_eq!(
        runs("@daily", "UTC", T0, 1),
        instants(&["2026-11-23T00:00:00Z"])
    );
    assert_eq!(
        runs("@HOURLY", "UTC", T0, 1),
        instants(&["2026-11-22T15:00:00Z"])
    );
    assert_eq!(
        runs("@weekly", "UTC", T0, 1),
        instants(&["2026-11-29T00:00:00Z"])
    );
    assert_eq!(
        runs("@monthly", "UTC", T0, 1),
        instants(&["2026-12-01T00:00:00Z"])
    );
    assert_eq!(
        runs("@yearly", "UTC", T0, 1),
        instants(&["2027-01-01T00:00:00Z"])
    );
    let reboot = CronSpec::parse("@reboot").unwrap_err();
    assert_eq!(reboot.code(), ErrorCode::InvalidValue);
    assert_eq!(reboot.found.as_deref(), Some("@reboot"));
}

#[test]
fn cron_rejects_zero_step_decreasing_range_names_and_long_text() {
    for expression in [
        "*/0 * * * *",
        "30-10 * * * *",
        "* * * * funday",
        "* 24 * * *",
        "* * 0 * *",
        "* * * 13 *",
        "* * * * 8",
        "L * * * *",
    ] {
        let error = CronSpec::parse(expression).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue, "{expression}");
        assert_eq!(error.found.as_deref(), Some(expression));
    }
    let long = format!("0 0 * * {}", "1,".repeat(100));
    let error = CronSpec::parse(&long).unwrap_err();
    assert!(error.message.contains("200"), "{}", error.message);
}

#[test]
fn unknown_timezone_offers_similar_names() {
    let error = TriggerDef::cron("0 3 * * *", "Europe/Warsw", "UTC", at(T0)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert_eq!(error.found.as_deref(), Some("Europe/Warsw"));
    let first = error.candidates.first().expect("a candidate");
    assert_eq!(first.program, "timezone: \"Europe/Warsaw\"");
    assert_eq!(first.rank, 1);
}

#[test]
fn local_timezone_is_saved_as_host_zone() {
    let settings = ScheduleSettings::default();
    let saved = TriggerDef::every(minutes(10), "local", "Europe/Warsaw", &settings).unwrap();
    assert_eq!(saved.timezone, "Europe/Warsaw");
    let without_host_zone = TriggerDef::every(minutes(10), "LOCAL", "", &settings).unwrap();
    assert_eq!(without_host_zone.timezone, "UTC");
    let explicit = TriggerDef::cron("0 3 * * *", "Asia/Tokyo", "Europe/Warsaw", at(T0)).unwrap();
    assert_eq!(explicit.timezone, "Asia/Tokyo");
}

#[test]
fn normalized_forms_name_the_kind() {
    let cron = TriggerDef::cron(" */10 * * * * ", "UTC", "UTC", at(T0)).unwrap();
    assert_eq!(cron.normalized(), "cron */10 * * * *");
    let single =
        TriggerDef::at(at("2026-11-23T02:00:00Z"), "Europe/Warsaw", "UTC", at(T0)).unwrap();
    assert_eq!(single.normalized(), "at 2026-11-23T03:00:00+01:00");
    let unreadable = TriggerDef {
        kind: TriggerKind::At,
        expression: "soon".to_owned(),
        timezone: "UTC".to_owned(),
    };
    assert_eq!(unreadable.normalized(), "at soon");
    assert_eq!(
        format_local(at("2026-07-01T10:00:00Z"), "Europe/Warsaw").as_deref(),
        Some("2026-07-01T12:00:00+02:00")
    );
    assert_eq!(format_local(at(T0), "Nowhere/City"), None);
}

#[test]
fn every_counts_from_creation_then_from_now_after_a_pause() {
    let trigger = every(minutes(10));
    let created = at(T0);
    assert_eq!(
        trigger.first_run(created, created),
        Some(at("2026-11-22T14:40:00Z"))
    );
    assert_eq!(
        trigger.first_run(created, at("2026-11-22T15:00:00Z")),
        Some(at("2026-11-22T15:10:00Z"))
    );
    assert_eq!(
        trigger.following(at("2026-11-22T14:40:00Z"), at("2026-11-22T14:40:01Z")),
        Some(at("2026-11-22T14:50:00Z"))
    );
    assert_eq!(
        trigger.following(at("2026-11-22T14:40:00Z"), at("2026-11-22T15:07:00Z")),
        Some(at("2026-11-22T15:17:00Z"))
    );
    assert!(!trigger.is_single());
}

#[test]
fn single_time_runs_once() {
    let def = TriggerDef::at(at("2026-11-22T14:40:00Z"), "UTC", "UTC", at(T0)).unwrap();
    let trigger = Trigger::compile(&def).unwrap();
    assert!(trigger.is_single());
    assert_eq!(
        trigger.first_run(at(T0), at(T0)),
        Some(at("2026-11-22T14:40:00Z"))
    );
    assert_eq!(trigger.next_after(at(T0)), Some(at("2026-11-22T14:40:00Z")));
    assert_eq!(trigger.next_after(at("2026-11-22T14:40:00Z")), None);
    assert_eq!(trigger.following(at("2026-11-22T14:40:00Z"), at(T0)), None);
}

#[test]
fn cron_first_run_counts_from_the_later_of_creation_and_now() {
    let def = TriggerDef::cron("0 * * * *", "UTC", "UTC", at(T0)).unwrap();
    let trigger = Trigger::compile(&def).unwrap();
    assert_eq!(
        trigger.first_run(at(T0), at(T0)),
        Some(at("2026-11-22T15:00:00Z"))
    );
    assert_eq!(
        trigger.first_run(at(T0), at("2026-11-22T16:10:00Z")),
        Some(at("2026-11-22T17:00:00Z"))
    );
    assert_eq!(
        trigger.following(at("2026-11-22T15:00:00Z"), at("2026-11-22T15:00:30Z")),
        Some(at("2026-11-22T16:00:00Z"))
    );
}

#[test]
fn saved_triggers_compile_or_report_the_fault() {
    let interval = TriggerDef {
        kind: TriggerKind::Every,
        expression: "1h30m".to_owned(),
        timezone: "UTC".to_owned(),
    };
    let trigger = Trigger::compile(&interval).unwrap();
    assert_eq!(trigger.next_after(at(T0)), Some(at("2026-11-22T16:00:00Z")));

    for (kind, expression) in [
        (TriggerKind::Every, "10x"),
        (TriggerKind::Every, "m"),
        (TriggerKind::At, "tomorrow"),
        (TriggerKind::Cron, "* *"),
    ] {
        let def = TriggerDef {
            kind,
            expression: expression.to_owned(),
            timezone: "UTC".to_owned(),
        };
        let error = Trigger::compile(&def).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue, "{expression}");
    }
    let unknown_zone = TriggerDef {
        kind: TriggerKind::Cron,
        expression: "0 3 * * *".to_owned(),
        timezone: "Nowhere/City".to_owned(),
    };
    assert_eq!(
        Trigger::compile(&unknown_zone).unwrap_err().code(),
        ErrorCode::InvalidValue
    );
}

#[test]
fn durations_are_written_in_the_largest_units() {
    assert_eq!(format_duration(Duration::from_secs(5_400)), "1h30m");
    assert_eq!(format_duration(Duration::from_secs(86_400 + 1)), "1d1s");
    assert_eq!(format_duration(Duration::from_millis(1_250)), "1s250ms");
    assert_eq!(format_duration(Duration::ZERO), "0ms");
}

#[test]
fn time_helpers_saturate() {
    let start = at(T0);
    assert_eq!(add(start, minutes(10)), at("2026-11-22T14:40:00Z"));
    assert_eq!(since(at("2026-11-22T14:40:00Z"), start), minutes(10));
    assert_eq!(since(start, at("2026-11-22T14:40:00Z")), Duration::ZERO);
    let wall = WallTime::from_unix_nanos(i128::from(start.as_unix_nanos()));
    assert_eq!(instant_from_wall(wall), start);
    let far = WallTime::from_unix_nanos(i128::MAX);
    assert_eq!(instant_from_wall(far).as_unix_nanos(), i64::MAX);
}

#[test]
fn a_saved_interval_of_zero_does_not_compile() {
    for expression in ["", "0s", "0ms", "0d0h"] {
        let def = TriggerDef {
            kind: TriggerKind::Every,
            expression: expression.to_owned(),
            timezone: "UTC".to_owned(),
        };
        let error = Trigger::compile(&def).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue, "{expression}");
    }
    let valid = TriggerDef {
        kind: TriggerKind::Every,
        expression: "1ms".to_owned(),
        timezone: "UTC".to_owned(),
    };
    assert!(Trigger::compile(&valid).is_ok());
}

#[test]
fn a_saved_interval_with_a_multibyte_unit_is_invalid() {
    let def = TriggerDef {
        kind: TriggerKind::Every,
        expression: "5ł".to_owned(),
        timezone: "UTC".to_owned(),
    };
    assert!(Trigger::compile(&def).is_err());
}
