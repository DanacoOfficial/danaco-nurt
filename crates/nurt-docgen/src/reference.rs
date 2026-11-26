// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Sections of a command page that are generated from the catalog.

use std::collections::HashMap;

use nurt_contracts::spec::{FormSpec, IoSpec, ParamSpec, StageSpec};

use crate::catalog::keyword;
use crate::markdown::split_cells;

fn escape_cell(text: &str) -> String {
    text.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn first_alternative(ty: &str) -> &str {
    ty.split(" | ").next().unwrap_or(ty)
}

fn positional_token(param: &ParamSpec, required: bool) -> String {
    let name = &param.name;
    if param.repeated {
        format!("<{name}>...")
    } else if required {
        format!("<{name}>")
    } else {
        format!("[<{name}>]")
    }
}

fn named_token(param: &ParamSpec) -> String {
    format!(
        "{}: <{}>",
        param.name,
        first_alternative(&param.ty.to_string())
    )
}

fn group_token(spec: &StageSpec, group: &str) -> String {
    let members: Vec<String> = spec
        .params
        .iter()
        .filter(|param| param.exclusive_group.as_deref() == Some(group))
        .map(named_token)
        .collect();
    format!("({})", members.join(" | "))
}

fn kinds(kinds: &[IoSpec]) -> String {
    kinds
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("|")
}

struct LineSpec<'a> {
    input: &'a [IoSpec],
    positional: Vec<&'a ParamSpec>,
    required: Vec<&'a ParamSpec>,
    groups: Vec<&'a str>,
    options: bool,
}

fn line_of(spec: &StageSpec, line: &LineSpec<'_>) -> String {
    let mut tokens: Vec<String> = Vec::new();
    if !line.input.iter().any(|kind| kind.to_string() == "none") && !line.input.is_empty() {
        tokens.push(format!("{} |>", kinds(line.input)));
    }
    tokens.push(spec.name.clone());
    for param in &line.positional {
        let required = line.required.iter().any(|r| r.name == param.name);
        tokens.push(positional_token(param, required));
    }
    for param in &line.required {
        if !line.positional.iter().any(|p| p.name == param.name) {
            tokens.push(named_token(param));
        }
    }
    for group in &line.groups {
        tokens.push(group_token(spec, group));
    }
    if line.options {
        tokens.push("[name: value]...".to_string());
    }
    tokens.join(" ")
}

fn form_line(spec: &StageSpec, form: &FormSpec) -> String {
    let accepted: Vec<&ParamSpec> = spec
        .params
        .iter()
        .filter(|param| form.params.is_empty() || form.params.contains(&param.name))
        .collect();
    let positional: Vec<&ParamSpec> = form
        .positional
        .iter()
        .filter_map(|name| spec.param(name))
        .collect();
    let groups: Vec<&str> = form
        .required
        .iter()
        .filter(|name| spec.param(name).is_none())
        .map(String::as_str)
        .collect();
    let required: Vec<&ParamSpec> = form
        .required
        .iter()
        .filter_map(|name| spec.param(name))
        .collect();
    let options = accepted.iter().any(|param| {
        !positional.iter().any(|p| p.name == param.name)
            && !required.iter().any(|p| p.name == param.name)
    });
    line_of(
        spec,
        &LineSpec {
            input: &form.input,
            positional,
            required,
            groups,
            options,
        },
    )
}

/// Returns the synopsis of a command: one line per form in variant A.
///
/// Required positional parameters are written `<name>`, optional ones
/// `[<name>]`, repeated ones `<name>...`; required named parameters are
/// written `name: <type>`; the remaining named parameters are represented by
/// `[name: value]...`. A form that takes input starts with the input kinds
/// and `|>`.
#[must_use]
pub fn synopsis(spec: &StageSpec) -> String {
    if !spec.forms.is_empty() {
        return spec
            .forms
            .iter()
            .map(|form| form_line(spec, form))
            .collect::<Vec<_>>()
            .join("\n");
    }
    let positional = spec.positional();
    let required: Vec<&ParamSpec> = spec.params.iter().filter(|param| param.required).collect();
    let groups: Vec<&str> = spec.required_groups.iter().map(String::as_str).collect();
    let options = spec
        .params
        .iter()
        .any(|param| !param.is_positional() && !param.required && param.exclusive_group.is_none());
    line_of(
        spec,
        &LineSpec {
            input: &spec.input,
            positional,
            required,
            groups,
            options,
        },
    )
}

/// Returns the default column text of a parameter.
#[must_use]
pub fn default_text(spec: &StageSpec, param: &ParamSpec) -> String {
    if param.required {
        return "required".to_string();
    }
    if let Some(group) = &param.exclusive_group
        && spec.required_groups.contains(group)
    {
        return format!("one of `{group}`");
    }
    if let Some(default) = &param.default {
        return format!("`{}`", escape_cell(default.as_str()));
    }
    if param.unbounded_default {
        return "no limit".to_string();
    }
    if let Some(note) = &param.default_note {
        return escape_cell(note);
    }
    "none".to_string()
}

/// Returns the parameters in table order: required first, then the others in
/// the order of the catalog.
#[must_use]
pub fn ordered_params(spec: &StageSpec) -> Vec<&ParamSpec> {
    let mut ordered: Vec<&ParamSpec> = spec.params.iter().filter(|p| p.required).collect();
    ordered.extend(spec.params.iter().filter(|p| !p.required));
    ordered
}

/// Returns the rows `[name, type, default, description]` of the parameter table.
///
/// `descriptions` replaces the catalog summary of a parameter by the longer
/// description written on the page.
#[must_use]
pub fn parameter_rows(
    spec: &StageSpec,
    descriptions: &HashMap<String, String>,
) -> Vec<[String; 4]> {
    ordered_params(spec)
        .into_iter()
        .map(|param| {
            let description = descriptions
                .get(&param.name)
                .cloned()
                .unwrap_or_else(|| escape_cell(&param.summary));
            [
                format!("`{}`", param.name),
                format!("`{}`", escape_cell(&param.ty.to_string())),
                default_text(spec, param),
                description,
            ]
        })
        .collect()
}

/// Returns the parameter table in Markdown.
#[must_use]
pub fn parameters_table(spec: &StageSpec, descriptions: &HashMap<String, String>) -> String {
    if spec.params.is_empty() {
        return "The command has no parameters.\n".to_string();
    }
    let mut out = String::from("| Parameter | Type | Default | Description |\n|---|---|---|---|\n");
    for row in parameter_rows(spec, descriptions) {
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            row[0], row[1], row[2], row[3]
        ));
    }
    out
}

/// Reads the descriptions written in a parameter table of a page, by parameter name.
#[must_use]
pub fn written_descriptions(body: &str) -> HashMap<String, String> {
    let mut found = HashMap::new();
    for line in body
        .lines()
        .filter(|line| line.trim_start().starts_with('|'))
    {
        let cells = split_cells(line);
        if cells.len() < 4 {
            continue;
        }
        let name = cells[0].trim_matches('`');
        if name == "Parameter" || name.starts_with("---") || name.is_empty() {
            continue;
        }
        if !cells[3].is_empty() {
            found.insert(name.to_string(), escape_cell(&cells[3]));
        }
    }
    found
}

/// Returns the table of accepted and produced record kinds.
#[must_use]
pub fn io_section(spec: &StageSpec) -> String {
    let mut out = String::from("| Form | Input | Output |\n|---|---|---|\n");
    let list = |items: &[IoSpec]| {
        items
            .iter()
            .map(|kind| format!("`{kind}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if spec.forms.is_empty() {
        out.push_str(&format!(
            "| default | {} | {} |\n",
            list(&spec.input),
            list(&spec.output)
        ));
    } else {
        for form in &spec.forms {
            let output = if form.output.is_empty() {
                &spec.output
            } else {
                &form.output
            };
            out.push_str(&format!(
                "| `{}` | {} | {} |\n",
                form.name,
                list(&form.input),
                list(output)
            ));
        }
    }
    out
}

/// Returns the effect description: category, effect, reversibility, external
/// programs and actions with their own effect.
#[must_use]
pub fn effect_section(spec: &StageSpec) -> String {
    let mut out = String::from("| Property | Value |\n|---|---|\n");
    let mut row = |name: &str, value: String| {
        out.push_str(&format!("| {name} | {value} |\n"));
    };
    row("Category", format!("`{}`", keyword(&spec.category)));
    row("Effect", format!("`{}`", keyword(&spec.effect)));
    row(
        "Reversibility",
        format!("`{}`", keyword(&spec.reversibility)),
    );
    row(
        "Implementation",
        format!("`{}`", keyword(&spec.implementation)),
    );
    if let Some(note) = &spec.reversibility_note {
        row("Reversibility conditions", escape_cell(note));
    }
    if spec.waits {
        row("Waits", "always blocks the call".to_string());
    }
    if !spec.os.is_empty() {
        let systems: Vec<String> = spec
            .os
            .iter()
            .map(|os| format!("`{}`", keyword(os)))
            .collect();
        row("Operating systems", systems.join(", "));
    }
    if let Some(fallback) = &spec.fallback {
        row("Fallback", format!("`{}`", keyword(fallback)));
    }
    if let Some(external) = &spec.external {
        let mut text = format!("`{}`", keyword(&external.kind));
        if !external.programs.is_empty() {
            let programs: Vec<String> =
                external.programs.iter().map(|p| format!("`{p}`")).collect();
            text.push_str(&format!(": {}", programs.join(", ")));
        }
        if let Some(when) = &external.when {
            text.push_str(&format!("; used when `{when}` is given"));
        }
        if let Some(note) = &external.note {
            text.push_str(&format!("; {}", escape_cell(note)));
        }
        row("External use", text);
    }
    if !spec.effects.is_empty() {
        out.push_str("\n| Action | Effect | Reversibility | Note |\n|---|---|---|---|\n");
        for action in &spec.effects {
            out.push_str(&format!(
                "| `{}` | `{}` | `{}` | {} |\n",
                action.action,
                keyword(&action.effect),
                keyword(&action.reversibility),
                action.note.as_deref().map(escape_cell).unwrap_or_default()
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::fixtures::catalog;

    #[test]
    fn synopsis_lists_positional_and_options() {
        let catalog = catalog();
        let find = catalog.command("find").expect("fixture command");
        assert_eq!(synopsis(find), "find <path> [<glob>] [name: value]...");
    }

    #[test]
    fn synopsis_marks_stage_commands_with_input() {
        let catalog = catalog();
        let filter = catalog.command("filter").expect("fixture command");
        assert_eq!(synopsis(filter), "any |> filter <condition>");
    }

    #[test]
    fn synopsis_of_command_without_parameters_is_its_name() {
        let catalog = catalog();
        let count = catalog.command("count").expect("fixture command");
        assert_eq!(synopsis(count), "any |> count");
    }

    #[test]
    fn parameter_table_puts_required_first_and_escapes_pipes() {
        let catalog = catalog();
        let find = catalog.command("find").expect("fixture command");
        let table = parameters_table(find, &HashMap::new());
        let rows: Vec<&str> = table.lines().skip(2).collect();
        assert!(rows[0].starts_with("| `path` | `path` | required |"));
        assert!(rows[1].contains("`glob \\| list<glob>`"));
        assert!(rows[1].contains("`\"**\"`"));
        assert!(rows[3].contains("| none |"));
    }

    #[test]
    fn escape_cell_keeps_a_cell_on_one_row() {
        assert_eq!(escape_cell("a|b\r\nc"), "a\\|b  c");
        assert_eq!(
            escape_cell("za\u{17c}\u{f3}\u{142}\u{107}"),
            "za\u{17c}\u{f3}\u{142}\u{107}"
        );
    }

    #[test]
    fn written_descriptions_override_summaries() {
        let catalog = catalog();
        let find = catalog.command("find").expect("fixture command");
        let page = "| Parameter | Type | Default | Description |\n|---|---|---|---|\n| `depth` | `int` | none | Longer text. |\n";
        let descriptions = written_descriptions(page);
        let table = parameters_table(find, &descriptions);
        assert!(table.contains("| Longer text. |"));
        assert!(table.contains("Directory to search."));
    }

    #[test]
    fn io_section_lists_kinds() {
        let catalog = catalog();
        let remove = catalog.command("remove").expect("fixture command");
        assert!(io_section(remove).contains("| default | `none`, `entry` | `effect` |"));
    }

    #[test]
    fn effect_section_lists_keywords() {
        let catalog = catalog();
        let remove = catalog.command("remove").expect("fixture command");
        let text = effect_section(remove);
        assert!(text.contains("| Category | `change` |"));
        assert!(text.contains("| Reversibility | `reversible` |"));
    }
}
