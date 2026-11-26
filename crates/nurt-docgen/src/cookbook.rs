// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Cookbook: recipes grouped in categories, one Markdown file per category.
//!
//! A category file has a front matter with `category` and `title`. Every
//! recipe is a level-2 section: the title, a paragraph that states the
//! problem, one block `nurt example`, optionally `json fs-before` and
//! `text expected`, and optionally the equivalents in `bash` and
//! `powershell` blocks.

use std::path::Path;

use crate::check::ProgramChecker;
use crate::error::{DocgenError, Result, list_files, read_text};
use crate::examples::{EXAMPLE_INFO, EXPECTED_INFO, FS_BEFORE_INFO, check_snapshot};
use crate::markdown::{Block, parse, plain_text, split_front_matter};

/// Recipe of the cookbook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    /// Title of the recipe.
    pub title: String,
    /// Statement of the problem.
    pub problem: String,
    /// Programs of the `nurt example` blocks.
    pub programs: Vec<String>,
    /// Snapshot of the file system before the run.
    pub fs_before: Option<String>,
    /// Fragment of the expected text result.
    pub expected: Option<String>,
    /// Equivalent in bash.
    pub bash: Option<String>,
    /// Equivalent in PowerShell.
    pub powershell: Option<String>,
}

/// Category of recipes read from one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    /// Identifier, equal to the file name without the extension.
    pub id: String,
    /// Title shown on the site.
    pub title: String,
    /// Recipes in file order.
    pub recipes: Vec<Recipe>,
}

/// Parses the text of a category file.
///
/// # Errors
///
/// Returns an error when the file has no front matter or no title.
pub fn parse_category(id: &str, source: &str) -> Result<Category> {
    let (front, body) = split_front_matter(source);
    let front =
        front.ok_or_else(|| DocgenError::invalid(id, "the category file has no front matter"))?;
    let title = front
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.trim() == "title")
        .map(|(_, value)| value.trim().to_string())
        .ok_or_else(|| DocgenError::invalid(id, "the front matter has no title"))?;
    let mut recipes: Vec<Recipe> = Vec::new();
    for block in parse(body).blocks {
        match block {
            Block::Heading { level: 2, content } => recipes.push(Recipe {
                title: plain_text(&content),
                problem: String::new(),
                programs: Vec::new(),
                fs_before: None,
                expected: None,
                bash: None,
                powershell: None,
            }),
            other => {
                if let Some(recipe) = recipes.last_mut() {
                    absorb(recipe, other);
                }
            }
        }
    }
    Ok(Category {
        id: id.to_string(),
        title,
        recipes,
    })
}

fn absorb(recipe: &mut Recipe, block: Block) {
    match block {
        Block::Paragraph(content) if recipe.problem.is_empty() => {
            recipe.problem = plain_text(&content);
        }
        Block::Code { info, text } => match info.as_str() {
            EXAMPLE_INFO => recipe.programs.push(text),
            FS_BEFORE_INFO => recipe.fs_before = Some(text),
            EXPECTED_INFO => recipe.expected = Some(text),
            "bash" => recipe.bash = Some(text),
            "powershell" => recipe.powershell = Some(text),
            _ => {}
        },
        _ => {}
    }
}

/// Loads all category files of a directory, sorted by file name.
///
/// # Errors
///
/// Returns an error when the directory or a file cannot be read or parsed.
pub fn load_cookbook(dir: &Path) -> Result<Vec<Category>> {
    let mut categories = Vec::new();
    for path in list_files(dir, "md")? {
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        if id == "README" {
            continue;
        }
        categories.push(parse_category(&id, &read_text(&path)?)?);
    }
    Ok(categories)
}

/// Returns the number of recipes in the categories.
#[must_use]
pub fn recipe_count(categories: &[Category]) -> usize {
    categories.iter().map(|c| c.recipes.len()).sum()
}

/// Problem found in a recipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeIssue {
    /// Category and recipe title.
    pub recipe: String,
    /// Description of the problem.
    pub message: String,
}

/// Checks all recipes: structure, snapshots and programs.
///
/// A recipe needs a problem statement, exactly one program that passes the
/// checker, an expected result fragment and a valid snapshot when one is
/// given. Recipe titles are unique in the cookbook.
#[must_use]
pub fn check_cookbook(categories: &[Category], checker: &dyn ProgramChecker) -> Vec<RecipeIssue> {
    let mut issues = Vec::new();
    let mut titles: Vec<String> = Vec::new();
    for category in categories {
        for recipe in &category.recipes {
            let name = format!("{}: {}", category.id, recipe.title);
            let mut report = |message: String| {
                issues.push(RecipeIssue {
                    recipe: name.clone(),
                    message,
                });
            };
            check_recipe(recipe, checker, &mut report);
            if titles.contains(&recipe.title) {
                report("the title is used by another recipe".to_string());
            }
            titles.push(recipe.title.clone());
        }
    }
    issues
}

fn check_recipe(recipe: &Recipe, checker: &dyn ProgramChecker, report: &mut dyn FnMut(String)) {
    if recipe.problem.is_empty() {
        report("the recipe has no problem statement".to_string());
    }
    match recipe.programs.as_slice() {
        [program] => {
            for issue in checker.check(program) {
                report(issue.to_string());
            }
        }
        other => report(format!(
            "the recipe has {} programs, exactly one is required",
            other.len()
        )),
    }
    if recipe.expected.is_none() {
        report("the recipe has no expected result".to_string());
    }
    if let Some(snapshot) = &recipe.fs_before {
        for problem in check_snapshot(snapshot) {
            report(problem);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::fixtures::catalog;
    use crate::check::CatalogChecker;

    const FILE: &str = "---\ncategory: search\ntitle: Search\n---\n\n## Count Go files\n\nCount the Go files of a project.\n\n```nurt example\nfind \"/home/user/app\" glob: \"**/*.go\" |> count\n```\n\n```text expected\n3\n```\n\n```bash\nfind /home/user/app -name '*.go' | wc -l\n```\n\n## Second\n\nSecond problem.\n\n```nurt example\nfnid \"/a\"\n```\n";

    #[test]
    fn parses_recipes_with_blocks() {
        let category = parse_category("search", FILE).expect("category parses");
        assert_eq!(category.title, "Search");
        assert_eq!(category.recipes.len(), 2);
        let first = &category.recipes[0];
        assert_eq!(first.title, "Count Go files");
        assert_eq!(first.problem, "Count the Go files of a project.");
        assert_eq!(first.expected.as_deref(), Some("3"));
        assert!(first.bash.as_deref().is_some_and(|b| b.starts_with("find")));
        assert_eq!(first.powershell, None);
    }

    #[test]
    fn category_without_title_is_an_error() {
        assert!(parse_category("x", "---\ncategory: x\n---\n").is_err());
    }

    #[test]
    fn check_accepts_valid_recipe_and_reports_invalid_one() {
        let category = parse_category("search", FILE).expect("category parses");
        let catalog = catalog();
        let issues = check_cookbook(&[category], &CatalogChecker::new(&catalog));
        let messages: Vec<&str> = issues.iter().map(|i| i.message.as_str()).collect();
        assert!(issues.iter().all(|i| i.recipe == "search: Second"));
        assert!(messages.iter().any(|m| m.starts_with("E_UNKNOWN_COMMAND")));
        assert!(messages.iter().any(|m| m.contains("no expected result")));
    }

    #[test]
    fn check_reports_duplicate_titles() {
        let category = parse_category("search", FILE).expect("category parses");
        let catalog = catalog();
        let issues = check_cookbook(
            &[category.clone(), category],
            &CatalogChecker::new(&catalog),
        );
        assert!(
            issues
                .iter()
                .any(|i| i.message.contains("used by another recipe"))
        );
    }

    #[test]
    fn counts_recipes() {
        let category = parse_category("search", FILE).expect("category parses");
        assert_eq!(recipe_count(&[category.clone(), category]), 4);
    }

    #[test]
    fn a_title_may_contain_a_colon_and_extra_paragraphs_are_ignored() {
        let source = "---\ncategory: x\ntitle: Search: advanced\n---\n\n## One\n\nProblem.\n\nMore text.\n\n```nurt example\ncount\n```\n";
        let category = parse_category("x", source).expect("category parses");
        assert_eq!(category.title, "Search: advanced");
        assert_eq!(category.recipes[0].problem, "Problem.");
        assert_eq!(category.recipes[0].programs, ["count"]);
    }

    #[test]
    fn a_category_without_front_matter_is_an_error() {
        assert!(parse_category("x", "## One\n\nProblem.\n").is_err());
    }

    #[test]
    fn blocks_before_the_first_recipe_are_ignored() {
        let source = "---\ncategory: x\ntitle: X\n---\n\nIntroduction.\n\n```bash\nls\n```\n";
        let category = parse_category("x", source).expect("category parses");
        assert!(category.recipes.is_empty());
    }

    #[test]
    fn a_recipe_needs_exactly_one_program_and_a_problem() {
        let source = "---\ncategory: x\ntitle: X\n---\n\n## Two programs\n\n```nurt example\ncount\n```\n\n```nurt example\ncount\n```\n\n```text expected\n1\n```\n";
        let category = parse_category("x", source).expect("category parses");
        let catalog = catalog();
        let issues = check_cookbook(&[category], &CatalogChecker::new(&catalog));
        let messages: Vec<&str> = issues.iter().map(|i| i.message.as_str()).collect();
        assert!(messages.iter().any(|m| m.contains("no problem statement")));
        assert!(
            messages
                .iter()
                .any(|m| m.contains("exactly one is required"))
        );
    }

    #[test]
    fn load_cookbook_reads_categories_in_file_order_and_skips_the_readme() {
        let dir = tempfile::tempdir().expect("temporary directory");
        for (name, title) in [("b", "B"), ("a", "A")] {
            let text = format!("---\ncategory: {name}\ntitle: {title}\n---\n");
            std::fs::write(dir.path().join(format!("{name}.md")), text).expect("write");
        }
        std::fs::write(dir.path().join("README.md"), "Not a category.\n").expect("write");
        let categories = load_cookbook(dir.path()).expect("cookbook loads");
        let ids: Vec<&str> = categories.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
    }
}
