// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Consistency of the list of construct names with the source of the crate.
// Helpers outside `#[test]` functions fail the test on invalid fixtures.
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};

use nurt_contracts::error::ErrorCode;
use nurt_doc::constructs::{self, ALL};

/// Prefixes of construct names: the shared layer and the formats.
const PREFIXES: &[&str] = &[
    "doc",
    "env",
    "ini",
    "json",
    "jsonc",
    "csv",
    "tsv",
    "plist",
    "properties",
    "toml",
    "xml",
    "yaml",
];

fn is_construct_name(text: &str) -> bool {
    let Some((prefix, name)) = text.split_once('.') else {
        return false;
    };
    PREFIXES.contains(&prefix)
        && !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
}

fn sources(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("source directory is readable") {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            sources(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

/// String constants of the crate source whose value has the form of a
/// construct name, with their file.
fn declared_names() -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    sources(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let mut found = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(&file).expect("source is readable");
        for line in source.lines() {
            let Some((_, value)) = line.split_once("&str = \"") else {
                continue;
            };
            let Some((literal, _)) = value.split_once('"') else {
                continue;
            };
            if is_construct_name(literal) {
                found.push((file.clone(), literal.to_owned()));
            }
        }
    }
    found
}

#[test]
fn list_is_sorted_without_duplicates() {
    assert!(ALL.windows(2).all(|pair| pair[0] < pair[1]), "{ALL:?}");
}

#[test]
fn listed_names_are_well_formed_and_known() {
    for name in ALL {
        assert!(is_construct_name(name), "{name}");
        assert!(constructs::is_known(name), "{name}");
    }
    assert!(!constructs::is_known("doc.unknown"));
    assert!(!constructs::is_known(""));
}

#[test]
fn every_construct_constant_of_the_crate_is_listed() {
    let declared = declared_names();
    assert!(declared.len() >= ALL.len(), "{declared:?}");
    let unlisted: Vec<String> = declared
        .iter()
        .filter(|(_, name)| !constructs::is_known(name))
        .map(|(file, name)| format!("{name} in {}", file.display()))
        .collect();
    assert!(
        unlisted.is_empty(),
        "construct names missing from ALL:\n{}",
        unlisted.join("\n")
    );
}

#[test]
fn unsupported_error_names_the_construct_and_the_alternative() {
    let error = constructs::unsupported(constructs::TOML_NULL, "`null` cannot be written in TOML");
    assert_eq!(error.code, ErrorCode::UnsupportedConstruct);
    assert_eq!(error.found.as_deref(), Some("toml.null"));
    assert_eq!(
        error.message,
        "`null` cannot be written in TOML; nothing was changed, edit the text with `replace` or `edit` instead"
    );
}
