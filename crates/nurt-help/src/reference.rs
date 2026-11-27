// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-help::reference
// Responsibility:   Reference documentation of the commands in Markdown, man and HTML.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Reference documentation of the commands in Markdown, man and HTML.
//!
//! All three formats are generated from the catalog and from the command
//! pages, so they list the same commands, parameters and examples.

use nurt_contracts::error::Error;
use nurt_contracts::spec::StageSpec;

use crate::catalog::Catalog;
use crate::describe::{self, default_cell};
use crate::lookup;
use crate::signature;
use crate::text::TAGLINE;

/// Format of the reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReferenceFormat {
    /// Markdown with one section per command.
    Markdown,
    /// Manual page in the roff `man` macros.
    Man,
    /// A standalone HTML document.
    Html,
}

impl ReferenceFormat {
    /// Every format.
    pub const ALL: [ReferenceFormat; 3] = [Self::Markdown, Self::Man, Self::Html];

    /// The name of the format as written in `--format`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Man => "man",
            Self::Html => "html",
        }
    }

    /// The format with the given name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|format| format.name() == name)
    }
}

/// Section number of the manual page.
const MAN_SECTION: &str = "7";

/// Neutral content of the reference of one command.
struct Entry<'a> {
    spec: &'a StageSpec,
    usages: Vec<String>,
    facts: String,
    examples: Vec<&'a str>,
    errors: Vec<(&'a str, &'a str)>,
}

impl<'a> Entry<'a> {
    fn new(catalog: &'a Catalog, spec: &'a StageSpec) -> Self {
        let page = catalog.docs().page(&spec.name);
        Self {
            spec,
            usages: signature::usages(spec)
                .into_iter()
                .map(|usage| usage.text)
                .collect(),
            facts: format!(
                "Group: {}. Input: {}. Output: {}. Effect: {}.",
                spec.group,
                signature::io_text(&spec.input),
                signature::io_text(&spec.output),
                signature::effect_text(spec),
            ),
            examples: page
                .map(|page| page.examples.iter().map(|e| e.program.as_str()).collect())
                .unwrap_or_default(),
            errors: page
                .map(|page| {
                    page.errors
                        .iter()
                        .map(|note| (note.code.as_str(), note.meaning.as_str()))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

/// Generates the reference of one command, or of all commands for an empty
/// name, in `format`.
///
/// # Errors
///
/// Fails with `E_UNKNOWN_NAME` and the nearest command names in `expected`
/// when the command does not exist.
pub fn render(catalog: &Catalog, name: &str, format: ReferenceFormat) -> Result<String, Error> {
    if format == ReferenceFormat::Markdown {
        return describe::reference_text(catalog, name);
    }
    let specs: Vec<&StageSpec> = if name.is_empty() {
        catalog.commands().collect()
    } else {
        match catalog.command(name) {
            Some(spec) => vec![spec],
            None => {
                let names: Vec<String> = catalog.commands().map(|c| c.name.clone()).collect();
                return Err(lookup::unknown("command", name, &names));
            }
        }
    };
    let entries: Vec<Entry<'_>> = specs
        .into_iter()
        .map(|spec| Entry::new(catalog, spec))
        .collect();
    Ok(match format {
        ReferenceFormat::Man => man(name, &entries),
        _ => html(name, &entries),
    })
}

fn lowered_tagline() -> String {
    let mut chars = TAGLINE.chars();
    chars
        .next()
        .map(|first| first.to_lowercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Escapes a line of roff text: backslashes, hyphens and a leading control
/// character.
fn roff(text: &str) -> String {
    let escaped = text.replace('\\', "\\e").replace('-', "\\-");
    if escaped.starts_with('.') || escaped.starts_with('\'') {
        format!("\\&{escaped}")
    } else {
        escaped
    }
}

fn roff_block(text: &str) -> Vec<String> {
    text.lines().map(roff).collect()
}

fn man(name: &str, entries: &[Entry<'_>]) -> String {
    let title = if name.is_empty() {
        "NURT-REFERENCE".to_owned()
    } else {
        format!("NURT-{}", name.to_uppercase().replace('.', "-"))
    };
    let mut lines = vec![
        format!(".TH {title} {MAN_SECTION}"),
        ".SH NAME".to_owned(),
        roff(&format!("nurt - {}", lowered_tagline())),
        ".SH COMMANDS".to_owned(),
    ];
    for entry in entries {
        lines.push(format!(".SS {}", roff(&entry.spec.name)));
        lines.push(roff(&entry.spec.summary));
        lines.push(".PP".to_owned());
        lines.push(".nf".to_owned());
        lines.extend(entry.usages.iter().map(|usage| roff(usage)));
        lines.push(".fi".to_owned());
        lines.push(".PP".to_owned());
        lines.push(roff(&entry.facts));
        for param in &entry.spec.params {
            lines.push(".TP".to_owned());
            lines.push(format!(
                ".B {}",
                roff(&format!("{} ({})", param.name, param.ty))
            ));
            lines.push(roff(&format!(
                "{} Default: {}.",
                param.summary,
                default_cell(param)
            )));
        }
        if !entry.examples.is_empty() {
            lines.push(".PP".to_owned());
            lines.push("Examples:".to_owned());
            for example in &entry.examples {
                lines.push(".PP".to_owned());
                lines.push(".nf".to_owned());
                lines.extend(roff_block(example));
                lines.push(".fi".to_owned());
            }
        }
        if !entry.errors.is_empty() {
            lines.push(".PP".to_owned());
            lines.push("Errors:".to_owned());
            for (code, meaning) in &entry.errors {
                lines.push(".TP".to_owned());
                lines.push(format!(".B {}", roff(code)));
                lines.push(roff(meaning));
            }
        }
    }
    lines.join("\n")
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn html_pre(lines: &[String]) -> String {
    format!("<pre>{}</pre>", escape_html(&lines.join("\n")))
}

fn html(name: &str, entries: &[Entry<'_>]) -> String {
    let title = if name.is_empty() {
        "Command reference".to_owned()
    } else {
        format!("Reference of {name}")
    };
    let mut lines = vec![
        "<!DOCTYPE html>".to_owned(),
        "<html lang=\"en\">".to_owned(),
        "<head>".to_owned(),
        "<meta charset=\"utf-8\">".to_owned(),
        format!("<title>{}</title>", escape_html(&title)),
        "</head>".to_owned(),
        "<body>".to_owned(),
        format!("<h1>{}</h1>", escape_html(&title)),
    ];
    for entry in entries {
        let command = escape_html(&entry.spec.name);
        lines.push(format!("<section id=\"{command}\">"));
        lines.push(format!("<h2>{command}</h2>"));
        lines.push(format!("<p>{}</p>", escape_html(&entry.spec.summary)));
        lines.push(html_pre(&entry.usages));
        lines.push(format!("<p>{}</p>", escape_html(&entry.facts)));
        if !entry.spec.params.is_empty() {
            lines.push("<table>".to_owned());
            lines.push(
                "<tr><th>Parameter</th><th>Type</th><th>Default</th><th>Description</th></tr>"
                    .to_owned(),
            );
            for param in &entry.spec.params {
                lines.push(format!(
                    "<tr><td><code>{}</code></td><td><code>{}</code></td><td>{}</td><td>{}</td></tr>",
                    escape_html(&param.name),
                    escape_html(&param.ty.to_string()),
                    escape_html(&default_cell(param)),
                    escape_html(&param.summary),
                ));
            }
            lines.push("</table>".to_owned());
        }
        if !entry.examples.is_empty() {
            lines.push("<h3>Examples</h3>".to_owned());
            for example in &entry.examples {
                lines.push(html_pre(&[(*example).to_owned()]));
            }
        }
        if !entry.errors.is_empty() {
            lines.push("<h3>Errors</h3>".to_owned());
            lines.push("<ul>".to_owned());
            for (code, meaning) in &entry.errors {
                lines.push(format!(
                    "<li><code>{}</code>: {}</li>",
                    escape_html(code),
                    escape_html(meaning)
                ));
            }
            lines.push("</ul>".to_owned());
        }
        lines.push("</section>".to_owned());
    }
    lines.push("</body>".to_owned());
    lines.push("</html>".to_owned());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roff_escapes_backslashes_hyphens_and_leading_dots() {
        assert_eq!(roff("a-b\\c"), "a\\-b\\ec");
        assert_eq!(roff(".x"), "\\&.x");
        assert_eq!(roff("'x"), "\\&'x");
    }

    #[test]
    fn html_escapes_markup() {
        assert_eq!(escape_html("a<b>&\"c\""), "a&lt;b&gt;&amp;&quot;c&quot;");
    }

    #[test]
    fn format_names_round_trip() {
        for format in ReferenceFormat::ALL {
            assert_eq!(ReferenceFormat::from_name(format.name()), Some(format));
        }
        assert_eq!(ReferenceFormat::from_name("pdf"), None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
