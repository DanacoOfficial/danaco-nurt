// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Manual page (roff, `man` macros) renderer for the Markdown tree.

use crate::markdown::{Block, Document, Inline, plain_text};

/// Header data of a manual page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManHeader<'a> {
    /// Page name as shown in the title line, such as `nurt-find`.
    pub name: &'a str,
    /// Manual section number.
    pub section: u8,
    /// Date shown in the footer, in the form `YYYY-MM-DD`.
    pub date: &'a str,
    /// Source shown in the footer, such as `Danaco Nurt 0.1.0`.
    pub source: &'a str,
    /// Manual title shown in the header.
    pub manual: &'a str,
}

/// Escapes text for roff: backslashes, hyphens and a leading control character.
#[must_use]
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\e"),
            '-' => out.push_str("\\-"),
            _ => out.push(c),
        }
    }
    if out.starts_with('.') || out.starts_with('\'') {
        out.insert_str(0, "\\&");
    }
    out
}

fn inline_text(inlines: &[Inline], out: &mut String) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => out.push_str(&escape_inline(text)),
            Inline::Code(code) => {
                out.push_str("\\fB");
                out.push_str(&escape_inline(code));
                out.push_str("\\fR");
            }
            Inline::Emphasis(inner) => {
                out.push_str("\\fI");
                inline_text(inner, out);
                out.push_str("\\fR");
            }
            Inline::Strong(inner) => {
                out.push_str("\\fB");
                inline_text(inner, out);
                out.push_str("\\fR");
            }
            Inline::Link { text, url } => {
                inline_text(text, out);
                if !url.starts_with('#') && plain_text(text) != *url && url.contains("://") {
                    out.push_str(" <");
                    out.push_str(&escape_inline(url));
                    out.push('>');
                }
            }
            Inline::SoftBreak => out.push(' '),
        }
    }
}

fn escape_inline(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\\' => "\\e".to_string(),
            '-' => "\\-".to_string(),
            _ => c.to_string(),
        })
        .collect()
}

fn line_of(inlines: &[Inline]) -> String {
    let mut out = String::new();
    inline_text(inlines, &mut out);
    protect_line_start(out)
}

fn protect_line_start(line: String) -> String {
    if line.starts_with('.') || line.starts_with('\'') {
        format!("\\&{line}")
    } else {
        line
    }
}

/// Renders a document as a manual page.
///
/// A level-1 heading is not written: the title line carries the page name.
/// Level-2 headings become sections (`.SH`) and deeper headings become
/// subsections (`.SS`).
#[must_use]
pub fn render(header: &ManHeader<'_>, document: &Document) -> String {
    let mut out = title_line(header);
    blocks(&document.blocks, &mut out);
    out
}

/// Returns the title line (`.TH`) of a page.
#[must_use]
pub fn title_line(header: &ManHeader<'_>) -> String {
    format!(
        ".TH \"{}\" \"{}\" \"{}\" \"{}\" \"{}\"\n",
        header.name.to_ascii_uppercase().replace('-', "\\-"),
        header.section,
        header.date,
        header.source,
        header.manual
    )
}

/// Renders the blocks of a document without the title line.
#[must_use]
pub fn render_body(document: &Document) -> String {
    let mut out = String::new();
    blocks(&document.blocks, &mut out);
    out
}

fn blocks(items: &[Block], out: &mut String) {
    for block in items {
        match block {
            Block::Heading { level: 1, .. } | Block::Rule => {}
            Block::Heading { level: 2, content } => {
                out.push_str(&format!(
                    ".SH {}\n",
                    plain_text(content).to_uppercase().replace('-', "\\-")
                ));
            }
            Block::Heading { content, .. } => {
                out.push_str(&format!(".SS \"{}\"\n", escape(&plain_text(content))));
            }
            Block::Paragraph(content) => {
                out.push_str(".PP\n");
                out.push_str(&line_of(content));
                out.push('\n');
            }
            Block::Code { text, .. } => {
                out.push_str(".PP\n.RS 4\n.nf\n");
                for line in text.lines() {
                    out.push_str(&protect_line_start(escape_inline(line)));
                    out.push('\n');
                }
                out.push_str(".fi\n.RE\n");
            }
            Block::List { ordered, items } => list(*ordered, items, out),
            Block::Table { header, rows } => table(header, rows, out),
            Block::Quote(inner) => {
                out.push_str(".RS 4\n");
                blocks(inner, out);
                out.push_str(".RE\n");
            }
        }
    }
}

fn list(ordered: bool, items: &[Vec<Block>], out: &mut String) {
    for (index, item) in items.iter().enumerate() {
        if ordered {
            out.push_str(&format!(".IP {}. 4\n", index + 1));
        } else {
            out.push_str(".IP \\(bu 2\n");
        }
        for (position, block) in item.iter().enumerate() {
            match block {
                Block::Paragraph(content) => {
                    if position > 0 {
                        out.push_str(".PP\n");
                    }
                    out.push_str(&line_of(content));
                    out.push('\n');
                }
                other => blocks(std::slice::from_ref(other), out),
            }
        }
    }
}

fn table(header: &[Vec<Inline>], rows: &[Vec<Vec<Inline>>], out: &mut String) {
    for row in rows {
        let mut cells = row.iter();
        let Some(tag) = cells.next() else {
            continue;
        };
        out.push_str(".TP\n");
        out.push_str(&line_of(tag));
        out.push('\n');
        let mut parts: Vec<String> = Vec::new();
        for (column, cell) in cells.enumerate() {
            let text = plain_text(cell);
            if text.is_empty() || text == "\u{2014}" {
                continue;
            }
            let label = header.get(column + 1).map(|h| plain_text(h));
            let mut rendered = String::new();
            inline_text(cell, &mut rendered);
            match label {
                Some(label) if header.len() > 2 && column + 2 < header.len() => {
                    parts.push(format!("{}: {}", escape_inline(&label), rendered));
                }
                _ => parts.push(rendered),
            }
        }
        out.push_str(&protect_line_start(parts.join("; ")));
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::parse;

    const HEADER: ManHeader<'static> = ManHeader {
        name: "nurt-find",
        section: 1,
        date: "2026-01-01",
        source: "Danaco Nurt 0.1.0",
        manual: "Danaco Nurt",
    };

    #[test]
    fn writes_title_line_in_upper_case() {
        let out = render(&HEADER, &parse("# find\n"));
        assert_eq!(
            out,
            ".TH \"NURT\\-FIND\" \"1\" \"2026-01-01\" \"Danaco Nurt 0.1.0\" \"Danaco Nurt\"\n"
        );
    }

    #[test]
    fn writes_sections_and_paragraphs() {
        let out = render(&HEADER, &parse("## Description\n\nUse `find` here.\n"));
        assert!(out.contains(".SH DESCRIPTION\n.PP\nUse \\fBfind\\fR here.\n"));
    }

    #[test]
    fn escapes_backslashes_hyphens_and_leading_dots() {
        assert_eq!(escape("a-b\\c"), "a\\-b\\ec");
        assert_eq!(escape(".x"), "\\&.x");
    }

    #[test]
    fn writes_code_block_in_no_fill_mode() {
        let out = render(&HEADER, &parse("```text\nfind \"/a\"\n.hidden\n```\n"));
        assert!(out.contains(".nf\nfind \"/a\"\n\\&.hidden\n.fi\n"));
    }

    #[test]
    fn writes_table_rows_as_tagged_paragraphs() {
        let source =
            "| Parameter | Type | Description |\n|---|---|---|\n| `depth` | `int` | Levels. |\n";
        let out = render(&HEADER, &parse(source));
        assert!(out.contains(".TP\n\\fBdepth\\fR\nType: \\fBint\\fR; Levels.\n"));
    }

    #[test]
    fn writes_bullet_and_numbered_lists() {
        let out = render(&HEADER, &parse("- a\n\n1. b\n"));
        assert!(out.contains(".IP \\(bu 2\na\n"));
        assert!(out.contains(".IP 1. 4\nb\n"));
    }
}
