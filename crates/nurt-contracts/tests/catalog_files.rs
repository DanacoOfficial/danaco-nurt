// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Tests of the catalog files in `catalog/`: every file parses, is consistent,
//! agrees with the index and validates against the published schema.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::kinds::{Category, EffectKind, Implementation, Reversibility};
use nurt_contracts::spec::{
    CATALOG_FORMAT, CORE_GROUP, CatalogFile, CatalogIndex, DefaultKind, IoSpec, ParamSpec,
    ParamType, ScalarType, check_catalog,
};

/// Number of commands in the whole catalog.
const EXPECTED_TOTAL: u32 = 149;

/// Number of mutually exclusive conditions of `assert`.
const ASSERT_CONDITIONS: usize = 8;

const CORE_COMMANDS: [&str; 34] = [
    "list",
    "find",
    "info",
    "read",
    "lines",
    "search",
    "filter",
    "select",
    "sort",
    "first",
    "last",
    "skip",
    "count",
    "assert",
    "verify",
    "write",
    "replace",
    "insert",
    "edit",
    "copy",
    "move",
    "remove",
    "rename",
    "create",
    "attributes",
    "run",
    "jobs",
    "stop",
    "retry",
    "history",
    "next",
    "undo",
    "redo",
    "help",
];

const GROUPS: [&str; 14] = [
    "documents",
    "text",
    "code",
    "notebook",
    "git",
    "archive",
    "system",
    "admin",
    "package",
    "network",
    "schedule",
    "journal",
    "program",
    "verify",
];

const CALL_PARAMETERS: [&str; 15] = [
    "preview",
    "base",
    "limit",
    "cursor",
    "fields",
    "format",
    "verbosity",
    "diff",
    "diff_context",
    "max_chars",
    "save_to",
    "timeout",
    "session",
    "language",
    "script",
];

fn catalog_dir() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../catalog")
}

fn read_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn load_index() -> CatalogIndex {
    let path = catalog_dir().join("_index.toml");
    toml::from_str(&read_text(&path)).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn catalog_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(catalog_dir())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "toml")
        })
        .filter(|path| path.file_name().is_some_and(|name| name != "_index.toml"))
        .collect();
    paths.sort();
    paths
}

fn load_file(path: &Path) -> CatalogFile {
    let mut file: CatalogFile = toml::from_str(&read_text(path))
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    file.resolve_groups();
    file
}

fn load_files() -> Vec<CatalogFile> {
    catalog_paths().iter().map(|path| load_file(path)).collect()
}

fn load_core() -> CatalogFile {
    load_file(&catalog_dir().join("core.toml"))
}

fn report(issues: &[impl std::fmt::Display]) -> String {
    issues
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn index_lists_core_and_fourteen_groups() {
    let index = load_index();
    let issues = index.check();
    assert!(issues.is_empty(), "{}", report(&issues));
    assert_eq!(index.total, EXPECTED_TOTAL);
    assert_eq!(index.listed_total(), u64::from(EXPECTED_TOTAL));
    let names: Vec<&str> = index
        .files
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    let mut expected = vec![CORE_GROUP];
    expected.extend(GROUPS);
    assert_eq!(names, expected);
    let core = u32::try_from(CORE_COMMANDS.len()).unwrap();
    assert_eq!(index.entry(CORE_GROUP).unwrap().commands, core);
    let groups: u32 = index.files.iter().skip(1).map(|entry| entry.commands).sum();
    assert_eq!(groups, EXPECTED_TOTAL - core);
}

#[test]
fn every_catalog_file_is_listed_and_named_after_its_group() {
    let index = load_index();
    for path in catalog_paths() {
        let stem = path.file_stem().unwrap().to_str().unwrap().to_owned();
        let file = load_file(&path);
        assert_eq!(file.format, CATALOG_FORMAT, "{}", path.display());
        assert_eq!(file.group, stem, "{}", path.display());
        assert!(
            index.entry(&stem).is_some(),
            "{} is not in the index",
            path.display()
        );
    }
}

#[test]
fn catalog_agrees_with_the_index() {
    let index = load_index();
    let files = load_files();
    let issues = check_catalog(&index, &files);
    assert!(issues.is_empty(), "{}", report(&issues));
    let present: BTreeSet<&str> = files.iter().map(|file| file.group.as_str()).collect();
    let listed: u64 = index
        .files
        .iter()
        .filter(|entry| present.contains(entry.name.as_str()))
        .map(|entry| u64::from(entry.commands))
        .sum();
    let loaded =
        u64::try_from(files.iter().map(|file| file.commands.len()).sum::<usize>()).unwrap();
    assert_eq!(loaded, listed);
    if present.len() == index.files.len() {
        assert_eq!(loaded, u64::from(EXPECTED_TOTAL));
    }
}

#[test]
fn command_names_are_unique_across_the_catalog() {
    let mut seen = BTreeSet::new();
    for file in load_files() {
        for command in &file.commands {
            assert!(
                seen.insert(command.name.clone()),
                "{} is defined twice",
                command.name
            );
        }
    }
}

#[test]
fn core_holds_exactly_the_core_commands() {
    let core = load_core();
    let issues = core.check();
    assert!(issues.is_empty(), "{}", report(&issues));
    let names: Vec<&str> = core
        .commands
        .iter()
        .map(|command| command.name.as_str())
        .collect();
    assert_eq!(names, CORE_COMMANDS);
    assert!(
        core.commands
            .iter()
            .all(|command| command.group == CORE_GROUP)
    );
}

fn all_params(files: &[CatalogFile]) -> Vec<(String, &ParamSpec)> {
    files
        .iter()
        .flat_map(|file| file.commands.iter())
        .flat_map(|command| {
            command
                .params
                .iter()
                .map(move |param| (command.name.clone(), param))
        })
        .collect()
}

#[test]
fn every_type_expression_round_trips() {
    let files = load_files();
    for (command, param) in all_params(&files) {
        let canonical = param.ty.to_string();
        let reparsed: ParamType = canonical
            .parse()
            .unwrap_or_else(|error| panic!("{command}.{}: {error}", param.name));
        assert_eq!(reparsed, param.ty, "{command}.{}", param.name);
    }
}

fn default_fits_type(kind: DefaultKind, ty: &ParamType) -> bool {
    let has = |scalar| ty.contains_scalar(scalar);
    let textual = [
        ScalarType::Text,
        ScalarType::Path,
        ScalarType::Glob,
        ScalarType::Key,
        ScalarType::Instant,
        ScalarType::Size,
        ScalarType::Duration,
        ScalarType::Percent,
        ScalarType::Any,
    ];
    match kind {
        DefaultKind::Bool => has(ScalarType::Bool),
        DefaultKind::Int => {
            has(ScalarType::Int) || has(ScalarType::Number) || has(ScalarType::Range)
        }
        DefaultKind::Number => has(ScalarType::Number),
        DefaultKind::Size => has(ScalarType::Size),
        DefaultKind::Duration => has(ScalarType::Duration),
        DefaultKind::Range => has(ScalarType::Range),
        DefaultKind::Text => textual.into_iter().any(has),
        DefaultKind::Regex => has(ScalarType::Regex),
        DefaultKind::List => ty.accepts_list(),
        DefaultKind::Map => ty
            .alternatives()
            .iter()
            .any(|alternative| matches!(alternative, ParamType::Map(_))),
        DefaultKind::Word | DefaultKind::Null => true,
        _ => false,
    }
}

#[test]
fn defaults_are_literals_of_the_parameter_type() {
    let files = load_files();
    for (command, param) in all_params(&files) {
        if let Some(default) = &param.default {
            assert!(
                !default.is_blank(),
                "{command}.{}: empty default",
                param.name
            );
            assert!(
                default_fits_type(default.kind(), &param.ty),
                "{command}.{}: default {default} does not fit type {}",
                param.name,
                param.ty
            );
        }
    }
}

#[test]
fn positions_form_a_sequence_from_one() {
    for file in load_files() {
        for command in &file.commands {
            let positions: Vec<u32> = command
                .positional()
                .iter()
                .filter_map(|param| param.position)
                .collect();
            let expected: Vec<u32> = (1..).take(positions.len()).collect();
            assert_eq!(positions, expected, "{}", command.name);
        }
    }
}

#[test]
fn call_parameters_are_not_core_parameters() {
    let core = load_core();
    let allowed = [
        ("history", "limit"),
        ("run", "timeout"),
        ("retry", "timeout"),
        ("write", "format"),
        ("search", "language"),
        ("select", "fields"),
    ];
    for command in &core.commands {
        for param in &command.params {
            if CALL_PARAMETERS.contains(&param.name.as_str()) {
                assert!(
                    allowed.contains(&(command.name.as_str(), param.name.as_str())),
                    "{}.{} repeats a call parameter",
                    command.name,
                    param.name
                );
            }
        }
    }
}

#[test]
fn core_signatures_keep_the_safety_rules() {
    let core = load_core();
    let write = core.command("write").unwrap();
    let mode = write.param("mode").unwrap();
    assert!(mode.required && mode.default.is_none());
    let create = core.command("create").unwrap();
    let kind = create.param("type").unwrap();
    assert!(kind.required && kind.default.is_none());
    assert_eq!(
        kind.values,
        ["file", "directory", "link", "junction", "fifo", "device"]
    );
    let assert = core.command("assert").unwrap();
    assert_eq!(assert.required_groups, ["condition"]);
    assert_eq!(
        assert.exclusive_groups()["condition"].len(),
        ASSERT_CONDITIONS
    );
    let run = core.command("run").unwrap();
    assert_eq!(run.effect, EffectKind::Process);
    assert_eq!(run.implementation, Implementation::UserProgram);
    assert!(run.param("wait").unwrap().waits);
    assert_eq!(
        core.command("stop").unwrap().reversibility,
        Reversibility::Irreversible
    );
    assert_eq!(
        core.command("verify").unwrap().input,
        [IoSpec::Record("effect".to_owned())]
    );
}

#[test]
fn polymorphic_core_commands_select_forms_by_input() {
    let core = load_core();
    let replace = core.command("replace").unwrap();
    let record = |kind: &str| IoSpec::Record(kind.to_owned());
    assert_eq!(replace.form_for(&IoSpec::None).unwrap().name, "source");
    assert_eq!(
        replace.form_for(&record("match")).unwrap().positional,
        ["with"]
    );
    assert_eq!(
        replace.form_for(&record("line")).unwrap().positional,
        ["pattern", "with"]
    );
    let lines = core.command("lines").unwrap();
    assert_eq!(
        lines.form_for(&record("content")).unwrap().positional,
        ["range"]
    );
    let remove = core.command("remove").unwrap();
    assert!(remove.form_for(&record("job")).is_some());
    assert_eq!(remove.effects[0].action, "job");
    assert_eq!(remove.effects[0].reversibility, Reversibility::Irreversible);
}

#[test]
fn derived_defaults_and_source_expansions_are_data() {
    let core = load_core();
    let run = core.command("run").unwrap();
    let cwd = run.param("cwd").unwrap();
    assert!(cwd.required);
    assert_eq!(cwd.default_from.as_deref(), Some("base"));
    assert_eq!(
        run.param("watch").unwrap().default_from.as_deref(),
        Some("cwd")
    );
    let expansions: Vec<(&str, &str)> = core
        .commands
        .iter()
        .flat_map(|command| {
            command
                .forms
                .iter()
                .filter_map(move |form| Some((command.name.as_str(), form.expand.as_deref()?)))
        })
        .collect();
    assert_eq!(
        expansions,
        [
            ("copy", "from"),
            ("move", "from"),
            ("remove", "path"),
            ("rename", "path"),
            ("attributes", "path"),
        ]
    );

    let mut unknown_source = run.clone();
    for param in &mut unknown_source.params {
        if param.name == "watch" {
            param.default_from = Some("directory".to_owned());
        }
    }
    let issues = unknown_source.check();
    assert!(
        issues
            .iter()
            .any(|issue| issue.message.contains("default_from")),
        "{}",
        report(&issues)
    );
    let mut stage_expansion = core.command("remove").unwrap().clone();
    stage_expansion.forms[1].expand = Some("path".to_owned());
    let issues = stage_expansion.check();
    assert!(
        issues
            .iter()
            .any(|issue| issue.message.contains("expand requires a source form")),
        "{}",
        report(&issues)
    );
}

#[test]
fn core_categories_match_the_catalog_legend() {
    let core = load_core();
    let count = |category: Category| {
        core.commands
            .iter()
            .filter(|command| command.category == category)
            .count()
    };
    assert_eq!(count(Category::Source), 8);
    assert_eq!(count(Category::Transform), 7);
    assert_eq!(count(Category::Check), 2);
    assert_eq!(count(Category::Change), 10);
    assert_eq!(count(Category::Run), 3);
    assert_eq!(count(Category::Journal), 3);
    assert_eq!(count(Category::Help), 1);
}

#[test]
fn catalog_files_validate_against_the_published_schema() {
    let schema_path = Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../spec/schemas/nurt-catalog-1.schema.json");
    let schema: serde_json::Value = serde_json::from_str(&read_text(&schema_path)).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for path in catalog_paths() {
        let instance: serde_json::Value = toml::from_str(&read_text(&path)).unwrap();
        let errors: Vec<String> = validator
            .iter_errors(&instance)
            .map(|error| error.to_string())
            .collect();
        assert!(
            errors.is_empty(),
            "{}:\n{}",
            path.display(),
            errors.join("\n")
        );
    }
}

#[test]
fn catalog_files_round_trip_through_json() {
    for path in catalog_paths() {
        let file = load_file(&path);
        let json = serde_json::to_value(&file).unwrap();
        let back: CatalogFile = serde_json::from_value(json).unwrap();
        assert_eq!(back, file, "{}", path.display());
    }
}
