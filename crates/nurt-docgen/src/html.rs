// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! HTML renderer for the Markdown tree.

use std::collections::HashMap;

use crate::markdown::{Block, Document, Inline, plain_text};

/// Escapes the characters that are special in HTML text and attributes.
#[must_use]
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// Schemes whose links execute or embed content when followed.
const ACTIVE_SCHEMES: &[&str] = &["javascript:", "vbscript:", "data:"];

/// Replaces a link destination with an active scheme by `#`.
///
/// Browsers ignore whitespace and control characters inside the scheme and
/// compare it without regard to case, so `JaVa\tScRiPt:` is `javascript:`.
fn safe_destination(target: String) -> String {
    let compact: String = target
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect::<String>()
        .to_ascii_lowercase();
    if ACTIVE_SCHEMES
        .iter()
        .any(|scheme| compact.starts_with(scheme))
    {
        "#".to_owned()
    } else {
        target
    }
}

/// Converts a heading text to an identifier of lower-case letters, digits and hyphens.
#[must_use]
pub fn slugify(text: &str) -> String {
    let mut slug = String::new();
    let mut pending_dash = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(c.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    slug
}

/// Heading of a rendered document, for tables of contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadingRef {
    /// Level, from 1 to 6.
    pub level: u8,
    /// Plain text of the heading.
    pub text: String,
    /// Identifier of the heading element.
    pub id: String,
}

/// Result of rendering a document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rendered {
    /// HTML of the document body.
    pub html: String,
    /// Headings in document order.
    pub headings: Vec<HeadingRef>,
}

struct Renderer<'a> {
    rewrite: &'a dyn Fn(&str) -> String,
    out: String,
    headings: Vec<HeadingRef>,
    used_ids: HashMap<String, usize>,
}

/// Renders a document to HTML.
///
/// `rewrite` maps the destination of every link to the destination written
/// to the output, which lets the site builder turn `page.md` into `page.html`.
#[must_use]
pub fn render(document: &Document, rewrite: &dyn Fn(&str) -> String) -> Rendered {
    let mut renderer = Renderer {
        rewrite,
        out: String::new(),
        headings: Vec::new(),
        used_ids: HashMap::new(),
    };
    renderer.blocks(&document.blocks, false);
    Rendered {
        html: renderer.out,
        headings: renderer.headings,
    }
}

impl Renderer<'_> {
    fn unique_id(&mut self, text: &str) -> String {
        let base = match slugify(text) {
            slug if slug.is_empty() => "section".to_string(),
            slug => slug,
        };
        let count = self.used_ids.entry(base.clone()).or_insert(0);
        *count += 1;
        if *count == 1 {
            base
        } else {
            format!("{base}-{count}")
        }
    }

    fn blocks(&mut self, blocks: &[Block], tight: bool) {
        for block in blocks {
            match block {
                Block::Heading { level, content } => {
                    let text = plain_text(content);
                    let id = self.unique_id(&text);
                    self.out.push_str(&format!("<h{level} id=\"{id}\">"));
                    self.inlines(content);
                    self.out.push_str(&format!("</h{level}>\n"));
                    self.headings.push(HeadingRef {
                        level: *level,
                        text,
                        id,
                    });
                }
                Block::Paragraph(content) => {
                    if tight {
                        self.inlines(content);
                        self.out.push('\n');
                    } else {
                        self.out.push_str("<p>");
                        self.inlines(content);
                        self.out.push_str("</p>\n");
                    }
                }
                Block::Code { info, text } => {
                    let mut words = info.split_whitespace();
                    let language = words.next().unwrap_or("");
                    match words.next() {
                        Some(role) => self
                            .out
                            .push_str(&format!("<pre data-role=\"{}\">", escape(role))),
                        None => self.out.push_str("<pre>"),
                    }
                    if language.is_empty() {
                        self.out.push_str("<code>");
                    } else {
                        self.out
                            .push_str(&format!("<code class=\"language-{}\">", escape(language)));
                    }
                    self.out.push_str(&escape(text));
                    self.out.push_str("</code></pre>\n");
                }
                Block::List {
                    ordered,
                    tight,
                    items,
                } => {
                    let tag = if *ordered { "ol" } else { "ul" };
                    self.out.push_str(&format!("<{tag}>\n"));
                    for item in items {
                        self.out.push_str("<li>");
                        self.blocks(item, *tight);
                        self.out.push_str("</li>\n");
                    }
                    self.out.push_str(&format!("</{tag}>\n"));
                }
                Block::Table { header, rows } => self.table(header, rows),
                Block::Quote(inner) => {
                    self.out.push_str("<blockquote>\n");
                    self.blocks(inner, false);
                    self.out.push_str("</blockquote>\n");
                }
                Block::Rule => self.out.push_str("<hr>\n"),
            }
        }
    }

    fn table(&mut self, header: &[Vec<Inline>], rows: &[Vec<Vec<Inline>>]) {
        self.out.push_str("<table>\n<thead>\n<tr>");
        for cell in header {
            self.out.push_str("<th scope=\"col\">");
            self.inlines(cell);
            self.out.push_str("</th>");
        }
        self.out.push_str("</tr>\n</thead>\n<tbody>\n");
        for row in rows {
            self.out.push_str("<tr>");
            for cell in row {
                self.out.push_str("<td>");
                self.inlines(cell);
                self.out.push_str("</td>");
            }
            self.out.push_str("</tr>\n");
        }
        self.out.push_str("</tbody>\n</table>\n");
    }

    fn inlines(&mut self, inlines: &[Inline]) {
        for inline in inlines {
            match inline {
                Inline::Text(text) => self.out.push_str(&escape(text)),
                Inline::Code(code) => {
                    self.out.push_str("<code>");
                    self.out.push_str(&escape(code));
                    self.out.push_str("</code>");
                }
                Inline::Emphasis(inner) => {
                    self.out.push_str("<em>");
                    self.inlines(inner);
                    self.out.push_str("</em>");
                }
                Inline::Strong(inner) => {
                    self.out.push_str("<strong>");
                    self.inlines(inner);
                    self.out.push_str("</strong>");
                }
                Inline::Link { text, url } => {
                    let target = safe_destination((self.rewrite)(url));
                    self.out
                        .push_str(&format!("<a href=\"{}\">", escape(&target)));
                    self.inlines(text);
                    self.out.push_str("</a>");
                }
                Inline::SoftBreak => self.out.push('\n'),
                Inline::HardBreak => self.out.push_str("<br>\n"),
            }
        }
    }
}

/// Checks the tag structure of an HTML document produced by the generator.
///
/// The check verifies that every non-void element is closed in the right
/// order, that no end tag appears without a start tag and that attribute
/// quotes are balanced. It returns the list of problems; an empty list means
/// the structure is valid.
#[must_use]
pub fn check_structure(html: &str) -> Vec<String> {
    let chars: Vec<char> = html.chars().collect();
    let mut problems = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '<' {
            i += 1;
            continue;
        }
        match scan_markup(&chars, i) {
            Ok((next, markup)) => {
                i = next;
                apply_markup(markup, &mut stack, &mut problems);
            }
            Err(problem) => {
                problems.push(problem.to_string());
                break;
            }
        }
    }
    for open in stack {
        problems.push(format!("element {open} is not closed"));
    }
    problems
}

/// Elements that have no end tag.
const VOID_ELEMENTS: &[&str] = &[
    "meta", "link", "br", "hr", "img", "input", "area", "base", "col", "source", "wbr",
];

enum Markup {
    Ignored,
    Start(String),
    End(String),
}

fn find_comment_end(chars: &[char], from: usize) -> Option<usize> {
    (from..chars.len().saturating_sub(2)).find(|&k| chars[k..].starts_with(&['-', '-', '>']))
}

fn find_tag_end(chars: &[char], from: usize) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (offset, c) in chars[from..].iter().enumerate() {
        match (quote, *c) {
            (None, '>') => return Some(from + offset),
            (None, q @ ('"' | '\'')) => quote = Some(q),
            (Some(open), c) if c == open => quote = None,
            _ => {}
        }
    }
    None
}

fn scan_markup(chars: &[char], start: usize) -> Result<(usize, Markup), &'static str> {
    if chars[start..].starts_with(&['<', '!', '-', '-']) {
        let end = find_comment_end(chars, start + 4).ok_or("unterminated comment")?;
        return Ok((end + 3, Markup::Ignored));
    }
    let end = find_tag_end(chars, start + 1).ok_or("unterminated tag")?;
    let inner: String = chars[start + 1..end].iter().collect();
    if inner.starts_with('!') {
        return Ok((end + 1, Markup::Ignored));
    }
    if let Some(name) = inner.strip_prefix('/') {
        return Ok((end + 1, Markup::End(name.trim().to_ascii_lowercase())));
    }
    let name: String = inner
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    let self_closed = inner.ends_with('/') || VOID_ELEMENTS.contains(&name.as_str());
    let markup = if self_closed && !name.is_empty() {
        Markup::Ignored
    } else {
        Markup::Start(name)
    };
    Ok((end + 1, markup))
}

fn apply_markup(markup: Markup, stack: &mut Vec<String>, problems: &mut Vec<String>) {
    match markup {
        Markup::Ignored => {}
        Markup::Start(name) if name.is_empty() => problems.push("tag without a name".to_string()),
        Markup::Start(name) => stack.push(name),
        Markup::End(name) => match stack.pop() {
            Some(open) if open == name => {}
            Some(open) => problems.push(format!("end tag {name} closes {open}")),
            None => problems.push(format!("end tag {name} without start tag")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::parse;

    fn html_of(source: &str) -> Rendered {
        render(&parse(source), &|url| url.to_string())
    }

    #[test]
    fn escapes_html_in_text_and_code() {
        let rendered = html_of("a <b> & `<c>`");
        assert_eq!(
            rendered.html,
            "<p>a &lt;b&gt; &amp; <code>&lt;c&gt;</code></p>\n"
        );
    }

    #[test]
    fn slugifies_heading_text() {
        assert_eq!(slugify("Input and output"), "input-and-output");
        assert_eq!(slugify("`git.status` (1)"), "git-status-1");
    }

    #[test]
    fn assigns_unique_heading_ids() {
        let rendered = html_of("## Same\n\n## Same\n");
        let ids: Vec<&str> = rendered.headings.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, vec!["same", "same-2"]);
    }

    #[test]
    fn renders_code_block_language_class_and_role() {
        let rendered = html_of("```nurt example\nfind \"/a\"\n```\n\n```text\nx\n```\n");
        assert_eq!(
            rendered.html,
            "<pre data-role=\"example\"><code class=\"language-nurt\">find &quot;/a&quot;</code></pre>\n<pre><code class=\"language-text\">x</code></pre>\n"
        );
    }

    #[test]
    fn renders_table_with_header_scope() {
        let rendered = html_of("| A |\n|---|\n| x |\n");
        assert!(rendered.html.contains("<th scope=\"col\">A</th>"));
        assert!(rendered.html.contains("<td>x</td>"));
    }

    #[test]
    fn renders_tight_list_without_paragraphs() {
        let rendered = html_of("- a\n- b\n");
        assert_eq!(rendered.html, "<ul>\n<li>a\n</li>\n<li>b\n</li>\n</ul>\n");
    }

    #[test]
    fn renders_loose_list_with_paragraphs() {
        let rendered = html_of("- a\n\n- b\n");
        assert_eq!(
            rendered.html,
            "<ul>\n<li><p>a</p>\n</li>\n<li><p>b</p>\n</li>\n</ul>\n"
        );
    }

    #[test]
    fn renders_hard_break_as_line_break_element() {
        let rendered = html_of("a\\\nb");
        assert_eq!(rendered.html, "<p>a<br>\nb</p>\n");
    }

    #[test]
    fn rewrites_link_destinations() {
        let rendered = render(&parse("[a](x.md)"), &|url| url.replace(".md", ".html"));
        assert_eq!(rendered.html, "<p><a href=\"x.html\">a</a></p>\n");
    }

    #[test]
    fn replaces_links_with_active_schemes() {
        for url in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "java&#x09;script:x",
            " data:text/html;base64,AAAA",
            "vbscript:x",
        ] {
            let rendered = html_of(&format!("[a](<{url}>)"));
            assert_eq!(rendered.html, "<p><a href=\"#\">a</a></p>\n", "{url}");
        }
        let kept = html_of("[a](https://example.com/?q=1&r=2) [b](#anchor) [c](data.md)");
        assert!(
            kept.html
                .contains("href=\"https://example.com/?q=1&amp;r=2\"")
        );
        assert!(kept.html.contains("href=\"#anchor\"") && kept.html.contains("href=\"data.md\""));
    }

    #[test]
    fn escapes_quotes_in_link_destinations() {
        let rendered = html_of("[a](<x\"onmouseover=\"y>)");
        assert_eq!(
            rendered.html,
            "<p><a href=\"x&quot;onmouseover=&quot;y\">a</a></p>\n"
        );
    }

    #[test]
    fn slug_of_multi_byte_heading_is_deterministic() {
        assert_eq!(slugify("Za\u{17c}\u{f3}\u{142}\u{107} Nurt"), "za-nurt");
        assert_eq!(slugify("\u{17c}\u{f3}\u{142}"), "");
        let rendered = html_of("# \u{17c}\u{f3}\u{142}\n\n# \u{107}\n");
        let ids: Vec<&str> = rendered.headings.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, vec!["section", "section-2"]);
    }

    #[test]
    fn structure_check_accepts_rendered_documents() {
        let rendered = html_of("# T\n\n- a\n  - b\n\n| A |\n|---|\n| x |\n\n```text\nx\n```\n");
        assert!(check_structure(&rendered.html).is_empty());
    }

    #[test]
    fn structure_check_reports_unbalanced_tags() {
        let problems = check_structure("<div><p>text</div>");
        assert_eq!(
            problems,
            vec!["end tag div closes p", "element div is not closed"]
        );
    }

    #[test]
    fn structure_check_ignores_void_elements_and_comments() {
        assert!(check_structure("<!-- <b> --><p>a<br>b<hr></p>").is_empty());
    }
}
