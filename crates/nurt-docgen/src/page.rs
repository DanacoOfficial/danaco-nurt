// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Command pages (`docs/commands/<command>.md`): parsing, composition and checks.
//!
//! A page has a front matter block, a title and the sections `Synopsis`,
//! `Description`, `Parameters`, `Input and output`, `Effect`, `Examples`,
//! `Errors` and `Related`. The sections `Synopsis`, `Parameters`,
//! `Input and output` and `Effect` are generated from the catalog; a page
//! contains the generated text, optionally followed by written notes.

use std::collections::BTreeMap;
use std::path::Path;

use nurt_contracts::spec::StageSpec;

use crate::catalog::{CALL_PARAMETERS, Catalog};
use crate::error::{DocgenError, Result, read_text};
use crate::examples::{check_snapshot, extract};
use crate::markdown::{parse, split_front_matter};
use crate::reference::{
    effect_section, io_section, parameters_table, synopsis, written_descriptions,
};

/// Headings of the sections of a page, in order.
pub const SECTIONS: [&str; 8] = [
    "Synopsis",
    "Description",
    "Parameters",
    "Input and output",
    "Effect",
    "Examples",
    "Errors",
    "Related",
];

/// Minimum number of examples on a page.
pub const MIN_EXAMPLES: usize = 3;

/// Front matter of a page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrontMatter {
    /// Full command name.
    pub command: String,
    /// Group of the command.
    pub group: String,
    /// Summary, equal to the catalog summary.
    pub summary: String,
    /// Version that introduced the command.
    pub since: String,
}

/// Section of a page: the heading and the Markdown text under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Heading text.
    pub heading: String,
    /// Body in Markdown, without the heading.
    pub body: String,
}

/// Parsed command page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandPage {
    /// Front matter.
    pub front: FrontMatter,
    /// Text of the level-1 heading.
    pub title: String,
    /// Level-2 sections in source order.
    pub sections: Vec<Section>,
}

impl CommandPage {
    /// Returns the body of the section with the given heading.
    #[must_use]
    pub fn section(&self, heading: &str) -> Option<&str> {
        self.sections
            .iter()
            .find(|section| section.heading == heading)
            .map(|section| section.body.as_str())
    }
}

fn parse_front(front: &str) -> BTreeMap<String, String> {
    front
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .collect()
}

/// Parses the text of a command page.
///
/// # Errors
///
/// Returns an error when the page has no front matter.
pub fn parse_page(source: &str, name: &str) -> Result<CommandPage> {
    let (front, body) = split_front_matter(source);
    let front = front.ok_or_else(|| DocgenError::invalid(name, "the page has no front matter"))?;
    let fields = parse_front(front);
    let field = |key: &str| fields.get(key).cloned().unwrap_or_default();
    let mut page = CommandPage {
        front: FrontMatter {
            command: field("command"),
            group: field("group"),
            summary: field("summary"),
            since: field("since"),
        },
        ..CommandPage::default()
    };
    let mut fence: Option<String> = None;
    let mut current: Option<Section> = None;
    let mut in_comment = false;
    for line in body.lines() {
        if fence.is_none() {
            if in_comment {
                in_comment = !line.contains("-->");
                continue;
            }
            if line.trim_start().starts_with("<!--") {
                in_comment = !line.contains("-->");
                continue;
            }
            if let Some(title) = line.strip_prefix("# ") {
                page.title = title.trim().to_string();
                continue;
            }
            if let Some(heading) = line.strip_prefix("## ") {
                page.sections.extend(current.take());
                current = Some(Section {
                    heading: heading.trim().to_string(),
                    body: String::new(),
                });
                continue;
            }
        }
        track_fence(line, &mut fence);
        if let Some(section) = current.as_mut() {
            section.body.push_str(line);
            section.body.push('\n');
        }
    }
    page.sections.extend(current);
    Ok(page)
}

fn track_fence(line: &str, fence: &mut Option<String>) {
    let trimmed = line.trim_start();
    let marker: String = trimmed.chars().take_while(|c| *c == '`').collect();
    if marker.len() < 3 {
        return;
    }
    match fence {
        Some(open) if marker.len() >= open.len() && trimmed.trim_matches('`').is_empty() => {
            *fence = None;
        }
        None => *fence = Some(marker),
        Some(_) => {}
    }
}

/// Reads and parses a command page from a file.
///
/// # Errors
///
/// Returns an error when the file cannot be read or has no front matter.
pub fn load_page(path: &Path) -> Result<CommandPage> {
    let source = read_text(path)?;
    parse_page(&source, &path.display().to_string())
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn fenced(text: &str) -> String {
    format!("```text\n{}\n```\n", text.trim_end())
}

/// Returns the generated text of a section of a command page, or `None` for
/// a written section.
#[must_use]
pub fn generated(spec: &StageSpec, heading: &str, page: Option<&CommandPage>) -> Option<String> {
    match heading {
        "Synopsis" => Some(fenced(&synopsis(spec))),
        "Parameters" => {
            let descriptions = page
                .and_then(|p| p.section("Parameters"))
                .map(written_descriptions)
                .unwrap_or_default();
            Some(parameters_table(spec, &descriptions))
        }
        "Input and output" => Some(io_section(spec)),
        "Effect" => Some(effect_section(spec)),
        _ => None,
    }
}

fn notes_after<'a>(body: &'a str, generated: &str) -> &'a str {
    let wanted = squash(generated);
    let mut end = 0;
    for line in body.split_inclusive('\n') {
        end += line.len();
        let prefix = squash(&body[..end]);
        if prefix == wanted {
            return &body[end..];
        }
        if prefix.len() > wanted.len() {
            break;
        }
    }
    ""
}

fn with_notes(heading: &str, text: String, existing: &str) -> String {
    let notes = if heading == "Parameters" {
        notes_after_table(existing)
    } else {
        notes_after(existing, &text).trim().to_string()
    };
    if notes.is_empty() {
        text
    } else {
        format!("{text}\n{notes}\n")
    }
}

/// Composes the Markdown of a complete page.
///
/// Generated sections come from the catalog; written sections come from the
/// page when there is one. A command without a page gets its summary as the
/// description and no example, error or related sections.
#[must_use]
pub fn compose(spec: &StageSpec, page: Option<&CommandPage>) -> String {
    let mut out = format!("# {}\n\n", spec.name);
    for heading in SECTIONS {
        let written = page.and_then(|p| p.section(heading));
        let body = match (generated(spec, heading, page), written) {
            (Some(text), Some(existing)) => with_notes(heading, text, existing),
            (Some(text), None) => text,
            (None, Some(existing)) => existing.trim_end().to_string() + "\n",
            (None, None) if heading == "Description" => format!("{}\n", spec.summary),
            (None, None) => continue,
        };
        out.push_str(&format!("## {heading}\n\n{}\n", body.trim_end()));
    }
    out
}

fn notes_after_table(body: &str) -> String {
    let mut lines = body
        .lines()
        .skip_while(|line| !line.trim_start().starts_with('|'));
    let rest: Vec<&str> = lines
        .by_ref()
        .skip_while(|line| line.trim_start().starts_with('|'))
        .collect();
    rest.join("\n").trim().to_string()
}

/// Problem found on a command page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageIssue {
    /// Command the page belongs to.
    pub command: String,
    /// Description of the problem.
    pub message: String,
}

fn backtick_tokens(text: &str) -> Vec<&str> {
    text.split('`').skip(1).step_by(2).collect()
}

/// Checks a page against the catalog.
///
/// The checks are: the front matter agrees with the catalog, the sections
/// appear in order, the generated sections equal the generated text, the page
/// has at least three examples with valid snapshots, and parameters named in
/// backticks as `name:` exist.
#[must_use]
pub fn check_page(catalog: &Catalog, spec: &StageSpec, page: &CommandPage) -> Vec<PageIssue> {
    let mut issues = Vec::new();
    let mut report = |message: String| {
        issues.push(PageIssue {
            command: spec.name.clone(),
            message,
        });
    };
    if page.front.command != spec.name {
        report(format!("front matter command is {}", page.front.command));
    }
    if page.front.group != spec.group {
        report(format!("front matter group is {}", page.front.group));
    }
    if page.front.summary != spec.summary {
        report("front matter summary differs from the catalog".to_string());
    }
    if page.front.since.is_empty() {
        report("front matter has no since".to_string());
    }
    if page.title != spec.name {
        report(format!("title is {}", page.title));
    }
    let headings: Vec<&str> = page.sections.iter().map(|s| s.heading.as_str()).collect();
    let mut expected = SECTIONS.iter().copied();
    for heading in &headings {
        match expected.find(|wanted| wanted == heading) {
            Some(_) => {}
            None if *heading == "Notes" => {}
            None => report(format!("section {heading} is out of order or unknown")),
        }
    }
    for heading in SECTIONS {
        if !headings.contains(&heading) {
            report(format!("section {heading} is missing"));
        }
    }
    for heading in ["Synopsis", "Input and output", "Effect"] {
        let Some(text) = generated(spec, heading, Some(page)) else {
            continue;
        };
        if let Some(body) = page.section(heading)
            && !squash(body).starts_with(&squash(&text))
        {
            report(format!("section {heading} differs from the generated text"));
        }
    }
    check_parameter_rows(spec, page, &mut report);
    check_examples(page, &mut report);
    check_mentions(catalog, spec, page, &mut report);
    issues
}

fn check_parameter_rows(spec: &StageSpec, page: &CommandPage, report: &mut dyn FnMut(String)) {
    let Some(body) = page.section("Parameters") else {
        return;
    };
    let written = |text: &str| -> Vec<Vec<String>> {
        text.lines()
            .filter(|l| l.trim_start().starts_with('|'))
            .skip(2)
            .map(|l| {
                crate::markdown::split_cells(l)
                    .into_iter()
                    .take(3)
                    .collect()
            })
            .collect()
    };
    let expected = generated(spec, "Parameters", None).unwrap_or_default();
    if written(body) != written(&expected) {
        report("table of parameters differs from the catalog".to_string());
    }
}

fn check_examples(page: &CommandPage, report: &mut dyn FnMut(String)) {
    let body = page.section("Examples").unwrap_or_default();
    let examples = extract(&parse(body));
    if examples.len() < MIN_EXAMPLES {
        report(format!(
            "the page has {} examples, at least {MIN_EXAMPLES} are required",
            examples.len()
        ));
    }
    for (index, example) in examples.iter().enumerate() {
        if let Some(snapshot) = &example.fs_before {
            for problem in check_snapshot(snapshot) {
                report(format!("example {}: {problem}", index + 1));
            }
        }
    }
}

fn check_mentions(
    catalog: &Catalog,
    spec: &StageSpec,
    page: &CommandPage,
    report: &mut dyn FnMut(String),
) {
    for section in &page.sections {
        if section.heading == "Examples" {
            continue;
        }
        let mut in_fence = false;
        let mut prose = String::new();
        for line in section.body.lines() {
            if line.trim_start().starts_with("```") {
                in_fence = !in_fence;
            } else if !in_fence {
                prose.push_str(line);
                prose.push('\n');
            }
        }
        for token in backtick_tokens(&prose) {
            if let Some(name) = token.strip_suffix(':')
                && name.chars().all(|c| c.is_ascii_lowercase() || c == '_')
                && !name.is_empty()
                && spec.param(name).is_none()
                && !CALL_PARAMETERS.contains(&name)
                && !catalog.commands().any(|c| c.param(name).is_some())
            {
                report(format!(
                    "parameter {name} named in backticks does not exist in the catalog"
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::fixtures::catalog;

    fn page_text(catalog: &Catalog) -> String {
        let spec = catalog.command("find").expect("fixture command");
        let mut text = String::from(
            "---\ncommand: find\ngroup: core\nsummary: Find entries under a directory.\nsince: 0.1.0\n---\n# find\n\n",
        );
        for heading in SECTIONS {
            text.push_str(&format!("## {heading}\n\n"));
            match generated(spec, heading, None) {
                Some(body) => text.push_str(&body),
                None if heading == "Examples" => {
                    for n in 1..=3 {
                        text.push_str(&format!("Example {n}.\n\n```nurt example\nfind \"/home/user\" depth: {n}\n```\n\n"));
                    }
                }
                None => text.push_str("Text.\n"),
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn parses_front_matter_and_sections() {
        let catalog = catalog();
        let page = parse_page(&page_text(&catalog), "find.md").expect("page parses");
        assert_eq!(page.front.command, "find");
        assert_eq!(page.title, "find");
        assert_eq!(page.sections.len(), SECTIONS.len());
        assert!(
            page.section("Synopsis")
                .is_some_and(|b| b.contains("find <path>"))
        );
    }

    #[test]
    fn page_without_front_matter_is_an_error() {
        assert!(parse_page("# find\n", "find.md").is_err());
    }

    #[test]
    fn heading_inside_code_fence_is_not_a_section() {
        let source = "---\ncommand: x\n---\n# x\n\n## A\n\n```text\n## B\n```\n";
        let page = parse_page(source, "x.md").expect("page parses");
        assert_eq!(page.sections.len(), 1);
    }

    #[test]
    fn generated_page_passes_all_checks() {
        let catalog = catalog();
        let spec = catalog.command("find").expect("fixture command");
        let page = parse_page(&page_text(&catalog), "find.md").expect("page parses");
        assert_eq!(check_page(&catalog, spec, &page), Vec::new());
    }

    #[test]
    fn reports_front_matter_and_table_differences() {
        let catalog = catalog();
        let spec = catalog.command("find").expect("fixture command");
        let text = page_text(&catalog)
            .replace(
                "summary: Find entries under a directory.",
                "summary: Other.",
            )
            .replace("| `depth` | `int` | none |", "| `depth` | `text` | none |");
        let page = parse_page(&text, "find.md").expect("page parses");
        let messages: Vec<String> = check_page(&catalog, spec, &page)
            .into_iter()
            .map(|i| i.message)
            .collect();
        assert!(messages.iter().any(|m| m.contains("summary differs")));
        assert!(messages.iter().any(|m| m.contains("table of parameters")));
    }

    #[test]
    fn reports_missing_sections_and_few_examples() {
        let catalog = catalog();
        let spec = catalog.command("find").expect("fixture command");
        let source = "---\ncommand: find\ngroup: core\nsummary: Find entries under a directory.\nsince: 0.1.0\n---\n# find\n\n## Synopsis\n\n```text\nfind <path> [<glob>] [name: value]...\n```\n";
        let page = parse_page(source, "find.md").expect("page parses");
        let messages: Vec<String> = check_page(&catalog, spec, &page)
            .into_iter()
            .map(|i| i.message)
            .collect();
        assert!(
            messages
                .iter()
                .any(|m| m == "section Description is missing")
        );
        assert!(messages.iter().any(|m| m.contains("0 examples")));
    }

    #[test]
    fn reports_unknown_parameter_in_backticks() {
        let catalog = catalog();
        let spec = catalog.command("find").expect("fixture command");
        let text = page_text(&catalog).replace(
            "Text.\n\n## Parameters",
            "Use `nothing:` here.\n\n## Parameters",
        );
        let page = parse_page(&text, "find.md").expect("page parses");
        let messages: Vec<String> = check_page(&catalog, spec, &page)
            .into_iter()
            .map(|i| i.message)
            .collect();
        assert!(messages.iter().any(|m| m.contains("nothing")));
    }

    #[test]
    fn compose_keeps_notes_after_generated_text() {
        let catalog = catalog();
        let spec = catalog.command("find").expect("fixture command");
        let text =
            page_text(&catalog).replace("## Examples\n\n", "## Examples\n\nWritten note.\n\n");
        let page = parse_page(&text, "find.md").expect("page parses");
        let composed = compose(spec, Some(&page));
        assert!(composed.starts_with("# find\n\n## Synopsis"));
        assert!(composed.contains("## Examples"));
    }

    #[test]
    fn compose_without_page_uses_summary() {
        let catalog = catalog();
        let spec = catalog.command("count").expect("fixture command");
        let composed = compose(spec, None);
        assert!(composed.contains("## Description\n\nCount the input records."));
        assert!(!composed.contains("## Examples"));
    }
}
