// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Markdown and plain text documents, and rendering of blocks.

use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

use regex::Regex;

use super::{Block, BlockKind, Link, LinkStyle, Page, TextForm};

/// Longest heading marker of Markdown.
const MAX_HEADING_LEVEL: usize = 6;

fn link_pattern() -> Option<&'static Regex> {
    static PATTERN: OnceLock<Option<Regex>> = OnceLock::new();
    PATTERN
        .get_or_init(|| Regex::new(r"!?\[([^\]]*)\]\(([^)\s]+)(?:\s+[^)]*)?\)").ok())
        .as_ref()
}

/// Converts a Markdown document. Headings are recognised outside fenced
/// code blocks only.
///
/// ```
/// use nurt_net::html::{convert_markdown, BlockKind};
///
/// let page = convert_markdown("# API\n\n```\n# not a heading\n```\n\nText [a](https://a.test/).");
/// assert_eq!(page.blocks[0].kind, BlockKind::Heading(1));
/// assert_eq!(page.blocks[1].kind, BlockKind::Code);
/// assert_eq!(page.blocks[2].text, "Text a.");
/// assert_eq!(page.links[0].url, "https://a.test/");
/// ```
#[must_use]
pub fn convert_markdown(source: &str) -> Page {
    let mut parser = MarkdownParser::default();
    for line in source.lines() {
        parser.line(line);
    }
    parser.finish()
}

/// Converts plain text: paragraphs are separated by blank lines.
#[must_use]
pub fn convert_text(source: &str) -> Page {
    let blocks = source
        .split("\n\n")
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty())
        .map(|paragraph| {
            Block::new(
                BlockKind::Paragraph,
                paragraph.to_owned(),
                paragraph.to_owned(),
                Vec::new(),
            )
        })
        .collect();
    Page {
        blocks,
        ..Page::default()
    }
}

#[derive(Default)]
struct MarkdownParser {
    blocks: Vec<Block>,
    links: Vec<Link>,
    /// Position in `links` of each address, so that a repeated address is
    /// found without a scan of the list.
    link_index: HashMap<String, usize>,
    paragraph: Vec<String>,
    paragraph_kind: Option<BlockKind>,
    fence: Option<String>,
    code: Vec<String>,
}

impl MarkdownParser {
    fn line(&mut self, line: &str) {
        let trimmed = line.trim_start();
        if let Some(fence) = &self.fence {
            if trimmed.starts_with(fence.as_str()) {
                let code = self.code.join("\n");
                self.blocks
                    .push(Block::new(BlockKind::Code, code.clone(), code, Vec::new()));
                self.code.clear();
                self.fence = None;
            } else {
                self.code.push(line.to_owned());
            }
            return;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            self.flush();
            self.fence = Some(trimmed[..3].to_owned());
            return;
        }
        if trimmed.is_empty() {
            self.flush();
            return;
        }
        if let Some(level) = heading_level(trimmed) {
            self.flush();
            let text = trimmed[level..].trim().trim_end_matches('#').trim();
            self.push(BlockKind::Heading(level_u8(level)), text);
            return;
        }
        if let Some((depth, item)) = list_item(line) {
            self.flush();
            self.paragraph_kind = Some(BlockKind::ListItem(depth));
            self.paragraph.push(item.to_owned());
            return;
        }
        if let Some(quoted) = trimmed.strip_prefix('>') {
            if self.paragraph_kind != Some(BlockKind::Quote) {
                self.flush();
            }
            self.paragraph_kind = Some(BlockKind::Quote);
            self.paragraph.push(quoted.trim().to_owned());
            return;
        }
        let table = trimmed.starts_with('|');
        let kind = if table {
            BlockKind::Table
        } else {
            BlockKind::Paragraph
        };
        if self.paragraph_kind.is_some_and(|current| {
            (current == BlockKind::Table) != table || current == BlockKind::Quote
        }) {
            self.flush();
        }
        if self.paragraph_kind.is_none() {
            self.paragraph_kind = Some(kind);
        }
        self.paragraph.push(line.trim_end().to_owned());
    }

    fn flush(&mut self) {
        if self.paragraph.is_empty() {
            self.paragraph_kind = None;
            return;
        }
        let kind = self.paragraph_kind.take().unwrap_or(BlockKind::Paragraph);
        let separator = if kind == BlockKind::Table { "\n" } else { " " };
        let joined = self
            .paragraph
            .iter()
            .map(|line| line.trim())
            .collect::<Vec<_>>()
            .join(separator);
        self.paragraph.clear();
        self.push(kind, &joined);
    }

    fn push(&mut self, kind: BlockKind, markdown: &str) {
        let mut links = Vec::new();
        let text = match link_pattern() {
            Some(pattern) => pattern
                .replace_all(markdown, |captures: &regex::Captures<'_>| {
                    let label = captures.get(1).map_or("", |m| m.as_str()).to_owned();
                    let url = captures.get(2).map_or("", |m| m.as_str());
                    if !captures[0].starts_with('!') {
                        links.push(self.register(&label, url));
                    }
                    label
                })
                .into_owned(),
            None => markdown.to_owned(),
        };
        let text = if kind == BlockKind::Code {
            text
        } else {
            strip_emphasis(&text)
        };
        self.blocks
            .push(Block::new(kind, markdown.to_owned(), text, links));
    }

    fn register(&mut self, text: &str, url: &str) -> Link {
        if let Some(existing) = self.link_index.get(url).and_then(|at| self.links.get(*at)) {
            return existing.clone();
        }
        let link = Link {
            number: self.links.len() + 1,
            text: text.to_owned(),
            url: url.to_owned(),
        };
        self.link_index.insert(url.to_owned(), self.links.len());
        self.links.push(link.clone());
        link
    }

    fn finish(mut self) -> Page {
        if self.fence.is_some() {
            let code = self.code.join("\n");
            self.blocks
                .push(Block::new(BlockKind::Code, code.clone(), code, Vec::new()));
        }
        self.flush();
        Page {
            blocks: self.blocks,
            links: self.links,
            ..Page::default()
        }
    }
}

fn heading_level(line: &str) -> Option<usize> {
    let level = line.chars().take_while(|c| *c == '#').count();
    let valid = (1..=MAX_HEADING_LEVEL).contains(&level)
        && line[level..].chars().next().is_none_or(char::is_whitespace);
    valid.then_some(level)
}

fn level_u8(level: usize) -> u8 {
    u8::try_from(level).unwrap_or(u8::MAX)
}

fn list_item(line: &str) -> Option<(u8, &str)> {
    let indent = line.len() - line.trim_start().len();
    let trimmed = line.trim_start();
    let rest = if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        rest
    } else {
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        trimmed[digits..]
            .strip_prefix(". ")
            .or_else(|| trimmed[digits..].strip_prefix(") "))?
    };
    let depth = u8::try_from(indent / 2 + 1).unwrap_or(u8::MAX);
    Some((depth, rest.trim()))
}

fn strip_emphasis(text: &str) -> String {
    text.replace("**", "").replace('`', "")
}

/// Renders blocks as Markdown or plain text. With [`LinkStyle::List`] the
/// addresses of the links of the rendered blocks follow the content as
/// `[n]: address` lines.
///
/// ```
/// use nurt_net::html::{convert_markdown, render, LinkStyle, TextForm};
///
/// let page = convert_markdown("## Usage\n\n- first\n- second");
/// assert_eq!(render(&page.blocks, TextForm::Markdown, LinkStyle::Inline), "## Usage\n\n- first\n- second");
/// assert_eq!(render(&page.blocks, TextForm::Text, LinkStyle::Inline), "Usage\n\n- first\n- second");
/// ```
#[must_use]
pub fn render(blocks: &[Block], form: TextForm, links: LinkStyle) -> String {
    let mut out = String::new();
    let mut previous: Option<BlockKind> = None;
    for block in blocks {
        if !out.is_empty() {
            let tight = matches!(
                (previous, block.kind),
                (Some(BlockKind::ListItem(_)), BlockKind::ListItem(_))
            );
            out.push_str(if tight { "\n" } else { "\n\n" });
        }
        out.push_str(&render_block(block, form));
        previous = Some(block.kind);
    }
    if links == LinkStyle::List {
        let mut seen = BTreeSet::new();
        let references: Vec<String> = blocks
            .iter()
            .flat_map(|block| &block.links)
            .filter(|link| seen.insert(link.number))
            .map(|link| format!("[{}]: {}", link.number, link.url))
            .collect();
        if !references.is_empty() {
            out.push_str("\n\n");
            out.push_str(&references.join("\n"));
        }
    }
    out
}

/// Renders one block.
pub(crate) fn render_block(block: &Block, form: TextForm) -> String {
    let content = match form {
        TextForm::Markdown => &block.markdown,
        TextForm::Text => &block.text,
    };
    match (block.kind, form) {
        (BlockKind::Heading(level), TextForm::Markdown) => {
            format!("{} {content}", "#".repeat(usize::from(level)))
        }
        (BlockKind::ListItem(depth), _) => {
            format!(
                "{}- {content}",
                "  ".repeat(usize::from(depth.saturating_sub(1)))
            )
        }
        (BlockKind::Code, TextForm::Markdown) => format!("```\n{content}\n```"),
        (BlockKind::Quote, TextForm::Markdown) => format!("> {content}"),
        _ => content.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lists_quotes_tables_and_unclosed_fences() {
        let page = convert_markdown(
            "Intro **bold** line\nsecond line\n\n1. one\n  - nested\n> quote\n> more\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```\ncode",
        );
        let kinds: Vec<BlockKind> = page.blocks.iter().map(|block| block.kind).collect();
        assert_eq!(
            kinds,
            vec![
                BlockKind::Paragraph,
                BlockKind::ListItem(1),
                BlockKind::ListItem(2),
                BlockKind::Quote,
                BlockKind::Table,
                BlockKind::Code,
            ]
        );
        assert_eq!(page.blocks[0].text, "Intro bold line second line");
        assert_eq!(page.blocks[3].text, "quote more");
        assert!(page.blocks[4].markdown.contains("| 1 | 2 |"));
    }

    #[test]
    fn rejects_hash_without_space_as_heading() {
        let page = convert_markdown("#hashtag\n\n####### seven");
        assert!(
            page.blocks
                .iter()
                .all(|block| block.heading_level().is_none())
        );
    }

    #[test]
    fn renders_link_lists_once() {
        let page = convert_markdown("See [a](https://a.test/) and [again](https://a.test/).");
        let out = render(&page.blocks, TextForm::Text, LinkStyle::List);
        assert_eq!(out, "See a and again.\n\n[1]: https://a.test/");
    }

    #[test]
    fn splits_plain_text_into_paragraphs() {
        let page = convert_text("one\n\n\n two \n\n");
        assert_eq!(page.blocks.len(), 2);
        assert_eq!(page.blocks[1].text, "two");
    }
}
