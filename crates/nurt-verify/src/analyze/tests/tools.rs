// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::analyze::tests::tools
// Responsibility:   Detection of tools in a project and their command lines.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

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
fn every_tool_has_a_unique_key_a_program_and_command_lines() {
    let mut keys = BTreeSet::new();
    for tool in Tool::ALL {
        assert!(keys.insert(tool.key()), "duplicate key {}", tool.key());
        assert_eq!(Tool::from_key(tool.key()), Some(*tool));
        assert!(!tool.name().is_empty());
        assert!(!tool.program().is_empty());
        assert!(!tool.language().is_empty());
        assert!(!tool.extensions().is_empty());
        assert!(!tool.check_args(&[]).is_empty());
        assert_eq!(tool.write_args(&[]).is_some(), tool.formats());
    }
    assert_eq!(Tool::from_key("unknown"), None);
}

#[test]
fn version_arguments_follow_the_tool() {
    assert_eq!(Tool::GoVet.version_args(), Some(vec!["version".to_owned()]));
    assert_eq!(Tool::Gofmt.version_args(), None);
    assert_eq!(
        Tool::Clippy.version_args(),
        Some(vec!["clippy".to_owned(), "--version".to_owned()])
    );
    assert_eq!(
        Tool::Rustfmt.version_args(),
        Some(vec!["fmt".to_owned(), "--version".to_owned()])
    );
    assert_eq!(
        Tool::Mypy.version_args(),
        Some(vec!["--version".to_owned()])
    );
}

#[test]
fn tools_that_check_the_whole_project_take_no_paths() {
    let paths = vec!["src/a.rs".to_owned()];
    for tool in [Tool::GoVet, Tool::Clippy, Tool::Rustfmt, Tool::Tsc] {
        assert!(!tool.accepts_paths());
        assert!(!tool.check_args(&paths).contains(&"src/a.rs".to_owned()));
    }
    for tool in [Tool::Mypy, Tool::BiomeLint, Tool::RuffFormat, Tool::Eslint] {
        assert!(tool.check_args(&paths).contains(&"src/a.rs".to_owned()));
    }
}

#[test]
fn clean_output_gives_no_findings_for_any_tool() {
    let output = crate::analyze::output::ToolOutput {
        exit_code: Some(0),
        ..Default::default()
    };
    for tool in Tool::ALL {
        assert!(tool.parse(&output, std::path::Path::new("/p")).is_empty());
    }
}

#[test]
fn paths_that_start_with_a_dash_are_given_relative_so_they_are_not_options() {
    let paths = vec![
        "-rf.ts".to_owned(),
        "--config=x.js".to_owned(),
        "src/a.ts".to_owned(),
    ];
    assert_eq!(
        Tool::Eslint.check_args(&paths),
        vec!["-f", "json", "./-rf.ts", "./--config=x.js", "src/a.ts"]
    );
    assert_eq!(
        Tool::Prettier.write_args(&paths),
        Some(
            ["--write", "./-rf.ts", "./--config=x.js", "src/a.ts"]
                .map(str::to_owned)
                .to_vec()
        )
    );
    assert_eq!(
        Tool::Gofmt.write_args(&["-w.go".to_owned()]),
        Some(vec!["-w".to_owned(), "./-w.go".to_owned()])
    );
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

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
