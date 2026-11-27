// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/u05_support/suite.rs
// Responsibility:   Loader of the YAML Test Suite in its source format.
// Dependencies:     saphyr
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Loader of the YAML Test Suite in its source format.
//!
//! Every file `<ID>.yaml` holds a list of cases. A case inherits each field
//! except `fail` from the previous case of the same file. Inputs use the
//! visual notation of the suite: `␣` is a space, `—»` and `»` a tab, `←` a
//! carriage return, `⇔` a byte order mark, `↵` marks a line end and `∎`
//! marks input without a final line break.

// The loader is shared by several test crates, each of which reads a
// different subset of its items.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use saphyr::{LoadableYamlNode, Yaml};

/// One case of the suite.
#[derive(Debug, Clone)]
pub(crate) struct SuiteCase {
    /// Identifier: `<ID>` for a single case, `<ID>-<nn>` otherwise.
    pub(crate) id: String,
    /// Input text.
    pub(crate) yaml: String,
    /// Expected event tree, one event per line, when the suite gives it.
    pub(crate) tree: Option<String>,
    /// Expected JSON values of the documents, when the suite gives them.
    pub(crate) json: Option<String>,
    /// Whether the input is invalid.
    pub(crate) fail: bool,
}

/// Directory of the suite inside the repository.
pub(crate) fn suite_dir() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../tests/d5/third_party/yaml/yaml-test-suite")
}

/// Every case of the suite, sorted by identifier.
pub(crate) fn load_cases() -> Vec<SuiteCase> {
    let mut files: Vec<PathBuf> = fs::read_dir(suite_dir())
        .expect("suite directory")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yaml"))
        .collect();
    files.sort();
    files.iter().flat_map(|path| load_file(path)).collect()
}

fn load_file(path: &Path) -> Vec<SuiteCase> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .expect("file name")
        .to_owned();
    let text = fs::read_to_string(path).expect("suite file");
    let docs = Yaml::load_from_str(&text).expect("suite file is valid YAML");
    let entries = docs
        .first()
        .and_then(Yaml::as_vec)
        .expect("suite file holds a list");
    let mut yaml: Option<String> = None;
    let mut tree: Option<String> = None;
    let mut json: Option<String> = None;
    let mut cases = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let field = |name: &str| {
            entry
                .as_mapping_get(name)
                .and_then(Yaml::as_str)
                .map(visual_to_raw)
        };
        if let Some(value) = field("yaml") {
            yaml = Some(value);
        }
        if let Some(value) = field("tree") {
            tree = Some(value);
        }
        if entry.as_mapping_get("json").is_some() {
            json = entry
                .as_mapping_get("json")
                .and_then(Yaml::as_str)
                .map(str::to_owned);
        }
        if entry.as_mapping_get("skip").is_some() {
            continue;
        }
        let fail = entry
            .as_mapping_get("fail")
            .and_then(Yaml::as_bool)
            .unwrap_or(false);
        let id = if entries.len() > 1 {
            format!("{stem}-{index:02}")
        } else {
            stem.clone()
        };
        cases.push(SuiteCase {
            id,
            yaml: yaml.clone().expect("case has an input"),
            tree: tree.clone(),
            json: if fail { None } else { json.clone() },
            fail,
        });
    }
    cases
}

/// Converts the visual notation of the suite to the raw input.
pub(crate) fn visual_to_raw(visual: &str) -> String {
    let mut raw = visual.to_owned();
    for (pattern, replacement) in [
        ("␣", " "),
        ("»", "\t"),
        ("—", ""),
        ("←", "\r"),
        ("⇔", "\u{FEFF}"),
        ("↵", ""),
        ("∎\n", ""),
    ] {
        raw = raw.replace(pattern, replacement);
    }
    raw
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
