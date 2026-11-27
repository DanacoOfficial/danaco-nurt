// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `run ["sudo", ...]` written in Nurt is corrected into the same `run`
//! without the elevation program and with `privilege: elevated`.

// Test failures report through panics.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod bind_support;

use bind_support::catalog;
use nurt_bind::{BindContext, Outcome, bind_with_correction};
use nurt_contracts::config::Config;
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::spec::Os;
use nurt_correct::tables::load_embedded;

fn corrected(text: &str) -> Outcome {
    let tables = load_embedded().unwrap();
    let context = BindContext::new(Os::Linux).with_corr_rev(tables.corr_rev());
    bind_with_correction(
        text,
        SyntaxVariant::A,
        catalog(),
        &tables,
        &Config::default(),
        &context,
    )
    .unwrap()
}

#[test]
fn sudo_in_a_run_list_becomes_privilege_elevated() {
    let outcome = corrected("run [\"sudo\", \"systemctl\", \"restart\", \"svc\"] cwd: \"/\"");
    assert_eq!(
        outcome.canonical,
        "run [\"systemctl\", \"restart\", \"svc\"] cwd: \"/\" privilege: elevated"
    );
    assert_eq!(outcome.corrections.len(), 1);
    assert_eq!(outcome.corrections[0].rule.to_string(), "B28");
}

#[test]
fn user_option_becomes_as_user_and_noninteractive_flags_are_dropped() {
    let outcome =
        corrected("run [\"sudo\", \"-n\", \"-u\", \"deploy\", \"--\", \"id\"] cwd: \"/\"");
    assert_eq!(
        outcome.canonical,
        "run [\"id\"] cwd: \"/\" as_user: \"deploy\""
    );
}

#[test]
fn doas_and_run0_are_corrected_too() {
    for program in ["doas", "run0", "/usr/bin/sudo"] {
        let outcome = corrected(&format!("run [\"{program}\", \"id\"] cwd: \"/\""));
        assert!(
            outcome.canonical.ends_with("privilege: elevated"),
            "{program}"
        );
    }
}

#[test]
fn a_run_without_the_elevation_program_is_left_alone() {
    let outcome = corrected("run [\"id\"] cwd: \"/\"");
    assert!(outcome.corrections.is_empty());
}

#[test]
fn a_device_node_accepts_the_elevated_call_parameters() {
    let outcome = corrected(
        "create \"/dev/null2\" type: device kind: char major: 1 minor: 3 privilege: elevated",
    );
    assert!(
        outcome.canonical.ends_with("privilege: elevated"),
        "{}",
        outcome.canonical
    );
    assert!(outcome.corrections.is_empty());
}

#[test]
fn a_bash_mknod_under_sudo_keeps_working_through_run() {
    let outcome =
        corrected("run [\"sudo\", \"mknod\", \"/dev/x\", \"c\", \"1\", \"3\"] cwd: \"/\"");
    assert!(outcome.canonical.ends_with("privilege: elevated"));
}
