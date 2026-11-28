// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-langserver::catalog
// Responsibility:   Command signatures of the catalog and their descriptions.
// Dependencies:     nurt_contracts, nurt_syntax, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Command signatures of the catalog and their descriptions.

use std::fmt::Write as _;
use std::ops::Range;

use nurt_contracts::spec::{ParamSpec, StageSpec};
use nurt_syntax::parse::Variant;
use serde_json::Value;

/// Command signatures ordered by name.
#[derive(Debug, Clone, Default)]
pub(crate) struct Commands {
    specs: Vec<StageSpec>,
}

impl Commands {
    /// Orders `specs` by name; of two commands with one name the first is
    /// kept.
    pub(crate) fn new(mut specs: Vec<StageSpec>) -> Self {
        specs.sort_by(|left, right| left.name.cmp(&right.name));
        specs.dedup_by(|later, earlier| later.name == earlier.name);
        Self { specs }
    }

    /// Returns the signature of command `name`.
    pub(crate) fn get(&self, name: &str) -> Option<&StageSpec> {
        let index = self
            .specs
            .binary_search_by(|spec| spec.name.as_str().cmp(name))
            .ok()?;
        self.specs.get(index)
    }

    /// Returns the signatures in name order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &StageSpec> {
        self.specs.iter()
    }
}

/// Returns the text between a parameter label and its value.
pub(crate) fn label_separator(variant: Variant) -> &'static str {
    match variant {
        Variant::C => "=",
        Variant::A | Variant::B => ": ",
    }
}

/// Returns the values of an enumeration parameter, or of a list of
/// enumeration values.
pub(crate) fn enum_values(param: &ParamSpec) -> Vec<&str> {
    if param.values.is_empty() {
        param.ty.enum_values()
    } else {
        param.values.iter().map(String::as_str).collect()
    }
}

/// Returns the type and the default of a parameter, for example
/// `int, default 1`.
pub(crate) fn param_detail(param: &ParamSpec) -> String {
    let mut detail = param.ty.to_string();
    if param.required {
        detail.push_str(", required");
    } else if let Some(default) = &param.default {
        let _ = write!(detail, ", default {default}");
    }
    detail
}

/// Returns the call form of a command with its positional and required
/// parameters in signature order.
pub(crate) fn signature(spec: &StageSpec, variant: Variant) -> String {
    let separator = label_separator(variant);
    let mut parts: Vec<String> = spec
        .positional()
        .into_iter()
        .map(|param| {
            let part = if variant == Variant::C {
                format!("{}{separator}<{}>", param.name, param.ty)
            } else {
                format!("<{}>", param.name)
            };
            if param.required {
                part
            } else {
                format!("[{part}]")
            }
        })
        .collect();
    parts.extend(
        spec.params
            .iter()
            .filter(|param| param.required && !param.is_positional())
            .map(|param| format!("{}{separator}<{}>", param.name, param.ty)),
    );
    match variant {
        Variant::B => format!("{}({})", spec.name, parts.join(", ")),
        Variant::A | Variant::C if parts.is_empty() => spec.name.clone(),
        Variant::A | Variant::C => format!("{} {}", spec.name, parts.join(" ")),
    }
}

/// Call form of a command with every parameter and the byte range of each
/// parameter in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FullSignature<'a> {
    /// The call form, for example `sort <by> [order: <enum(asc|desc)>]`.
    pub(crate) label: String,
    /// Parameters in the order of the form: positional ones by position,
    /// then the others in catalog order.
    pub(crate) params: Vec<(Range<usize>, &'a ParamSpec)>,
}

/// Returns the call form of a command with every parameter; optional
/// parameters are in brackets.
pub(crate) fn full_signature(spec: &StageSpec, variant: Variant) -> FullSignature<'_> {
    let separator = label_separator(variant);
    let ordered = spec
        .positional()
        .into_iter()
        .chain(spec.params.iter().filter(|param| !param.is_positional()));
    let (open, joiner) = match variant {
        Variant::B => ("(", ", "),
        Variant::A | Variant::C => (" ", " "),
    };
    let mut label = spec.name.clone();
    let mut params = Vec::new();
    for (index, param) in ordered.enumerate() {
        label.push_str(if index == 0 { open } else { joiner });
        let start = label.len();
        let body = if param.is_positional() && variant != Variant::C {
            format!("<{}>", param.name)
        } else {
            format!("{}{separator}<{}>", param.name, param.ty)
        };
        if param.required {
            label.push_str(&body);
        } else {
            let _ = write!(label, "[{body}]");
        }
        params.push((start..label.len(), param));
    }
    if variant == Variant::B {
        if params.is_empty() {
            label.push('(');
        }
        label.push(')');
    }
    FullSignature { label, params }
}

/// Describes a command in Markdown: call form, summary, classification and
/// parameters.
pub(crate) fn command_markdown(spec: &StageSpec, variant: Variant) -> String {
    let mut text = format!(
        "```nurt\n{}\n```\n\n{}\n\n",
        signature(spec, variant),
        spec.summary
    );
    let _ = writeln!(
        text,
        "Group `{}`, category `{}`, effect `{}`.",
        spec.group,
        tag(serde_json::to_value(spec.category)),
        tag(serde_json::to_value(spec.effect))
    );
    if !spec.params.is_empty() {
        text.push_str("\nParameters:\n\n");
        for param in &spec.params {
            let _ = writeln!(
                text,
                "- `{}` ({}): {}",
                param.name,
                param_detail(param),
                param.summary
            );
        }
    }
    text
}

/// Describes a parameter of a command in Markdown.
pub(crate) fn param_markdown(spec: &StageSpec, param: &ParamSpec, variant: Variant) -> String {
    let mut text = format!(
        "```nurt\n{} {}{}<{}>\n```\n\n{}\n\nParameter of `{}`: {}.",
        spec.name,
        param.name,
        label_separator(variant),
        param.ty,
        param.summary,
        spec.name,
        param_detail(param)
    );
    let values = enum_values(param);
    if !values.is_empty() {
        let listed: Vec<String> = values.iter().map(|value| format!("`{value}`")).collect();
        let _ = write!(text, " Values: {}.", listed.join(", "));
    }
    text.push('\n');
    text
}

/// Returns the serialized name of a classification, for example `transform`.
pub(crate) fn tag(serialized: serde_json::Result<Value>) -> String {
    match serialized {
        Ok(Value::String(name)) => name,
        _ => String::new(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use nurt_contracts::spec::CatalogFile;

    use super::*;

    const SAMPLE: &str = r#"
format = "nurt-catalog/1"
group = "core"
summary = "Core commands."

[[command]]
name = "find"
category = "source"
reversibility = "read"
effect = "read"
implementation = "builtin"
input = ["none"]
output = ["entry"]
summary = "Find entries under a directory."

[[command.param]]
name = "path"
type = "path"
position = 1
required = true
summary = "Directory to search."

[[command]]
name = "sort"
category = "transform"
reversibility = "read"
effect = "read"
implementation = "builtin"
input = ["any"]
output = ["same"]
summary = "Sort the records."

[[command.param]]
name = "by"
type = "list<field>"
position = 1
required = true
summary = "Fields to sort by."

[[command.param]]
name = "order"
type = "enum(asc|desc)"
default = "asc"
values = ["asc", "desc"]
summary = "Sort order."

[[command]]
name = "count"
category = "transform"
reversibility = "read"
effect = "read"
implementation = "builtin"
input = ["any"]
output = ["int"]
summary = "Count the input records."
"#;

    /// Commands `count`, `find` and `sort` of a small catalog.
    pub(crate) fn sample() -> Commands {
        let mut file: CatalogFile = toml::from_str(SAMPLE).unwrap();
        file.resolve_groups();
        Commands::new(file.commands)
    }

    #[test]
    fn orders_commands_by_name_and_finds_them() {
        let commands = sample();
        let names: Vec<&str> = commands.iter().map(|spec| spec.name.as_str()).collect();
        assert_eq!(names, ["count", "find", "sort"]);
        assert_eq!(
            commands.get("sort").map(|spec| spec.group.as_str()),
            Some("core")
        );
        assert!(commands.get("remove").is_none());
    }

    #[test]
    fn keeps_the_first_of_two_commands_with_one_name() {
        let commands = sample();
        let mut specs: Vec<StageSpec> = commands.iter().cloned().collect();
        let mut copy = specs[0].clone();
        copy.summary = "Other.".to_owned();
        specs.push(copy);
        let commands = Commands::new(specs);
        assert_eq!(commands.iter().count(), 3);
        assert_eq!(
            commands.get("count").map(|spec| spec.summary.as_str()),
            Some("Count the input records.")
        );
    }

    #[test]
    fn renders_signatures_of_each_variant() {
        let commands = sample();
        let sort = commands.get("sort").unwrap();
        assert_eq!(signature(sort, Variant::A), "sort <by>");
        assert_eq!(signature(sort, Variant::B), "sort(<by>)");
        assert_eq!(signature(sort, Variant::C), "sort by=<list<field>>");
        assert_eq!(
            signature(commands.get("count").unwrap(), Variant::A),
            "count"
        );
    }

    #[test]
    fn renders_full_signatures_with_parameter_ranges() {
        let commands = sample();
        let sort = commands.get("sort").unwrap();
        let full = full_signature(sort, Variant::A);
        assert_eq!(full.label, "sort <by> [order: <enum(asc|desc)>]");
        let parts: Vec<&str> = full
            .params
            .iter()
            .map(|(range, _)| &full.label[range.clone()])
            .collect();
        assert_eq!(parts, ["<by>", "[order: <enum(asc|desc)>]"]);
        assert_eq!(
            full_signature(sort, Variant::B).label,
            "sort(<by>, [order: <enum(asc|desc)>])"
        );
        assert_eq!(
            full_signature(sort, Variant::C).label,
            "sort by=<list<field>> [order=<enum(asc|desc)>]"
        );
        let count = commands.get("count").unwrap();
        assert_eq!(full_signature(count, Variant::B).label, "count()");
        assert_eq!(full_signature(count, Variant::A).label, "count");
    }

    #[test]
    fn describes_commands_and_parameters() {
        let commands = sample();
        let sort = commands.get("sort").unwrap();
        let command = command_markdown(sort, Variant::A);
        assert!(command.starts_with("```nurt\nsort <by>\n```\n\nSort the records.\n"));
        assert!(command.contains("Group `core`, category `transform`, effect `read`."));
        assert!(command.contains("- `order` (enum(asc|desc), default asc): Sort order.\n"));
        let order = sort.param("order").unwrap();
        let param = param_markdown(sort, order, Variant::C);
        assert!(param.starts_with("```nurt\nsort order=<enum(asc|desc)>\n```\n"));
        assert!(param.ends_with(
            "Parameter of `sort`: enum(asc|desc), default asc. Values: `asc`, `desc`.\n"
        ));
        assert_eq!(
            param_detail(sort.param("by").unwrap()),
            "list<field>, required"
        );
        assert_eq!(enum_values(order), ["asc", "desc"]);
        assert_eq!(tag(Ok(Value::Null)), "");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
