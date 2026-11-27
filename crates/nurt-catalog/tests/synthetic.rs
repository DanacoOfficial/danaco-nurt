// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of catalog loading, validation, extensions and results on synthetic
//! catalogs.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by panicking"
)]

mod common;

use std::fs;
use std::path::Path;

use common::{command, core_catalog, file, index};
use nurt_catalog::{CATALOG_RESULT_FORMAT, CatalogQuery, GroupOrigin, Registry};
use nurt_contracts::error::{Error, ErrorCode};
use tempfile::TempDir;

fn write_dir(index_text: &str, files: &[(String, String)]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("_index.toml"), index_text).unwrap();
    for (name, text) in files {
        fs::write(dir.path().join(name), text).unwrap();
    }
    dir
}

fn load(index_text: &str, files: &[(String, String)]) -> Result<Registry, Error> {
    let dir = write_dir(index_text, files);
    Registry::load_dir(dir.path())
}

fn issues_of(error: &Error) -> Vec<String> {
    error
        .details
        .get("issues")
        .and_then(|issues| issues.as_array())
        .map(|issues| {
            issues
                .iter()
                .filter_map(|issue| issue["message"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn assert_rejected(index_text: &str, files: &[(String, String)], fragment: &str) {
    let error = load(index_text, files).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Catalog);
    let text = format!("{error} {:?}", issues_of(&error));
    assert!(text.contains(fragment), "{fragment:?} not in {text}");
}

#[test]
fn loads_a_directory_with_two_groups() {
    let core = file("core", &[command("count", "\"any\"", "")]);
    let extra = file("misc", &[command("misc_count", "\"any\"", "")]);
    let index_text = index(&[("core", 1), ("misc", 1)], 2);
    let registry = load(
        &index_text,
        &[
            ("core.toml".to_owned(), core),
            ("misc.toml".to_owned(), extra),
        ],
    )
    .unwrap();
    assert_eq!(registry.len(), 2);
    assert_eq!(registry.command("misc_count").unwrap().group, "misc");
    assert_eq!(registry.group("core").unwrap().origin, GroupOrigin::Core);
    assert_eq!(
        registry.group("misc").unwrap().origin,
        GroupOrigin::Optional
    );
}

#[test]
fn rejects_duplicate_command_names_in_a_file() {
    let (index_text, files) = core_catalog(&["count", "count"]);
    assert_rejected(&index_text, &files, "duplicate command name");
}

#[test]
fn rejects_command_name_used_in_two_groups() {
    let core = file("core", &[command("count", "\"any\"", "")]);
    let extra = file("misc", &[command("count", "\"any\"", "")]);
    let index_text = index(&[("core", 1), ("misc", 1)], 2);
    let files = [
        ("core.toml".to_owned(), core),
        ("misc.toml".to_owned(), extra),
    ];
    assert_rejected(&index_text, &files, "several groups");
}

#[test]
fn rejects_command_count_that_differs_from_the_index() {
    let (_, files) = core_catalog(&["count", "sort"]);
    assert_rejected(
        &index(&[("core", 3)], 3),
        &files,
        "has 2 commands, the index lists 3",
    );
}

#[test]
fn rejects_total_that_differs_from_the_listed_counts() {
    let (_, files) = core_catalog(&["count"]);
    assert_rejected(&index(&[("core", 1)], 5), &files, "total is 5");
}

#[test]
fn rejects_positions_that_skip_a_number() {
    let param = "\n[[command.param]]\nname = \"path\"\ntype = \"path\"\nposition = 2\nsummary = \"A path.\"\n";
    let files = [(
        "core.toml".to_owned(),
        file("core", &[command("count", "\"any\"", param)]),
    )];
    assert_rejected(
        &index(&[("core", 1)], 1),
        &files,
        "do not form the sequence",
    );
}

#[test]
fn rejects_unparsable_parameter_type() {
    let param = "\n[[command.param]]\nname = \"path\"\ntype = \"list<\"\nsummary = \"A path.\"\n";
    let files = [(
        "core.toml".to_owned(),
        file("core", &[command("count", "\"any\"", param)]),
    )];
    assert_rejected(&index(&[("core", 1)], 1), &files, "core.toml");
}

#[test]
fn rejects_enum_values_that_differ_from_the_type() {
    let param = "\n[[command.param]]\nname = \"mode\"\ntype = \"enum(a|b)\"\nvalues = [\"a\"]\nsummary = \"A mode.\"\n";
    let files = [(
        "core.toml".to_owned(),
        file("core", &[command("count", "\"any\"", param)]),
    )];
    assert_rejected(
        &index(&[("core", 1)], 1),
        &files,
        "do not repeat the enumeration values",
    );
}

#[test]
fn rejects_default_that_is_not_an_enum_value() {
    let param = "\n[[command.param]]\nname = \"mode\"\ntype = \"enum(a|b)\"\nvalues = [\"a\", \"b\"]\ndefault = \"c\"\nsummary = \"A mode.\"\n";
    let files = [(
        "core.toml".to_owned(),
        file("core", &[command("count", "\"any\"", param)]),
    )];
    assert_rejected(
        &index(&[("core", 1)], 1),
        &files,
        "is not a value of the type",
    );
}

#[test]
fn rejects_exclusive_group_with_a_single_member() {
    let param = "\n[[command.param]]\nname = \"left\"\ntype = \"text\"\nexclusive_group = \"side\"\nsummary = \"Left.\"\n";
    let files = [(
        "core.toml".to_owned(),
        file("core", &[command("count", "\"any\"", param)]),
    )];
    assert_rejected(&index(&[("core", 1)], 1), &files, "has a single member");
}

#[test]
fn rejects_required_group_that_no_parameter_declares() {
    let files = [(
        "core.toml".to_owned(),
        file(
            "core",
            &[command(
                "count",
                "\"any\"",
                "required_groups = [\"side\"]\n",
            )],
        ),
    )];
    assert_rejected(
        &index(&[("core", 1)], 1),
        &files,
        "is not an exclusive group",
    );
}

#[test]
fn rejects_file_whose_group_differs_from_the_index() {
    let files = [(
        "core.toml".to_owned(),
        file("other", &[command("count", "\"any\"", "")]),
    )];
    assert_rejected(
        &index(&[("core", 1)], 1),
        &files,
        "has group other, the index lists core",
    );
}

#[test]
fn rejects_index_without_core_first() {
    let files = [(
        "misc.toml".to_owned(),
        file("misc", &[command("count", "\"any\"", "")]),
    )];
    assert_rejected(&index(&[("misc", 1)], 1), &files, "must be core");
}

#[test]
fn rejects_listed_file_that_is_missing() {
    let error = load(&index(&[("core", 1)], 1), &[]).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Catalog);
    assert!(
        error.to_string().contains("cannot read core.toml"),
        "{error}"
    );
}

#[test]
fn rejects_toml_file_that_the_index_does_not_list() {
    let (index_text, mut files) = core_catalog(&["count"]);
    files.push(("stray.toml".to_owned(), String::new()));
    assert_rejected(&index_text, &files, "stray.toml is not listed in the index");
}

#[test]
fn rejects_unknown_key_in_a_command() {
    let files = [(
        "core.toml".to_owned(),
        file("core", &[command("count", "\"any\"", "colour = \"red\"\n")]),
    )];
    assert_rejected(&index(&[("core", 1)], 1), &files, "colour");
}

#[test]
fn rejects_malformed_index_text() {
    let error = load("format = ", &[]).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Catalog);
    assert!(
        error.to_string().starts_with("E_CATALOG: _index.toml"),
        "{error}"
    );
}

#[test]
fn rejects_missing_directory() {
    let error = Registry::load_dir(Path::new("/nonexistent-catalog-directory")).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Catalog);
}

#[test]
fn from_toml_builds_a_catalog_from_texts() {
    let (index_text, files) = core_catalog(&["count", "sort"]);
    let pairs: Vec<(&str, &str)> = files
        .iter()
        .map(|(n, t)| (n.as_str(), t.as_str()))
        .collect();
    let registry = Registry::from_toml(&index_text, &pairs).unwrap();
    assert_eq!(registry.len(), 2);
    let missing = Registry::from_toml(&index_text, &[]).unwrap_err();
    assert!(issues_of(&missing)[0].contains("missing"));
}

fn extension_command(registry: &Registry, name: &str) -> nurt_contracts::spec::StageSpec {
    let mut spec = registry.command("count").unwrap().clone();
    spec.name = name.to_owned();
    spec.group = String::new();
    spec
}

fn small_registry() -> Registry {
    let (index_text, files) = core_catalog(&["count", "git.status"]);
    let pairs: Vec<(&str, &str)> = files
        .iter()
        .map(|(n, t)| (n.as_str(), t.as_str()))
        .collect();
    Registry::from_toml(&index_text, &pairs).unwrap()
}

#[test]
fn registers_an_extension_with_a_prefix() {
    let mut registry = small_registry();
    let spec = extension_command(&registry, "acme.count");
    registry
        .register_extension("acme", "Acme commands.", vec![spec])
        .unwrap();
    assert_eq!(registry.command("acme.count").unwrap().group, "acme");
    assert_eq!(
        registry.group("acme").unwrap().origin,
        GroupOrigin::Extension
    );
    assert_eq!(registry.len(), 3);
}

#[test]
fn rejects_extension_command_without_the_prefix() {
    let mut registry = small_registry();
    let spec = extension_command(&registry, "count2");
    let error = registry
        .register_extension("acme", "Acme commands.", vec![spec])
        .unwrap_err();
    assert!(issues_of(&error)[0].contains("does not start with `acme.`"));
    assert!(registry.group("acme").is_none());
}

#[test]
fn rejects_extension_that_reuses_a_core_command_name() {
    let mut registry = small_registry();
    let spec = extension_command(&registry, "count");
    let error = registry
        .register_extension("count", "Shadow.", vec![spec])
        .unwrap_err();
    assert!(
        issues_of(&error)
            .iter()
            .any(|issue| issue.contains("is a name of the catalog"))
    );
}

#[test]
fn rejects_extension_prefix_of_an_existing_dotted_family() {
    let mut registry = small_registry();
    let spec = extension_command(&registry, "git.extra");
    let error = registry
        .register_extension("git", "Shadow.", vec![spec])
        .unwrap_err();
    let issues = issues_of(&error);
    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("used by catalog commands")),
        "{issues:?}"
    );
}

#[test]
fn rejects_extension_prefix_that_names_an_existing_group() {
    let mut registry = small_registry();
    let spec = extension_command(&registry, "core.extra");
    assert!(
        registry
            .register_extension("core", "Core.", vec![spec])
            .is_err()
    );
}

#[test]
fn rejects_extension_with_repeated_or_foreign_group_commands() {
    let mut registry = small_registry();
    let first = extension_command(&registry, "acme.count");
    let mut foreign = extension_command(&registry, "acme.sort");
    foreign.group = "other".to_owned();
    let error = registry
        .register_extension(
            "acme",
            "Acme commands.",
            vec![first.clone(), first, foreign],
        )
        .unwrap_err();
    let issues = issues_of(&error);
    assert!(
        issues.iter().any(|issue| issue.contains("is given twice")),
        "{issues:?}"
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("instead of `acme`")),
        "{issues:?}"
    );
}

#[test]
fn rejects_extension_with_invalid_prefix_or_summary() {
    let mut registry = small_registry();
    let spec = extension_command(&registry, "Acme.count");
    let error = registry
        .register_extension("Acme", "no period", vec![spec])
        .unwrap_err();
    let issues = issues_of(&error);
    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("is not an identifier")),
        "{issues:?}"
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("summary must be one sentence")),
        "{issues:?}"
    );
}

#[test]
fn catalog_rev_is_stable_across_loads() {
    let first = Registry::load_embedded().unwrap().catalog_rev().unwrap();
    let second = Registry::load_embedded().unwrap().catalog_rev().unwrap();
    assert_eq!(first, second);
    assert!(first.starts_with("sha256:"));
    assert_eq!(first.len(), "sha256:".len() + 64);
}

#[test]
fn catalog_rev_changes_when_a_file_changes() {
    let (index_text, mut files) = core_catalog(&["count", "sort"]);
    let before = load(&index_text, &files).unwrap().catalog_rev().unwrap();
    files[0].1 = files[0]
        .1
        .replace("Count the input records.", "Count the records.");
    let after = load(&index_text, &files).unwrap().catalog_rev().unwrap();
    assert_ne!(before, after);
}

#[test]
fn catalog_rev_ignores_white_space_and_comments() {
    let (index_text, mut files) = core_catalog(&["count"]);
    let before = load(&index_text, &files).unwrap().catalog_rev().unwrap();
    files[0].1 = format!("# a comment\n\n{}\n\n", files[0].1.replace(" = ", "   =  "));
    let after = load(&index_text, &files).unwrap().catalog_rev().unwrap();
    assert_eq!(before, after);
}

#[test]
fn catalog_rev_changes_when_an_extension_is_registered() {
    let mut registry = small_registry();
    let before = registry.catalog_rev().unwrap();
    let spec = extension_command(&registry, "acme.count");
    registry
        .register_extension("acme", "Acme commands.", vec![spec])
        .unwrap();
    assert_ne!(before, registry.catalog_rev().unwrap());
}

#[test]
fn summary_result_lists_groups_with_counts() {
    let registry = Registry::load_embedded().unwrap();
    let result = registry.catalog_json(&CatalogQuery::Summary).unwrap();
    assert_eq!(result["format"], CATALOG_RESULT_FORMAT);
    assert_eq!(result["total"], 166);
    assert_eq!(result["groups"][0]["name"], "core");
    assert_eq!(result["groups"][0]["commands"], 33);
    assert_eq!(result["groups"][0]["origin"], "core");
    assert_eq!(result["catalog_rev"], registry.catalog_rev().unwrap());
}

#[test]
fn group_result_lists_command_briefs() {
    let registry = Registry::load_embedded().unwrap();
    let result = registry
        .catalog_json(&CatalogQuery::Group("verify"))
        .unwrap();
    assert_eq!(result["group"]["name"], "verify");
    let commands = result["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 8);
    assert!(commands[0]["category"].is_string());
    assert!(commands[0].get("param").is_none());
}

#[test]
fn command_result_holds_the_full_specification() {
    let registry = Registry::load_embedded().unwrap();
    let result = registry
        .catalog_json(&CatalogQuery::Command("remove"))
        .unwrap();
    assert_eq!(result["command"]["name"], "remove");
    assert!(result["command"]["param"].is_array());
    assert!(result["command"]["form"].is_array());
}

#[test]
fn catalog_result_is_deterministic() {
    let first = Registry::load_embedded()
        .unwrap()
        .catalog_json(&CatalogQuery::Summary)
        .unwrap();
    let second = Registry::load_embedded()
        .unwrap()
        .catalog_json(&CatalogQuery::Summary)
        .unwrap();
    assert_eq!(first.to_string(), second.to_string());
}

#[test]
fn catalog_result_reports_unknown_names() {
    let registry = Registry::load_embedded().unwrap();
    let command = registry
        .catalog_json(&CatalogQuery::Command("nope"))
        .unwrap_err();
    assert_eq!(command.code(), ErrorCode::UnknownCommand);
    let group = registry
        .catalog_json(&CatalogQuery::Group("nope"))
        .unwrap_err();
    assert_eq!(group.code(), ErrorCode::UnknownName);
}

#[cfg(target_os = "linux")]
#[test]
fn rejects_an_unlisted_toml_file_whose_name_is_not_utf8() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let (index_text, files) = core_catalog(&["count"]);
    let dir = write_dir(&index_text, &files);
    let name = OsStr::from_bytes(b"caf\xe9.toml");
    fs::write(dir.path().join(name), "").unwrap();
    let error = Registry::load_dir(dir.path()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Catalog);
    assert!(
        issues_of(&error)
            .iter()
            .any(|issue| issue.contains("is not listed in the index")),
        "{error}"
    );
}

#[test]
fn rejects_an_unlisted_toml_file_with_a_multi_byte_name() {
    let (index_text, mut files) = core_catalog(&["count"]);
    files.push(("żółw.toml".to_owned(), String::new()));
    assert_rejected(&index_text, &files, "żółw.toml is not listed in the index");
}

#[test]
fn rejects_an_extension_prefix_written_with_multi_byte_letters() {
    let mut registry = small_registry();
    let spec = extension_command(&registry, "ünï.count");
    let error = registry
        .register_extension("ünï", "Unicode commands.", vec![spec])
        .unwrap_err();
    assert!(
        issues_of(&error)
            .iter()
            .any(|issue| issue.contains("is not an identifier")),
        "{error}"
    );
    assert!(registry.group("ünï").is_none());
    assert_eq!(registry.len(), 2);
}

#[test]
fn group_and_command_results_list_the_extension_commands() {
    let mut registry = small_registry();
    let spec = extension_command(&registry, "acme.count");
    registry
        .register_extension("acme", "Acme commands.", vec![spec])
        .unwrap();
    let group = registry.catalog_json(&CatalogQuery::Group("acme")).unwrap();
    assert_eq!(group["group"]["origin"], "extension");
    assert_eq!(group["group"]["commands"], 1);
    assert_eq!(group["commands"][0]["name"], "acme.count");
    let command = registry
        .catalog_json(&CatalogQuery::Command("acme.count"))
        .unwrap();
    assert_eq!(command["command"]["name"], "acme.count");
    let summary = registry.catalog_json(&CatalogQuery::Summary).unwrap();
    assert_eq!(summary["total"], 3);
    assert_eq!(summary["groups"][1]["name"], "acme");
}
