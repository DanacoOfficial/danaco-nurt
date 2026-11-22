// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Text forms of command signatures, written in variant A of the syntax.
//!
//! Every signature shown by `help`, in the description layers and in the
//! reference is produced here from the catalog, so that names, positions,
//! defaults and enumeration values cannot drift from the catalog.

use nurt_contracts::kinds::{Category, EffectKind, Reversibility};
use nurt_contracts::spec::{IoSpec, ParamSpec, ParamType, StageSpec};

/// Separator of the alternatives of a value hint.
const ALTERNATIVE_SEPARATOR: &str = "|";

/// Marker of a required parameter.
const REQUIRED_MARK: char = '*';

/// Marker of a repeated positional parameter.
const REPEATED_MARK: char = '…';

/// Label of a positional parameter: the name in capitals, followed by `*`
/// when required and by `…` when the parameter collects a list.
#[must_use]
pub fn positional_label(param: &ParamSpec, required: bool) -> String {
    let mut label = param.name.to_ascii_uppercase();
    if param.repeated {
        label.push(REPEATED_MARK);
    }
    if required {
        label.push(REQUIRED_MARK);
    }
    label
}

/// The values a named parameter accepts: the default first, then the other
/// enumeration values, or the literal default when the type is not an
/// enumeration.
#[must_use]
pub fn value_hint(param: &ParamSpec) -> String {
    let default = param.default.as_ref().map(|value| value.as_str().trim());
    if param.values.is_empty() {
        return default.map(compact_literal).unwrap_or_default();
    }
    if matches!(param.ty, ParamType::List(_)) {
        return format!("[{}]", param.values.join(ALTERNATIVE_SEPARATOR));
    }
    let mut items: Vec<&str> = Vec::with_capacity(param.values.len() + 1);
    if let Some(default) = default {
        items.push(default);
    }
    items.extend(
        param
            .values
            .iter()
            .map(String::as_str)
            .filter(|value| Some(*value) != default),
    );
    items.join(ALTERNATIVE_SEPARATOR)
}

/// A list or map literal without the spaces after its separators, so that
/// it stays one word of a signature.
fn compact_literal(literal: &str) -> String {
    if literal.starts_with(['[', '{']) {
        literal.split_whitespace().collect()
    } else {
        literal.to_owned()
    }
}

/// Label of a named parameter: `name:value-hint`, followed by `*` when
/// required.
#[must_use]
pub fn named_label(param: &ParamSpec) -> String {
    let mut label = format!("{}:{}", param.name, value_hint(param));
    if param.required {
        label.push(REQUIRED_MARK);
    }
    label
}

/// Named parameters of a command in catalog order.
pub fn named_params(spec: &StageSpec) -> impl Iterator<Item = &ParamSpec> {
    spec.params.iter().filter(|param| !param.is_positional())
}

/// Compact signature: the command name, the positional parameters and at
/// most `named_limit` named parameters.
#[must_use]
pub fn compact(spec: &StageSpec, named_limit: usize) -> String {
    let mut parts = vec![spec.name.clone()];
    parts.extend(
        spec.positional()
            .into_iter()
            .map(|param| positional_label(param, param.required)),
    );
    parts.extend(named_params(spec).take(named_limit).map(named_label));
    parts.join(" ")
}

/// Usage line of one form of a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage {
    /// Name of the form, empty for a command without forms.
    pub form: String,
    /// Usage text: the stage prefix `… |> ` for a form that needs input, the
    /// command name and the positional parameters.
    pub text: String,
    /// One-sentence description of the form.
    pub summary: String,
}

/// Usage lines of a command, one per form.
#[must_use]
pub fn usages(spec: &StageSpec) -> Vec<Usage> {
    if spec.forms.is_empty() {
        let mut parts = vec![spec.name.clone()];
        parts.extend(
            spec.positional()
                .into_iter()
                .map(|param| positional_label(param, param.required)),
        );
        return vec![Usage {
            form: String::new(),
            text: parts.join(" "),
            summary: spec.summary.clone(),
        }];
    }
    spec.forms
        .iter()
        .map(|form| {
            let prefix = if form.input.iter().all(IoSpec::is_none) {
                ""
            } else {
                "… |> "
            };
            let mut parts = vec![spec.name.clone()];
            for name in &form.positional {
                if let Some(param) = spec.param(name) {
                    let required = form.required.contains(name);
                    parts.push(positional_label(param, required));
                }
            }
            Usage {
                form: form.name.clone(),
                text: format!("{prefix}{}", parts.join(" ")),
                summary: form.summary.clone(),
            }
        })
        .collect()
}

/// Record kinds of an input or output list, separated by ` | `.
#[must_use]
pub fn io_text(kinds: &[IoSpec]) -> String {
    kinds
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Name of a category as written in the catalog.
#[must_use]
pub const fn category_name(category: Category) -> &'static str {
    match category {
        Category::Source => "source",
        Category::Transform => "transform",
        Category::Check => "check",
        Category::Change => "change",
        Category::Run => "run",
        Category::System => "system",
        Category::Journal => "journal",
        Category::Help => "help",
    }
}

/// One-sentence statement of the effect and the reversibility of a command.
#[must_use]
pub fn effect_text(spec: &StageSpec) -> String {
    let effect = match spec.effect {
        EffectKind::Read => "reads only",
        EffectKind::Local => "changes local files",
        EffectKind::External => "acts outside the local file system",
        EffectKind::Process => "runs a program",
    };
    let reversibility = match spec.reversibility {
        Reversibility::Read => "",
        Reversibility::Reversible => "; reversible",
        Reversibility::ReversibleIfPreserved => "; reversible when the content is preserved",
        Reversibility::Irreversible => "; irreversible",
    };
    format!("{effect}{reversibility}")
}

/// Joins items into lines of at most `width` characters, each line starting
/// with `indent`.
#[must_use]
pub fn wrap(items: &[String], indent: &str, width: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::from(indent);
    let mut empty = true;
    for item in items {
        if !empty && line.chars().count() + 1 + item.chars().count() > width {
            lines.push(std::mem::replace(&mut line, String::from(indent)));
            empty = true;
        }
        if !empty {
            line.push(' ');
        }
        line.push_str(item);
        empty = false;
    }
    if !empty {
        lines.push(line);
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::spec::CatalogFile;

    const FILE: &str = r#"
format = "nurt-catalog/1"
group = "core"
summary = "Core commands."

[[command]]
name = "list"
category = "source"
reversibility = "read"
effect = "read"
implementation = "builtin"
input = ["none"]
output = ["entry"]
summary = "List the entries of a directory."

[[command.param]]
name = "path"
type = "path"
position = 1
required = true
summary = "Directory."

[[command.param]]
name = "type"
type = "enum(file|directory|any)"
default = "any"
values = ["file", "directory", "any"]
summary = "Kind of entry."

[[command.param]]
name = "hidden"
type = "bool"
default = "true"
summary = "Include hidden entries."

[[command.param]]
name = "depth"
type = "int"
summary = "Depth."
"#;

    fn list() -> StageSpec {
        let file: CatalogFile = toml::from_str(FILE).unwrap();
        file.commands.into_iter().next().unwrap()
    }

    #[test]
    fn required_positional_parameter_is_capitalised_and_marked() {
        let spec = list();
        assert_eq!(positional_label(&spec.params[0], true), "PATH*");
        assert_eq!(positional_label(&spec.params[0], false), "PATH");
    }

    #[test]
    fn enumeration_hint_lists_the_default_first() {
        let spec = list();
        assert_eq!(value_hint(&spec.params[1]), "any|file|directory");
    }

    #[test]
    fn literal_default_and_missing_default_are_shown() {
        let spec = list();
        assert_eq!(named_label(&spec.params[2]), "hidden:true");
        assert_eq!(named_label(&spec.params[3]), "depth:");
    }

    #[test]
    fn compact_signature_limits_the_named_parameters() {
        let spec = list();
        assert_eq!(compact(&spec, 0), "list PATH*");
        assert_eq!(
            compact(&spec, 2),
            "list PATH* type:any|file|directory hidden:true"
        );
    }

    #[test]
    fn command_without_forms_has_one_usage() {
        let usages = usages(&list());
        assert_eq!(usages.len(), 1);
        assert_eq!(usages[0].text, "list PATH*");
    }

    #[test]
    fn wrap_breaks_lines_at_the_width() {
        let items: Vec<String> = ["aaaa", "bbbb", "cccc"].map(str::to_owned).to_vec();
        assert_eq!(wrap(&items, "  ", 11), "  aaaa bbbb\n  cccc");
    }

    #[test]
    fn effect_text_states_reversibility() {
        assert_eq!(effect_text(&list()), "reads only");
    }
}
