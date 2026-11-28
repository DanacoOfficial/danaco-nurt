// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-langserver::fields
// Responsibility:   Fields of the records that flow into a stage.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Fields of the records that flow into a stage.
//!
//! Field names and their descriptions come from the schema of typed
//! records, so completion offers exactly the fields of the contract. The
//! records that flow into a stage are those of the nearest stage before it
//! whose output names a record kind; a stage whose output is `same` passes
//! its input on.

use std::collections::HashMap;
use std::sync::OnceLock;

use nurt_contracts::schema;
use nurt_contracts::spec::{IoSpec, ParamType, ScalarType};
use serde_json::Value;

use crate::catalog::Commands;

/// Prefix of a reference to a definition of the schema.
const DEFINITION_PREFIX: &str = "#/$defs/";

/// Field of a record kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Field {
    /// Name of the field.
    pub(crate) name: String,
    /// Description of the field.
    pub(crate) summary: String,
}

/// Field of the input records of a stage with its record kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InputField<'a> {
    /// The field.
    pub(crate) field: &'static Field,
    /// Kind of the records that have the field.
    pub(crate) kind: &'a str,
}

/// Returns the fields of a record kind in name order; none for a kind
/// without a typed structure.
pub(crate) fn of_kind(kind: &str) -> &'static [Field] {
    static FIELDS: OnceLock<HashMap<String, Vec<Field>>> = OnceLock::new();
    FIELDS
        .get_or_init(|| read_schema(schema::VALUE))
        .get(kind)
        .map_or(&[], Vec::as_slice)
}

/// Reads the fields of every typed record kind from the value schema.
fn read_schema(text: &str) -> HashMap<String, Vec<Field>> {
    let Ok(schema) = serde_json::from_str::<Value>(text) else {
        return HashMap::new();
    };
    let definitions = &schema["$defs"];
    let Some(kinds) = definitions["TypedRecord"]["oneOf"].as_array() else {
        return HashMap::new();
    };
    kinds
        .iter()
        .filter_map(|kind| {
            let properties = &kind["properties"];
            let name = properties["kind"]["const"].as_str()?;
            let definition = properties["record"]["$ref"]
                .as_str()?
                .strip_prefix(DEFINITION_PREFIX)?;
            let fields = definitions[definition]["properties"]
                .as_object()?
                .iter()
                .map(|(field, description)| Field {
                    name: field.clone(),
                    summary: description["description"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                })
                .collect();
            Some((name.to_owned(), fields))
        })
        .collect()
}

/// Returns the fields of the records that the stages `upstream` (the
/// nearest first) pass to a stage; none when the kind is not known.
pub(crate) fn input_fields<'a>(commands: &'a Commands, upstream: &[String]) -> Vec<InputField<'a>> {
    for name in upstream {
        let Some(spec) = commands.get(name) else {
            break;
        };
        let kinds: Vec<&str> = spec
            .output
            .iter()
            .filter_map(|output| match output {
                IoSpec::Record(kind) => Some(kind.as_str()),
                _ => None,
            })
            .collect();
        if !kinds.is_empty() {
            let mut fields: Vec<InputField<'a>> = Vec::new();
            for kind in kinds {
                for field in of_kind(kind) {
                    if !fields.iter().any(|known| known.field.name == field.name) {
                        fields.push(InputField { field, kind });
                    }
                }
            }
            return fields;
        }
        if !spec.output.contains(&IoSpec::Same) {
            break;
        }
    }
    Vec::new()
}

/// Whether a value of the type names fields of the input records: a field,
/// an expression or a list of them.
pub(crate) fn takes_field(ty: &ParamType) -> bool {
    match ty {
        ParamType::Scalar(scalar) => matches!(
            scalar,
            ScalarType::Field | ScalarType::Expression | ScalarType::NamedExpression
        ),
        ParamType::List(inner) => takes_field(inner),
        ParamType::Union(alternatives) => alternatives.iter().any(takes_field),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::tests::sample;

    fn names(fields: &[InputField<'_>]) -> Vec<String> {
        fields
            .iter()
            .map(|input| input.field.name.clone())
            .collect()
    }

    #[test]
    fn reads_the_fields_of_typed_record_kinds() {
        let entry = of_kind("entry");
        let size = entry.iter().find(|field| field.name == "size").unwrap();
        assert!(!size.summary.is_empty());
        assert!(entry.iter().any(|field| field.name == "path"));
        assert!(of_kind("no_such_kind").is_empty());
        assert!(read_schema("not json").is_empty());
        assert!(read_schema("{}").is_empty());
    }

    #[test]
    fn takes_the_fields_of_the_nearest_record_output() {
        let commands = sample();
        let upstream = ["sort".to_owned(), "find".to_owned()];
        let fields = input_fields(&commands, &upstream);
        assert!(names(&fields).contains(&"size".to_owned()));
        assert!(fields.iter().all(|input| input.kind == "entry"));
        let counted = ["count".to_owned(), "find".to_owned()];
        assert!(input_fields(&commands, &counted).is_empty());
        let unknown = ["frobnicate".to_owned(), "find".to_owned()];
        assert!(input_fields(&commands, &unknown).is_empty());
        assert!(input_fields(&commands, &[]).is_empty());
    }

    #[test]
    fn recognizes_types_that_name_fields() {
        for (text, expected) in [
            ("field", true),
            ("list<field>", true),
            ("expression", true),
            ("text | field", true),
            ("path", false),
            ("enum(asc|desc)", false),
            ("map<text>", false),
        ] {
            let ty: ParamType = text.parse().unwrap();
            assert_eq!(takes_field(&ty), expected, "{text}");
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
