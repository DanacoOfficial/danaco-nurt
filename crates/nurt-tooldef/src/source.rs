// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Arguments of the tool, derived from the request schema `nurt-request/1`.
//!
//! The tool exposes the inputs of the envelope (`program`, `ast`, `cursor`,
//! `accept`) and the call options as top-level arguments. The envelope
//! fields filled by the host are not arguments: `nurt` is the format
//! version, and `variant` is left out because the instance detects the
//! notation variant of the program text. The result is the neutral input
//! schema; it keeps the root `oneOf` of the request schema, which the rules
//! of [`crate::rules`] turn into a description.

use serde_json::{Map, Value};

use crate::error::ToolDefError;

/// Envelope fields that the host fills and the tool does not expose.
const HOST_FIELDS: &[&str] = &["nurt", "variant"];

/// Name of the envelope field that holds the call options.
pub(crate) const OPTIONS_FIELD: &str = "options";

/// Root keywords of the request schema that only describe the document.
const METADATA_KEYWORDS: &[&str] = &["$schema", "$id", "title", "description", "$defs"];

/// Prefix of the local references resolved by the generator.
const DEFS_PREFIX: &str = "#/$defs/";

/// Largest depth of nested references; deeper chains are treated as cycles.
const MAX_REF_DEPTH: usize = 16;

/// Arguments of the tool and their neutral input schema.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Source {
    /// Names of the arguments that are inputs of the envelope.
    pub(crate) inputs: Vec<String>,
    /// Names of the arguments that are call options.
    pub(crate) options: Vec<String>,
    /// Neutral input schema, with the root `oneOf` of the request schema.
    pub(crate) schema: Map<String, Value>,
}

impl Source {
    /// Builds the arguments from the published request schema.
    pub(crate) fn published() -> Result<Self, ToolDefError> {
        Self::from_request_schema(nurt_contracts::schema::REQUEST)
    }

    /// Builds the arguments from the text of a request schema.
    pub(crate) fn from_request_schema(text: &str) -> Result<Self, ToolDefError> {
        let document: Value = serde_json::from_str(text)
            .map_err(|error| unsupported(format!("not valid JSON: {error}")))?;
        let root = document
            .as_object()
            .ok_or_else(|| unsupported("the root is not an object".to_owned()))?;
        let defs = root
            .get("$defs")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut source = Self {
            inputs: Vec::new(),
            options: Vec::new(),
            schema: Map::new(),
        };
        source
            .schema
            .insert("type".to_owned(), Value::from("object"));
        let mut properties = Map::new();
        for (keyword, value) in root {
            match keyword.as_str() {
                "type" if value.as_str() == Some("object") => {}
                "required" | "dependentRequired" => check_host_only(keyword, value)?,
                "additionalProperties" | "oneOf" => {
                    source.schema.insert(keyword.clone(), value.clone());
                }
                "properties" => {
                    source.collect_properties(value, &defs, &mut properties)?;
                }
                _ if METADATA_KEYWORDS.contains(&keyword.as_str()) => {}
                _ => {
                    return Err(unsupported(format!(
                        "root keyword `{keyword}` is not translated"
                    )));
                }
            }
        }
        source
            .schema
            .insert("properties".to_owned(), Value::Object(properties));
        Ok(source)
    }

    fn collect_properties(
        &mut self,
        value: &Value,
        defs: &Map<String, Value>,
        into: &mut Map<String, Value>,
    ) -> Result<(), ToolDefError> {
        let properties = value
            .as_object()
            .ok_or_else(|| unsupported("`properties` is not an object".to_owned()))?;
        let mut options = Map::new();
        for (name, schema) in properties {
            if HOST_FIELDS.contains(&name.as_str()) {
                continue;
            }
            let schema = resolve(schema, defs, 0)?;
            if name == OPTIONS_FIELD {
                options = schema
                    .get("properties")
                    .and_then(Value::as_object)
                    .cloned()
                    .ok_or_else(|| unsupported("`options` has no properties".to_owned()))?;
            } else {
                self.inputs.push(name.clone());
                into.insert(name.clone(), schema);
            }
        }
        for (name, schema) in options {
            if into.contains_key(&name) {
                return Err(unsupported(format!(
                    "option `{name}` has the name of an input"
                )));
            }
            self.options.push(name.clone());
            into.insert(name, resolve(&schema, defs, 0)?);
        }
        Ok(())
    }
}

fn check_host_only(keyword: &str, value: &Value) -> Result<(), ToolDefError> {
    let host_only = match value {
        Value::Array(items) => items.iter().all(|item| {
            item.as_str()
                .is_some_and(|name| HOST_FIELDS.contains(&name))
        }),
        Value::Object(map) => map.keys().all(|name| HOST_FIELDS.contains(&name.as_str())),
        _ => false,
    };
    if host_only {
        Ok(())
    } else {
        Err(unsupported(format!(
            "`{keyword}` names fields that are arguments of the tool"
        )))
    }
}

/// Replaces local references `#/$defs/<name>` by the referenced schemas;
/// keywords next to a reference take precedence over the referenced ones.
fn resolve(value: &Value, defs: &Map<String, Value>, depth: usize) -> Result<Value, ToolDefError> {
    if depth > MAX_REF_DEPTH {
        return Err(unsupported("references nest too deeply".to_owned()));
    }
    match value {
        Value::Object(map) => {
            let mut resolved = match map.get("$ref") {
                Some(reference) => {
                    let name = reference
                        .as_str()
                        .and_then(|text| text.strip_prefix(DEFS_PREFIX))
                        .ok_or_else(|| {
                            unsupported(format!("reference {reference} is not local"))
                        })?;
                    let target = defs.get(name).ok_or_else(|| {
                        unsupported(format!("reference {reference} has no definition"))
                    })?;
                    match resolve(target, defs, depth + 1)? {
                        Value::Object(target) => target,
                        _ => {
                            return Err(unsupported(format!(
                                "reference {reference} is not a schema object"
                            )));
                        }
                    }
                }
                None => Map::new(),
            };
            for (keyword, inner) in map {
                if keyword != "$ref" {
                    resolved.insert(keyword.clone(), resolve(inner, defs, depth)?);
                }
            }
            Ok(Value::Object(resolved))
        }
        Value::Array(items) => items
            .iter()
            .map(|item| resolve(item, defs, depth))
            .collect::<Result<_, _>>()
            .map(Value::Array),
        _ => Ok(value.clone()),
    }
}

fn unsupported(reason: String) -> ToolDefError {
    ToolDefError::SourceSchema { reason }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn source(schema: &Value) -> Result<Source, ToolDefError> {
        Source::from_request_schema(&schema.to_string())
    }

    fn reason(result: Result<Source, ToolDefError>) -> String {
        match result {
            Err(ToolDefError::SourceSchema { reason }) => reason,
            other => panic!("expected a source schema error, got {other:?}"),
        }
    }

    #[test]
    fn published_schema_gives_inputs_then_options() {
        let source = Source::published().unwrap();
        assert_eq!(source.inputs, ["program", "ast", "cursor", "accept"]);
        assert!(source.options.contains(&"preview".to_owned()));
        assert!(!source.options.contains(&"nurt".to_owned()));
        let properties = source.schema["properties"].as_object().unwrap();
        assert!(!properties.contains_key("variant"));
        assert!(!properties.contains_key("options"));
        assert_eq!(properties["format"]["default"], "text");
        assert_eq!(source.schema["additionalProperties"], false);
        assert_eq!(source.schema["oneOf"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn sibling_keywords_override_referenced_ones() {
        let schema = json!({
            "type": "object",
            "properties": {
                "program": {"type": "string"},
                "options": {"$ref": "#/$defs/options"}
            },
            "$defs": {
                "options": {"type": "object", "properties": {
                    "format": {"$ref": "#/$defs/format", "description": "Format."}
                }},
                "format": {"enum": ["text"], "description": "Replaced."}
            }
        });
        let source = source(&schema).unwrap();
        assert_eq!(
            source.schema["properties"]["format"],
            json!({"enum": ["text"], "description": "Format."})
        );
    }

    #[test]
    fn rejects_text_that_is_not_json() {
        let error = Source::from_request_schema("{").unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("unsupported request schema: not valid JSON")
        );
    }

    #[test]
    fn rejects_root_that_is_not_an_object() {
        assert_eq!(reason(source(&json!([]))), "the root is not an object");
    }

    #[test]
    fn rejects_unknown_root_keyword() {
        assert_eq!(
            reason(source(&json!({"type": "object", "anyOf": []}))),
            "root keyword `anyOf` is not translated"
        );
        assert_eq!(
            reason(source(&json!({"type": "array"}))),
            "root keyword `type` is not translated"
        );
    }

    #[test]
    fn rejects_required_arguments_at_the_root() {
        assert_eq!(
            reason(source(&json!({"required": ["nurt", "program"]}))),
            "`required` names fields that are arguments of the tool"
        );
        assert_eq!(
            reason(source(&json!({"dependentRequired": 1}))),
            "`dependentRequired` names fields that are arguments of the tool"
        );
    }

    #[test]
    fn rejects_malformed_properties() {
        assert_eq!(
            reason(source(&json!({"properties": []}))),
            "`properties` is not an object"
        );
        assert_eq!(
            reason(source(
                &json!({"properties": {"options": {"type": "object"}}})
            )),
            "`options` has no properties"
        );
        assert_eq!(
            reason(source(&json!({"properties": {
                "limit": {"type": "integer"},
                "options": {"properties": {"limit": {}}}
            }}))),
            "option `limit` has the name of an input"
        );
    }

    #[test]
    fn rejects_unresolvable_references() {
        assert_eq!(
            reason(source(
                &json!({"properties": {"a": {"$ref": "other.json"}}})
            )),
            "reference \"other.json\" is not local"
        );
        assert_eq!(
            reason(source(&json!({"properties": {"a": {"$ref": "#/$defs/b"}}}))),
            "reference \"#/$defs/b\" has no definition"
        );
        assert_eq!(
            reason(source(&json!({
                "properties": {"a": {"$ref": "#/$defs/b"}},
                "$defs": {"b": true}
            }))),
            "reference \"#/$defs/b\" is not a schema object"
        );
        assert_eq!(
            reason(source(&json!({
                "properties": {"a": {"$ref": "#/$defs/b"}},
                "$defs": {"b": {"$ref": "#/$defs/b"}}
            }))),
            "references nest too deeply"
        );
    }

    #[test]
    fn leaves_out_metadata_and_fields_filled_by_the_host() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "urn:test",
            "title": "Request",
            "description": "Text.",
            "type": "object",
            "required": ["nurt"],
            "dependentRequired": {"variant": ["nurt"]},
            "properties": {
                "nurt": {"type": "string"},
                "variant": {"type": "string"},
                "program": {"type": "string"}
            },
            "additionalProperties": false,
            "oneOf": [{"required": ["program"]}]
        });
        let source = source(&schema).unwrap();
        assert_eq!(source.inputs, ["program"]);
        assert!(source.options.is_empty());
        assert_eq!(
            Value::Object(source.schema),
            json!({
                "type": "object",
                "properties": {"program": {"type": "string"}},
                "additionalProperties": false,
                "oneOf": [{"required": ["program"]}]
            })
        );
    }

    #[test]
    fn rejects_host_only_keywords_that_name_other_fields_or_are_malformed() {
        assert_eq!(
            reason(source(&json!({"dependentRequired": {"program": ["nurt"]}}))),
            "`dependentRequired` names fields that are arguments of the tool"
        );
        assert_eq!(
            reason(source(&json!({"required": "program"}))),
            "`required` names fields that are arguments of the tool"
        );
    }

    #[test]
    fn resolves_references_inside_arrays() {
        let schema = json!({
            "properties": {"a": {"anyOf": [{"$ref": "#/$defs/b"}]}},
            "$defs": {"b": {"type": "string"}}
        });
        assert_eq!(
            source(&schema).unwrap().schema["properties"]["a"],
            json!({"anyOf": [{"type": "string"}]})
        );
    }
}
