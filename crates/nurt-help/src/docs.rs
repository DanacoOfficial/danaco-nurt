// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Documentation pages of commands and guides.
//!
//! The pages in `docs/commands` and the guides in `docs/guides` are Markdown
//! files with a front matter block. This module parses them into the data the
//! help and the description layers use. It performs no file access: the
//! caller supplies the text of each file.

use std::collections::BTreeMap;
use std::fmt;

/// Info string of a fenced block that holds a program.
const EXAMPLE_FENCE: &str = "nurt example";
/// Info string of a fenced block that holds the file system before an example.
const FS_BEFORE_FENCE: &str = "json fs-before";
/// Info string of a fenced block that holds the expected result.
const EXPECTED_FENCE: &str = "text expected";
/// Marker of a fenced block.
const FENCE: &str = "```";
/// Marker of the front matter block.
const FRONT_MATTER_MARK: &str = "---";

/// Files of `docs/commands` and `docs/guides`, embedded by the build script.
mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded_docs.rs"));
}

/// Error in a documentation file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocsError {
    /// Name of the file.
    pub source: String,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for DocsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.source, self.message)
    }
}

impl std::error::Error for DocsError {}

/// One example of a command page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Example {
    /// Sentence that says what the example does.
    pub caption: String,
    /// Program text in variant A.
    pub program: String,
    /// Snapshot of the virtual file system before the example.
    pub fs_before: Option<String>,
    /// Fragment that must appear in the text result.
    pub expected: Option<String>,
}

/// Error code with its meaning and fix, from the `Errors` section of a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageError {
    /// Error code, for example `E_ASSERT`.
    pub code: String,
    /// Meaning in the context of the command.
    pub meaning: String,
    /// Typical fix.
    pub fix: String,
}

/// Row of the `Parameters` table of a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterRow {
    /// Parameter name.
    pub name: String,
    /// Type expression.
    pub ty: String,
    /// Default value, `required`, or `-` when there is none.
    pub default: String,
    /// Description.
    pub description: String,
}

/// Page of one command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPage {
    /// Full command name.
    pub command: String,
    /// Group of the command.
    pub group: String,
    /// Summary, equal to the catalog summary.
    pub summary: String,
    /// Version that introduced the command.
    pub since: String,
    /// Lines of the `Synopsis` block.
    pub synopsis: Vec<String>,
    /// Rows of the `Parameters` table.
    pub parameters: Vec<ParameterRow>,
    /// Examples in the order of the page.
    pub examples: Vec<Example>,
    /// Rows of the `Errors` table.
    pub errors: Vec<PageError>,
    /// Names listed in the `Related` section.
    pub related: Vec<String>,
}

/// Section of a guide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuideSection {
    /// Heading text.
    pub heading: String,
    /// Body text.
    pub text: String,
}

/// Guide: a short document on one area of work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guide {
    /// Name of the guide as given to the guide topic of `help`.
    pub name: String,
    /// One-sentence description.
    pub description: String,
    /// Commands the guide mentions.
    pub commands: Vec<String>,
    /// Sections in the order of the file.
    pub sections: Vec<GuideSection>,
}

impl Guide {
    /// The guide as one text: description followed by the sections.
    #[must_use]
    pub fn render(&self) -> String {
        let mut text = format!("{} — {}", self.name, self.description);
        for section in &self.sections {
            text.push_str("\n\n");
            text.push_str(&section.heading);
            text.push('\n');
            text.push_str(&section.text);
        }
        text
    }
}

/// All pages and guides.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Docs {
    pages: BTreeMap<String, CommandPage>,
    guides: BTreeMap<String, Guide>,
}

impl Docs {
    /// Documentation without pages and guides.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Parses the command pages and the guides of the repository that were
    /// embedded in the build.
    ///
    /// # Errors
    ///
    /// Returns the first embedded file whose front matter or structure is
    /// invalid.
    pub fn embedded() -> Result<Self, DocsError> {
        Self::from_sources(embedded::PAGES, embedded::GUIDES)
    }

    /// Parses pages and guides given as `(file name, text)` pairs.
    ///
    /// # Errors
    ///
    /// Returns the first file whose front matter or structure is invalid.
    pub fn from_sources(
        pages: &[(&str, &str)],
        guides: &[(&str, &str)],
    ) -> Result<Self, DocsError> {
        let mut docs = Self::empty();
        for (source, text) in pages {
            let page = parse_page(source, text)?;
            if docs.pages.contains_key(&page.command) {
                return Err(error(
                    source,
                    format!("the command `{}` has another page already", page.command),
                ));
            }
            docs.pages.insert(page.command.clone(), page);
        }
        for (source, text) in guides {
            let guide = parse_guide(source, text)?;
            if docs.guides.contains_key(&guide.name) {
                return Err(error(
                    source,
                    format!("the guide `{}` exists already", guide.name),
                ));
            }
            docs.guides.insert(guide.name.clone(), guide);
        }
        Ok(docs)
    }

    /// Page of a command.
    #[must_use]
    pub fn page(&self, command: &str) -> Option<&CommandPage> {
        self.pages.get(command)
    }

    /// Guide by name.
    #[must_use]
    pub fn guide(&self, name: &str) -> Option<&Guide> {
        self.guides.get(name)
    }

    /// All pages ordered by command name.
    pub fn pages(&self) -> impl Iterator<Item = &CommandPage> {
        self.pages.values()
    }

    /// All guides ordered by name.
    pub fn guides(&self) -> impl Iterator<Item = &Guide> {
        self.guides.values()
    }
}

struct FrontMatter {
    entries: Vec<(String, String)>,
}

impl FrontMatter {
    fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    fn require(&self, source: &str, key: &str) -> Result<String, DocsError> {
        self.get(key)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| error(source, format!("front matter lacks `{key}`")))
    }
}

fn error(source: &str, message: impl Into<String>) -> DocsError {
    DocsError {
        source: source.to_owned(),
        message: message.into(),
    }
}

fn is_comment_line(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("<!--") && line.ends_with("-->")
}

/// Splits a file into its front matter and the lines after it. Blank lines
/// and one-line comments (the file header) may precede the block.
fn split_front_matter<'a>(
    source: &str,
    text: &'a str,
) -> Result<(FrontMatter, Vec<&'a str>), DocsError> {
    let mut lines = text.lines();
    let first = lines
        .by_ref()
        .find(|line| !line.trim().is_empty() && !is_comment_line(line));
    if first.map(str::trim) != Some(FRONT_MATTER_MARK) {
        return Err(error(
            source,
            "the file does not start with a front matter block",
        ));
    }
    let mut entries = Vec::new();
    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim() == FRONT_MATTER_MARK {
            closed = true;
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            let value = value.trim().trim_matches('"');
            entries.push((key.trim().to_owned(), value.to_owned()));
        }
    }
    if !closed {
        return Err(error(source, "the front matter block is not closed"));
    }
    Ok((FrontMatter { entries }, lines.collect()))
}

/// Splits lines into sections introduced by `## ` headings. Lines before the
/// first heading are dropped. Fenced blocks are kept whole.
fn split_sections<'a>(lines: &[&'a str]) -> Vec<(String, Vec<&'a str>)> {
    let mut sections: Vec<(String, Vec<&str>)> = Vec::new();
    let mut in_fence = false;
    for line in lines {
        if line.trim_start().starts_with(FENCE) {
            in_fence = !in_fence;
        }
        match line.strip_prefix("## ") {
            Some(heading) if !in_fence => sections.push((heading.trim().to_owned(), Vec::new())),
            _ => {
                if let Some((_, body)) = sections.last_mut() {
                    body.push(line);
                }
            }
        }
    }
    sections
}

fn parse_page(source: &str, text: &str) -> Result<CommandPage, DocsError> {
    let (front, lines) = split_front_matter(source, text)?;
    let mut page = CommandPage {
        command: front.require(source, "command")?,
        group: front.require(source, "group")?,
        summary: front.require(source, "summary")?,
        since: front.require(source, "since")?,
        synopsis: Vec::new(),
        parameters: Vec::new(),
        examples: Vec::new(),
        errors: Vec::new(),
        related: Vec::new(),
    };
    for (heading, body) in split_sections(&lines) {
        match heading.as_str() {
            "Synopsis" => page.synopsis = fenced_lines(&body),
            "Parameters" => page.parameters = parameter_rows(&body),
            "Examples" => page.examples = examples(&body),
            "Errors" => page.errors = error_rows(&body),
            "Related" => page.related = backtick_spans(&body.join("\n")),
            _ => {}
        }
    }
    Ok(page)
}

fn parse_guide(source: &str, text: &str) -> Result<Guide, DocsError> {
    let (front, lines) = split_front_matter(source, text)?;
    let commands = front
        .get("commands")
        .map(|value| {
            value
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .map(|name| name.trim().trim_matches('"').to_owned())
                .filter(|name| !name.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let sections = split_sections(&lines)
        .into_iter()
        .map(|(heading, body)| GuideSection {
            heading,
            text: body.join("\n").trim().to_owned(),
        })
        .collect();
    Ok(Guide {
        name: front.require(source, "name")?,
        description: front.require(source, "description")?,
        commands,
        sections,
    })
}

fn fenced_lines(body: &[&str]) -> Vec<String> {
    let mut inside = false;
    let mut lines = Vec::new();
    for line in body {
        if line.trim_start().starts_with(FENCE) {
            inside = !inside;
        } else if inside && !line.trim().is_empty() {
            lines.push((*line).to_owned());
        }
    }
    lines
}

fn examples(body: &[&str]) -> Vec<Example> {
    let mut examples: Vec<Example> = Vec::new();
    let mut caption = String::new();
    let mut fence: Option<(String, Vec<&str>)> = None;
    for line in body {
        let trimmed = line.trim_start();
        match fence.take() {
            None => {
                if let Some(info) = trimmed.strip_prefix(FENCE) {
                    fence = Some((info.trim().to_owned(), Vec::new()));
                } else if !trimmed.is_empty() {
                    caption = trimmed.to_owned();
                }
            }
            Some((info, content)) => {
                if trimmed.starts_with(FENCE) {
                    close_fence(&mut examples, &mut caption, &info, &content);
                } else {
                    let mut content = content;
                    content.push(line);
                    fence = Some((info, content));
                }
            }
        }
    }
    examples
}

fn close_fence(examples: &mut Vec<Example>, caption: &mut String, info: &str, content: &[&str]) {
    let text = content.join("\n");
    match info {
        EXAMPLE_FENCE => examples.push(Example {
            caption: std::mem::take(caption),
            program: text,
            fs_before: None,
            expected: None,
        }),
        FS_BEFORE_FENCE => {
            if let Some(last) = examples.last_mut() {
                last.fs_before = Some(text);
            }
        }
        EXPECTED_FENCE => {
            if let Some(last) = examples.last_mut() {
                last.expected = Some(text);
            }
        }
        _ => {}
    }
}

/// Cells of a Markdown table row; `\|` is an escaped separator.
fn table_cells(line: &str) -> Option<Vec<String>> {
    let line = line.trim();
    let inner = line.strip_prefix('|')?.strip_suffix('|')?;
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' if chars.peek() == Some(&'|') => {
                cell.push('|');
                chars.next();
            }
            '|' => cells.push(std::mem::take(&mut cell).trim().to_owned()),
            _ => cell.push(ch),
        }
    }
    cells.push(cell.trim().to_owned());
    Some(cells)
}

fn is_separator_row(cells: &[String]) -> bool {
    cells
        .iter()
        .all(|cell| !cell.is_empty() && cell.chars().all(|ch| ch == '-' || ch == ':'))
}

fn strip_ticks(cell: &str) -> String {
    cell.trim().replace('`', "")
}

fn table_rows(body: &[&str]) -> Vec<Vec<String>> {
    body.iter()
        .filter_map(|line| table_cells(line))
        .filter(|cells| !is_separator_row(cells))
        .skip(1)
        .collect()
}

fn parameter_rows(body: &[&str]) -> Vec<ParameterRow> {
    table_rows(body)
        .into_iter()
        .filter(|cells| cells.len() >= 4)
        .map(|cells| ParameterRow {
            name: strip_ticks(&cells[0]),
            ty: strip_ticks(&cells[1]),
            default: strip_ticks(&cells[2]),
            description: cells[3].clone(),
        })
        .collect()
}

fn error_rows(body: &[&str]) -> Vec<PageError> {
    table_rows(body)
        .into_iter()
        .filter(|cells| cells.len() >= 3 && cells[0] != "Code")
        .map(|cells| PageError {
            code: strip_ticks(&cells[0]),
            meaning: cells[1].clone(),
            fix: cells[2].clone(),
        })
        .collect()
}

/// Texts between pairs of backticks, in order.
pub(crate) fn backtick_spans(text: &str) -> Vec<String> {
    text.split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "<!-- header -->\n---\ncommand: count\ngroup: core\nsummary: Count records.\nsince: 0.1.0\n---\n\n# count\n\n## Synopsis\n\n```text\ncount\n```\n\n## Parameters\n\n| Parameter | Type | Default | Description |\n|---|---|---|---|\n| `by` | `enum(a\\|b)` | `a` | Key. |\n\n## Examples\n\nCount files.\n\n```nurt example\nfind \"/x\" |> count\n```\n\n```text expected\n3\n```\n\n## Errors\n\n| Code | Meaning | Typical fix |\n|---|---|---|\n| `E_TYPE` | wrong input | use a source |\n\n## Related\n\n`first`, `last`.\n";

    #[test]
    fn page_sections_are_parsed() {
        let page = parse_page("count.md", PAGE).unwrap();
        assert_eq!(page.command, "count");
        assert_eq!(page.synopsis, ["count"]);
        assert_eq!(page.parameters[0].ty, "enum(a|b)");
        assert_eq!(page.examples.len(), 1);
        assert_eq!(page.examples[0].caption, "Count files.");
        assert_eq!(page.examples[0].expected.as_deref(), Some("3"));
        assert_eq!(page.errors[0].code, "E_TYPE");
        assert_eq!(page.related, ["first", "last"]);
    }

    #[test]
    fn missing_front_matter_is_rejected() {
        let error = parse_page("x.md", "# x\n").unwrap_err();
        assert!(error.message.contains("front matter"));
    }

    #[test]
    fn unclosed_front_matter_is_rejected() {
        assert!(parse_page("x.md", "---\ncommand: x\n").is_err());
    }

    #[test]
    fn missing_key_is_reported_by_name() {
        let error = parse_page("x.md", "---\ncommand: x\n---\n").unwrap_err();
        assert!(error.message.contains("`group`"));
    }

    #[test]
    fn guide_sections_and_commands_are_parsed() {
        let text = "---\nname: demo\ndescription: A demo.\ncommands: [find, remove]\n---\n\n# demo\n\n## One\n\nText one.\n\n## Two\n\nText two.\n";
        let guide = parse_guide("demo.md", text).unwrap();
        assert_eq!(guide.commands, ["find", "remove"]);
        assert_eq!(guide.sections.len(), 2);
        assert_eq!(guide.sections[1].text, "Text two.");
        assert!(guide.render().starts_with("demo — A demo."));
    }

    #[test]
    fn docs_index_pages_and_guides_by_name() {
        let guide = "---\nname: g\ndescription: D.\ncommands: []\n---\n";
        let docs = Docs::from_sources(&[("count.md", PAGE)], &[("g.md", guide)]).unwrap();
        assert!(docs.page("count").is_some());
        assert!(docs.guide("g").is_some());
        assert_eq!(docs.pages().count(), 1);
        assert_eq!(docs.guides().count(), 1);
    }

    #[test]
    fn a_second_page_or_guide_of_the_same_name_is_rejected_not_overwritten() {
        let error = Docs::from_sources(&[("a.md", PAGE), ("b.md", PAGE)], &[]).unwrap_err();
        assert_eq!(error.source, "b.md");
        assert!(error.message.contains("`count`"));
        let guide = "---\nname: g\ndescription: D.\ncommands: []\n---\n";
        let error = Docs::from_sources(&[], &[("a.md", guide), ("b.md", guide)]).unwrap_err();
        assert_eq!(error.source, "b.md");
        assert!(error.message.contains("`g`"));
    }
}
