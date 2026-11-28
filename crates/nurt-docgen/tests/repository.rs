// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-docgen/tests/repository.rs
// Responsibility:   Checks of the documentation of the repository against its catalog.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Checks of the documentation of the repository against its catalog.

// The helpers of this test file fail the test through `expect` when the repository cannot be read.
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

use nurt_docgen::catalog::{CORE_GROUP, Catalog};
use nurt_docgen::check::CatalogChecker;
use nurt_docgen::cli::{self, Tree};
use nurt_docgen::cookbook::{load_cookbook, recipe_count};
use nurt_docgen::html::check_structure;
use nurt_docgen::layout::Layout;
use nurt_docgen::manpages::{self, ManOptions};
use nurt_docgen::site;
use nurt_docgen::tasks::{CheckOptions, check_repository};

/// Minimum number of recipes in the cookbook of the first wave.
const MIN_RECIPES: usize = 150;

/// NAME line of `nurt(1)` as fixed by the product texts.
const PROGRAM_NAME_LINE: &str =
    "nurt - command shell and language where every change can be undone";

/// Sentence of the product description used on the site and in the pages.
const PRODUCT_SENTENCE: &str = "is a command shell and language where every change can be undone.";

/// Former product tagline, replaced by the product sentence.
const FORMER_TAGLINE: &str = "command console for AI agents and people";

/// Audience suffix reserved for the repository README and description.
const PEOPLE_AND_AGENTS: &str = "for people and AI agents";

#[test]
fn site_pages_use_the_product_sentence_only() {
    let layout = layout();
    let mut files = Vec::new();
    collect_files(&layout.site(), "md", &mut files);
    collect_files(&layout.site(), "html", &mut files);
    for path in &files {
        let text = std::fs::read_to_string(path).expect("page is readable");
        assert!(!text.contains(FORMER_TAGLINE), "{}", path.display());
        assert!(!text.contains(PEOPLE_AND_AGENTS), "{}", path.display());
    }
    let index = std::fs::read_to_string(layout.site().join("index.md")).expect("index exists");
    assert!(index.contains(&format!("Danaco Nurt (`nurt`) {PRODUCT_SENTENCE}")));
    let command_line =
        std::fs::read_to_string(layout.site().join("command-line.md")).expect("page exists");
    assert!(command_line.contains(&format!("`nurt` {PRODUCT_SENTENCE}")));
}

fn layout() -> Layout {
    let root = Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("..")
    .join("..");
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
    assert_eq!(catalog.group_size(CORE_GROUP), 34);
    assert_eq!(catalog.total(), 177);
    assert_eq!(catalog.optional_groups(), 14);
}

#[test]
fn documentation_passes_all_checks() {
    let problems = check_repository(&layout(), &CheckOptions::default()).expect("check runs");
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
    for name in std::iter::once("favicon.svg").chain(site::SITE_ASSETS) {
        let copied = std::fs::read(out.path().join(name)).expect("identity file is copied");
        let source = std::fs::read(layout.site().join(name)).expect("identity file exists");
        assert_eq!(copied, source, "{name} differs from the source");
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
    let tree = Tree::load(&layout).expect("the table of subcommands loads");
    let extra = tree
        .subcommands()
        .iter()
        .filter(|s| catalog.command(&s.name).is_none())
        .count();
    assert_eq!(count, catalog.total() + 2 + extra);
    let program = std::fs::read_to_string(out.path().join("nurt.1")).expect("nurt.1 is readable");
    assert_eq!(
        manpages::name_line(&program).as_deref(),
        Some(PROGRAM_NAME_LINE)
    );
    assert!(program.contains(".SH COMMANDS\n"));
    assert!(!program.contains(PEOPLE_AND_AGENTS));
    if let Ok(output) = Command::new("groff")
        .args(["-man", "-T", "ascii", "-P", "-cbou"])
        .arg(out.path().join("nurt.1"))
        .output()
        && output.status.success()
        && !output.stdout.is_empty()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            text.lines().any(|line| line.trim() == PROGRAM_NAME_LINE),
            "{text}"
        );
    }
    let Ok(status) = Command::new("mandoc")
        .args(["-T", "lint", "-W", "error"])
        .arg(out.path().join("nurt.1"))
        .arg(out.path().join("nurt-commands.7"))
        .arg(out.path().join("nurt-find.1"))
        .status()
    else {
        assert!(
            std::env::var("NURT_REQUIRE_MANDOC").as_deref() != Ok("1"),
            "NURT_REQUIRE_MANDOC is 1, but mandoc is not installed"
        );
        eprintln!("mandoc is not installed: the lint of the manual pages is skipped");
        return;
    };
    assert!(status.success());
}

#[test]
fn every_subcommand_has_a_manual_page_and_completions() {
    let layout = layout();
    let catalog = catalog(&layout);
    let tree = Tree::load(&layout).expect("the table of subcommands loads");
    let man = tempfile::tempdir().expect("temporary directory");
    let options = ManOptions {
        version: "0.1.0",
        date: "2026-01-01",
    };
    manpages::build(&layout, &catalog, man.path(), &options).expect("man pages build");
    let completions = tempfile::tempdir().expect("temporary directory");
    let scripts =
        cli::write_completions(&tree, completions.path()).expect("completions are written");
    assert_eq!(scripts.len(), cli::SHELLS.len());
    for sub in tree.subcommands() {
        let page = man.path().join(format!("nurt-{}.1", sub.name));
        assert!(page.is_file(), "no manual page for {}", sub.name);
        for script in &scripts {
            let text =
                std::fs::read_to_string(completions.path().join(script)).expect("script exists");
            assert!(text.contains(&sub.name), "{script} lacks {}", sub.name);
        }
    }
    let serve = std::fs::read_to_string(man.path().join("nurt-serve.1")).expect("serve page");
    assert!(serve.contains(".SH Framing") || serve.contains(".SH FRAMING"));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
