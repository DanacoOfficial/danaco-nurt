// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Checks of the documentation of the repository against its catalog.

// The helpers of this test file fail the test through `expect` when the repository cannot be read.
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

use nurt_docgen::catalog::{CORE_GROUP, Catalog};
use nurt_docgen::check::CatalogChecker;
use nurt_docgen::cookbook::{load_cookbook, recipe_count};
use nurt_docgen::html::check_structure;
use nurt_docgen::layout::Layout;
use nurt_docgen::manpages::{self, ManOptions};
use nurt_docgen::site;
use nurt_docgen::tasks::{CheckOptions, check_repository};

/// Minimum number of recipes in the cookbook of the first wave.
const MIN_RECIPES: usize = 150;

fn layout() -> Layout {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    Layout::new(&root)
}

fn catalog(layout: &Layout) -> Catalog {
    Catalog::load(&layout.catalog()).expect("the catalog of the repository loads")
}

fn collect_files(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("directory is readable") {
        let path = entry.expect("entry is readable").path();
        if path.is_dir() {
            collect_files(&path, extension, out);
        } else if path.extension().is_some_and(|e| e == extension) {
            out.push(path);
        }
    }
}

#[test]
fn catalog_has_the_documented_command_counts() {
    let catalog = catalog(&layout());
    assert_eq!(catalog.group_size(CORE_GROUP), 33);
    assert_eq!(catalog.total(), 148);
    assert_eq!(catalog.optional_groups(), 14);
}

#[test]
fn documentation_passes_all_checks() {
    let options = CheckOptions {
        allow_missing_pages: true,
        ..CheckOptions::default()
    };
    let problems = check_repository(&layout(), &options).expect("check runs");
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn cookbook_has_enough_recipes_and_all_bind() {
    let layout = layout();
    let categories = load_cookbook(&layout.cookbook()).expect("cookbook loads");
    assert!(recipe_count(&categories) >= MIN_RECIPES);
    let catalog = catalog(&layout);
    let checker = CatalogChecker::new(&catalog);
    let issues = nurt_docgen::cookbook::check_cookbook(&categories, &checker);
    assert!(issues.is_empty(), "{issues:?}");
}

#[test]
fn site_builds_valid_pages_without_unresolved_links() {
    let layout = layout();
    let catalog = catalog(&layout);
    let out = tempfile::tempdir().expect("temporary directory");
    let report = site::build(&layout, &catalog, out.path(), "0.1.0").expect("site builds");
    assert!(
        report.unresolved_links.is_empty(),
        "{:?}",
        report.unresolved_links
    );
    let mut pages = Vec::new();
    collect_files(out.path(), "html", &mut pages);
    assert_eq!(pages.len(), report.pages);
    for page in pages {
        let html = std::fs::read_to_string(&page).expect("page is readable");
        let problems = check_structure(&html);
        assert!(problems.is_empty(), "{}: {problems:?}", page.display());
        assert!(
            !html.contains("<script"),
            "{} uses a script",
            page.display()
        );
        assert!(!html.contains("http://fonts") && !html.contains("https://fonts"));
    }
    let index =
        std::fs::read_to_string(out.path().join("search-index.json")).expect("index exists");
    let parsed: serde_json::Value = serde_json::from_str(&index).expect("index is JSON");
    assert!(parsed.as_array().is_some_and(|a| a.len() == report.pages));
}

#[test]
fn manual_pages_exist_for_every_command_and_lint_when_mandoc_is_available() {
    let layout = layout();
    let catalog = catalog(&layout);
    let out = tempfile::tempdir().expect("temporary directory");
    let options = ManOptions {
        version: "0.1.0",
        date: "2026-01-01",
    };
    let count = manpages::build(&layout, &catalog, out.path(), &options).expect("man pages build");
    assert_eq!(count, catalog.total() + 2);
    let Ok(status) = Command::new("mandoc")
        .args(["-T", "lint", "-W", "error"])
        .arg(out.path().join("nurt.1"))
        .arg(out.path().join("nurt-commands.7"))
        .arg(out.path().join("nurt-find.1"))
        .status()
    else {
        return;
    };
    assert!(status.success());
}
