// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Detection of the test frameworks of a project directory.

use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use super::Framework;

/// Subdirectories searched for script test files besides the project
/// directory itself.
const SCRIPT_TEST_DIRS: [&str; 5] = [".", "test", "tests", "__tests__", "src"];

/// Subdirectories searched for Python test files besides the project
/// directory itself.
const PYTHON_TEST_DIRS: [&str; 3] = [".", "tests", "test"];

/// Value of the `test` script that `npm init` writes and that runs nothing.
const PLACEHOLDER_TEST_SCRIPT: &str = "no test specified";

static BUN_SCRIPT: LazyLock<Regex> = LazyLock::new(|| compile(r"\bbun test\b"));
static SCRIPT_TEST_FILE: LazyLock<Regex> =
    LazyLock::new(|| compile(r"\.(?:test|spec)\.[cm]?[jt]sx?$"));
static PYTHON_TEST_FILE: LazyLock<Regex> =
    LazyLock::new(|| compile(r"^test_.*\.py$|_test\.py$"));
static PYTEST_SECTION: LazyLock<Regex> =
    LazyLock::new(|| compile(r"\[tool[.:]pytest|\[pytest\]"));

fn compile(pattern: &str) -> Regex {
    match Regex::new(pattern) {
        Ok(regex) => regex,
        Err(_) => unreachable!("detection patterns are constant and valid"),
    }
}

/// Frameworks found in the directory, in order of precedence.
pub(crate) fn detect(dir: &Path) -> Vec<Framework> {
    let mut found = node_frameworks(dir);
    if has_python_tests(dir) {
        found.push(Framework::Pytest);
    }
    if dir.join("go.mod").is_file() {
        found.push(Framework::Go);
    }
    if dir.join("Cargo.toml").is_file() {
        found.push(Framework::Cargo);
    }
    found
}

fn has_any(dir: &Path, names: &[&str]) -> bool {
    names.iter().any(|name| dir.join(name).exists())
}

fn read_manifest(dir: &Path) -> Option<Value> {
    let text = fs::read_to_string(dir.join("package.json")).ok()?;
    serde_json::from_str(&text).ok()
}

fn has_package(manifest: &Value, name: &str) -> bool {
    ["dependencies", "devDependencies"]
        .iter()
        .any(|section| manifest.get(section).and_then(|deps| deps.get(name)).is_some())
}

fn node_frameworks(dir: &Path) -> Vec<Framework> {
    let manifest = read_manifest(dir);
    let script = manifest
        .as_ref()
        .and_then(|manifest| manifest.pointer("/scripts/test"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut found = Vec::new();
    if has_any(dir, &["bunfig.toml", "bun.lock", "bun.lockb"]) || BUN_SCRIPT.is_match(script) {
        found.push(Framework::Bun);
    }
    let has = |name: &str| manifest.as_ref().is_some_and(|m| has_package(m, name));
    if has("vitest")
        || has_any(
            dir,
            &["vitest.config.ts", "vitest.config.js", "vitest.config.mts"],
        )
    {
        found.push(Framework::Vitest);
    }
    let jest_section = manifest.as_ref().is_some_and(|m| m.get("jest").is_some());
    if has("jest")
        || jest_section
        || has_any(
            dir,
            &[
                "jest.config.js",
                "jest.config.ts",
                "jest.config.mjs",
                "jest.config.cjs",
            ],
        )
    {
        found.push(Framework::Jest);
    }
    if found.is_empty() && !script.is_empty() && !script.contains(PLACEHOLDER_TEST_SCRIPT) {
        found.push(Framework::Npm);
    }
    if found.is_empty() && has_script_tests(dir) {
        found.push(Framework::Bun);
    }
    found
}

fn directory_has_file(path: &Path, pattern: &Regex) -> bool {
    let Ok(entries) = fs::read_dir(path) else {
        return false;
    };
    entries
        .flatten()
        .any(|entry| pattern.is_match(&entry.file_name().to_string_lossy()))
}

fn has_script_tests(dir: &Path) -> bool {
    SCRIPT_TEST_DIRS
        .iter()
        .map(|folder| dir.join(folder))
        .any(|path| path.is_dir() && directory_has_file(&path, &SCRIPT_TEST_FILE))
}

fn has_python_tests(dir: &Path) -> bool {
    if has_any(dir, &["pytest.ini", "conftest.py", "tox.ini"]) {
        return true;
    }
    let configured = ["pyproject.toml", "setup.cfg"].iter().any(|name| {
        fs::read_to_string(dir.join(name)).is_ok_and(|text| PYTEST_SECTION.is_match(&text))
    });
    configured
        || PYTHON_TEST_DIRS
            .iter()
            .map(|folder| dir.join(folder))
            .any(|path| path.is_dir() && directory_has_file(&path, &PYTHON_TEST_FILE))
}
