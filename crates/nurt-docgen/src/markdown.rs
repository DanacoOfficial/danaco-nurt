// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-docgen::markdown
// Responsibility:   Markdown documents of `docs/` as a tree of blocks and inline elements.
// Dependencies:     pulldown_cmark
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Markdown documents of `docs/` as a tree of blocks and inline elements.
//!
//! Parsing follows CommonMark with GitHub tables and is done by
//! `pulldown-cmark`. The module turns its events into the tree defined here
//! ([`Document`], [`Block`], [`Inline`]); the HTML and manual page renderers
//! consume the tree. Raw HTML is kept as text and HTML comments are dropped,
//! so a page cannot inject markup into the site.

use std::iter::Peekable;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};

/// Inline element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    /// Plain text.
    Text(String),
    /// Code span.
    Code(String),
    /// Emphasis.
    Emphasis(Vec<Inline>),
    /// Strong emphasis.
    Strong(Vec<Inline>),
    /// Link with its text and destination.
    Link {
        /// Link text.
        text: Vec<Inline>,
        /// Destination as written; e-mail autolinks carry the `mailto:` scheme.
        url: String,
    },
    /// Line break inside a paragraph, rendered as a space.
    SoftBreak,
    /// Hard line break, written as a trailing backslash or two trailing spaces.
    HardBreak,
}

/// Block element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// Heading of level 1 to 6.
    Heading {
        /// Level, from 1 for `#` to 6 for `######`.
        level: u8,
        /// Heading content.
        content: Vec<Inline>,
    },
    /// Paragraph.
    Paragraph(Vec<Inline>),
    /// Fenced or indented code block.
    Code {
        /// Info string after the opening fence, trimmed; empty for an indented block.
        info: String,
        /// Content without the fences and without the final line break.
        text: String,
    },
    /// Bullet or numbered list.
    List {
        /// `true` for a numbered list.
        ordered: bool,
        /// `true` when no item is separated from the next by a blank line.
        tight: bool,
        /// Items, each a sequence of blocks.
        items: Vec<Vec<Block>>,
    },
    /// Table with a header row.
    Table {
        /// Header cells.
        header: Vec<Vec<Inline>>,
        /// Body rows.
        rows: Vec<Vec<Vec<Inline>>>,
    },
    /// Block quote.
    Quote(Vec<Block>),
    /// Thematic break.
    Rule,
}

/// Parsed Markdown document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    /// Top-level blocks in source order.
    pub blocks: Vec<Block>,
}

/// Splits a leading YAML front matter block from the text.
///
/// Returns the front matter without the `---` lines and the remaining text.
/// When the text does not start with a front matter block the first value is
/// `None` and the second is the whole text.
#[must_use]
pub fn split_front_matter(source: &str) -> (Option<&str>, &str) {
    let Some(rest) = source.strip_prefix("---\n") else {
        return (None, source);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            let front = rest[..offset].trim_end_matches('\n');
            return (Some(front), &rest[offset + line.len()..]);
        }
        offset += line.len();
    }
    (None, source)
}

/// Parses Markdown text as CommonMark with GitHub tables.
#[must_use]
pub fn parse(source: &str) -> Document {
    let mut events = Parser::new_ext(source, Options::ENABLE_TABLES).peekable();
    Document {
        blocks: read_blocks(&mut events).0,
    }
}

/// Returns the plain text of inline elements, without markup.
#[must_use]
pub fn plain_text(inlines: &[Inline]) -> String {
    let mut text = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(value) | Inline::Code(value) => text.push_str(value),
            Inline::Emphasis(inner) | Inline::Strong(inner) => text.push_str(&plain_text(inner)),
            Inline::Link { text: inner, .. } => text.push_str(&plain_text(inner)),
            Inline::SoftBreak | Inline::HardBreak => text.push(' '),
        }
    }
    text
}

/// Returns the plain text of all blocks, separated by spaces.
#[must_use]
pub fn blocks_text(blocks: &[Block]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for block in blocks {
        match block {
            Block::Heading { content, .. } | Block::Paragraph(content) => {
                parts.push(plain_text(content));
            }
            Block::Code { text, .. } => parts.push(text.clone()),
            Block::List { items, .. } => {
                for item in items {
                    parts.push(blocks_text(item));
                }
            }
            Block::Table { header, rows } => {
                for cell in header {
                    parts.push(plain_text(cell));
                }
                for row in rows {
                    for cell in row {
                        parts.push(plain_text(cell));
                    }
                }
            }
            Block::Quote(inner) => parts.push(blocks_text(inner)),
            Block::Rule => {}
        }
    }
    parts.join(" ")
}

/// Splits a table row of Markdown source into trimmed cells, resolving `\|` to `|`.
///
/// The function serves checks that read one table line at a time.
#[must_use]
pub fn split_cells(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let trimmed = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut chars = trimmed.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'|') {
            current.push('|');
            chars.next();
        } else if c == '|' {
            cells.push(current.trim().to_string());
            current = String::new();
        } else {
            current.push(c);
        }
    }
    if !current.trim().is_empty() {
        cells.push(current.trim().to_string());
    }
    cells
}

fn is_comment(raw: &str) -> bool {
    raw.trim_start().starts_with("<!--")
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn is_inline(event: &Event<'_>) -> bool {
    matches!(
        event,
        Event::Text(_)
            | Event::Code(_)
            | Event::InlineHtml(_)
            | Event::SoftBreak
            | Event::HardBreak
            | Event::Start(
                Tag::Emphasis
                    | Tag::Strong
                    | Tag::Strikethrough
                    | Tag::Link { .. }
                    | Tag::Image { .. }
            )
    )
}

fn push_inline(out: &mut Vec<Inline>, inline: Inline) {
    match (out.last_mut(), inline) {
        (Some(Inline::Text(last)), Inline::Text(text)) => last.push_str(&text),
        (_, inline) => out.push(inline),
    }
}

/// Reads blocks up to the end of the enclosing element, which is consumed.
///
/// The flag is `true` when a paragraph is marked explicitly. Inside a list
/// item `pulldown-cmark` marks paragraphs only when the list is loose.
fn read_blocks<'a, I>(events: &mut Peekable<I>) -> (Vec<Block>, bool)
where
    I: Iterator<Item = Event<'a>>,
{
    let mut blocks = Vec::new();
    let mut explicit_paragraphs = false;
    loop {
        if events.peek().is_some_and(is_inline) {
            let content = read_inline_run(events);
            if !content.is_empty() {
                blocks.push(Block::Paragraph(content));
            }
            continue;
        }
        match events.next() {
            None | Some(Event::End(_)) => break,
            Some(Event::Start(Tag::Paragraph)) => {
                explicit_paragraphs = true;
                let content = read_inlines(events);
                if !content.is_empty() {
                    blocks.push(Block::Paragraph(content));
                }
            }
            Some(Event::Start(tag)) => read_block(tag, events, &mut blocks),
            Some(Event::Rule) => blocks.push(Block::Rule),
            Some(_) => {}
        }
    }
    (blocks, explicit_paragraphs)
}

fn read_block<'a, I>(tag: Tag<'a>, events: &mut Peekable<I>, blocks: &mut Vec<Block>)
where
    I: Iterator<Item = Event<'a>>,
{
    match tag {
        Tag::Heading { level, .. } => blocks.push(Block::Heading {
            level: heading_level(level),
            content: read_inlines(events),
        }),
        Tag::BlockQuote(_) => blocks.push(Block::Quote(read_blocks(events).0)),
        Tag::CodeBlock(kind) => {
            let info = match kind {
                CodeBlockKind::Fenced(info) => info.trim().to_string(),
                CodeBlockKind::Indented => String::new(),
            };
            let raw = read_raw(events);
            let text = raw.strip_suffix('\n').unwrap_or(&raw).to_string();
            blocks.push(Block::Code { info, text });
        }
        Tag::HtmlBlock => {
            let raw = read_raw(events);
            if !is_comment(&raw) {
                blocks.push(Block::Paragraph(vec![Inline::Text(
                    raw.trim_end().to_string(),
                )]));
            }
        }
        Tag::List(start) => blocks.push(read_list(start.is_some(), events)),
        Tag::Table(_) => blocks.push(read_table(events)),
        _ => blocks.extend(read_blocks(events).0),
    }
}

/// Reads the text of a code or HTML block up to its end, which is consumed.
fn read_raw<'a, I>(events: &mut Peekable<I>) -> String
where
    I: Iterator<Item = Event<'a>>,
{
    let mut text = String::new();
    for event in events.by_ref() {
        match event {
            Event::End(_) => break,
            Event::Text(part) | Event::Html(part) | Event::Code(part) => text.push_str(&part),
            _ => {}
        }
    }
    text
}

/// Reads inline elements up to the end of the enclosing element, which is consumed.
fn read_inlines<'a, I>(events: &mut Peekable<I>) -> Vec<Inline>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut out = Vec::new();
    while let Some(event) = events.next() {
        if matches!(event, Event::End(_)) {
            break;
        }
        read_inline(event, events, &mut out);
    }
    out
}

/// Reads inline elements that stand outside a paragraph, as in a tight list item.
fn read_inline_run<'a, I>(events: &mut Peekable<I>) -> Vec<Inline>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut out = Vec::new();
    while let Some(event) = events.next_if(is_inline) {
        read_inline(event, events, &mut out);
    }
    out
}

fn read_inline<'a, I>(event: Event<'a>, events: &mut Peekable<I>, out: &mut Vec<Inline>)
where
    I: Iterator<Item = Event<'a>>,
{
    match event {
        Event::Text(text) => push_inline(out, Inline::Text(text.to_string())),
        Event::Code(code) => out.push(Inline::Code(code.to_string())),
        Event::InlineHtml(raw) | Event::Html(raw) if !is_comment(&raw) => {
            push_inline(out, Inline::Text(raw.to_string()));
        }
        Event::SoftBreak => out.push(Inline::SoftBreak),
        Event::HardBreak => out.push(Inline::HardBreak),
        Event::Start(Tag::Emphasis) => out.push(Inline::Emphasis(read_inlines(events))),
        Event::Start(Tag::Strong) => out.push(Inline::Strong(read_inlines(events))),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            ..
        }) => {
            let url = if matches!(link_type, LinkType::Email) {
                format!("mailto:{dest_url}")
            } else {
                dest_url.to_string()
            };
            out.push(Inline::Link {
                text: read_inlines(events),
                url,
            });
        }
        Event::Start(_) => {
            for inner in read_inlines(events) {
                push_inline(out, inner);
            }
        }
        _ => {}
    }
}

fn read_list<'a, I>(ordered: bool, events: &mut Peekable<I>) -> Block
where
    I: Iterator<Item = Event<'a>>,
{
    let mut items = Vec::new();
    let mut tight = true;
    while let Some(event) = events.next() {
        match event {
            Event::Start(Tag::Item) => {
                let (item, explicit_paragraphs) = read_blocks(events);
                tight &= !explicit_paragraphs;
                items.push(item);
            }
            Event::End(_) => break,
            _ => {}
        }
    }
    Block::List {
        ordered,
        tight,
        items,
    }
}

fn read_table<'a, I>(events: &mut Peekable<I>) -> Block
where
    I: Iterator<Item = Event<'a>>,
{
    let mut header = Vec::new();
    let mut rows = Vec::new();
    while let Some(event) = events.next() {
        match event {
            Event::Start(Tag::TableHead) => header = read_cells(events, TagEnd::TableHead),
            Event::Start(Tag::TableRow) => rows.push(read_cells(events, TagEnd::TableRow)),
            Event::End(_) => break,
            _ => {}
        }
    }
    Block::Table { header, rows }
}

fn read_cells<'a, I>(events: &mut Peekable<I>, end: TagEnd) -> Vec<Vec<Inline>>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut cells = Vec::new();
    while let Some(event) = events.next() {
        match event {
            Event::Start(Tag::TableCell) => cells.push(read_inlines(events)),
            Event::End(found) if found == end => break,
            _ => {}
        }
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_inlines(source: &str) -> Vec<Inline> {
        match parse(source).blocks.into_iter().next() {
            Some(Block::Paragraph(inlines)) => inlines,
            other => panic!("expected a paragraph, found {other:?}"),
        }
    }

    #[test]
    fn splits_front_matter_from_body() {
        let (front, body) = split_front_matter("---\na: 1\nb: 2\n---\n# T\n");
        assert_eq!(front, Some("a: 1\nb: 2"));
        assert_eq!(body, "# T\n");
    }

    #[test]
    fn keeps_text_without_front_matter() {
        assert_eq!(split_front_matter("# T\n"), (None, "# T\n"));
        assert_eq!(split_front_matter("---\nopen\n"), (None, "---\nopen\n"));
    }

    #[test]
    fn parses_headings_with_levels() {
        let doc = parse("# One\n\n### Three ##\n");
        assert_eq!(
            doc.blocks,
            vec![
                Block::Heading {
                    level: 1,
                    content: vec![Inline::Text("One".into())]
                },
                Block::Heading {
                    level: 3,
                    content: vec![Inline::Text("Three".into())]
                },
            ]
        );
    }

    #[test]
    fn joins_paragraph_lines_with_soft_breaks() {
        let inlines = first_inlines("first\nsecond\n\nthird");
        assert_eq!(
            inlines,
            vec![
                Inline::Text("first".into()),
                Inline::SoftBreak,
                Inline::Text("second".into())
            ]
        );
    }

    #[test]
    fn keeps_hard_breaks() {
        let inlines = first_inlines("first  \nsecond\\\nthird");
        assert_eq!(
            inlines,
            vec![
                Inline::Text("first".into()),
                Inline::HardBreak,
                Inline::Text("second".into()),
                Inline::HardBreak,
                Inline::Text("third".into())
            ]
        );
    }

    #[test]
    fn parses_code_spans_with_special_characters() {
        let inlines = first_inlines("use `a | b` and ``x ` y``");
        assert_eq!(
            inlines,
            vec![
                Inline::Text("use ".into()),
                Inline::Code("a | b".into()),
                Inline::Text(" and ".into()),
                Inline::Code("x ` y".into()),
            ]
        );
    }

    #[test]
    fn parses_emphasis_strong_and_links() {
        let inlines = first_inlines("*a* **b** [c](d.md \"t\") <https://example.com>");
        assert_eq!(
            inlines,
            vec![
                Inline::Emphasis(vec![Inline::Text("a".into())]),
                Inline::Text(" ".into()),
                Inline::Strong(vec![Inline::Text("b".into())]),
                Inline::Text(" ".into()),
                Inline::Link {
                    text: vec![Inline::Text("c".into())],
                    url: "d.md".into()
                },
                Inline::Text(" ".into()),
                Inline::Link {
                    text: vec![Inline::Text("https://example.com".into())],
                    url: "https://example.com".into()
                },
            ]
        );
    }

    #[test]
    fn gives_email_autolinks_the_mailto_scheme() {
        let inlines = first_inlines("<support@example.com>");
        assert_eq!(
            inlines,
            vec![Inline::Link {
                text: vec![Inline::Text("support@example.com".into())],
                url: "mailto:support@example.com".into()
            }]
        );
    }

    #[test]
    fn resolves_reference_links() {
        let inlines = first_inlines("[spec][s]\n\n[s]: ../spec/catalog.md\n");
        assert_eq!(
            inlines,
            vec![Inline::Link {
                text: vec![Inline::Text("spec".into())],
                url: "../spec/catalog.md".into()
            }]
        );
    }

    #[test]
    fn keeps_underscores_inside_words() {
        let inlines = first_inlines("on_missing and case_sensitive");
        assert_eq!(
            inlines,
            vec![Inline::Text("on_missing and case_sensitive".into())]
        );
    }

    #[test]
    fn unescapes_backslash_punctuation_and_entities() {
        let inlines = first_inlines(r"a \* b \[c\] &amp; d");
        assert_eq!(inlines, vec![Inline::Text("a * b [c] & d".into())]);
    }

    #[test]
    fn keeps_inline_html_as_text_and_drops_comments() {
        let inlines = first_inlines("run <program> now<!-- note -->");
        assert_eq!(inlines, vec![Inline::Text("run <program> now".into())]);
    }

    #[test]
    fn parses_fenced_code_with_info_string() {
        let doc = parse("```nurt example\nfind \"/a\"\n|> count\n```\n");
        assert_eq!(
            doc.blocks,
            vec![Block::Code {
                info: "nurt example".into(),
                text: "find \"/a\"\n|> count".into()
            }]
        );
    }

    #[test]
    fn parses_indented_code_without_info_string() {
        let doc = parse("text\n\n    code line\n");
        assert_eq!(
            doc.blocks[1],
            Block::Code {
                info: String::new(),
                text: "code line".into()
            }
        );
    }

    #[test]
    fn longer_fence_may_contain_shorter_fence() {
        let doc = parse("````markdown\n```text\nx\n```\n````\n");
        assert_eq!(
            doc.blocks,
            vec![Block::Code {
                info: "markdown".into(),
                text: "```text\nx\n```".into()
            }]
        );
    }

    #[test]
    fn parses_tables_with_escaped_pipes() {
        let doc = parse("| A | B |\n|---|:-:|\n| `x \\| y` | z |\n");
        let Some(Block::Table { header, rows }) = doc.blocks.first() else {
            panic!("expected a table");
        };
        assert_eq!(header.len(), 2);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0], vec![Inline::Code("x | y".into())]);
    }

    #[test]
    fn parses_nested_tight_lists() {
        let doc = parse("- a\n  - b\n  - c\n- d\n");
        let Some(Block::List {
            ordered,
            tight,
            items,
        }) = doc.blocks.first()
        else {
            panic!("expected a list");
        };
        assert!(!ordered);
        assert!(tight);
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0][0],
            Block::Paragraph(vec![Inline::Text("a".into())])
        );
        assert!(matches!(items[0][1], Block::List { .. }));
    }

    #[test]
    fn marks_lists_with_blank_lines_as_loose() {
        let doc = parse("- a\n\n- b\n");
        let Some(Block::List { tight, items, .. }) = doc.blocks.first() else {
            panic!("expected a list");
        };
        assert!(!tight);
        assert_eq!(
            items[1],
            vec![Block::Paragraph(vec![Inline::Text("b".into())])]
        );
    }

    #[test]
    fn parses_numbered_lists_and_lazy_continuation() {
        let doc = parse("1. first\ncontinued\n2. second\n");
        let Some(Block::List { ordered, items, .. }) = doc.blocks.first() else {
            panic!("expected a list");
        };
        assert!(ordered);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn drops_html_comments() {
        let doc = parse("<!-- one\ntwo -->\ntext\n");
        assert_eq!(
            doc.blocks,
            vec![Block::Paragraph(vec![Inline::Text("text".into())])]
        );
    }

    #[test]
    fn keeps_html_blocks_as_text() {
        let doc = parse("<details>\n\ntext\n");
        assert_eq!(
            doc.blocks[0],
            Block::Paragraph(vec![Inline::Text("<details>".into())])
        );
    }

    #[test]
    fn parses_quotes_and_rules() {
        let doc = parse("> quoted\n\n---\n");
        assert!(matches!(doc.blocks[0], Block::Quote(_)));
        assert_eq!(doc.blocks[1], Block::Rule);
    }

    #[test]
    fn extracts_plain_text_of_blocks() {
        let doc = parse("# T\n\nA `b` **c**\n");
        assert_eq!(blocks_text(&doc.blocks), "T A b c");
    }

    #[test]
    fn splits_table_cells_of_source_lines() {
        assert_eq!(split_cells("| a | b \\| c |"), vec!["a", "b | c"]);
        assert_eq!(split_cells("a | b"), vec!["a", "b"]);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
