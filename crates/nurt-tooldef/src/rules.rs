// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Rules that adapt the neutral input schema to each tool format.
//!
//! | Rule | `anthropic` | `openai`, `openai-responses` | `openai-strict`, `openai-responses-strict` | `gemini` | `mcp` | `json-schema` |
//! |---|---|---|---|---|---|---|
//! | root `oneOf`/`anyOf` turned into the root description | yes | yes | yes | yes | yes | yes |
//! | `additionalProperties` kept | yes | yes | yes, `false` on every object | removed | yes | yes |
//! | `default` written into the description | no | no | yes | yes | no | no |
//! | `minLength` written into the description | no | no | yes | no | no | no |
//! | every property required, absence given as `null` | no | no | yes | no | no | no |
//! | object without declared properties passed as JSON text | no | no | yes | yes | no | no |
//! | explicit `type` on enumerations | no | no | yes | yes | no | no |
//! | OpenAPI type names (`STRING`) and `format: enum` | no | no | no | yes | no | no |
//!
//! Every rule keeps the information of the neutral schema: a constraint that
//! a format cannot express is written into a description, and the
//! constraints are checked again by the instance against `nurt-request/1`.

use serde_json::{Map, Value};

use crate::error::ToolDefError;
use crate::format::ToolFormat;

/// Sentence added to an object argument that is passed as JSON text.
const JSON_TEXT_NOTE: &str = "Pass the object as JSON text.";

/// Order of the leading keywords of an adapted root schema.
const ROOT_ORDER: &[&str] = &[
    "$schema",
    "type",
    "description",
    "properties",
    "required",
    "additionalProperties",
];

/// Keyword whose value is written into the description of its schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Described {
    /// `default`.
    Default,
    /// `minLength`.
    MinLength,
}

impl Described {
    const fn keyword(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::MinLength => "minLength",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::MinLength => "Minimum length",
        }
    }
}

/// Rules applied for one format; see the table of the module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Profile {
    /// `additionalProperties` is kept.
    pub(crate) additional_properties: bool,
    /// Keywords written into the description.
    pub(crate) described: &'static [Described],
    /// Every property is required and absence is given as `null`.
    pub(crate) strict: bool,
    /// Objects without declared properties are passed as JSON text.
    pub(crate) objects_as_text: bool,
    /// Enumerations carry an explicit `type`.
    pub(crate) typed_enums: bool,
    /// Type names are the OpenAPI enumeration values and enumerations are
    /// marked with `format: enum`.
    pub(crate) openapi_types: bool,
}

const PLAIN: Profile = Profile {
    additional_properties: true,
    described: &[],
    strict: false,
    objects_as_text: false,
    typed_enums: false,
    openapi_types: false,
};

const OPENAI_STRICT: Profile = Profile {
    additional_properties: true,
    described: &[Described::Default, Described::MinLength],
    strict: true,
    objects_as_text: true,
    typed_enums: true,
    openapi_types: false,
};

const GEMINI: Profile = Profile {
    additional_properties: false,
    described: &[Described::Default],
    strict: false,
    objects_as_text: true,
    typed_enums: true,
    openapi_types: true,
};

/// Returns the rules of a format.
pub(crate) const fn profile(format: ToolFormat) -> Profile {
    match format {
        ToolFormat::OpenAiChatStrict | ToolFormat::OpenAiResponsesStrict => OPENAI_STRICT,
        ToolFormat::Gemini => GEMINI,
        ToolFormat::Anthropic
        | ToolFormat::OpenAiChat
        | ToolFormat::OpenAiResponses
        | ToolFormat::Mcp
        | ToolFormat::JsonSchema => PLAIN,
    }
}

/// Whether a schema describes an object without declared properties.
pub(crate) fn is_free_object(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("object")
        && schema.get("properties").is_none()
}

/// Adapts the neutral input schema to a format.
pub(crate) fn adapt(
    mut root: Map<String, Value>,
    profile: Profile,
) -> Result<Map<String, Value>, ToolDefError> {
    describe_root_combinators(&mut root)?;
    adapt_object(&mut root, profile)?;
    Ok(order_root(root))
}

fn describe_root_combinators(root: &mut Map<String, Value>) -> Result<(), ToolDefError> {
    if root.contains_key("allOf") {
        return Err(ToolDefError::SourceSchema {
            reason: "root `allOf` is not translated".to_owned(),
        });
    }
    for (keyword, quantity) in [("oneOf", "exactly one"), ("anyOf", "at least one")] {
        if let Some(branches) = root.shift_remove(keyword) {
            let names = required_names(keyword, &branches)?;
            append_description(root, &format!("Give {quantity} of {}.", list(&names)));
        }
    }
    Ok(())
}

/// Returns the single required name of every branch of a root combinator.
fn required_names(keyword: &str, branches: &Value) -> Result<Vec<String>, ToolDefError> {
    let single = |branch: &Value| -> Option<String> {
        let branch = branch.as_object().filter(|map| map.len() == 1)?;
        match branch.get("required")?.as_array()?.as_slice() {
            [name] => name.as_str().map(str::to_owned),
            _ => None,
        }
    };
    branches
        .as_array()
        .filter(|items| !items.is_empty())
        .and_then(|items| items.iter().map(single).collect::<Option<Vec<_>>>())
        .ok_or_else(|| ToolDefError::SourceSchema {
            reason: format!("root `{keyword}` has a branch other than one required name"),
        })
}

fn list(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    match quoted.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        _ => quoted.concat(),
    }
}

/// Keywords whose value is a list of schemas that are adapted like any other
/// nested schema.
const SCHEMA_LISTS: [&str; 4] = ["anyOf", "oneOf", "allOf", "prefixItems"];

fn adapt_node(node: &mut Value, profile: Profile) -> Result<(), ToolDefError> {
    if profile.objects_as_text && is_free_object(node) {
        let description = node.get("description").cloned();
        let mut text = Map::new();
        text.insert("type".to_owned(), Value::from("string"));
        if let Some(description) = description {
            text.insert("description".to_owned(), description);
        }
        append_description(&mut text, JSON_TEXT_NOTE);
        *node = Value::Object(text);
    }
    match node {
        Value::Object(map) => adapt_object(map, profile),
        _ => Ok(()),
    }
}

fn adapt_object(map: &mut Map<String, Value>, profile: Profile) -> Result<(), ToolDefError> {
    if profile.typed_enums && map.contains_key("enum") && !map.contains_key("type") {
        map.insert("type".to_owned(), Value::from(enum_type(map)?));
    }
    for described in profile.described {
        if let Some(value) = map.shift_remove(described.keyword()) {
            let sentence = format!("{}: {}.", described.label(), literal(&value));
            append_description(map, &sentence);
        }
    }
    if !profile.additional_properties {
        map.shift_remove("additionalProperties");
    }
    if let Some(items) = map.get_mut("items") {
        adapt_node(items, profile)?;
    }
    for keyword in SCHEMA_LISTS {
        if let Some(Value::Array(schemas)) = map.get_mut(keyword) {
            for schema in schemas {
                adapt_node(schema, profile)?;
            }
        }
    }
    if let Some(schema @ Value::Object(_)) = map.get_mut("additionalProperties") {
        adapt_node(schema, profile)?;
    }
    if let Some(Value::Object(properties)) = map.get_mut("properties") {
        for property in properties.values_mut() {
            adapt_node(property, profile)?;
        }
        if profile.strict {
            require_all(map);
        }
    }
    if profile.openapi_types {
        to_openapi_types(map)?;
    }
    Ok(())
}

fn enum_type(map: &Map<String, Value>) -> Result<&'static str, ToolDefError> {
    let all_strings = map
        .get("enum")
        .and_then(Value::as_array)
        .is_some_and(|values| values.iter().all(Value::is_string));
    if all_strings {
        Ok("string")
    } else {
        Err(ToolDefError::SourceSchema {
            reason: "an enumeration without a type has values other than strings".to_owned(),
        })
    }
}

/// Makes every property of an object required and nullable, and closes the
/// object, as structured outputs require.
fn require_all(map: &mut Map<String, Value>) {
    let mut names = Vec::new();
    if let Some(Value::Object(properties)) = map.get_mut("properties") {
        for (name, property) in properties.iter_mut() {
            make_nullable(property);
            names.push(Value::from(name.clone()));
        }
    }
    map.insert("required".to_owned(), Value::Array(names));
    map.insert("additionalProperties".to_owned(), Value::Bool(false));
}

fn make_nullable(property: &mut Value) {
    let Value::Object(map) = property else {
        return;
    };
    match map.get_mut("type") {
        Some(Value::String(name)) => {
            let name = std::mem::take(name);
            map.insert(
                "type".to_owned(),
                Value::Array(vec![Value::from(name), Value::from("null")]),
            );
        }
        Some(Value::Array(names)) if !names.contains(&Value::from("null")) => {
            names.push(Value::from("null"));
        }
        _ => {}
    }
    if let Some(Value::Array(values)) = map.get_mut("enum")
        && !values.contains(&Value::Null)
    {
        values.push(Value::Null);
    }
}

fn to_openapi_types(map: &mut Map<String, Value>) -> Result<(), ToolDefError> {
    if let Some(kind) = map.get("type") {
        let name = kind
            .as_str()
            .ok_or_else(|| ToolDefError::SourceSchema {
                reason: format!("type {kind} has no OpenAPI equivalent"),
            })?
            .to_ascii_uppercase();
        map.insert("type".to_owned(), Value::from(name));
    }
    if map.contains_key("enum") {
        map.insert("format".to_owned(), Value::from("enum"));
    }
    Ok(())
}

/// Renders a keyword value for a description: strings as they are, other
/// values as JSON, both in backticks.
fn literal(value: &Value) -> String {
    match value {
        Value::String(text) => format!("`{text}`"),
        other => format!("`{other}`"),
    }
}

fn append_description(map: &mut Map<String, Value>, sentence: &str) {
    let text = match map.get("description").and_then(Value::as_str) {
        Some(existing) if !existing.is_empty() => format!("{existing} {sentence}"),
        _ => sentence.to_owned(),
    };
    map.insert("description".to_owned(), Value::from(text));
}

fn order_root(mut root: Map<String, Value>) -> Map<String, Value> {
    let mut ordered = Map::new();
    for keyword in ROOT_ORDER {
        if let Some(value) = root.shift_remove(*keyword) {
            ordered.insert((*keyword).to_owned(), value);
        }
    }
    ordered.extend(root);
    ordered
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn adapted(schema: Value, format: ToolFormat) -> Result<Value, ToolDefError> {
        let Value::Object(root) = schema else {
            panic!("test schema is not an object");
        };
        adapt(root, profile(format)).map(Value::Object)
    }

    fn sample() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "oneOf": [{"required": ["program"]}, {"required": ["ast"]}, {"required": ["cursor"]}],
            "properties": {
                "program": {"type": "string", "description": "Program text."},
                "ast": {"type": "object", "required": ["nurt"], "description": "Tree."},
                "cursor": {"type": "string", "minLength": 1},
                "limit": {"type": "integer", "minimum": 1, "default": 50},
                "format": {"enum": ["text", "json"], "default": "text"},
                "fields": {"type": "array", "items": {"type": "string", "minLength": 1}}
            }
        })
    }

    #[test]
    fn plain_formats_keep_the_schema_and_describe_the_root_one_of() {
        for format in [
            ToolFormat::Anthropic,
            ToolFormat::OpenAiChat,
            ToolFormat::OpenAiResponses,
            ToolFormat::Mcp,
            ToolFormat::JsonSchema,
        ] {
            let schema = adapted(sample(), format).unwrap();
            let mut expected = sample();
            let expected = expected.as_object_mut().unwrap();
            expected.shift_remove("oneOf");
            assert_eq!(
                schema["description"],
                "Give exactly one of `program`, `ast` or `cursor`."
            );
            assert_eq!(schema["properties"], expected["properties"], "{format}");
            assert_eq!(schema["additionalProperties"], false);
            assert!(schema.get("oneOf").is_none());
        }
    }

    #[test]
    fn root_keywords_come_in_a_fixed_order() {
        let schema = adapted(sample(), ToolFormat::Anthropic).unwrap();
        let keys: Vec<&String> = schema.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["type", "description", "properties", "additionalProperties"]
        );
    }

    #[test]
    fn strict_requires_every_property_and_allows_null() {
        let schema = adapted(sample(), ToolFormat::OpenAiChatStrict).unwrap();
        assert_eq!(
            schema["required"],
            json!(["program", "ast", "cursor", "limit", "format", "fields"])
        );
        assert_eq!(schema["additionalProperties"], false);
        let properties = &schema["properties"];
        assert_eq!(
            properties["limit"],
            json!({"type": ["integer", "null"], "minimum": 1, "description": "Default: `50`."})
        );
        assert_eq!(
            properties["format"],
            json!({
                "enum": ["text", "json", null],
                "type": ["string", "null"],
                "description": "Default: `text`."
            })
        );
        assert_eq!(
            properties["cursor"],
            json!({"type": ["string", "null"], "description": "Minimum length: `1`."})
        );
        assert_eq!(
            properties["fields"]["items"],
            json!({"type": "string", "description": "Minimum length: `1`."})
        );
        assert_eq!(
            properties["ast"],
            json!({
                "type": ["string", "null"],
                "description": "Tree. Pass the object as JSON text."
            })
        );
    }

    #[test]
    fn gemini_uses_openapi_types_without_additional_properties() {
        let schema = adapted(sample(), ToolFormat::Gemini).unwrap();
        assert_eq!(schema["type"], "OBJECT");
        assert!(schema.get("additionalProperties").is_none());
        assert!(schema.get("required").is_none());
        let properties = &schema["properties"];
        assert_eq!(
            properties["format"],
            json!({
                "enum": ["text", "json"],
                "type": "STRING",
                "description": "Default: `text`.",
                "format": "enum"
            })
        );
        assert_eq!(
            properties["cursor"],
            json!({"type": "STRING", "minLength": 1})
        );
        assert_eq!(properties["fields"]["items"]["type"], "STRING");
        assert_eq!(properties["ast"]["type"], "STRING");
    }

    #[test]
    fn nested_combinators_additional_properties_and_prefix_items_are_adapted() {
        let source = json!({
            "type": "object",
            "properties": {
                "choice": {"anyOf": [
                    {"type": "string"},
                    {"type": "object", "properties": {"a": {"type": "integer"}}}
                ]},
                "all": {"allOf": [{"type": "object", "properties": {"b": {"type": "string"}}}]},
                "one": {"oneOf": [{"type": "boolean"}]},
                "map": {"type": "object", "additionalProperties": {"type": "number"}},
                "pair": {"type": "array", "prefixItems": [{"type": "string"}, {"type": "integer"}]}
            }
        });
        let gemini = adapted(source.clone(), ToolFormat::Gemini).unwrap();
        let properties = &gemini["properties"];
        assert_eq!(properties["choice"]["anyOf"][0]["type"], "STRING");
        assert_eq!(properties["choice"]["anyOf"][1]["type"], "OBJECT");
        assert_eq!(
            properties["choice"]["anyOf"][1]["properties"]["a"]["type"],
            "INTEGER"
        );
        assert_eq!(
            properties["all"]["allOf"][0]["properties"]["b"]["type"],
            "STRING"
        );
        assert_eq!(properties["one"]["oneOf"][0]["type"], "BOOLEAN");
        assert_eq!(properties["pair"]["prefixItems"][1]["type"], "INTEGER");

        let strict = adapted(source, ToolFormat::OpenAiChatStrict).unwrap();
        let object = &strict["properties"]["choice"]["anyOf"][1];
        assert_eq!(object["additionalProperties"], false);
        assert_eq!(object["required"], json!(["a"]));
        assert_eq!(
            object["properties"]["a"]["type"],
            json!(["integer", "null"])
        );
        let all = &strict["properties"]["all"]["allOf"][0];
        assert_eq!(all["required"], json!(["b"]));
    }

    #[test]
    fn describes_root_any_of() {
        let schema = adapted(
            json!({"anyOf": [{"required": ["a"]}], "properties": {}}),
            ToolFormat::Mcp,
        )
        .unwrap();
        assert_eq!(schema["description"], "Give at least one of `a`.");
    }

    #[test]
    fn appends_the_root_description_to_an_existing_one() {
        let schema = adapted(
            json!({"description": "Arguments.", "oneOf": [{"required": ["a"]}, {"required": ["b"]}]}),
            ToolFormat::Mcp,
        )
        .unwrap();
        assert_eq!(
            schema["description"],
            "Arguments. Give exactly one of `a` or `b`."
        );
    }

    #[test]
    fn rejects_root_combinators_that_are_not_single_required_names() {
        for schema in [
            json!({"allOf": []}),
            json!({"oneOf": []}),
            json!({"oneOf": [{"required": ["a", "b"]}]}),
            json!({"oneOf": [{"required": ["a"], "type": "object"}]}),
            json!({"oneOf": [{"required": [1]}]}),
        ] {
            assert!(matches!(
                adapted(schema, ToolFormat::Anthropic),
                Err(ToolDefError::SourceSchema { .. })
            ));
        }
    }

    #[test]
    fn rejects_untyped_enumeration_with_non_string_values() {
        let schema = json!({"properties": {"level": {"enum": [1, 2]}}});
        assert!(matches!(
            adapted(schema, ToolFormat::Gemini),
            Err(ToolDefError::SourceSchema { .. })
        ));
    }

    #[test]
    fn rejects_type_lists_for_openapi_types() {
        let schema = json!({"properties": {"a": {"type": ["string", "null"]}}});
        assert!(matches!(
            adapted(schema, ToolFormat::Gemini),
            Err(ToolDefError::SourceSchema { .. })
        ));
    }

    #[test]
    fn strict_extends_type_lists_and_keeps_untyped_schemas() {
        let schema = json!({"properties": {
            "a": {"type": ["string", "integer"]},
            "b": {"type": ["string", "null"]},
            "c": {"description": "Any value."},
            "d": true
        }});
        let schema = adapted(schema, ToolFormat::OpenAiResponsesStrict).unwrap();
        let properties = &schema["properties"];
        assert_eq!(
            properties["a"]["type"],
            json!(["string", "integer", "null"])
        );
        assert_eq!(properties["b"]["type"], json!(["string", "null"]));
        assert_eq!(properties["c"], json!({"description": "Any value."}));
        assert_eq!(properties["d"], true);
    }

    #[test]
    fn free_object_without_description_gets_the_json_text_note() {
        let schema = json!({"properties": {"tree": {"type": "object"}}});
        let schema = adapted(schema, ToolFormat::Gemini).unwrap();
        assert_eq!(
            schema["properties"]["tree"],
            json!({"type": "STRING", "description": "Pass the object as JSON text."})
        );
    }

    #[test]
    fn describes_non_string_defaults_as_json() {
        let schema = json!({"properties": {"preview": {"type": "boolean", "default": false}}});
        let schema = adapted(schema, ToolFormat::Gemini).unwrap();
        assert_eq!(
            schema["properties"]["preview"]["description"],
            "Default: `false`."
        );
    }
}
