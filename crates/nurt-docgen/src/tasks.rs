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
        if !options.allow_missing_pages {
            problems.push(format!(
                "{}: the directory of the command pages is missing",
                dir.display()
            ));
        }
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
        if !SheetComments::of(&current).has_producer_header() {
            problems.push(format!(
                "{}: the sheet has no producer header",
                path.display()
            ));
        }
    }
    Ok(())
}

/// Producer header written into a sheet that has none.
const PRODUCER_HEADER: &str = "<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->\n<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->\n<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->\n";

fn is_comment_line(line: &str) -> bool {
    line.starts_with("<!--") && line.ends_with("-->")
}

/// Joins the lines, each with its line break.
fn lines_with_breaks(lines: &[&str]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

/// The single-line HTML comments of a sheet that regenerating keeps: the
/// producer header and footer. They stand before the front matter, directly
/// after it, or at the end of the file.
#[derive(Debug, Default, PartialEq, Eq)]
struct SheetComments {
    before: String,
    after_front: String,
    footer: String,
}

impl SheetComments {
    fn of(text: &str) -> Self {
        let lines: Vec<&str> = text.lines().collect();
        let before_count = lines.iter().take_while(|l| is_comment_line(l)).count();
        let mut at = before_count;
        let mut after_front = Vec::new();
        if lines.get(at) == Some(&"---")
            && let Some(close) = lines[at + 1..].iter().position(|l| *l == "---")
        {
            at += close + 2;
            after_front = lines[at..]
                .iter()
                .take_while(|l| is_comment_line(l))
                .copied()
                .collect();
        }
        let body_start = at + after_front.len();
        let body = &lines[body_start.min(lines.len())..];
        let mut footer: Vec<&str> = body
            .iter()
            .rev()
            .take_while(|l| is_comment_line(l) || l.trim().is_empty())
            .filter(|l| is_comment_line(l))
            .copied()
            .collect();
        footer.reverse();
        Self {
            before: lines_with_breaks(&lines[..before_count]),
            after_front: lines_with_breaks(&after_front),
            footer: lines_with_breaks(&footer),
        }
    }

    /// A producer header is a comment line with the copyright notice.
    fn has_producer_header(&self) -> bool {
        self.before
            .lines()
            .chain(self.after_front.lines())
            .any(|line| line.contains("Copyright (c)"))
    }

    /// `rendered` with the comments placed back; a missing header is added.
    fn apply(&self, rendered: &str) -> String {
        let header = if self.has_producer_header() {
            self.after_front.as_str()
        } else {
            PRODUCER_HEADER
        };
        let (front, body) = match rendered
            .strip_prefix("---\n")
            .and_then(|rest| rest.find("\n---\n").map(|i| i + 4 + 5))
        {
            Some(end) => rendered.split_at(end),
            None => ("", rendered),
        };
        let mut out = format!("{}{front}{header}{body}", self.before);
        if !self.footer.is_empty() {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&self.footer);
        }
        out
    }
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
        let comments = if path.exists() {
            SheetComments::of(&read_text(&path)?)
        } else {
            SheetComments::default()
        };
        write_text(&path, &comments.apply(&cheatsheet::render(&table)))?;
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
    fn regenerating_a_sheet_keeps_its_producer_header() {
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
        let sheet = write_cheatsheets(&layout)
            .expect("sheets written")
            .remove(0);
        let generated = fs::read_to_string(&sheet).expect("sheet");
        assert!(generated.contains(PRODUCER_HEADER));
        let header = "<!-- Copyright (c) 2026 Example -->\n<!-- https://example.org -->\n";
        let custom = generated.replace(PRODUCER_HEADER, header);
        let footer = "<!-- footer of the producer -->\n";
        fs::write(&sheet, format!("{custom}{footer}")).expect("write");
        write_cheatsheets(&layout).expect("sheets written again");
        let again = fs::read_to_string(&sheet).expect("sheet");
        assert_eq!(again, format!("{custom}{footer}"));
        let before = format!("<!-- first -->\n{generated}");
        fs::write(&sheet, &before).expect("write");
        write_cheatsheets(&layout).expect("sheets written");
        assert!(
            fs::read_to_string(&sheet)
                .expect("sheet")
                .starts_with("<!-- first -->\n---\n")
        );
        assert_eq!(SheetComments::of("text\n<!-- late -->\n").before, "");
        assert_eq!(SheetComments::of(""), SheetComments::default());
    }

    #[test]
    fn check_reports_a_sheet_without_a_producer_header() {
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
        let sheet = write_cheatsheets(&layout)
            .expect("sheets written")
            .remove(0);
        let generated = fs::read_to_string(&sheet).expect("sheet");
        let ok = check_repository(&layout, &CheckOptions::default()).expect("check runs");
        assert!(!ok.iter().any(|p| p.contains("producer header")));
        fs::write(&sheet, generated.replace(PRODUCER_HEADER, "")).expect("write");
        let problems = check_repository(&layout, &CheckOptions::default()).expect("check runs");
        assert!(problems.iter().any(|p| p.contains("no producer header")));
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

    #[test]
    fn a_missing_directory_of_pages_is_reported_unless_missing_pages_are_allowed() {
        let dir = repository();
        let layout = Layout::new(dir.path());
        let strict = check_repository(&layout, &CheckOptions::default()).expect("check runs");
        assert!(
            strict
                .iter()
                .any(|p| p.contains("directory of the command pages"))
        );
        let relaxed = check_repository(
            &layout,
            &CheckOptions {
                allow_missing_pages: true,
                ..CheckOptions::default()
            },
        )
        .expect("check runs");
        assert!(
            !relaxed
                .iter()
                .any(|p| p.contains("directory of the command pages"))
        );
    }
}
