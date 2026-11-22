// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// A failed unwrap, expectation or panic is how a test reports a failure.
// Each test file uses a part of these helpers, so unused ones are expected.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]

use std::path::{Path, PathBuf};

use nurt_contracts::spec::CatalogIndex;
use nurt_help::catalog::Catalog;
use nurt_help::docs::Docs;

/// Root of the repository, relative to the manifest of the crate.
pub(crate) fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub(crate) fn read(relative: &str) -> String {
    let path = root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Names and texts of the Markdown files of a directory, ordered by name.
pub(crate) fn markdown_files(directory: &str) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = std::fs::read_dir(root().join(directory))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".md") && !name.starts_with('_') && name != "README.md")
        .map(|name| {
            let text = read(&format!("{directory}/{name}"));
            (name, text)
        })
        .collect();
    files.sort();
    files
}

pub(crate) fn docs() -> Docs {
    let pages = markdown_files("docs/commands");
    let guides = markdown_files("docs/guides");
    let pages: Vec<(&str, &str)> = pages
        .iter()
        .map(|(n, t)| (n.as_str(), t.as_str()))
        .collect();
    let guides: Vec<(&str, &str)> = guides
        .iter()
        .map(|(n, t)| (n.as_str(), t.as_str()))
        .collect();
    Docs::from_sources(&pages, &guides).unwrap()
}

pub(crate) fn catalog() -> Catalog {
    let index_text = read("catalog/_index.toml");
    let index: CatalogIndex = toml::from_str(&index_text).unwrap();
    let files: Vec<(String, String)> = index
        .files
        .iter()
        .map(|entry| (entry.file.clone(), read(&format!("catalog/{}", entry.file))))
        .collect();
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(n, t)| (n.as_str(), t.as_str()))
        .collect();
    Catalog::from_sources(&index_text, &files, docs()).unwrap()
}
