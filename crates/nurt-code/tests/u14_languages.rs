// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-code/tests/u14_languages.rs
// Responsibility:   Tests of `code.languages`: grammar languages, configured servers and the.
// Dependencies:     nurt_contracts, u14_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests of `code.languages`: grammar languages, configured servers and the
//! state of the server pool.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod u14_support;

use std::path::PathBuf;
use std::sync::Arc;

use nurt_code::commands::languages::CodeLanguages;
use nurt_code::semantic::ServerStatus;
use nurt_contracts::config::{Config, ServerAllow, ServerAllowMode};
use nurt_contracts::record::Language;
use nurt_contracts::services::LanguageId;
use nurt_contracts::stage::{BoundArgs, StageImpl, StageInput};

use u14_support::{ScriptedProvider, context};

fn languages(stage: &CodeLanguages, config: Config) -> Vec<Language> {
    let ctx = context(config);
    let args = BoundArgs::new("code.languages");
    let output = stage
        .as_read()
        .unwrap()
        .execute(&args, StageInput::none(), &ctx)
        .unwrap();
    StageInput::from(output)
        .into_records()
        .unwrap()
        .map(|record| Language::try_from(record.unwrap()).unwrap())
        .collect()
}

fn find<'a>(records: &'a [Language], name: &str) -> &'a Language {
    records
        .iter()
        .find(|record| record.language == name)
        .unwrap()
}

#[test]
fn running_server_is_reported_with_its_process_and_root() {
    let provider = ScriptedProvider {
        statuses: vec![ServerStatus {
            language: LanguageId::new("python").unwrap(),
            server: Some("pyright-langserver".to_owned()),
            found: true,
            allowed: true,
            running: true,
            pid: Some(4242),
            root: Some(PathBuf::from("/work/app")),
            loading: false,
        }],
        ..ScriptedProvider::default()
    };
    let records = languages(
        &CodeLanguages::new(Some(Arc::new(provider))),
        Config::default(),
    );
    let python = find(&records, "python");
    assert_eq!(python.extensions, vec!["py".to_owned()]);
    assert_eq!(python.grammar_version.as_deref(), Some("0.1.0"));
    assert_eq!(python.server.as_deref(), Some("pyright-langserver"));
    assert!(python.server_found);
    assert_eq!(python.server_running, Some(true));
    assert_eq!(python.server_pid, Some(4242));
    assert_eq!(
        python.server_root.as_ref().map(|root| root.as_str()),
        Some("/work/app")
    );
}

#[test]
fn server_not_allowed_is_not_reported_as_found() {
    let provider = ScriptedProvider {
        statuses: vec![ServerStatus {
            language: LanguageId::new("python").unwrap(),
            server: Some("pyright-langserver".to_owned()),
            found: true,
            allowed: false,
            running: false,
            pid: None,
            root: None,
            loading: false,
        }],
        ..ScriptedProvider::default()
    };
    let records = languages(
        &CodeLanguages::new(Some(Arc::new(provider))),
        Config::default(),
    );
    assert!(!find(&records, "python").server_found);
    assert_eq!(find(&records, "python").server_running, Some(false));
}

#[test]
fn without_a_provider_configured_servers_are_listed_as_not_found() {
    let records = languages(&CodeLanguages::new(None), Config::default());
    let typescript = find(&records, "typescript");
    assert_eq!(
        typescript.server.as_deref(),
        Some("typescript-language-server")
    );
    assert!(!typescript.server_found);
    assert_eq!(typescript.server_running, None);
    let names: Vec<&str> = records
        .iter()
        .map(|record| record.language.as_str())
        .collect();
    assert_eq!(&names[..3], &["python", "typescript", "javascript"]);
    for configured in ["c", "cpp", "go", "rust"] {
        let record = find(&records, configured);
        assert!(record.extensions.is_empty());
        assert!(record.grammar_version.is_none());
        assert!(record.server.is_some());
    }
}

#[test]
fn servers_allow_off_hides_the_configured_programs() {
    let mut config = Config::default();
    config.code.servers_allow = ServerAllow::Mode(ServerAllowMode::Off);
    let records = languages(&CodeLanguages::new(None), config);
    assert!(records.iter().all(|record| record.server.is_none()));
}

#[test]
fn servers_allow_list_keeps_only_the_listed_languages() {
    let mut config = Config::default();
    config.code.servers_allow = ServerAllow::Languages(vec!["go".to_owned()]);
    let records = languages(&CodeLanguages::new(None), config);
    assert_eq!(find(&records, "go").server.as_deref(), Some("gopls"));
    assert!(find(&records, "python").server.is_none());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
