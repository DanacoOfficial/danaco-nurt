// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Command line tree of `nurt`, shell completions and manual pages of the subcommands.
//!
//! The tree comes from `tables/cli-subcommands.toml`. `nurt-cli` parses its
//! arguments without `clap`, so the tree is rebuilt here as a `clap::Command`
//! for `clap_complete` and `clap_mangen`.

use std::path::Path;

use clap::{Arg, ArgAction, Command};
use clap_complete::{Generator, Shell};
use nurt_contracts::product::{PROGRAM_NAME, TAGLINE};
use serde::Deserialize;

use crate::error::{DocgenError, Result, read_text, write_text};
use crate::layout::Layout;
use crate::man::{ManHeader, render_body, title_line};
use crate::manpages::{MANUAL, ManOptions};
use crate::markdown::parse;

/// Format identifier expected in the table of subcommands.
const FORMAT: &str = "nurt-cli-subcommands/1";

/// Shells for which completions are generated.
pub const SHELLS: [Shell; 5] = [
    Shell::Bash,
    Shell::Zsh,
    Shell::Fish,
    Shell::PowerShell,
    Shell::Elvish,
];

/// One subcommand of the `nurt` executable.
#[derive(Debug, Clone, Deserialize)]
pub struct Subcommand {
    /// Name of the subcommand.
    pub name: String,
    /// One-sentence description.
    pub description: String,
    /// Arguments in the notation of the table, such as `[--limit N]`.
    #[serde(default)]
    pub arguments: Vec<String>,
}

/// Table of the subcommands of `nurt`.
#[derive(Debug, Clone, Deserialize)]
pub struct Tree {
    format: String,
    #[serde(rename = "subcommand")]
    subcommands: Vec<Subcommand>,
}

impl Tree {
    /// Parses the table from its TOML text.
    ///
    /// # Errors
    ///
    /// Returns an error when the text is not valid TOML, has another format
    /// identifier or lists a name twice.
    pub fn parse(input: &str, text: &str) -> Result<Self> {
        let tree: Tree =
            toml::from_str(text).map_err(|e| DocgenError::invalid(input, e.to_string()))?;
        if tree.format != FORMAT {
            return Err(DocgenError::invalid(
                input,
                format!("format is {}, expected {FORMAT}", tree.format),
            ));
        }
        for (at, sub) in tree.subcommands.iter().enumerate() {
            if tree.subcommands[..at].iter().any(|s| s.name == sub.name) {
                return Err(DocgenError::invalid(
                    input,
                    format!("subcommand {} is listed twice", sub.name),
                ));
            }
        }
        Ok(tree)
    }

    /// Loads the table of a repository.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read or is invalid.
    pub fn load(layout: &Layout) -> Result<Self> {
        let path = layout.cli_subcommands();
        Self::parse(&path.display().to_string(), &read_text(&path)?)
    }

    /// Subcommands in the order of the table.
    #[must_use]
    pub fn subcommands(&self) -> &[Subcommand] {
        &self.subcommands
    }

    /// Returns the whole tree as the command `nurt`.
    #[must_use]
    pub fn command(&self) -> Command {
        let mut root = Command::new(PROGRAM_NAME)
            .about(program_summary())
            .disable_help_subcommand(true)
            .disable_version_flag(true)
            .arg(
                Arg::new("version")
                    .short('V')
                    .long("version")
                    .action(ArgAction::SetTrue)
                    .help("Print the version"),
            );
        for sub in &self.subcommands {
            root = root.subcommand(subcommand(sub, false));
        }
        root
    }
}

/// Returns the description of the program: the product sentence with a lowercase first letter.
#[must_use]
pub fn program_summary() -> String {
    let mut chars = TAGLINE.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn subcommand(sub: &Subcommand, page: bool) -> Command {
    let mut cmd = if page {
        Command::new(format!("{PROGRAM_NAME}-{}", sub.name))
            .bin_name(format!("{PROGRAM_NAME} {}", sub.name))
    } else {
        Command::new(sub.name.clone())
    };
    cmd = cmd.about(sub.description.clone());
    let mut positionals = 0;
    for text in &sub.arguments {
        let (optional, inner) = match text.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            Some(inner) => (true, inner),
            None => (false, text.as_str()),
        };
        let choices = split_choices(inner);
        if choices.len() > 1 {
            positionals += 1;
            cmd = cmd.arg(
                Arg::new(format!("choice{positionals}"))
                    .value_name(choices.join("|"))
                    .value_parser(choices)
                    .required(!optional),
            );
            continue;
        }
        let alternatives = split_alternatives(inner);
        let required = !optional && alternatives.len() == 1;
        for alternative in &alternatives {
            cmd = add_argument(cmd, alternative, required, &mut positionals);
        }
    }
    cmd
}

fn add_argument(cmd: Command, text: &str, required: bool, positionals: &mut usize) -> Command {
    if let Some(option) = text.strip_prefix("--") {
        let (name, value) = match option.split_once(' ') {
            Some((name, value)) => (name, Some(value.trim())),
            None => (option, None),
        };
        let arg = Arg::new(name.to_string()).long(name.to_string());
        return cmd.arg(match value {
            None => arg.action(ArgAction::SetTrue),
            Some(value) => {
                let choices = split_choices(value);
                if choices.len() > 1 {
                    arg.value_name(choices.join("|")).value_parser(choices)
                } else {
                    arg.value_name(value.trim_matches(['<', '>']).to_string())
                }
            }
        });
    }
    let words = split_words(text);
    *positionals += 1;
    if words.iter().any(|w| w.starts_with('<')) {
        let mut cmd = cmd;
        for word in words {
            let name = word
                .trim_start_matches('<')
                .split([' ', '>'])
                .next()
                .unwrap_or("value")
                .to_string();
            let id = format!("{name}{positionals}");
            cmd = cmd.arg(Arg::new(id).value_name(name).required(required));
            *positionals += 1;
        }
        return cmd;
    }
    let name = words.first().cloned().unwrap_or_else(|| "value".into());
    cmd.arg(
        Arg::new(format!("{name}{positionals}"))
            .value_name(name)
            .num_args(0..)
            .required(required),
    )
}

fn is_word(text: &str) -> bool {
    !text.is_empty()
        && text.starts_with(|c: char| c.is_ascii_lowercase())
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn split_choices(text: &str) -> Vec<String> {
    let choices: Vec<String> = text.split('|').map(|c| c.trim().to_string()).collect();
    if choices.iter().all(|c| is_word(c)) {
        choices
    } else {
        vec![text.to_string()]
    }
}

/// Splits at ` | ` outside angle brackets.
fn split_alternatives(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0_u32;
    let mut skip_space = false;
    for c in text.chars() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            '|' if depth == 0 && current.ends_with(' ') => {
                parts.push(current.trim().to_string());
                current.clear();
                skip_space = true;
                continue;
            }
            ' ' if skip_space => {
                skip_space = false;
                continue;
            }
            _ => {}
        }
        skip_space = false;
        current.push(c);
    }
    parts.push(current.trim().to_string());
    parts.retain(|p| !p.is_empty());
    parts
}

/// Splits at spaces outside angle brackets.
fn split_words(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0_u32;
    for c in text.chars() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ' ' if depth == 0 => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Writes the completion scripts of all shells into a directory and returns their paths.
///
/// # Errors
///
/// Returns an error when a script cannot be written.
pub fn write_completions(tree: &Tree, out: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for shell in SHELLS {
        let mut command = tree.command();
        let mut buffer = Vec::new();
        clap_complete::generate(shell, &mut command, PROGRAM_NAME, &mut buffer);
        let name = shell.file_name(PROGRAM_NAME);
        write_text(&out.join(&name), &String::from_utf8_lossy(&buffer))?;
        names.push(name);
    }
    Ok(names)
}

fn mangen_section(
    render: impl FnOnce(&clap_mangen::Man, &mut Vec<u8>) -> std::io::Result<()>,
    cmd: &Command,
    input: &str,
) -> Result<String> {
    let man = clap_mangen::Man::new(cmd.clone());
    let mut buffer = Vec::new();
    render(&man, &mut buffer).map_err(|e| DocgenError::invalid(input, e.to_string()))?;
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

/// Returns the manual page `nurt-<name>(1)` of a subcommand.
///
/// `extra` is Markdown whose level-2 headings become further sections of the page.
///
/// # Errors
///
/// Returns an error when `clap_mangen` fails to render a section.
pub fn subcommand_page(
    sub: &Subcommand,
    extra: Option<&str>,
    options: &ManOptions<'_>,
) -> Result<String> {
    let cmd = subcommand(sub, true);
    let name = format!("{PROGRAM_NAME}-{}", sub.name);
    let source = format!("Danaco Nurt {}", options.version);
    let header = ManHeader {
        name: &name,
        section: 1,
        date: options.date,
        source: &source,
        manual: MANUAL,
    };
    let mut out = title_line(&header);
    out.push_str(&mangen_section(
        |m, b| m.render_name_section(b),
        &cmd,
        &name,
    )?);
    out.push_str(&mangen_section(
        |m, b| m.render_synopsis_section(b),
        &cmd,
        &name,
    )?);
    out.push_str(&mangen_section(
        |m, b| m.render_description_section(b),
        &cmd,
        &name,
    )?);
    out.push_str(&mangen_section(
        |m, b| m.render_options_section(b),
        &cmd,
        &name,
    )?);
    if let Some(markdown) = extra {
        out.push_str(&render_body(&parse(markdown)));
    }
    out.push_str(".SH SEE ALSO\n\\fBnurt\\fR(1), \\fBnurt\\-commands\\fR(7)\n");
    Ok(out)
}

/// Returns the section `COMMAND LINE` of the page of a catalog command that
/// has the name of a subcommand.
///
/// # Errors
///
/// Returns an error when `clap_mangen` fails to render the synopsis.
pub fn command_line_section(sub: &Subcommand) -> Result<String> {
    let cmd = subcommand(sub, true);
    let name = format!("{PROGRAM_NAME}-{}", sub.name);
    let synopsis = mangen_section(|m, b| m.render_synopsis_section(b), &cmd, &name)?;
    Ok(synopsis.replacen(".SH SYNOPSIS", ".SH \"COMMAND LINE\"", 1))
}

/// Returns the Markdown of the reference page of `nurt serve` as sections of a manual page.
///
/// The title and the synopsis are left out: the manual page derives the
/// synopsis from the table of subcommands.
#[must_use]
pub fn reference_sections(markdown: &str) -> String {
    let (_, body) = crate::markdown::split_front_matter(markdown);
    let mut intro = Vec::new();
    let mut rest = Vec::new();
    let mut seen_section = false;
    let mut skipping = false;
    let mut in_code = false;
    for line in body.lines() {
        if line.starts_with("```") {
            in_code = !in_code;
        }
        let heading = !in_code && line.starts_with("## ");
        if heading {
            seen_section = true;
            skipping = line.trim() == "## Synopsis";
        }
        if line.starts_with("# ") && !in_code && !seen_section {
            continue;
        }
        if skipping {
            continue;
        }
        if seen_section {
            rest.push(line);
        } else {
            intro.push(line);
        }
    }
    let intro = intro.join("\n");
    let intro = intro.trim();
    let mut out = String::new();
    if !intro.is_empty() {
        out.push_str("## Description\n\n");
        out.push_str(intro);
        out.push_str("\n\n");
    }
    out.push_str(&rest.join("\n"));
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = r#"
format = "nurt-cli-subcommands/1"

[[subcommand]]
name = "exec"
description = "Run a program given as arguments."
arguments = ["[--variant a|b|c]", "[--preview]", "[--json | --format F]", "<program>"]

[[subcommand]]
name = "serve"
description = "Serve the request protocol."
arguments = ["--stdio | --socket <path>"]

[[subcommand]]
name = "service"
description = "Manage the background service."
arguments = ["install | uninstall | start"]

[[subcommand]]
name = "journal"
description = "Maintain the journal."
arguments = ["salvage <volume>"]

[[subcommand]]
name = "undo"
description = "Undo recorded operations."
arguments = ["[arguments of the undo command]"]

[[subcommand]]
name = "check"
description = "Check a program."
arguments = ["<file | -e program>"]
"#;

    const OPTIONS: ManOptions<'static> = ManOptions {
        version: "0.1.0",
        date: "2026-01-01",
    };

    fn tree() -> Tree {
        Tree::parse("table", TABLE).expect("fixture parses")
    }

    #[test]
    fn rejects_unknown_format_and_duplicates() {
        let wrong = TABLE.replace("/1", "/2");
        assert!(Tree::parse("table", &wrong).is_err());
        let twice = format!("{TABLE}\n[[subcommand]]\nname = \"exec\"\ndescription = \"x\"\n");
        assert!(Tree::parse("table", &twice).is_err());
    }

    #[test]
    fn command_tree_lists_every_subcommand() {
        let command = tree().command();
        let names: Vec<_> = command.get_subcommands().map(Command::get_name).collect();
        assert_eq!(
            names,
            ["exec", "serve", "service", "journal", "undo", "check"]
        );
    }

    #[test]
    fn summary_lowercases_the_first_letter_of_the_product_sentence() {
        assert_eq!(
            program_summary(),
            "command shell and language where every change can be undone"
        );
    }

    #[test]
    fn arguments_become_options_and_positionals() {
        let mut command = tree().command();
        command.build();
        let exec = command.find_subcommand("exec").expect("exec");
        assert!(
            exec.get_arguments()
                .any(|a| a.get_long() == Some("preview"))
        );
        assert!(exec.get_arguments().any(|a| a.get_long() == Some("format")));
        let variant = exec
            .get_arguments()
            .find(|a| a.get_long() == Some("variant"))
            .expect("variant");
        assert_eq!(variant.get_possible_values().len(), 3);
        assert!(exec.get_positionals().any(|a| a.is_required_set()));
    }

    #[test]
    fn completions_exist_for_every_shell_and_subcommand() {
        let out = tempfile::tempdir().expect("temporary directory");
        let tree = tree();
        let names = write_completions(&tree, out.path()).expect("completions are written");
        assert_eq!(names.len(), SHELLS.len());
        for name in &names {
            let script = std::fs::read_to_string(out.path().join(name)).expect("script");
            for sub in tree.subcommands() {
                assert!(script.contains(&sub.name), "{name} lacks {}", sub.name);
            }
        }
    }

    #[test]
    fn page_has_name_synopsis_and_options() {
        let tree = tree();
        let exec = &tree.subcommands()[0];
        let page = subcommand_page(exec, None, &OPTIONS).expect("page renders");
        assert!(page.starts_with(".TH \"NURT\\-EXEC\" \"1\""));
        assert!(page.contains("nurt\\-exec \\- Run a program given as arguments"));
        assert!(page.contains(".SH SYNOPSIS"));
        assert!(page.contains("\\-\\-variant"));
        assert!(page.contains(".SH SEE ALSO"));
    }

    #[test]
    fn command_line_section_replaces_the_synopsis_heading() {
        let tree = tree();
        let section = command_line_section(&tree.subcommands()[0]).expect("section renders");
        assert!(section.contains(".SH \"COMMAND LINE\""));
        assert!(!section.contains(".SH SYNOPSIS"));
    }

    #[test]
    fn reference_sections_drop_title_and_synopsis() {
        let text =
            "# Title\n\nIntro.\n\n## Synopsis\n\n```\nnurt serve\n```\n\n## Framing\n\nText.\n";
        let sections = reference_sections(text);
        assert!(sections.starts_with("## Description\n\nIntro."));
        assert!(sections.contains("## Framing"));
        assert!(!sections.contains("Synopsis"));
        assert!(!sections.contains("Title"));
    }
}
