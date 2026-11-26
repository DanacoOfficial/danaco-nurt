// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conversion of the element tree to blocks.

use std::collections::HashMap;
use std::mem;

use url::Url;

use super::dom::{self, Element, Node, collapse_whitespace};
use super::{Block, BlockKind, ConvertOptions, ImageStyle, Link, LinkStyle, Metadata, Page, Table};

/// Elements never converted to text.
const NEVER: &[&str] = &[
    "script", "style", "noscript", "template", "svg", "canvas", "iframe", "object", "embed",
    "head", "title", "meta", "link", "button", "select", "input", "textarea", "option", "video",
    "audio", "map",
];

/// Elements of the page frame left out with `main_only`.
const CHROME: &[&str] = &["nav", "header", "footer", "aside", "form", "dialog", "menu"];

/// ARIA roles of the page frame left out with `main_only`.
const CHROME_ROLES: &[&str] = &[
    "navigation",
    "banner",
    "contentinfo",
    "complementary",
    "search",
    "dialog",
    "alertdialog",
    "menu",
    "menubar",
];

/// Words in `class` and `id` that mark blocks left out with `main_only`.
const NOISE: &[&str] = &[
    "nav",
    "navbar",
    "navigation",
    "menu",
    "breadcrumb",
    "breadcrumbs",
    "sidebar",
    "footer",
    "cookie",
    "cookies",
    "consent",
    "gdpr",
    "banner",
    "advert",
    "ad",
    "ads",
    "sponsor",
    "sponsored",
    "promo",
    "promoted",
    "related",
    "recommended",
    "recommendations",
    "share",
    "sharing",
    "social",
    "login",
    "signin",
    "signup",
    "newsletter",
    "subscribe",
    "toc",
    "comments",
    "popup",
    "modal",
];

/// Scheme of an address that holds a script, not a link.
const SCRIPT_SCHEME: &str = "javascript:";

/// Largest number of cells of one table that are kept; the cells of a hostile
/// page cannot multiply by padding every row to the widest one.
const MAX_TABLE_CELLS: usize = 1_000_000;

/// Largest number of columns of one table that are kept.
const MAX_TABLE_COLUMNS: usize = 256;

/// Elements that start a new paragraph.
const BLOCK_ELEMENTS: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "main",
    "header",
    "footer",
    "nav",
    "aside",
    "body",
    "html",
    "figure",
    "figcaption",
    "form",
    "fieldset",
    "details",
    "summary",
    "address",
    "center",
    "dl",
    "dd",
    "dt",
    "hr",
    "#root",
];

/// Converts an HTML document.
///
/// ```
/// use nurt_net::html::{convert_html, render, BlockKind, ConvertOptions, TextForm};
///
/// let page = convert_html(
///     "<nav>Menu</nav><main><h1>Title</h1><p>See <a href=\"/a\">this</a>.</p></main>",
///     &ConvertOptions { base: url::Url::parse("https://example.test/").ok(), ..Default::default() },
/// );
/// assert_eq!(page.blocks[0].kind, BlockKind::Heading(1));
/// let markdown = render(&page.blocks, TextForm::Markdown, Default::default());
/// assert_eq!(markdown, "# Title\n\nSee [this](https://example.test/a).");
/// ```
#[must_use]
pub fn convert_html(source: &str, options: &ConvertOptions) -> Page {
    let root = dom::parse(source);
    let metadata = metadata(&root, options.base.as_ref());
    let (content, frame_is_page) = content_root(&root, options.main_only);
    let mut walker = Walker {
        options,
        skip_chrome: options.main_only && frame_is_page,
        blocks: Vec::new(),
        inline: Inline::default(),
        links: Vec::new(),
        link_index: HashMap::new(),
        tables: Vec::new(),
        depth: 0,
        quote: false,
    };
    walker.children(content);
    walker.flush(BlockKind::Paragraph);
    Page {
        blocks: walker.blocks,
        links: walker.links,
        tables: walker.tables,
        metadata,
    }
}

fn content_root(root: &Element, main_only: bool) -> (&Element, bool) {
    let all = root.descendants();
    if main_only {
        let main = all
            .iter()
            .find(|element| element.name == "main" || element.attr("role") == Some("main"));
        if let Some(main) = main {
            return (main, false);
        }
        let articles: Vec<_> = all
            .iter()
            .filter(|element| element.name == "article")
            .collect();
        if let [article] = articles.as_slice() {
            return (article, false);
        }
    }
    match all.iter().find(|element| element.name == "body") {
        Some(body) => (body, true),
        None => (root, true),
    }
}

fn metadata(root: &Element, base: Option<&Url>) -> Metadata {
    let all = root.descendants();
    let meta = |key: &str| {
        all.iter()
            .filter(|element| element.name == "meta")
            .find(|element| {
                element
                    .attr("name")
                    .or_else(|| element.attr("property"))
                    .is_some_and(|name| name.eq_ignore_ascii_case(key))
            })
            .and_then(|element| element.attr("content"))
            .map(collapse_whitespace)
            .filter(|value| !value.is_empty())
    };
    let title = all
        .iter()
        .find(|element| element.name == "title")
        .map(|title| title.text())
        .filter(|title| !title.is_empty())
        .or_else(|| meta("og:title"));
    let canonical = all
        .iter()
        .find(|element| {
            element.name == "link"
                && element.attr("rel").is_some_and(|rel| {
                    rel.split_whitespace()
                        .any(|r| r.eq_ignore_ascii_case("canonical"))
                })
        })
        .and_then(|element| element.attr("href"))
        .map(|href| resolve(base, href));
    Metadata {
        title,
        description: meta("description").or_else(|| meta("og:description")),
        language: all
            .iter()
            .find(|element| element.name == "html")
            .and_then(|html| html.attr("lang"))
            .map(str::to_owned)
            .filter(|lang| !lang.is_empty()),
        canonical,
    }
}

fn resolve(base: Option<&Url>, href: &str) -> String {
    base.and_then(|base| base.join(href.trim()).ok())
        .map_or_else(|| href.trim().to_owned(), String::from)
}

#[derive(Default)]
struct Inline {
    markdown: String,
    text: String,
    links: Vec<Link>,
}

struct Walker<'a> {
    options: &'a ConvertOptions,
    skip_chrome: bool,
    blocks: Vec<Block>,
    inline: Inline,
    links: Vec<Link>,
    /// Position in `links` of each address, so that a repeated address is
    /// found without a scan of the list.
    link_index: HashMap<String, usize>,
    tables: Vec<Table>,
    depth: u8,
    quote: bool,
}

impl Walker<'_> {
    fn children(&mut self, element: &Element) {
        for child in &element.children {
            match child {
                Node::Text(text) => self.push_text(text),
                Node::Element(child) => self.element(child),
            }
        }
    }

    fn push_text(&mut self, text: &str) {
        let spaced = space_runs(text);
        self.inline.markdown.push_str(&spaced);
        self.inline.text.push_str(&spaced);
    }

    fn skipped(&self, element: &Element) -> bool {
        if NEVER.contains(&element.name.as_str()) || hidden(element) {
            return true;
        }
        self.options.main_only
            && (self.skip_chrome && CHROME.contains(&element.name.as_str())
                || element
                    .attr("role")
                    .is_some_and(|role| CHROME_ROLES.contains(&role))
                || noisy(element))
    }

    fn element(&mut self, element: &Element) {
        if self.skipped(element) {
            return;
        }
        let name = element.name.as_str();
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => self.heading(element),
            "ul" | "ol" => {
                self.flush(BlockKind::Paragraph);
                self.depth = self.depth.saturating_add(1);
                self.children(element);
                self.flush(BlockKind::ListItem(self.depth));
                self.depth = self.depth.saturating_sub(1);
            }
            "li" => {
                self.flush(BlockKind::ListItem(self.depth.max(1)));
                self.children(element);
                self.flush(BlockKind::ListItem(self.depth.max(1)));
            }
            "pre" => self.code(element),
            "blockquote" => {
                self.flush(BlockKind::Paragraph);
                let outer = mem::replace(&mut self.quote, true);
                self.children(element);
                self.flush(BlockKind::Quote);
                self.quote = outer;
            }
            "table" => self.table(element),
            "br" => self.push_text(" "),
            "a" => self.link(element),
            "img" => self.image(element),
            "strong" | "b" => self.wrapped(element, "**"),
            "em" | "i" => self.wrapped(element, "_"),
            "code" | "kbd" | "samp" => self.wrapped(element, "`"),
            _ if BLOCK_ELEMENTS.contains(&name) => {
                self.flush(BlockKind::Paragraph);
                self.children(element);
                self.flush(BlockKind::Paragraph);
            }
            _ => self.children(element),
        }
    }

    fn capture(&mut self, element: &Element) -> Inline {
        let outer = mem::take(&mut self.inline);
        self.children(element);
        mem::replace(&mut self.inline, outer)
    }

    fn append(&mut self, inner: Inline, markdown: String) {
        self.inline.markdown.push_str(&markdown);
        self.inline.text.push_str(&inner.text);
        self.inline.links.extend(inner.links);
    }

    fn wrapped(&mut self, element: &Element, mark: &str) {
        let inner = self.capture(element);
        let content = inner.markdown.trim().to_owned();
        let markdown = if content.is_empty() {
            String::new()
        } else {
            format!("{mark}{content}{mark}")
        };
        self.append(inner, markdown);
    }

    fn heading(&mut self, element: &Element) {
        self.flush(BlockKind::Paragraph);
        let level = element.name[1..].parse::<u8>().unwrap_or(1);
        let inner = self.capture(element);
        self.inline = inner;
        self.flush(BlockKind::Heading(level));
    }

    fn code(&mut self, element: &Element) {
        self.flush(BlockKind::Paragraph);
        let mut raw = String::new();
        raw_text(element, &mut raw);
        let raw = raw.trim_matches('\n').to_owned();
        if !raw.trim().is_empty() {
            self.blocks
                .push(Block::new(BlockKind::Code, raw.clone(), raw, Vec::new()));
        }
    }

    fn link(&mut self, element: &Element) {
        let inner = self.capture(element);
        let label = collapse_whitespace(&inner.markdown);
        let href = element.attr("href").map(str::trim).unwrap_or_default();
        let scripted = href
            .get(..SCRIPT_SCHEME.len())
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case(SCRIPT_SCHEME));
        if href.is_empty() || href.starts_with('#') || scripted {
            self.append(inner, label);
            return;
        }
        let url = resolve(self.options.base.as_ref(), href);
        let text = collapse_whitespace(&inner.text);
        let link = self.register(&text, &url);
        let markdown = match self.options.links {
            LinkStyle::Inline if !label.is_empty() => format!("[{label}]({url})"),
            LinkStyle::List => format!("{label}[{}]", link.number),
            _ => label,
        };
        let text_form = match self.options.links {
            LinkStyle::List => format!("{text}[{}]", link.number),
            _ => text,
        };
        self.inline.markdown.push_str(&markdown);
        self.inline.text.push_str(&text_form);
        self.inline.links.extend(inner.links);
        self.inline.links.push(link);
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

    fn image(&mut self, element: &Element) {
        if self.options.images == ImageStyle::Drop {
            return;
        }
        let alt = collapse_whitespace(element.attr("alt").unwrap_or_default());
        if alt.is_empty() {
            return;
        }
        self.inline.markdown.push_str(&format!("![{alt}]"));
        self.inline.text.push_str(&alt);
    }

    fn table(&mut self, element: &Element) {
        self.flush(BlockKind::Paragraph);
        let table = parse_table(element);
        if table.rows.is_empty() && table.columns.is_empty() {
            return;
        }
        let markdown = table_markdown(&table);
        let text = table
            .rows
            .iter()
            .map(|row| row.join("\t"))
            .collect::<Vec<_>>()
            .join("\n");
        self.blocks
            .push(Block::new(BlockKind::Table, markdown, text, Vec::new()));
        self.tables.push(table);
    }

    fn flush(&mut self, kind: BlockKind) {
        let inline = mem::take(&mut self.inline);
        let text = collapse_whitespace(&inline.text);
        if text.is_empty() {
            return;
        }
        let kind = if self.quote && kind == BlockKind::Paragraph {
            BlockKind::Quote
        } else {
            kind
        };
        let markdown = collapse_whitespace(&inline.markdown);
        self.blocks
            .push(Block::new(kind, markdown, text, inline.links));
    }
}

fn space_runs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() && c != '\u{a0}' {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    out
}

fn raw_text(element: &Element, out: &mut String) {
    for child in &element.children {
        match child {
            Node::Text(text) => out.push_str(text),
            Node::Element(child) if child.name == "br" => out.push('\n'),
            Node::Element(child) => raw_text(child, out),
        }
    }
}

fn hidden(element: &Element) -> bool {
    element.attr("hidden").is_some()
        || element.attr("aria-hidden") == Some("true")
        || element.attr("style").is_some_and(|style| {
            let style: String = style.chars().filter(|c| !c.is_whitespace()).collect();
            style.to_ascii_lowercase().contains("display:none")
        })
}

fn noisy(element: &Element) -> bool {
    if matches!(element.name.as_str(), "body" | "html" | "main" | "article") {
        return false;
    }
    let names = [element.attr("class"), element.attr("id")];
    names.iter().flatten().any(|value| {
        value
            .split(|c: char| c.is_whitespace() || c == '-' || c == '_')
            .any(|word| NOISE.contains(&word.to_ascii_lowercase().as_str()))
    })
}

fn parse_table(element: &Element) -> Table {
    let mut table = Table {
        caption: element.find("caption").map(Element::text),
        ..Table::default()
    };
    let rows: Vec<&Element> = element
        .descendants()
        .into_iter()
        .filter(|candidate| candidate.name == "tr")
        .collect();
    let mut header: Option<Vec<String>> = None;
    for (index, row) in rows.iter().enumerate() {
        let cells: Vec<&Element> = row
            .children
            .iter()
            .filter_map(|child| match child {
                Node::Element(cell) if matches!(cell.name.as_str(), "td" | "th") => Some(cell),
                _ => None,
            })
            .collect();
        let texts: Vec<String> = cells
            .iter()
            .take(MAX_TABLE_COLUMNS)
            .map(|cell| cell.text())
            .collect();
        if index == 0 && !cells.is_empty() && cells.iter().all(|cell| cell.name == "th") {
            header = Some(texts);
        } else if !texts.is_empty() {
            table.rows.push(texts);
        }
    }
    let width = table
        .rows
        .iter()
        .map(Vec::len)
        .chain(header.iter().map(Vec::len))
        .max()
        .unwrap_or(0);
    table.rows.truncate(MAX_TABLE_CELLS / width.max(1));
    table.columns = match header {
        Some(names) => (0..width)
            .map(|index| {
                names
                    .get(index)
                    .filter(|name| !name.is_empty())
                    .cloned()
                    .unwrap_or_else(|| (index + 1).to_string())
            })
            .collect(),
        None => (1..=width).map(|index| index.to_string()).collect(),
    };
    table
}

fn table_markdown(table: &Table) -> String {
    let escape = |cell: &str| cell.replace('|', "\\|");
    let line = |cells: &[String]| {
        let padded: Vec<String> = (0..table.columns.len())
            .map(|index| escape(cells.get(index).map_or("", String::as_str)))
            .collect();
        format!("| {} |", padded.join(" | "))
    };
    let mut out = vec![
        line(&table.columns),
        format!("|{}", "---|".repeat(table.columns.len())),
    ];
    out.extend(table.rows.iter().map(|row| line(row)));
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> ConvertOptions {
        ConvertOptions {
            base: Url::parse("https://example.test/dir/").ok(),
            ..ConvertOptions::default()
        }
    }

    const PAGE: &str = "<html lang=\"pl\"><head><title>Docs</title>\
        <meta name=\"description\" content=\"About it\">\
        <link rel=\"canonical\" href=\"/docs\"></head><body>\
        <header><a href=\"/\">Home</a></header><nav><a href=\"/x\">Nav</a></nav>\
        <div class=\"cookie-banner\">Accept cookies</div>\
        <h1>Install</h1><p>Run <code>make</code> then <a href=\"next\">continue</a>.</p>\
        <ul><li>one</li><li>two<ul><li>nested</li></ul></li></ul>\
        <pre>line 1\nline 2</pre><blockquote><p>quoted</p></blockquote>\
        <img alt=\"diagram\" src=\"d.png\"><table><tr><th>Name</th><th>Size</th></tr>\
        <tr><td>a</td><td>1</td></tr></table><footer>Copyright</footer></body></html>";

    #[test]
    fn keeps_main_content_and_reads_metadata() {
        let page = convert_html(PAGE, &options());
        let texts: Vec<&str> = page
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect();
        assert!(
            !texts
                .iter()
                .any(|text| text.contains("Nav") || text.contains("Copyright"))
        );
        assert!(!texts.iter().any(|text| text.contains("cookies")));
        assert_eq!(page.blocks[0].kind, BlockKind::Heading(1));
        assert_eq!(
            page.blocks[1].markdown,
            "Run `make` then [continue](https://example.test/dir/next)."
        );
        assert_eq!(page.metadata.title.as_deref(), Some("Docs"));
        assert_eq!(page.metadata.description.as_deref(), Some("About it"));
        assert_eq!(page.metadata.language.as_deref(), Some("pl"));
        assert_eq!(
            page.metadata.canonical.as_deref(),
            Some("https://example.test/docs")
        );
    }

    #[test]
    fn converts_lists_code_quotes_images_and_tables() {
        let page = convert_html(PAGE, &options());
        let kinds: Vec<BlockKind> = page.blocks.iter().map(|block| block.kind).collect();
        assert!(kinds.contains(&BlockKind::ListItem(2)));
        assert!(kinds.contains(&BlockKind::Code));
        assert!(kinds.contains(&BlockKind::Quote));
        assert!(
            page.blocks
                .iter()
                .any(|block| block.markdown == "![diagram]")
        );
        assert_eq!(page.tables.len(), 1);
        assert_eq!(page.tables[0].columns, vec!["Name", "Size"]);
        assert_eq!(page.tables[0].rows, vec![vec!["a", "1"]]);
    }

    #[test]
    fn full_page_and_list_links_and_dropped_images() {
        let page = convert_html(
            PAGE,
            &ConvertOptions {
                main_only: false,
                links: LinkStyle::List,
                images: ImageStyle::Drop,
                ..options()
            },
        );
        let texts: Vec<&str> = page
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect();
        assert!(texts.iter().any(|text| text.contains("Copyright")));
        assert!(texts.iter().any(|text| text.contains("Nav[2]")));
        assert!(
            !page
                .blocks
                .iter()
                .any(|block| block.markdown.contains("diagram"))
        );
        assert_eq!(page.links[0].url, "https://example.test/");
    }

    #[test]
    fn repeated_addresses_share_one_numbered_link() {
        let page = convert_html(
            "<p><a href=\"/a\">one</a> <a href=\"/b\">two</a> <a href=\"/a\">again</a></p>",
            &options(),
        );
        assert_eq!(page.links.len(), 2);
        let numbers: Vec<usize> = page.blocks[0].links.iter().map(|link| link.number).collect();
        assert_eq!(numbers, vec![1, 2, 1]);
    }

    #[test]
    fn many_distinct_links_are_collected_in_order() {
        let html: String = (0..20_000)
            .map(|n| format!("<a href=\"/p{n}\">l{n}</a> "))
            .collect();
        let page = convert_html(&html, &options());
        assert_eq!(page.links.len(), 20_000);
        assert_eq!(page.links[19_999].number, 20_000);
    }

    #[test]
    fn script_addresses_are_not_links_in_any_case() {
        let page = convert_html(
            "<p><a href=\"JavaScript:alert(1)\">x</a> <a href=\" javascript:y\">y</a></p>",
            &options(),
        );
        assert!(page.links.is_empty());
        assert_eq!(page.blocks[0].text, "x y");
    }

    #[test]
    fn wide_tables_are_cut_to_the_column_and_cell_limits() {
        let wide: String = (0..600).map(|_| "<td>x</td>").collect();
        let rows: String = (0..10_000).map(|_| "<tr><td>y</td></tr>").collect();
        let page = convert_html(&format!("<table><tr>{wide}</tr>{rows}</table>"), &options());
        let table = &page.tables[0];
        assert_eq!(table.columns.len(), MAX_TABLE_COLUMNS);
        assert!(table.rows.len() <= MAX_TABLE_CELLS / MAX_TABLE_COLUMNS);
        assert!(table.rows.iter().all(|row| row.len() <= MAX_TABLE_COLUMNS));
    }

    #[test]
    fn prefers_the_main_element() {
        let page = convert_html(
            "<body><p>outside</p><main><p>inside</p></main></body>",
            &options(),
        );
        assert_eq!(page.blocks.len(), 1);
        assert_eq!(page.blocks[0].text, "inside");
    }

    #[test]
    fn numbers_tables_without_header() {
        let page = convert_html("<table><tr><td>x</td><td>y|z</td></tr></table>", &options());
        assert_eq!(page.tables[0].columns, vec!["1", "2"]);
        assert!(page.blocks[0].markdown.contains("y\\|z"));
    }
}
