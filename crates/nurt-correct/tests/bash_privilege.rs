// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Translation of `sudo`, `doas`, `run0` and `su`: the program that follows is
//! translated and the translation carries the call parameters `privilege` and
//! `as_user`.

// Test failures report through panics.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_contracts::error::{Error, ErrorCode};
use nurt_correct::bash::{Mode, TranslateContext, Translation, translate_with};

fn live(input: &str) -> Result<Translation, Error> {
    translate_with(input, &TranslateContext::default()).expect("a bash habit")
}

#[test]
fn sudo_translates_the_command_with_elevated_privilege() {
    let translation = live("sudo rm /etc/app.conf").unwrap();
    assert_eq!(
        translation.text(),
        "remove \"/etc/app.conf\" on_missing: error privilege: elevated"
    );
    assert!(
        translation
            .rules
            .iter()
            .any(|rule| rule.rule.to_string() == "B28")
    );
}

#[test]
fn a_command_without_sudo_asks_for_no_privilege() {
    let translation = live("rm -rf /etc/app.d").unwrap();
    assert_eq!(translation.text(), "remove \"/etc/app.d\"");
}

#[test]
fn sudo_translates_a_service_change() {
    let translation = live("sudo systemctl restart nginx").unwrap();
    assert_eq!(
        translation.text(),
        "services name: \"nginx\"\n|> restart privilege: elevated"
    );
}

#[test]
fn user_options_become_as_user() {
    for line in [
        "sudo -u deploy rm /srv/x",
        "sudo --user deploy rm /srv/x",
        "sudo --user=deploy rm /srv/x",
        "doas -u deploy rm /srv/x",
        "run0 --user=deploy rm /srv/x",
    ] {
        let translation = live(line).unwrap_or_else(|error| panic!("{line}: {error:?}"));
        assert_eq!(
            translation.text(),
            "remove \"/srv/x\" on_missing: error as_user: \"deploy\"",
            "{line}"
        );
    }
}

#[test]
fn doas_and_run0_translate_like_sudo() {
    for line in [
        "doas rm /srv/x",
        "run0 rm /srv/x",
        "sudo -n rm /srv/x",
        "sudo -- rm /srv/x",
    ] {
        let translation = live(line).unwrap_or_else(|error| panic!("{line}: {error:?}"));
        assert_eq!(
            translation.text(),
            "remove \"/srv/x\" on_missing: error privilege: elevated",
            "{line}"
        );
    }
}

#[test]
fn a_program_without_a_counterpart_runs_with_elevated_privilege() {
    let translation = live("cd /srv/app && sudo ./deploy.sh --fast").unwrap();
    assert_eq!(
        translation.text(),
        "run [\"./deploy.sh\", \"--fast\"] cwd: \"/srv/app\" privilege: elevated"
    );
}

#[test]
fn only_the_statement_with_sudo_is_elevated() {
    let translation = translate_with(
        "sudo rm /srv/a\nrm /srv/b",
        &TranslateContext {
            base: None,
            lets: &[],
            mode: Mode::Script,
        },
    )
    .expect("a script")
    .unwrap();
    assert_eq!(
        translation.text(),
        "remove \"/srv/a\" on_missing: error privilege: elevated\nremove \"/srv/b\" on_missing: error"
    );
}

#[test]
fn an_elevated_session_is_ambiguous_and_names_the_next_step() {
    for line in [
        "sudo -i",
        "sudo -s",
        "su",
        "su -",
        "sudo",
        "doas -s",
        "sudo --login",
    ] {
        let error = live(line).expect_err(line);
        assert_eq!(error.code(), ErrorCode::Ambiguous, "{line}");
        assert_eq!(
            error.data().problem.as_deref(),
            Some("elevated_session"),
            "{line}"
        );
        assert!(
            error
                .data()
                .next_steps
                .iter()
                .any(|step| step.contains("privilege: elevated")),
            "{line}"
        );
    }
}

#[test]
fn an_option_without_a_translation_is_reported() {
    let error = live("sudo -g wheel rm /srv/x").expect_err("a group has no translation");
    assert_eq!(error.code(), ErrorCode::Uncorrectable);
    assert_eq!(error.data().problem.as_deref(), Some("elevated_option"));
}

#[test]
fn the_error_of_the_translated_command_is_kept() {
    let error = live("sudo for f in /srv/*; do rm $f; done").expect_err("loops stay rejected");
    assert_eq!(error.code(), ErrorCode::Uncorrectable);
}

#[test]
fn options_that_change_the_run_are_not_dropped() {
    for line in [
        "sudo -E rm /srv/x",
        "sudo -b rm /srv/x",
        "sudo --preserve-env rm /srv/x",
    ] {
        let error = live(line).expect_err(line);
        assert_eq!(
            error.data().problem.as_deref(),
            Some("elevated_option"),
            "{line}"
        );
    }
}

#[test]
fn an_account_that_is_not_a_literal_is_not_guessed() {
    for line in ["sudo -u $USER rm /srv/x", "sudo -u $(whoami) rm /srv/x"] {
        let error = live(line).expect_err(line);
        assert_eq!(
            error.data().problem.as_deref(),
            Some("elevated_option"),
            "{line}"
        );
    }
}
