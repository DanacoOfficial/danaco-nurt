// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Manual pages: `nurt(1)`, `nurt-<command>(1)` and `nurt-commands(7)`.
//!
//! A subcommand of the executable has the page `nurt-<subcommand>(1)`, built
//! from `tables/cli-subcommands.toml`. When a catalog command has the same
//! name, the page of the command gets the section `COMMAND LINE` instead.
//!
//! The pages come from the sources of the site: the catalog, the command
//! pages and the page `docs/site/command-line.md`. `nurt-docgen man` writes
//! them into the directory given with `--out`; the repository keeps only the
//! sources.

use std::path::Path;

use nurt_contracts::spec::StageSpec;

use crate::catalog::Catalog;
use crate::cli::{self, Tree, program_summary};
use crate::error::{Result, read_text, write_text};
use crate::layout::Layout;
use crate::man::{ManHeader, escape, render_body, title_line};
use crate::markdown::{parse, split_front_matter};
use crate::page::{compose, load_page};

/// Manual title shown in the page header.
pub const MANUAL: &str = "Danaco Nurt";

/// Heading before which `nurt(1)` lists the command groups of the catalog.
const SEE_ALSO_HEADING: &str = "\n## See also";

/// Version and date written to the footer of every page.
#[derive(Debug, Clone, Copy)]
pub struct ManOptions<'a> {
    /// Product version.
    pub version: &'a str,
    /// Date in the form `YYYY-MM-DD`.
    pub date: &'a str,
}

/// Escapes the pipe character, which would end a cell of a Markdown table.
fn table_cell(text: &str) -> String {
    text.replace('|', "\\|")
}

fn page(
    name: &str,
    section: u8,
    summary: &str,
    markdown: &str,
    options: &ManOptions<'_>,
) -> String {
    let source = format!("Danaco Nurt {}", options.version);
    let header = ManHeader {
        name,
        section,
        date: options.date,
        source: &source,
        manual: MANUAL,
    };
    let mut out = title_line(&header);
    out.push_str(&format!(
        ".SH NAME\n{} \\- {}\n",
        escape(name),
        escape(summary.trim_end_matches('.'))
    ));
    out.push_str(&render_body(&parse(markdown)));
    out
}

/// Returns the manual page of a command.
#[must_use]
pub fn command_page(spec: &StageSpec, composed: &str, options: &ManOptions<'_>) -> String {
    page(
        &format!("nurt-{}", spec.name),
        1,
        &spec.summary,
        composed,
        options,
    )
}

/// Returns the page `nurt-commands(7)`, an index of all commands by group.
#[must_use]
pub fn commands_index(catalog: &Catalog, options: &ManOptions<'_>) -> String {
    let mut markdown = String::from(
        "## Description\n\nThe commands of the Danaco Nurt language, by group. The page `nurt-<command>(1)` describes one command.\n",
    );
    for file in &catalog.groups {
        markdown.push_str(&format!(
            "\n### {}\n\n{}\n\n| Command | Summary |\n|---|---|\n",
            file.group, file.summary
        ));
        for spec in &file.commands {
            markdown.push_str(&format!(
                "| `{}` | {} |\n",
                spec.name,
                table_cell(&spec.summary)
            ));
        }
    }
    markdown.push_str("\n## See also\n\n`nurt(1)`\n");
    page(
        "nurt-commands",
        7,
        "commands of the Danaco Nurt language",
        &markdown,
        options,
    )
}

/// Returns the section of `nurt(1)` that lists the command groups of the catalog.
fn commands_section(catalog: &Catalog) -> String {
    let mut out = format!(
        "## Commands\n\nThe language has {} commands in {} groups. `nurt-commands(7)` lists them and `nurt-<command>(1)` describes one command.\n\n| Group | Commands | Summary |\n|---|---|---|\n",
        catalog.total(),
        catalog.groups.len()
    );
    for file in &catalog.groups {
        out.push_str(&format!(
            "| `{}` | {} | {} |\n",
            file.group,
            file.commands.len(),
            table_cell(&file.summary)
        ));
    }
    out
}

/// Returns the page `nurt(1)`.
///
/// The body is the Markdown of the command line description; the section on
/// the command groups of the catalog is inserted before the section
/// "See also", or appended when the description has none.
#[must_use]
pub fn program_page(markdown: &str, catalog: &Catalog, options: &ManOptions<'_>) -> String {
    let (_, body) = split_front_matter(markdown);
    let section = commands_section(catalog);
    let body = match body.find(SEE_ALSO_HEADING) {
        Some(at) => format!("{}\n{section}{}", &body[..at], &body[at..]),
        None => format!("{body}\n{section}"),
    };
    page("nurt", 1, &program_summary(), &body, options)
}

/// Returns the line of the NAME section of a manual page with the roff
/// escapes `\-`, `\e` and `\&` resolved, or `None` when the page has no
/// NAME section.
#[must_use]
pub fn name_line(page: &str) -> Option<String> {
    let mut lines = page.lines().skip_while(|line| *line != ".SH NAME");
    lines.next()?;
    lines.next().map(unescape)
}

fn unescape(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('-') => out.push('-'),
            Some('e') => out.push('\\'),
            Some('&') | None => {}
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

/// Writes all manual pages into a directory and returns their number.
///
/// # Errors
///
/// Returns an error when an input cannot be read or a page cannot be written.
pub fn build(
    layout: &Layout,
    catalog: &Catalog,
    out: &Path,
    options: &ManOptions<'_>,
) -> Result<usize> {
    let mut written = 0;
    let tree = Tree::load(layout)?;
    let program = read_text(&layout.site().join("command-line.md"))?;
    write_text(
        &out.join("nurt.1"),
        &program_page(&program, catalog, options),
    )?;
    write_text(
        &out.join("nurt-commands.7"),
        &commands_index(catalog, options),
    )?;
    written += 2;
    for spec in catalog.commands() {
        let path = layout.commands().join(format!("{}.md", spec.name));
        let written_page = if path.exists() {
            Some(load_page(&path)?)
        } else {
            None
        };
        let composed = compose(spec, written_page.as_ref());
        let mut text = command_page(spec, &composed, options);
        if let Some(sub) = tree.subcommands().iter().find(|s| s.name == spec.name) {
            text.push_str(&cli::command_line_section(sub)?);
        }
        write_text(&out.join(format!("nurt-{}.1", spec.name)), &text)?;
        written += 1;
    }
    for sub in tree.subcommands() {
        if catalog.command(&sub.name).is_some() {
            continue;
        }
        let extra = if sub.name == "serve" {
            let reference = layout.reference().join("serve-protocol.md");
            Some(cli::reference_sections(&read_text(&reference)?))
        } else {
            None
        };
        let text = cli::subcommand_page(sub, extra.as_deref(), options)?;
        write_text(&out.join(format!("nurt-{}.1", sub.name)), &text)?;
        written += 1;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::fixtures::catalog;

    const OPTIONS: ManOptions<'static> = ManOptions {
        version: "0.1.0",
        date: "2026-01-01",
    };

    #[test]
    fn command_page_has_title_name_and_sections() {
        let catalog = catalog();
        let spec = catalog.command("find").expect("fixture command");
        let composed = compose(spec, None);
        let text = command_page(spec, &composed, &OPTIONS);
        assert!(text.starts_with(
            ".TH \"NURT\\-FIND\" \"1\" \"2026-01-01\" \"Danaco Nurt 0.1.0\" \"Danaco Nurt\"\n"
        ));
        assert!(text.contains(".SH NAME\nnurt\\-find \\- Find entries under a directory\n"));
        assert!(text.contains(".SH SYNOPSIS\n"));
        assert!(text.contains(".SH PARAMETERS\n"));
    }

    #[test]
    fn index_lists_every_command() {
        let catalog = catalog();
        let text = commands_index(&catalog, &OPTIONS);
        assert!(text.contains(".TH \"NURT\\-COMMANDS\" \"7\""));
        assert!(text.contains("\\fBgit.status\\fR"));
        assert!(text.contains(".SS \"core\""));
    }

    #[test]
    fn program_page_names_the_program_with_the_product_sentence() {
        let text = program_page(
            "---\ntitle: Command line\n---\n## Synopsis\n\nnurt exec\n",
            &catalog(),
            &OPTIONS,
        );
        assert_eq!(
            name_line(&text).as_deref(),
            Some("nurt - command shell and language where every change can be undone")
        );
        assert!(text.contains(
            ".SH NAME\nnurt \\- command shell and language where every change can be undone\n"
        ));
        assert!(!text.contains("title"));
    }

    #[test]
    fn program_page_lists_groups_before_see_also() {
        let catalog = catalog();
        let text = program_page(
            "## Description\n\nText.\n\n## See also\n\n`nurt-commands(7)`\n",
            &catalog,
            &OPTIONS,
        );
        let commands = text.find(".SH COMMANDS").expect("commands section");
        let see_also = text.find(".SH SEE ALSO").expect("see also section");
        assert!(commands < see_also);
        assert!(text.contains(&format!(
            "The language has {} commands in {} groups.",
            catalog.total(),
            catalog.groups.len()
        )));
        assert!(text.contains(".TP\n\\fBcore\\fR\n"));
    }

    #[test]
    fn program_page_appends_groups_without_see_also() {
        let text = program_page("## Description\n\nText.\n", &catalog(), &OPTIONS);
        assert!(text.contains(".SH COMMANDS"));
    }

    #[test]
    fn table_cells_keep_pipes_inside_the_cell() {
        assert_eq!(table_cell("a | b"), "a \\| b");
        let rows = crate::markdown::parse(&format!(
            "| A | B |\n|---|---|\n| x | {} |\n",
            table_cell("p | q")
        ));
        let Some(crate::markdown::Block::Table { rows, .. }) = rows.blocks.first() else {
            panic!("expected a table");
        };
        assert_eq!(rows[0].len(), 2);
    }

    #[test]
    fn page_escapes_quotes_and_backslashes_of_the_name_line() {
        let text = page("nurt-x", 1, "a \"quoted\" back\\slash.", "", &OPTIONS);
        assert!(
            text.contains(".SH NAME\nnurt\\-x \\- a \"quoted\" back\\eslash\n"),
            "{text}"
        );
    }

    #[test]
    fn name_line_resolves_roff_escapes() {
        let page = ".TH \"X\" \"1\"\n.SH NAME\nnurt\\-find \\- a\\eb \\&.c\n.SH SYNOPSIS\n";
        assert_eq!(name_line(page).as_deref(), Some("nurt-find - a\\b .c"));
        assert_eq!(name_line(".TH \"X\" \"1\"\n"), None);
    }
}
