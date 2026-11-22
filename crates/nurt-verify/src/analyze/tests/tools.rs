// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Detection of tools in a project and their command lines.

use std::collections::BTreeSet;
use std::ffi::OsString;

use nurt_contracts::record::FindingKind;

use super::support::Project;
use crate::analyze::tools::{Tool, candidates, resolve_program};

fn extensions(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|item| (*item).to_owned()).collect()
}

fn tools_of(project: &Project, list: &[&str]) -> Vec<Tool> {
    candidates(project.dir(), &extensions(list))
        .into_iter()
        .map(|candidate| candidate.tool)
        .collect()
}

#[test]
fn detects_script_tools_from_configuration_files() {
    let project = Project::with_files(&[
        ("tsconfig.json", "{}"),
        (".prettierrc", "{}"),
        ("eslint.config.js", ""),
    ]);
    assert_eq!(
        tools_of(&project, &["ts"]),
        vec![Tool::Eslint, Tool::Tsc, Tool::Prettier]
    );
}

#[test]
fn detects_biome_for_lint_and_format_without_prettier() {
    let project = Project::with_files(&[("biome.json", "{}")]);
    assert_eq!(
        tools_of(&project, &["ts"]),
        vec![Tool::BiomeLint, Tool::BiomeFormat]
    );
}

#[test]
fn detects_prettier_declared_in_package_manifest() {
    let project = Project::with_files(&[("package.json", "{\"prettier\": {}}")]);
    assert_eq!(tools_of(&project, &[]), vec![Tool::Prettier]);
}

#[test]
fn detects_python_tools_and_mypy_section() {
    let project = Project::with_files(&[("pyproject.toml", "[tool.mypy]\nstrict = true\n")]);
    let found = candidates(project.dir(), &extensions(&["py"]));
    let tools: Vec<Tool> = found.iter().map(|candidate| candidate.tool).collect();
    assert_eq!(tools, vec![Tool::RuffLint, Tool::RuffFormat, Tool::Mypy]);
    assert_eq!(found[2].config.as_deref(), Some("pyproject.toml"));
}

#[test]
fn detects_go_and_rust_tools_from_manifests() {
    let project = Project::with_files(&[("go.mod", "module x"), ("Cargo.toml", "")]);
    assert_eq!(
        tools_of(&project, &[]),
        vec![Tool::GoVet, Tool::Gofmt, Tool::Clippy, Tool::Rustfmt]
    );
}

#[test]
fn detects_nothing_in_an_empty_project() {
    let project = Project::with_files(&[]);
    assert!(tools_of(&project, &["md"]).is_empty());
}

#[test]
fn prefers_program_installed_with_the_project() {
    let project = Project::with_files(&[("node_modules/.bin/eslint", "")]);
    let found = resolve_program(Tool::Eslint, project.dir(), None).unwrap();
    assert!(found.ends_with("node_modules/.bin/eslint"));
}

#[test]
fn finds_programs_on_the_search_path() {
    let bin = Project::with_files(&[("gofmt", "")]);
    let search = OsString::from(bin.dir());
    assert!(resolve_program(Tool::Gofmt, bin.dir(), Some(&search)).is_some());
    assert!(resolve_program(Tool::Mypy, bin.dir(), Some(&search)).is_none());
}

#[test]
fn builds_check_commands_with_machine_readable_output() {
    let paths = vec!["src/a.py".to_owned()];
    assert_eq!(
        Tool::RuffLint.check_args(&paths),
        vec!["check", "--output-format=json", "--no-cache", "src/a.py"]
    );
    assert_eq!(Tool::Eslint.check_args(&[]), vec!["-f", "json", "."]);
    assert_eq!(
        Tool::Clippy.check_args(&paths),
        vec!["clippy", "--quiet", "--message-format=json"]
    );
    assert_eq!(Tool::Gofmt.check_args(&paths), vec!["-l", "src/a.py"]);
}

#[test]
fn builds_write_commands_only_for_formatters() {
    let paths = vec!["a.ts".to_owned()];
    assert_eq!(
        Tool::Prettier.write_args(&paths),
        Some(vec!["--write".to_owned(), "a.ts".to_owned()])
    );
    assert_eq!(Tool::Gofmt.write_args(&paths).unwrap()[0], "-w");
    assert_eq!(Tool::Eslint.write_args(&paths), None);
    assert!(Tool::Prettier.formats());
    assert_eq!(Tool::Prettier.check(), FindingKind::Format);
}
