// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Operations of the generator over a repository: checks and generated files.

use std::path::PathBuf;

use crate::catalog::Catalog;
use crate::cheatsheet::{self, Table};
use crate::check::CatalogChecker;
use crate::consistency::{Facts, check_examples_in_tree, check_tree};
use crate::cookbook::{check_cookbook, load_cookbook};
use crate::error::{Result, list_files, read_text, write_text};
use crate::layout::Layout;
use crate::page::{check_page, load_page};

/// Options of the repository check.
#[derive(Debug, Clone, Copy, Default)]
pub struct CheckOptions {
    /// A command without a page is not a problem.
    pub allow_missing_pages: bool,
    /// Measured token count of the core description.
    pub core_tokens: Option<u64>,
    /// Measured token count of the whole description.
    pub total_tokens: Option<u64>,
}

fn is_page_file(name: &str) -> bool {
    name.ends_with(".md") && !name.starts_with('_') && name != "README.md"
}

fn check_command_pages(
    layout: &Layout,
    catalog: &Catalog,
    options: &CheckOptions,
    problems: &mut Vec<String>,
) -> Result<()> {
    let dir = layout.commands();
    if !dir.exists() {
        return Ok(());
    }
    for spec in catalog.commands() {
        let path = dir.join(format!("{}.md", spec.name));
        if !path.exists() {
            if !options.allow_missing_pages {
                problems.push(format!("{}: the command has no page", spec.name));
            }
            continue;
        }
        let page = load_page(&path)?;
        problems.extend(
            check_page(catalog, spec, &page)
                .into_iter()
                .map(|issue| format!("{}: {}", issue.command, issue.message)),
        );
    }
    for path in list_files(&dir, "md")? {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let stem = name.trim_end_matches(".md");
        if is_page_file(name) && catalog.command(stem).is_none() {
            problems.push(format!(
                "{name}: the page belongs to no command of the catalog"
            ));
        }
    }
    Ok(())
}

/// Loads the correction tables found in the cheat sheet directory.
///
/// # Errors
///
/// Returns an error when the directory or a table cannot be read.
pub fn load_tables(layout: &Layout) -> Result<Vec<Table>> {
    let dir = layout.cheatsheets();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    list_files(&dir, "toml")?
        .iter()
        .map(|path| cheatsheet::load_table(path))
        .collect()
}

fn check_sheets(
    layout: &Layout,
    checker: &CatalogChecker<'_>,
    problems: &mut Vec<String>,
) -> Result<()> {
    for table in load_tables(layout)? {
        for problem in cheatsheet::check_table(&table, checker) {
            problems.push(format!("{}: {problem}", table.shell));
        }
        let path = layout
            .cheatsheets()
            .join(cheatsheet::sheet_file_name(&table));
        let current = if path.exists() {
            read_text(&path)?
        } else {
            String::new()
        };
        if without_comment_lines(&current).trim() != cheatsheet::render(&table).trim() {
            problems.push(format!(
                "{}: the sheet is not generated from the table",
                path.display()
            ));
        }
    }
    Ok(())
}

/// Removes single-line HTML comments, which carry the producer header and are not generated.
fn without_comment_lines(text: &str) -> String {
    text.lines()
        .filter(|line| !(line.starts_with("<!--") && line.ends_with("-->")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Checks the documentation of a repository and returns the problems found.
///
/// The checks cover the catalog, the command pages, the cookbook, the cheat
/// sheets, the numbers stated in `docs/`, the absence of sentences that
/// narrow the scope, and the examples of the pages and general documents.
///
/// # Errors
///
/// Returns an error when an input cannot be read or parsed.
pub fn check_repository(layout: &Layout, options: &CheckOptions) -> Result<Vec<String>> {
    let catalog = Catalog::load(&layout.catalog())?;
    let checker = CatalogChecker::new(&catalog);
    let mut problems = Vec::new();
    check_command_pages(layout, &catalog, options, &mut problems)?;
    if layout.cookbook().exists() {
        let categories = load_cookbook(&layout.cookbook())?;
        problems.extend(
            check_cookbook(&categories, &checker)
                .into_iter()
                .map(|issue| format!("{}: {}", issue.recipe, issue.message)),
        );
    }
    check_sheets(layout, &checker, &mut problems)?;
    let facts = Facts::from_catalog(&catalog, options.core_tokens, options.total_tokens);
    problems.extend(check_tree(&layout.docs(), &facts)?);
    for dir in [layout.commands(), layout.site()] {
        if dir.exists() {
            problems.extend(check_examples_in_tree(&dir, &checker)?);
        }
    }
    Ok(problems)
}

/// Writes the cheat sheets generated from the correction tables and returns
/// the paths written.
///
/// # Errors
///
/// Returns an error when a table cannot be read or a sheet cannot be written.
pub fn write_cheatsheets(layout: &Layout) -> Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    for table in load_tables(layout)? {
        let path = layout
            .cheatsheets()
            .join(cheatsheet::sheet_file_name(&table));
        write_text(&path, &cheatsheet::render(&table))?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const INDEX: &str = "format = \"nurt-catalog-index/1\"\ntotal = 1\n\n[[file]]\nname = \"core\"\nfile = \"core.toml\"\nsummary = \"Core commands.\"\ncommands = 1\n";
    const CORE: &str = "format = \"nurt-catalog/1\"\ngroup = \"core\"\nsummary = \"Core commands.\"\n\n[[command]]\nname = \"count\"\ncategory = \"transform\"\nreversibility = \"read\"\neffect = \"read\"\nimplementation = \"builtin\"\ninput = [\"any\"]\noutput = [\"int\"]\nsummary = \"Count the input records.\"\n";
    const TABLE: &str = "format = \"nurt-correction-table/1\"\nshell = \"bash\"\ntitle = \"Nurt for bash users\"\n\n[[entry]]\nid = \"B1\"\ntopic = \"Files\"\nhabit = \"wc -l\"\ncanonical = \"find \\\"/home/user\\\" |> count\"\noutcome = \"corrected\"\n";

    fn repository() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temporary directory");
        let root = dir.path();
        fs::create_dir_all(root.join("catalog")).expect("create");
        fs::create_dir_all(root.join("docs").join("cheatsheets")).expect("create");
        fs::write(root.join("catalog").join("_index.toml"), INDEX).expect("write");
        fs::write(root.join("catalog").join("core.toml"), CORE).expect("write");
        dir
    }

    #[test]
    fn check_reports_stage_command_in_cheatsheet_and_stale_sheet() {
        let dir = repository();
        fs::write(
            dir.path()
                .join("docs")
                .join("cheatsheets")
                .join("bash.toml"),
            TABLE,
        )
        .expect("write");
        let layout = Layout::new(dir.path());
        let problems = check_repository(&layout, &CheckOptions::default()).expect("check runs");
        assert!(
            problems
                .iter()
                .any(|p| p.contains("not generated from the table"))
        );
        assert!(problems.iter().any(|p| p.contains("E_UNKNOWN_COMMAND")));
    }

    #[test]
    fn written_cheatsheets_agree_with_the_check() {
        let dir = repository();
        let table = TABLE.replace("find \\\"/home/user\\\" |> count", "count");
        fs::write(
            dir.path()
                .join("docs")
                .join("cheatsheets")
                .join("bash.toml"),
            table,
        )
        .expect("write");
        let layout = Layout::new(dir.path());
        let written = write_cheatsheets(&layout).expect("sheets written");
        assert_eq!(written.len(), 1);
        let problems = check_repository(&layout, &CheckOptions::default()).expect("check runs");
        assert!(!problems.iter().any(|p| p.contains("not generated")));
    }

    #[test]
    fn check_reports_pages_of_unknown_commands() {
        let dir = repository();
        fs::create_dir_all(dir.path().join("docs").join("commands")).expect("create");
        fs::write(
            dir.path().join("docs").join("commands").join("ghost.md"),
            "---\ncommand: ghost\n---\n# ghost\n",
        )
        .expect("write");
        let layout = Layout::new(dir.path());
        let problems = check_repository(
            &layout,
            &CheckOptions {
                allow_missing_pages: true,
                ..CheckOptions::default()
            },
        )
        .expect("check runs");
        assert!(problems.iter().any(|p| p.contains("belongs to no command")));
    }

    #[test]
    fn missing_pages_are_reported_unless_allowed() {
        let dir = repository();
        fs::create_dir_all(dir.path().join("docs").join("commands")).expect("create");
        let layout = Layout::new(dir.path());
        let strict = check_repository(&layout, &CheckOptions::default()).expect("check runs");
        assert!(strict.iter().any(|p| p.contains("has no page")));
        let relaxed = check_repository(
            &layout,
            &CheckOptions {
                allow_missing_pages: true,
                ..CheckOptions::default()
            },
        )
        .expect("check runs");
        assert!(!relaxed.iter().any(|p| p.contains("has no page")));
    }
}
