// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Static documentation site.
//!
//! The site is HTML and CSS only: reading needs no JavaScript and no network
//! access. The pages come from the general pages in `docs/site/`, the command
//! pages composed from the catalog, the cookbook, the cheat sheets and the
//! specification. A search index in JSON is written next to the pages.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use crate::catalog::Catalog;
use crate::cookbook::{Category, load_cookbook};
use crate::error::{DocgenError, Result, list_files, read_text, write_text};
use crate::html::{self, Rendered, slugify};
use crate::layout::Layout;
use crate::markdown::{Block, blocks_text, parse, plain_text, split_front_matter};
use crate::page::{compose, load_page};

/// Base address of the repository files linked from the site.
pub const REPOSITORY_BLOB: &str = "https://github.com/DanacoOfficial/nurt/blob/main";

/// Files of the identity that the build copies unchanged next to the pages: the
/// touch icon, the application icons and the web manifest.
pub const SITE_ASSETS: [&str; 5] = [
    "apple-touch-icon.png",
    "icon-192.png",
    "icon-512.png",
    "icon-maskable-512.png",
    "site.webmanifest",
];

/// Sections of the site in navigation order.
pub const SECTIONS: [&str; 5] = [
    "Guide",
    "Commands",
    "Cookbook",
    "Cheat sheets",
    "Specification",
];

/// Page of the site before rendering.
#[derive(Debug, Clone)]
struct SourcePage {
    source: String,
    output: String,
    title: String,
    section: &'static str,
    group: String,
    order: i64,
    markdown: String,
}

/// Result of a site build.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SiteReport {
    /// Number of HTML pages written.
    pub pages: usize,
    /// Relative links that point to no page and no repository file.
    pub unresolved_links: Vec<String>,
}

fn directory_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// Returns the relative path from a directory to a file, both given from the site root.
///
/// # Examples
///
/// ```
/// assert_eq!(nurt_docgen::site::relative_path("commands", "index.html"), "../index.html");
/// assert_eq!(nurt_docgen::site::relative_path("", "commands/find.html"), "commands/find.html");
/// assert_eq!(nurt_docgen::site::relative_path("cookbook", "cookbook/a.html"), "a.html");
/// ```
#[must_use]
pub fn relative_path(from_dir: &str, to: &str) -> String {
    let from: Vec<&str> = from_dir.split('/').filter(|p| !p.is_empty()).collect();
    let target: Vec<&str> = to.split('/').collect();
    let common = from
        .iter()
        .zip(target.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut parts: Vec<&str> = vec![".."; from.len() - common];
    parts.extend(&target[common..]);
    parts.join("/")
}

/// Text of a link target with its `%XX` escapes replaced by the bytes they
/// stand for; the target is left as written when the result is not UTF-8.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let escaped = bytes
            .get(at + 1..at + 3)
            .filter(|_| bytes[at] == b'%')
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match escaped {
            Some(byte) => {
                out.push(byte);
                at += 3;
            }
            None => {
                out.push(bytes[at]);
                at += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

struct Resolver<'a> {
    outputs: HashMap<String, String>,
    layout: &'a Layout,
    unresolved: RefCell<Vec<String>>,
}

impl Resolver<'_> {
    fn rewrite(&self, source: &str, output: &str, url: &str) -> String {
        if url.is_empty() || url.starts_with('#') || url.contains(':') {
            return url.to_string();
        }
        let (path, fragment) = match url.split_once('#') {
            Some((path, fragment)) => (path, format!("#{fragment}")),
            None => (url, String::new()),
        };
        let path = percent_decode(path);
        let key = normalize(&format!("{}/{path}", directory_of(source)));
        if let Some(target) = self.outputs.get(&key) {
            return format!("{}{fragment}", relative_path(directory_of(output), target));
        }
        if self.layout.root().join(&key).exists() {
            return format!("{REPOSITORY_BLOB}/{key}");
        }
        self.unresolved
            .borrow_mut()
            .push(format!("{source}: {url}"));
        url.to_string()
    }
}

fn front_value(front: &str, key: &str) -> Option<String> {
    front
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim() == key)
        .map(|(_, value)| value.trim().to_string())
}

fn first_heading(markdown: &str) -> Option<String> {
    parse(markdown).blocks.iter().find_map(|block| match block {
        Block::Heading { level: 1, content } => Some(plain_text(content)),
        _ => None,
    })
}

fn guide_pages(layout: &Layout) -> Result<Vec<SourcePage>> {
    let mut pages = Vec::new();
    for path in list_files(&layout.site(), "md")? {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        if stem.starts_with('_') {
            continue;
        }
        let text = read_text(&path)?;
        let (front, body) = split_front_matter(&text);
        let front = front.unwrap_or_default();
        let title = front_value(front, "title")
            .or_else(|| first_heading(body))
            .ok_or_else(|| {
                DocgenError::invalid(path.display().to_string(), "the page has no title")
            })?;
        let order = front_value(front, "order")
            .and_then(|o| o.parse().ok())
            .unwrap_or(1000);
        pages.push(SourcePage {
            source: format!("docs/site/{stem}.md"),
            output: format!("{stem}.html"),
            title,
            section: SECTIONS[0],
            group: String::new(),
            order,
            markdown: body.to_string(),
        });
    }
    pages.sort_by(|a, b| (a.order, &a.title).cmp(&(b.order, &b.title)));
    Ok(pages)
}

fn command_pages(layout: &Layout, catalog: &Catalog) -> Result<Vec<SourcePage>> {
    let mut pages = Vec::new();
    for spec in catalog.commands() {
        let path = layout.commands().join(format!("{}.md", spec.name));
        let written = if path.exists() {
            Some(load_page(&path)?)
        } else {
            None
        };
        pages.push(SourcePage {
            source: format!("docs/commands/{}.md", spec.name),
            output: format!("commands/{}.html", spec.name),
            title: spec.name.clone(),
            section: SECTIONS[1],
            group: spec.group.clone(),
            order: 0,
            markdown: compose(spec, written.as_ref()),
        });
    }
    let mut index = String::from(
        "# Commands\n\nEvery command has one page that lists its forms, parameters, record kinds and effect.\n",
    );
    for file in &catalog.groups {
        index.push_str(&format!(
            "\n## {}\n\n{}\n\n| Command | Summary |\n|---|---|\n",
            file.group, file.summary
        ));
        for spec in &file.commands {
            index.push_str(&format!(
                "| [`{0}`](../commands/{0}.md) | {1} |\n",
                spec.name,
                spec.summary.replace('|', "\\|")
            ));
        }
    }
    pages.insert(
        0,
        SourcePage {
            source: "docs/commands/README.md".to_string(),
            output: "commands/index.html".to_string(),
            title: "Commands".to_string(),
            section: SECTIONS[1],
            group: String::new(),
            order: 0,
            markdown: index,
        },
    );
    Ok(pages)
}

fn cookbook_pages(layout: &Layout, categories: &[Category]) -> Result<Vec<SourcePage>> {
    let mut pages = Vec::new();
    let mut index = String::from(
        "# Cookbook\n\nEach recipe states a problem, the program, the expected result and the equivalent in bash and PowerShell.\n",
    );
    for category in categories {
        let path = layout.cookbook().join(format!("{}.md", category.id));
        let text = read_text(&path)?;
        let (_, body) = split_front_matter(&text);
        pages.push(SourcePage {
            source: format!("docs/cookbook/{}.md", category.id),
            output: format!("cookbook/{}.html", category.id),
            title: category.title.clone(),
            section: SECTIONS[2],
            group: String::new(),
            order: 0,
            markdown: body.to_string(),
        });
        index.push_str(&format!(
            "\n## [{}](../cookbook/{}.md)\n\n",
            category.title, category.id
        ));
        for recipe in &category.recipes {
            index.push_str(&format!(
                "- [{}](../cookbook/{}.md#{})\n",
                recipe.title,
                category.id,
                slugify(&recipe.title)
            ));
        }
    }
    pages.insert(
        0,
        SourcePage {
            source: "docs/cookbook/README.md".to_string(),
            output: "cookbook/index.html".to_string(),
            title: "Cookbook".to_string(),
            section: SECTIONS[2],
            group: String::new(),
            order: 0,
            markdown: index,
        },
    );
    Ok(pages)
}

fn cheatsheet_pages(layout: &Layout) -> Result<Vec<SourcePage>> {
    let mut pages = Vec::new();
    for path in list_files(&layout.cheatsheets(), "md")? {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let text = read_text(&path)?;
        let (front, body) = split_front_matter(&text);
        let title = front
            .and_then(|f| front_value(f, "title"))
            .or_else(|| first_heading(body))
            .ok_or_else(|| {
                DocgenError::invalid(path.display().to_string(), "the sheet has no title")
            })?;
        pages.push(SourcePage {
            source: format!("docs/cheatsheets/{stem}.md"),
            output: format!("cheatsheets/{stem}.html"),
            title,
            section: SECTIONS[3],
            group: String::new(),
            order: 0,
            markdown: body.to_string(),
        });
    }
    Ok(pages)
}

fn spec_pages(layout: &Layout) -> Result<Vec<SourcePage>> {
    let mut pages = Vec::new();
    let mut index = String::from(
        "# Specification\n\nThe specification describes the language, the syntax tree, the catalog, the report, the journal and the configuration.\n\n| Document | Title |\n|---|---|\n",
    );
    for path in list_files(&layout.spec(), "md")? {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let text = read_text(&path)?;
        let (_, body) = split_front_matter(&text);
        let title = first_heading(body).unwrap_or_else(|| stem.clone());
        index.push_str(&format!(
            "| [`{stem}`](../spec/{stem}.md) | {} |\n",
            title.replace('|', "\\|")
        ));
        pages.push(SourcePage {
            source: format!("spec/{stem}.md"),
            output: format!("spec/{stem}.html"),
            title,
            section: SECTIONS[4],
            group: String::new(),
            order: 0,
            markdown: body.to_string(),
        });
    }
    pages.insert(
        0,
        SourcePage {
            source: "spec/README.md".to_string(),
            output: "spec/index.html".to_string(),
            title: "Specification".to_string(),
            section: SECTIONS[4],
            group: String::new(),
            order: 0,
            markdown: index,
        },
    );
    Ok(pages)
}

/// Replaces `{{name}}` placeholders of a template in a single pass.
#[must_use]
pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let name = &after[..end];
                match values.iter().find(|(key, _)| *key == name) {
                    Some((_, value)) => out.push_str(value),
                    None => out.push_str(&rest[start..start + 2 + end + 2]),
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn link(root: &str, target: &str, text: &str, current: bool) -> String {
    let marker = if current {
        " aria-current=\"page\""
    } else {
        ""
    };
    format!(
        "<a href=\"{}{}\"{marker}>{}</a>",
        html::escape(root),
        html::escape(target),
        html::escape(text)
    )
}

fn main_nav(pages: &[SourcePage], root: &str, current: &SourcePage) -> String {
    let mut items = String::new();
    for section in SECTIONS {
        let target = match section {
            "Guide" => Some("index.html".to_string()),
            "Commands" => Some("commands/index.html".to_string()),
            "Cookbook" => Some("cookbook/index.html".to_string()),
            "Specification" => Some("spec/index.html".to_string()),
            _ => pages
                .iter()
                .find(|p| p.section == section)
                .map(|p| p.output.clone()),
        };
        if let Some(target) = target {
            items.push_str(&format!(
                "<li>{}</li>\n",
                link(root, &target, section, current.section == section)
            ));
        }
    }
    format!("<nav aria-label=\"Sections\">\n<ul>\n{items}</ul>\n</nav>")
}

fn sidebar(pages: &[SourcePage], root: &str, current: &SourcePage) -> String {
    let mut items = String::new();
    for page in pages
        .iter()
        .filter(|p| p.section == current.section && p.group == current.group)
    {
        items.push_str(&format!(
            "<li>{}</li>\n",
            link(
                root,
                &page.output,
                &page.title,
                page.output == current.output
            )
        ));
    }
    format!(
        "<nav aria-label=\"{} pages\">\n<ul>\n{items}</ul>\n</nav>",
        current.section
    )
}

fn table_of_contents(rendered: &Rendered) -> String {
    let entries: Vec<_> = rendered.headings.iter().filter(|h| h.level == 2).collect();
    if entries.len() < 3 {
        return String::new();
    }
    let mut out = String::from("<nav aria-label=\"On this page\" class=\"toc\">\n<ul>\n");
    for heading in entries {
        out.push_str(&format!(
            "<li><a href=\"#{}\">{}</a></li>\n",
            heading.id,
            html::escape(&heading.text)
        ));
    }
    out.push_str("</ul>\n</nav>");
    out
}

fn summary_of(markdown: &str) -> String {
    let document = parse(markdown);
    let text = document
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Paragraph(content) => Some(plain_text(content)),
            _ => None,
        })
        .unwrap_or_default();
    text.chars().take(200).collect()
}

/// Copies a file byte for byte and creates the parent directories.
fn copy_file(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|source| DocgenError::io(parent, source))?;
    }
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(|source| DocgenError::io(from, source))
}

/// Builds the site into a directory.
///
/// # Errors
///
/// Returns an error when an input cannot be read or an output cannot be written.
pub fn build(layout: &Layout, catalog: &Catalog, out: &Path, version: &str) -> Result<SiteReport> {
    let template = read_text(&layout.site().join("template.html"))?;
    let style = read_text(&layout.site().join("style.css"))?;
    let favicon = read_text(&layout.site().join("favicon.svg"))?;
    let categories = load_cookbook(&layout.cookbook())?;
    let mut pages = guide_pages(layout)?;
    pages.extend(command_pages(layout, catalog)?);
    pages.extend(cookbook_pages(layout, &categories)?);
    pages.extend(cheatsheet_pages(layout)?);
    pages.extend(spec_pages(layout)?);
    let resolver = Resolver {
        outputs: pages
            .iter()
            .map(|p| (p.source.clone(), p.output.clone()))
            .collect(),
        layout,
        unresolved: RefCell::new(Vec::new()),
    };
    let mut search: Vec<serde_json::Value> = Vec::new();
    for page in &pages {
        let document = parse(&page.markdown);
        let rendered = html::render(&document, &|url| {
            resolver.rewrite(&page.source, &page.output, url)
        });
        let depth = page.output.matches('/').count();
        let root = "../".repeat(depth);
        let content = rendered.html.clone();
        let nav = main_nav(&pages, &root, page);
        let side = sidebar(&pages, &root, page);
        let toc = table_of_contents(&rendered);
        let title = html::escape(&page.title);
        let version_html = html::escape(version);
        let document_html = fill(
            &template,
            &[
                ("title", &title),
                ("root", &root),
                ("nav", &nav),
                ("sidebar", &side),
                ("toc", &toc),
                ("content", &content),
                ("version", &version_html),
            ],
        );
        write_text(&out.join(&page.output), &document_html)?;
        search.push(serde_json::json!({
            "title": page.title,
            "url": page.output,
            "section": page.section,
            "summary": summary_of(&page.markdown),
            "headings": rendered.headings.iter().filter(|h| h.level >= 2).map(|h| h.text.clone()).collect::<Vec<_>>(),
            "length": blocks_text(&document.blocks).len(),
        }));
    }
    write_text(&out.join("style.css"), &style)?;
    write_text(&out.join("favicon.svg"), &favicon)?;
    for name in SITE_ASSETS {
        copy_file(&layout.site().join(name), &out.join(name))?;
    }
    let index = serde_json::to_string_pretty(&search)
        .map_err(|e| DocgenError::invalid("search index", e.to_string()))?;
    write_text(&out.join("search-index.json"), &index)?;
    Ok(SiteReport {
        pages: pages.len(),
        unresolved_links: resolver.unresolved.into_inner(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_relative_paths() {
        assert_eq!(relative_path("commands", "index.html"), "../index.html");
        assert_eq!(
            relative_path("", "commands/find.html"),
            "commands/find.html"
        );
        assert_eq!(relative_path("cookbook", "cookbook/a.html"), "a.html");
        assert_eq!(
            relative_path("spec", "commands/x.html"),
            "../commands/x.html"
        );
    }

    #[test]
    fn decodes_percent_escapes_of_link_targets() {
        assert_eq!(percent_decode("my%20page.md"), "my page.md");
        assert_eq!(percent_decode("z%C5%82y.md"), "z\u{142}y.md");
        assert_eq!(percent_decode("100%.md"), "100%.md");
        assert_eq!(percent_decode("a%zzb"), "a%zzb");
        assert_eq!(percent_decode("bad%ff.md"), "bad%ff.md");
        assert_eq!(percent_decode("%"), "%");
    }

    #[test]
    fn resolves_a_link_that_percent_encodes_the_name_of_the_page() {
        let root = tempfile::tempdir().unwrap();
        let layout = Layout::new(root.path());
        let mut outputs = HashMap::new();
        outputs.insert("docs/site/my page.md".to_owned(), "my-page.html".to_owned());
        let resolver = Resolver {
            outputs,
            layout: &layout,
            unresolved: RefCell::new(Vec::new()),
        };
        let link = resolver.rewrite("docs/site/index.md", "index.html", "my%20page.md#top");
        assert_eq!(link, "my-page.html#top");
        assert!(resolver.unresolved.borrow().is_empty());
    }

    #[test]
    fn normalizes_dot_segments() {
        assert_eq!(
            normalize("docs/site/../commands/./find.md"),
            "docs/commands/find.md"
        );
    }

    #[test]
    fn fills_placeholders_once() {
        let out = fill("<{{a}}>{{b}}{{c}}", &[("a", "{{b}}"), ("b", "x")]);
        assert_eq!(out, "<{{b}}>x{{c}}");
    }

    #[test]
    fn fills_template_without_placeholders_and_unclosed_braces() {
        assert_eq!(fill("plain {{open", &[]), "plain {{open");
    }

    #[test]
    fn resolver_maps_pages_and_repository_files() {
        let layout = Layout::new(Path::new("."));
        let mut outputs = HashMap::new();
        outputs.insert(
            "docs/commands/find.md".to_string(),
            "commands/find.html".to_string(),
        );
        let resolver = Resolver {
            outputs,
            layout: &layout,
            unresolved: RefCell::new(Vec::new()),
        };
        let found = resolver.rewrite("docs/site/guide.md", "guide.html", "../commands/find.md#x");
        assert_eq!(found, "commands/find.html#x");
        assert_eq!(
            resolver.rewrite("a.md", "a.html", "https://example.com"),
            "https://example.com"
        );
        assert_eq!(resolver.rewrite("a.md", "a.html", "#top"), "#top");
        assert_eq!(
            resolver.rewrite("a.md", "a.html", "nowhere.md"),
            "nowhere.md"
        );
        assert_eq!(resolver.unresolved.borrow().len(), 1);
    }

    #[test]
    fn link_escapes_the_target_and_the_text() {
        assert_eq!(
            link("../", "a\"b<c>.html", "x & y", true),
            "<a href=\"../a&quot;b&lt;c&gt;.html\" aria-current=\"page\">x &amp; y</a>"
        );
    }

    #[test]
    fn fill_inserts_values_literally_and_keeps_multi_byte_text() {
        assert_eq!(
            fill(
                "za\u{17c}\u{f3}\u{142}\u{107} {{v}} \u{1f600}",
                &[("v", "<b>")]
            ),
            "za\u{17c}\u{f3}\u{142}\u{107} <b> \u{1f600}"
        );
    }

    #[test]
    fn relative_path_handles_equal_directories_and_deep_sources() {
        assert_eq!(relative_path("a/b", "a/b/c.html"), "c.html");
        assert_eq!(relative_path("a/b", "c.html"), "../../c.html");
        assert_eq!(relative_path("", "index.html"), "index.html");
    }

    #[test]
    fn normalize_does_not_climb_above_the_root() {
        assert_eq!(normalize("../../etc/passwd"), "etc/passwd");
        assert_eq!(normalize(""), "");
    }

    #[test]
    fn reads_front_matter_values() {
        let front = "title: Name: with colon\norder: 7";
        assert_eq!(
            front_value(front, "title").as_deref(),
            Some("Name: with colon")
        );
        assert_eq!(front_value(front, "order").as_deref(), Some("7"));
        assert_eq!(front_value(front, "missing"), None);
    }

    #[test]
    fn extracts_summary_from_first_paragraph() {
        assert_eq!(
            summary_of("# T\n\nFirst **para**.\n\nSecond."),
            "First para."
        );
    }
}
