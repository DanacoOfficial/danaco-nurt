// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-contracts/tests/product_texts.rs
// Responsibility:   Tests that the product texts outside the code use the canonical sentence.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests that the product texts outside the code use the canonical sentence
//! of `nurt_contracts::product`: the `description` of every package that
//! builds the `nurt` executable or is named `nurt`, the first paragraph of
//! `README.md` and every source of the manual page.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::product::{PRODUCT_NAME, PROGRAM_NAME, TAGLINE, man_name};

/// File name of the manual page source, with an optional format suffix such
/// as `nurt.1.md`.
const MANUAL_PAGE: &str = "nurt.1";

/// Directories that hold build output or version control data, not sources.
const SKIPPED_DIRECTORIES: &[&str] = &["target", ".git", "node_modules"];

/// Source files of the manual page in the repository.
fn manual_page_sources() -> Vec<PathBuf> {
    let mut sources = Vec::new();
    let mut pending = vec![root()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                if !SKIPPED_DIRECTORIES.contains(&name.as_str()) {
                    pending.push(entry.path());
                }
            } else if kind.is_file()
                && name
                    .strip_prefix(MANUAL_PAGE)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
            {
                sources.push(entry.path());
            }
        }
    }
    sources.sort();
    sources
}

#[test]
fn manual_page_sources_contain_the_name_line() {
    let expected = man_name();
    for source in manual_page_sources() {
        // roff writes the hyphen of the NAME line as an escaped minus sign.
        let text = fs::read_to_string(&source).unwrap().replace("\\-", "-");
        assert!(
            text.contains(&expected),
            "{} does not contain {expected:?}",
            source.display()
        );
    }
}

/// Suffix that names the audience; it appears only in the repository
/// description and in the first paragraph of `README.md`.
const AUDIENCE_SUFFIX: &str = "built for people and AI agents";

/// Second sentence of the first paragraph of `README.md`.
const PLATFORMS_SENTENCE: &str = "One tool for Linux, Windows and macOS.";

fn root() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../..")
}

fn lowercase_first(text: &str) -> String {
    let mut characters = text.chars();
    let first = characters.next().unwrap();
    first.to_ascii_lowercase().to_string() + characters.as_str()
}

/// Manifests of the packages that build the `nurt` executable or are named
/// `nurt`, as `(package name, description)`.
fn program_packages() -> Vec<(String, Option<String>)> {
    let mut packages = Vec::new();
    for entry in fs::read_dir(root().join("crates")).unwrap() {
        let manifest = entry.unwrap().path().join("Cargo.toml");
        let Ok(text) = fs::read_to_string(&manifest) else {
            continue;
        };
        let value: toml::Table = text.parse().unwrap();
        let package = value["package"].as_table().unwrap();
        let name = package["name"].as_str().unwrap().to_owned();
        let builds_program = value
            .get("bin")
            .and_then(toml::Value::as_array)
            .is_some_and(|targets| {
                targets.iter().any(|target| {
                    target.get("name").and_then(toml::Value::as_str) == Some(PROGRAM_NAME)
                })
            });
        if builds_program || name == PROGRAM_NAME {
            let description = package
                .get("description")
                .and_then(toml::Value::as_str)
                .map(str::to_owned);
            packages.push((name, description));
        }
    }
    packages.sort();
    packages
}

/// First paragraph of `README.md` after the title, with line breaks replaced
/// by spaces.
fn readme_first_paragraph() -> String {
    let text = fs::read_to_string(root().join("README.md")).unwrap();
    let mut lines = text
        .lines()
        .map(str::trim_end)
        .skip_while(|line| !line.starts_with("# "))
        .skip(1)
        .skip_while(|line| line.is_empty());
    let mut paragraph = Vec::new();
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
        paragraph.push(line);
    }
    paragraph.join(" ")
}

#[test]
fn program_package_description_is_the_canonical_sentence() {
    let packages = program_packages();
    assert!(
        !packages.is_empty(),
        "no package builds the {PROGRAM_NAME} executable"
    );
    for (name, description) in packages {
        assert_eq!(
            description.as_deref(),
            Some(TAGLINE),
            "description of the package {name}"
        );
    }
}

#[test]
fn readme_first_paragraph_contains_the_canonical_sentence() {
    let expected = format!(
        "{PRODUCT_NAME} is a {}, {AUDIENCE_SUFFIX}. {PLATFORMS_SENTENCE}",
        lowercase_first(TAGLINE)
    );
    let paragraph = readme_first_paragraph();
    assert_eq!(paragraph, expected, "first paragraph of README.md");
    assert!(
        paragraph.is_ascii(),
        "first paragraph of README.md is not ASCII"
    );
}

#[test]
fn canonical_sentence_has_no_audience_suffix() {
    assert!(TAGLINE.is_ascii());
    assert!(!TAGLINE.ends_with('.'));
    assert!(!TAGLINE.contains("AI agents"));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
