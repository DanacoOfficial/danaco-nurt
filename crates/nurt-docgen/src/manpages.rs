// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Manual pages: `nurt(1)`, `nurt-<command>(1)` and `nurt-commands(7)`.

use std::path::Path;

use nurt_contracts::spec::StageSpec;

use crate::catalog::Catalog;
use crate::error::{Result, read_text, write_text};
use crate::layout::Layout;
use crate::man::{ManHeader, escape, render_body, title_line};
use crate::markdown::{parse, split_front_matter};
use crate::page::{compose, load_page};

/// Manual title shown in the page header.
pub const MANUAL: &str = "Danaco Nurt";

/// Version and date written to the footer of every page.
#[derive(Debug, Clone, Copy)]
pub struct ManOptions<'a> {
    /// Product version.
    pub version: &'a str,
    /// Date in the form `YYYY-MM-DD`.
    pub date: &'a str,
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
            markdown.push_str(&format!("| `{}` | {} |\n", spec.name, spec.summary));
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

/// Returns the page `nurt(1)` from the Markdown of the command line description.
#[must_use]
pub fn program_page(markdown: &str, options: &ManOptions<'_>) -> String {
    let (_, body) = split_front_matter(markdown);
    page(
        "nurt",
        1,
        "command console with typed pipelines and per-operation undo",
        body,
        options,
    )
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
    let program = read_text(&layout.site().join("command-line.md"))?;
    write_text(&out.join("nurt.1"), &program_page(&program, options))?;
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
        write_text(
            &out.join(format!("nurt-{}.1", spec.name)),
            &command_page(spec, &composed, options),
        )?;
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
    fn program_page_drops_front_matter() {
        let text = program_page(
            "---\ntitle: Command line\n---\n## Synopsis\n\nnurt exec\n",
            &OPTIONS,
        );
        assert!(text.contains(".SH NAME\nnurt \\- command console"));
        assert!(!text.contains("title"));
    }
}
