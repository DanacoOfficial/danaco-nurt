// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-contracts/tests/services_edge.rs
// Responsibility:   Boundaries of the shared services: identifiers, byte ranges, the registry and the serialized names of the data types.
// Dependencies:     serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Boundaries of the shared services: identifiers, byte ranges, the registry
//! and the serialized names of the data types.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::path::Path;
use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::services::{
    ByteRange, ChildExit, DocFormatId, ExcludePreset, FileHolder, FileHolderService,
    FileListService, GitChange, InvalidServiceId, IssueSeverity, LanguageId, ProgramInput,
    ProgramOutcome, ProgramSpec, ReferenceRole, ReviewRule, ReviewRuleProvider, RuleSeverity,
    ServiceKind, Services, SpanClass, SymbolKind, TextEncoding, WalkIter, WalkKind, WalkSpec,
};
use serde_json::json;

struct Holders(u32);

impl FileHolderService for Holders {
    fn holders(&self, _path: &Path) -> Result<Vec<FileHolder>> {
        Ok(vec![FileHolder {
            pid: self.0,
            name: "editor".to_owned(),
            kind: Some("application".to_owned()),
        }])
    }
}

struct NoFiles;

impl FileListService for NoFiles {
    fn walk<'a>(&'a self, _spec: &WalkSpec) -> Result<WalkIter<'a>> {
        Ok(Box::new(std::iter::empty()))
    }
}

struct NoRules;

impl ReviewRuleProvider for NoRules {
    fn rules(&self) -> Vec<ReviewRule> {
        Vec::new()
    }
}

#[test]
fn byte_ranges_do_not_overflow_at_the_extremes() {
    let all = ByteRange::new(0, u64::MAX);
    assert_eq!(all.len(), u64::MAX);
    assert!(all.contains(0));
    assert!(all.contains(u64::MAX - 1));
    assert!(!all.contains(u64::MAX));
    assert!(!all.is_empty());

    let reversed = ByteRange::new(u64::MAX, 0);
    assert_eq!(reversed.len(), 0);
    assert!(reversed.is_empty());
    assert!(!reversed.contains(0));
    assert!(!reversed.contains(u64::MAX));

    let single = ByteRange::new(7, 8);
    assert_eq!(single.len(), 1);
    assert!(single.contains(7));
    assert!(!single.contains(6) && !single.contains(8));

    assert_eq!(ByteRange::default(), ByteRange::new(0, 0));
    assert!(ByteRange::default().is_empty());
}

#[test]
fn identifiers_accept_up_to_sixty_four_bytes() {
    let longest = format!("a{}", "b".repeat(63));
    assert_eq!(longest.len(), 64);
    assert_eq!(LanguageId::new(longest.clone()).unwrap().as_str(), longest);
    assert!(DocFormatId::new(format!("{longest}b")).is_err());
}

#[test]
fn identifiers_start_with_a_lowercase_letter() {
    for valid in ["a", "c++", "objective-c", "x_1", "a+-_9"] {
        assert!(LanguageId::new(valid).is_ok(), "{valid}");
    }
    for invalid in [
        "", "A", "+c", "-c", "_c", "9c", " c", "c ", "c.d", "c/d", "c\n", "\u{e9}", "c\u{e9}",
        "\u{ff41}", "a\0",
    ] {
        let error = LanguageId::new(invalid).unwrap_err();
        assert_eq!(error.value, invalid);
        assert!(error.to_string().contains("not a valid identifier"));
    }
}

#[test]
fn rejected_identifiers_are_reported_with_their_text() {
    let error: InvalidServiceId = "Bad Id".parse::<LanguageId>().unwrap_err();
    assert_eq!(error.value, "Bad Id");
    assert!(
        error
            .to_string()
            .starts_with("`Bad Id` is not a valid identifier")
    );
    assert!(serde_json::from_value::<DocFormatId>(json!(5)).is_err());
    assert!(serde_json::from_value::<DocFormatId>(json!("")).is_err());
}

#[test]
fn identifiers_compare_and_order_by_text() {
    let json = DocFormatId::new("json").unwrap();
    let jsonc = DocFormatId::new("jsonc").unwrap();
    assert_eq!(json, DocFormatId::JSON);
    assert!(json < jsonc);
    assert_eq!(json.to_string(), "json");
    let mut formats = [DocFormatId::YAML, DocFormatId::JSON, DocFormatId::TOML];
    formats.sort();
    let names: Vec<&str> = formats.iter().map(DocFormatId::as_str).collect();
    assert_eq!(names, ["json", "toml", "yaml"]);
}

#[test]
fn built_in_formats_are_distinct_valid_identifiers() {
    let names: Vec<&str> = DocFormatId::BUILT_IN
        .iter()
        .map(DocFormatId::as_str)
        .collect();
    assert_eq!(
        names,
        [
            "json",
            "jsonc",
            "yaml",
            "toml",
            "xml",
            "ini",
            "csv",
            "tsv",
            "properties",
            "env",
            "plist"
        ]
    );
    for format in &DocFormatId::BUILT_IN {
        assert_eq!(&DocFormatId::new(format.as_str()).unwrap(), format);
    }
}

#[test]
fn the_registry_keeps_the_latest_registration_of_a_service() {
    let services = Services::new()
        .with_file_holders(Arc::new(Holders(1)))
        .with_file_holders(Arc::new(Holders(2)));
    let holders = services
        .file_holders()
        .unwrap()
        .holders(Path::new("/home/user/a.txt"))
        .unwrap();
    assert_eq!(holders[0].pid, 2);
    assert_eq!(holders[0].kind.as_deref(), Some("application"));
}

#[test]
fn clones_of_the_registry_share_the_services() {
    let services = Services::new().with_file_holders(Arc::new(Holders(9)));
    let clone = services.clone();
    let pid = |services: &Services| {
        services
            .file_holders()
            .unwrap()
            .holders(Path::new("/x"))
            .unwrap()[0]
            .pid
    };
    assert_eq!(pid(&services), pid(&clone));
    assert!(clone.has(ServiceKind::FileHolders));
    assert!(!clone.has(ServiceKind::Doc));
}

#[test]
fn the_registry_lists_services_in_declaration_order() {
    let services = Services::new()
        .with_review_rules(Arc::new(NoRules))
        .with_file_holders(Arc::new(Holders(1)))
        .with_file_list(Arc::new(NoFiles));
    assert_eq!(
        services.available(),
        [
            ServiceKind::FileList,
            ServiceKind::FileHolders,
            ServiceKind::ReviewRules
        ]
    );
    assert_eq!(
        format!("{services:?}"),
        "Services { available: [FileList, FileHolders, ReviewRules] }"
    );
    assert!(services.review_rules().unwrap().rules().is_empty());
    assert!(services.file_list().is_some());
    assert!(services.git_query().is_none());
}

#[test]
fn service_kind_names_follow_the_wording_of_the_documentation() {
    let names: Vec<String> = ServiceKind::ALL.iter().map(ToString::to_string).collect();
    assert_eq!(
        names,
        [
            "doc",
            "code",
            "process_host",
            "file_list",
            "git_query",
            "text_codec",
            "file_holders",
            "review_rules"
        ]
    );
}

#[test]
fn program_specifications_start_with_the_configured_defaults() {
    let spec = ProgramSpec::new("make", vec!["test".to_owned()], "/home/user/project");
    assert_eq!(spec.program, "make");
    assert_eq!(spec.args, ["test"]);
    assert_eq!(spec.cwd, Path::new("/home/user/project"));
    assert!(spec.env.is_empty());
    assert!(spec.inherit_env);
    assert_eq!(spec.stdin, ProgramInput::Null);
    assert_eq!(spec.timeout_ms, None);
    assert_eq!(spec.output_limit, None);
    assert!(!spec.detect_changes);
}

#[test]
fn program_input_is_null_or_bytes() {
    assert_eq!(ProgramInput::default(), ProgramInput::Null);
    assert_eq!(
        serde_json::to_value(ProgramInput::Null).unwrap(),
        json!("null")
    );
    let bytes = ProgramInput::Bytes(vec![104, 105]);
    let value = serde_json::to_value(&bytes).unwrap();
    assert_eq!(value, json!({"bytes": [104, 105]}));
    assert_eq!(
        serde_json::from_value::<ProgramInput>(value).unwrap(),
        bytes
    );
}

#[test]
fn an_empty_outcome_has_no_exit_and_no_output() {
    let outcome = ProgramOutcome::default();
    assert_eq!(outcome.exit_code, None);
    assert_eq!(outcome.signal, None);
    assert!(!outcome.timed_out && !outcome.cancelled);
    assert!(outcome.stdout.is_empty() && outcome.stderr.is_empty());
    assert!(!outcome.stdout_truncated && !outcome.stderr_truncated);
    assert!(outcome.changed_paths.is_empty());
    let exit = ChildExit {
        exit_code: Some(1),
        signal: None,
    };
    assert_eq!(
        serde_json::to_value(exit).unwrap(),
        json!({"exit_code": 1, "signal": null})
    );
}

#[test]
fn walk_specifications_round_trip_with_their_limits() {
    let mut spec = WalkSpec::new("/home/user/project");
    spec.max_depth = Some(3);
    spec.kind = WalkKind::Any;
    spec.exclude = vec!["target".to_owned(), "!target/keep".to_owned()];
    spec.exclude_presets = vec![ExcludePreset::Build];
    let value = serde_json::to_value(&spec).unwrap();
    assert_eq!(value["max_depth"], 3);
    assert_eq!(value["kind"], "any");
    assert_eq!(value["exclude_presets"], json!(["build"]));
    assert_eq!(serde_json::from_value::<WalkSpec>(value).unwrap(), spec);
    assert_eq!(WalkKind::default(), WalkKind::File);
}

#[test]
fn names_of_the_data_enumerations_are_snake_case() {
    let presets = [
        (ExcludePreset::Vcs, "vcs"),
        (ExcludePreset::Deps, "deps"),
        (ExcludePreset::Build, "build"),
        (ExcludePreset::Cache, "cache"),
    ];
    for (preset, name) in presets {
        assert_eq!(serde_json::to_value(preset).unwrap(), json!(name));
    }
    let kinds = [
        (SymbolKind::Function, "function"),
        (SymbolKind::Method, "method"),
        (SymbolKind::Class, "class"),
        (SymbolKind::Struct, "struct"),
        (SymbolKind::Interface, "interface"),
        (SymbolKind::Enum, "enum"),
        (SymbolKind::Variable, "variable"),
        (SymbolKind::Constant, "constant"),
        (SymbolKind::Field, "field"),
        (SymbolKind::Module, "module"),
        (SymbolKind::Type, "type"),
        (SymbolKind::Macro, "macro"),
    ];
    for (kind, name) in kinds {
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(name));
    }
    assert_eq!(
        serde_json::to_value(SpanClass::String).unwrap(),
        json!("string")
    );
    assert_eq!(
        serde_json::to_value(ReferenceRole::Import).unwrap(),
        json!("import")
    );
    assert_eq!(
        serde_json::to_value(IssueSeverity::Warning).unwrap(),
        json!("warning")
    );
    assert_eq!(
        serde_json::to_value(RuleSeverity::Info).unwrap(),
        json!("info")
    );
    assert_eq!(
        serde_json::to_value(GitChange::TypeChanged).unwrap(),
        json!("type_changed")
    );
    assert_eq!(
        serde_json::to_value(GitChange::Untracked).unwrap(),
        json!("untracked")
    );
}

#[test]
fn the_default_text_encoding_is_utf8_without_a_mark() {
    let encoding = TextEncoding::utf8();
    assert_eq!(encoding.label, "utf-8");
    assert!(!encoding.bom);
    assert_eq!(
        serde_json::to_value(&encoding).unwrap(),
        json!({"label": "utf-8", "bom": false})
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
