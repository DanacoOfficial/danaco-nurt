// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::analyze::tools
// Responsibility:   Analyzers and formatters: detection in a project, command lines and.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Analyzers and formatters: detection in a project, command lines and
//! output parsers.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use nurt_contracts::record::{Finding, FindingKind};

use crate::analyze::analyzers::{
    biome, clippy, eslint, gofmt, govet, mypy, prettier, ruff, rustfmt, tsc,
};
use crate::analyze::output::ToolOutput;

const SCRIPT_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"];
const PRETTIER_EXTENSIONS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts", "json", "css", "scss", "md", "yaml",
    "yml", "html", "vue",
];
const PYTHON_EXTENSIONS: &[&str] = &["py", "pyi"];
const GO_EXTENSIONS: &[&str] = &["go"];
const RUST_EXTENSIONS: &[&str] = &["rs"];

const BIOME_CONFIGS: &[&str] = &["biome.json", "biome.jsonc"];
const ESLINT_CONFIGS: &[&str] = &[
    "eslint.config.js",
    "eslint.config.mjs",
    "eslint.config.cjs",
    "eslint.config.ts",
    ".eslintrc",
    ".eslintrc.json",
    ".eslintrc.js",
    ".eslintrc.cjs",
    ".eslintrc.yml",
];
const PRETTIER_CONFIGS: &[&str] = &[
    ".prettierrc",
    ".prettierrc.json",
    ".prettierrc.yml",
    ".prettierrc.yaml",
    ".prettierrc.js",
    ".prettierrc.cjs",
    ".prettierrc.mjs",
    "prettier.config.js",
    "prettier.config.mjs",
    "prettier.config.cjs",
];
const MYPY_CONFIGS: &[&str] = &["mypy.ini", ".mypy.ini"];
const RUFF_CONFIGS: &[&str] = &["ruff.toml", ".ruff.toml"];
const RUSTFMT_CONFIGS: &[&str] = &["rustfmt.toml", ".rustfmt.toml"];
const CLIPPY_CONFIGS: &[&str] = &["clippy.toml", ".clippy.toml"];

/// Edition passed to `rustfmt` when it formats files outside a package.
const RUSTFMT_EDITION: &str = "2024";

/// Directory of the programs installed with a JavaScript project.
const LOCAL_BIN: &str = "node_modules/.bin";

/// Largest number of paths passed to a tool on its command line; more paths make the
/// tool check the whole project.
pub(crate) const PATH_ARGUMENTS_MAX: usize = 400;

/// Analyzer or formatter known to the commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Tool {
    /// TypeScript type checker.
    Tsc,
    /// ESLint.
    Eslint,
    /// Biome linter.
    BiomeLint,
    /// Prettier.
    Prettier,
    /// Biome formatter.
    BiomeFormat,
    /// Ruff linter.
    RuffLint,
    /// Ruff formatter.
    RuffFormat,
    /// The mypy type checker.
    Mypy,
    /// `go vet`.
    GoVet,
    /// `gofmt`.
    Gofmt,
    /// Clippy.
    Clippy,
    /// The rustfmt formatter.
    Rustfmt,
}

impl Tool {
    /// Every tool in the order of detection.
    pub(crate) const ALL: &'static [Tool] = &[
        Tool::BiomeLint,
        Tool::Eslint,
        Tool::Tsc,
        Tool::Prettier,
        Tool::BiomeFormat,
        Tool::RuffLint,
        Tool::RuffFormat,
        Tool::Mypy,
        Tool::GoVet,
        Tool::Gofmt,
        Tool::Clippy,
        Tool::Rustfmt,
    ];

    /// Name of the tool in findings and in the `analyze_tool` record.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Tool::Tsc => tsc::TOOL,
            Tool::Eslint => eslint::TOOL,
            Tool::BiomeLint | Tool::BiomeFormat => biome::TOOL,
            Tool::Prettier => prettier::TOOL,
            Tool::RuffLint | Tool::RuffFormat => ruff::TOOL,
            Tool::Mypy => mypy::TOOL,
            Tool::GoVet => govet::TOOL,
            Tool::Gofmt => gofmt::TOOL,
            Tool::Clippy => clippy::TOOL,
            Tool::Rustfmt => rustfmt::TOOL,
        }
    }

    /// Check the tool performs.
    pub(crate) fn check(self) -> FindingKind {
        match self {
            Tool::Tsc | Tool::Mypy => FindingKind::Types,
            Tool::Eslint | Tool::BiomeLint | Tool::RuffLint | Tool::GoVet | Tool::Clippy => {
                FindingKind::Lint
            }
            Tool::Prettier | Tool::BiomeFormat | Tool::RuffFormat | Tool::Gofmt | Tool::Rustfmt => {
                FindingKind::Format
            }
        }
    }

    /// Language of the files the tool checks.
    pub(crate) fn language(self) -> &'static str {
        match self {
            Tool::Tsc | Tool::Eslint | Tool::BiomeLint | Tool::BiomeFormat | Tool::Prettier => {
                "typescript"
            }
            Tool::RuffLint | Tool::RuffFormat | Tool::Mypy => "python",
            Tool::GoVet | Tool::Gofmt => "go",
            Tool::Clippy | Tool::Rustfmt => "rust",
        }
    }

    /// Program started for the tool.
    pub(crate) fn program(self) -> &'static str {
        match self {
            Tool::Tsc => "tsc",
            Tool::Eslint => "eslint",
            Tool::BiomeLint | Tool::BiomeFormat => "biome",
            Tool::Prettier => "prettier",
            Tool::RuffLint | Tool::RuffFormat => "ruff",
            Tool::Mypy => "mypy",
            Tool::GoVet => "go",
            Tool::Gofmt => "gofmt",
            Tool::Clippy | Tool::Rustfmt => "cargo",
        }
    }

    /// Whether the program is installed with the project when it is a
    /// JavaScript tool.
    fn is_script_tool(self) -> bool {
        matches!(
            self,
            Tool::Tsc | Tool::Eslint | Tool::BiomeLint | Tool::BiomeFormat | Tool::Prettier
        )
    }

    /// File name extensions the tool handles, without the period.
    pub(crate) fn extensions(self) -> &'static [&'static str] {
        match self {
            Tool::Tsc | Tool::Eslint | Tool::BiomeLint | Tool::BiomeFormat => SCRIPT_EXTENSIONS,
            Tool::Prettier => PRETTIER_EXTENSIONS,
            Tool::RuffLint | Tool::RuffFormat | Tool::Mypy => PYTHON_EXTENSIONS,
            Tool::GoVet | Tool::Gofmt => GO_EXTENSIONS,
            Tool::Clippy | Tool::Rustfmt => RUST_EXTENSIONS,
        }
    }

    /// Whether the tool accepts a list of files; other tools check the whole
    /// project.
    pub(crate) fn accepts_paths(self) -> bool {
        !matches!(self, Tool::Tsc | Tool::GoVet | Tool::Clippy | Tool::Rustfmt)
    }

    /// Whether the tool can rewrite files.
    pub(crate) fn formats(self) -> bool {
        self.check() == FindingKind::Format
    }

    /// Arguments of the check run, after the program.
    pub(crate) fn check_args(self, paths: &[String]) -> Vec<String> {
        let mut args = match self {
            Tool::Tsc => words(&["--noEmit", "-p", "tsconfig.json", "--pretty", "false"]),
            Tool::Eslint => words(&["-f", "json"]),
            Tool::BiomeLint => words(&["lint", "--reporter=github"]),
            Tool::Prettier => words(&["--list-different"]),
            Tool::BiomeFormat => words(&["format", "--reporter=github"]),
            Tool::RuffLint => words(&["check", "--output-format=json", "--no-cache"]),
            Tool::RuffFormat => words(&["format", "--check", "--no-cache"]),
            Tool::Mypy => words(&[
                "--no-error-summary",
                "--show-column-numbers",
                "--no-color-output",
            ]),
            Tool::GoVet => return words(&["vet", "./..."]),
            Tool::Gofmt => words(&["-l"]),
            Tool::Clippy => return words(&["clippy", "--quiet", "--message-format=json"]),
            Tool::Rustfmt => return words(&["fmt", "--all", "--", "--check", "-l"]),
        };
        if self.accepts_paths() {
            args.extend(or_project(paths));
        }
        args
    }

    /// Arguments of the run that rewrites `paths`, after the program;
    /// `None` for tools that do not format.
    pub(crate) fn write_args(self, paths: &[String]) -> Option<Vec<String>> {
        let mut args = match self {
            Tool::Prettier => words(&["--write"]),
            Tool::BiomeFormat => words(&["format", "--write"]),
            Tool::RuffFormat => words(&["format", "--no-cache"]),
            Tool::Gofmt => words(&["-w"]),
            Tool::Rustfmt => words(&["--edition", RUSTFMT_EDITION]),
            _ => return None,
        };
        args.extend(paths.iter().map(|path| operand(path)));
        Some(args)
    }

    /// Findings in the output of a check run.
    pub(crate) fn parse(self, output: &ToolOutput, dir: &Path) -> Vec<Finding> {
        match self {
            Tool::Tsc => tsc::parse(output, dir),
            Tool::Eslint => eslint::parse(output, dir),
            Tool::BiomeLint => biome::parse_lint(output, dir),
            Tool::Prettier => prettier::parse(output, dir),
            Tool::BiomeFormat => biome::parse_format(output, dir),
            Tool::RuffLint => ruff::parse_lint(output, dir),
            Tool::RuffFormat => ruff::parse_format(output, dir),
            Tool::Mypy => mypy::parse(output, dir),
            Tool::GoVet => govet::parse(output, dir),
            Tool::Gofmt => gofmt::parse(output, dir),
            Tool::Clippy => clippy::parse(output, dir),
            Tool::Rustfmt => rustfmt::parse(output, dir),
        }
    }
}

/// Owned copies of literal arguments.
fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|item| (*item).to_owned()).collect()
}

fn or_project(paths: &[String]) -> Vec<String> {
    if paths.is_empty() {
        vec![".".to_owned()]
    } else {
        paths.iter().map(|path| operand(path)).collect()
    }
}

/// Path as a command line operand: a name that starts with `-` would be read
/// as an option of the tool, so it is given relative to the directory.
fn operand(path: &str) -> String {
    if path.starts_with('-') {
        format!("./{path}")
    } else {
        path.to_owned()
    }
}

/// Tool applicable to a project, with the configuration that selected it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    /// Tool.
    pub(crate) tool: Tool,
    /// Configuration file relative to the project directory.
    pub(crate) config: Option<String>,
}

/// Tools applicable to the project in `dir`, chosen by the configuration
/// files and by the extensions of the files in scope.
pub(crate) fn candidates(dir: &Path, extensions: &BTreeSet<String>) -> Vec<Candidate> {
    let present = |list: &[&str]| list.iter().any(|ext| extensions.contains(*ext));
    let mut found = Vec::new();
    if present(SCRIPT_EXTENSIONS) || dir.join("package.json").exists() {
        found.extend(script_candidates(dir));
    }
    if present(PYTHON_EXTENSIONS) {
        found.extend(python_candidates(dir));
    }
    if dir.join("go.mod").exists() {
        found.push(plain(Tool::GoVet, Some("go.mod".to_owned())));
        found.push(plain(Tool::Gofmt, Some("go.mod".to_owned())));
    }
    if dir.join("Cargo.toml").exists() {
        found.push(plain(Tool::Clippy, first_existing(dir, CLIPPY_CONFIGS)));
        found.push(plain(Tool::Rustfmt, first_existing(dir, RUSTFMT_CONFIGS)));
    }
    found
}

fn plain(tool: Tool, config: Option<String>) -> Candidate {
    Candidate { tool, config }
}

fn script_candidates(dir: &Path) -> Vec<Candidate> {
    let mut found = Vec::new();
    let biome = first_existing(dir, BIOME_CONFIGS);
    if biome.is_some() {
        found.push(plain(Tool::BiomeLint, biome.clone()));
    }
    if let Some(config) = first_existing(dir, ESLINT_CONFIGS) {
        found.push(plain(Tool::Eslint, Some(config)));
    }
    if dir.join("tsconfig.json").exists() {
        found.push(plain(Tool::Tsc, Some("tsconfig.json".to_owned())));
    }
    if let Some(config) = first_existing(dir, PRETTIER_CONFIGS) {
        found.push(plain(Tool::Prettier, Some(config)));
    } else if package_declares(dir, "prettier") {
        found.push(plain(Tool::Prettier, Some("package.json".to_owned())));
    } else if biome.is_some() {
        found.push(plain(Tool::BiomeFormat, biome));
    }
    found
}

fn python_candidates(dir: &Path) -> Vec<Candidate> {
    let ruff = first_existing(dir, RUFF_CONFIGS).or_else(|| section_file(dir, "[tool.ruff]"));
    let mut found = vec![
        plain(Tool::RuffLint, ruff.clone()),
        plain(Tool::RuffFormat, ruff),
    ];
    let mypy = first_existing(dir, MYPY_CONFIGS)
        .or_else(|| section_file(dir, "[tool.mypy]"))
        .or_else(|| file_with(dir, "setup.cfg", "[mypy]"));
    if mypy.is_some() {
        found.push(plain(Tool::Mypy, mypy));
    }
    found
}

fn first_existing(dir: &Path, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find(|name| dir.join(name).exists())
        .map(|name| (*name).to_owned())
}

fn file_with(dir: &Path, name: &str, marker: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(name)).ok()?;
    text.contains(marker).then(|| name.to_owned())
}

fn section_file(dir: &Path, marker: &str) -> Option<String> {
    file_with(dir, "pyproject.toml", marker)
}

fn package_declares(dir: &Path, key: &str) -> bool {
    std::fs::read_to_string(dir.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .is_some_and(|package| package.get(key).is_some())
}

/// Program of `tool` found for the project: a program installed with the
/// project first, then the search path.
pub(crate) fn resolve_program(
    tool: Tool,
    dir: &Path,
    search_path: Option<&OsStr>,
) -> Option<PathBuf> {
    let name = tool.program();
    if tool.is_script_tool() {
        let local = dir.join(LOCAL_BIN);
        if let Some(found) = executable_in(&local, name) {
            return Some(found);
        }
    }
    let search_path = search_path?;
    std::env::split_paths(search_path).find_map(|entry| executable_in(&entry, name))
}

fn executable_in(directory: &Path, name: &str) -> Option<PathBuf> {
    let plain = directory.join(name);
    if plain.is_file() {
        return Some(plain);
    }
    if cfg!(windows) {
        for suffix in ["exe", "cmd", "bat"] {
            let candidate = directory.join(format!("{name}.{suffix}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

impl Tool {
    /// Stable key of the tool, unique among the variants that share a
    /// program; stored with a background run.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Tool::Tsc => "tsc",
            Tool::Eslint => "eslint",
            Tool::BiomeLint => "biome-lint",
            Tool::Prettier => "prettier",
            Tool::BiomeFormat => "biome-format",
            Tool::RuffLint => "ruff-lint",
            Tool::RuffFormat => "ruff-format",
            Tool::Mypy => "mypy",
            Tool::GoVet => "go-vet",
            Tool::Gofmt => "gofmt",
            Tool::Clippy => "clippy",
            Tool::Rustfmt => "rustfmt",
        }
    }

    /// Tool with the given key.
    pub(crate) fn from_key(key: &str) -> Option<Tool> {
        Tool::ALL.iter().copied().find(|tool| tool.key() == key)
    }

    /// Arguments that print the version of the tool, after the program;
    /// `None` when the tool has no version output.
    pub(crate) fn version_args(self) -> Option<Vec<String>> {
        match self {
            Tool::GoVet => Some(words(&["version"])),
            Tool::Gofmt => None,
            Tool::Clippy => Some(words(&["clippy", "--version"])),
            Tool::Rustfmt => Some(words(&["fmt", "--version"])),
            _ => Some(words(&["--version"])),
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
