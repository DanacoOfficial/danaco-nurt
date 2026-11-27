// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           tests/d2/retention/main.rs
// Responsibility:   Retention scenario tests of the journal.
// Dependencies:     harness, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Retention scenario tests of the journal.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions report failures by panicking"
)]

#[allow(
    dead_code,
    reason = "the harness is shared by several test targets, each of which uses a subset"
)]
#[path = "../coverage/harness/mod.rs"]
mod harness;
mod leak;
mod precedence;
mod scenarios;
mod support;

use std::time::Duration;

use nurt_contracts::env::Clock;

use harness::clock::{START, VirtualClock};

#[test]
fn virtual_clock_counts_suspension_on_the_wall_clock_only() {
    let clock = VirtualClock::new();
    clock.advance(Duration::from_secs(60));
    clock.suspend(Duration::from_secs(3600));
    let stamp = clock.stamp();
    assert_eq!(stamp.wall, START.saturating_add(Duration::from_secs(3660)));
    assert_eq!(stamp.since_boot_with_suspend, Duration::from_secs(3660));
    assert_eq!(stamp.since_boot_active, Duration::from_secs(60));
}

#[test]
fn virtual_clock_reboot_starts_new_boot_at_zero() {
    let clock = VirtualClock::new();
    let first = clock.boot_id();
    clock.advance(Duration::from_secs(10));
    clock.reboot(Duration::from_secs(2400));
    let stamp = clock.stamp();
    assert_ne!(stamp.boot, first);
    assert_eq!(stamp.since_boot_active, Duration::ZERO);
    assert_eq!(stamp.wall, START.saturating_add(Duration::from_secs(2410)));
}

#[test]
fn virtual_clock_wall_steps_leave_boot_clocks_unchanged() {
    let clock = VirtualClock::new();
    clock.step_wall_back(Duration::from_secs(3600));
    clock.step_wall_forward(Duration::from_secs(7200));
    let stamp = clock.stamp();
    assert_eq!(stamp.wall, START.saturating_add(Duration::from_secs(3600)));
    assert_eq!(stamp.since_boot_with_suspend, Duration::ZERO);
    assert_eq!(clock.monotonic(), Duration::ZERO);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
