// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The call parameters `privilege` and `as_user`, the report fields of an
//! elevated operation and the configuration section `[privilege]`.

use nurt_contracts::config::{Config, ConfigLevel, ConfigNoteKind, KeyAccess, allowed_at};
use nurt_contracts::envelope::{EnvelopeError, Privilege, Request, RequestOptions};
use nurt_contracts::report::{Report, ReportStatus};
use serde_json::json;

#[test]
fn privilege_values_use_their_codes() {
    let codes: Vec<&str> = Privilege::ALL.iter().map(|value| value.as_str()).collect();
    assert_eq!(codes, ["account", "elevated", "auto"]);
    for value in Privilege::ALL {
        let text = serde_json::to_string(value).unwrap();
        assert_eq!(text, format!("\"{}\"", value.as_str()));
        let back: Privilege = serde_json::from_str(&text).unwrap();
        assert_eq!(back, *value);
    }
}

#[test]
fn absent_privilege_is_the_account() {
    let options = RequestOptions::default();
    assert_eq!(options.privilege(), Privilege::Account);
    assert_eq!(options.effective_privilege(), Privilege::Account);
    assert!(options.is_empty());
}

#[test]
fn as_user_implies_elevation_for_the_switch() {
    let options = RequestOptions {
        as_user: Some("deploy".to_owned()),
        ..RequestOptions::default()
    };
    assert_eq!(options.effective_privilege(), Privilege::Elevated);
    let auto = RequestOptions {
        privilege: Some(Privilege::Auto),
        as_user: Some("deploy".to_owned()),
        ..RequestOptions::default()
    };
    assert_eq!(auto.effective_privilege(), Privilege::Auto);
}

#[test]
fn unknown_privilege_value_is_rejected() {
    let value =
        json!({"nurt": "1.0", "program": "list \"/etc\"", "options": {"privilege": "root"}});
    assert!(serde_json::from_value::<Request>(value).is_err());
}

#[test]
fn privilege_and_as_user_round_trip_in_a_request() {
    let value = json!({
        "nurt": "1.0",
        "program": "write \"/etc/hosts\" \"x\"",
        "options": {"privilege": "elevated", "as_user": "deploy"}
    });
    let request: Request = serde_json::from_value(value).unwrap();
    assert_eq!(request.options.privilege, Some(Privilege::Elevated));
    assert_eq!(request.options.as_user.as_deref(), Some("deploy"));
    request.validate().unwrap();
}

#[test]
fn empty_as_user_is_rejected() {
    let options = RequestOptions {
        as_user: Some(String::new()),
        ..RequestOptions::default()
    };
    assert_eq!(
        options.validate(),
        Err(EnvelopeError::EmptyField { field: "as_user" })
    );
}

#[test]
fn report_omits_the_elevation_fields_until_elevated() {
    let report = Report::new(ReportStatus::Ok, "write");
    let value = serde_json::to_value(&report).unwrap();
    assert!(value.get("elevated").is_none());
    assert!(value.get("backend").is_none());
    let mut elevated = report;
    elevated.elevated = true;
    elevated.backend = Some("sudo".to_owned());
    elevated.as_user = Some("deploy".to_owned());
    let value = serde_json::to_value(&elevated).unwrap();
    assert_eq!(value["elevated"], json!(true));
    assert_eq!(value["backend"], json!("sudo"));
    assert_eq!(value["as_user"], json!("deploy"));
    let back: Report = serde_json::from_value(value).unwrap();
    assert_eq!(back, elevated);
}

#[test]
fn configuration_defaults_allow_elevation_without_changing_the_default() {
    let config = Config::default();
    assert_eq!(config.privilege.default, Privilege::Account);
    assert!(config.privilege.allowed);
    assert!(config.privilege.backends.is_none());
    assert!(config.privilege.group.is_none());
}

#[test]
fn a_project_file_cannot_set_privilege_keys() {
    for key in [
        "privilege.default",
        "privilege.allowed",
        "privilege.backends",
    ] {
        assert_eq!(
            allowed_at(ConfigLevel::Project, key),
            KeyAccess::Ignored(ConfigNoteKind::TrustConfigIgnored),
            "{key}"
        );
        assert_eq!(
            allowed_at(ConfigLevel::User, key),
            KeyAccess::Allowed,
            "{key}"
        );
        assert_eq!(
            allowed_at(ConfigLevel::Runtime, key),
            KeyAccess::Allowed,
            "{key}"
        );
    }
}

#[test]
fn an_account_name_that_looks_like_an_option_is_rejected() {
    for name in ["-root", "#0", "a b", "a=b", "a\nb"] {
        let options = RequestOptions {
            as_user: Some(name.to_owned()),
            ..RequestOptions::default()
        };
        assert!(
            matches!(
                options.validate(),
                Err(EnvelopeError::InvalidAccount { .. })
            ),
            "{name:?}"
        );
    }
}
