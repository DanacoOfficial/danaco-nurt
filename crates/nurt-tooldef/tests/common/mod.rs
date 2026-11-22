// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Helpers shared by the integration tests.

use std::fs;
use std::path::{Path, PathBuf};

use nurt_tooldef::{GroupSelection, RenderOptions, ToolFormat};
use serde_json::{Map, Value};

/// Description texts used by the tests; their content is not checked.
pub(crate) const CORE_TEXT: &str = "Runs programs of the Nurt language.";
pub(crate) const GIT_TEXT: &str = "Group git: Git repositories.";

/// Options with the core group, one optional group and a catalog version.
pub(crate) fn options() -> RenderOptions {
    RenderOptions::new(CORE_TEXT)
        .with_group_description("git", GIT_TEXT)
        .with_groups(GroupSelection::All)
        .with_catalog_version("1.0")
}

pub(crate) fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub(crate) fn read_json(path: &Path) -> Value {
    let text =
        fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

pub(crate) fn provider_schema(file: &str) -> Value {
    read_json(&crate_dir().join("tests/provider-schemas").join(file))
}

pub(crate) fn validator(schema: &Value) -> jsonschema::Validator {
    jsonschema::validator_for(schema).unwrap_or_else(|error| panic!("invalid schema: {error}"))
}

pub(crate) fn errors(validator: &jsonschema::Validator, instance: &Value) -> Vec<String> {
    validator
        .iter_errors(instance)
        .map(|error| format!("{} at {}", error, error.instance_path()))
        .collect()
}

/// Whether a format passes `ast` as JSON text.
pub(crate) fn passes_objects_as_text(format: ToolFormat) -> bool {
    matches!(
        format,
        ToolFormat::OpenAiChatStrict | ToolFormat::OpenAiResponsesStrict | ToolFormat::Gemini
    )
}

pub(crate) fn is_strict(format: ToolFormat) -> bool {
    matches!(
        format,
        ToolFormat::OpenAiChatStrict | ToolFormat::OpenAiResponsesStrict
    )
}

/// Returns the input schema of a rendered definition.
pub(crate) fn input_schema(format: ToolFormat, definition: &Value) -> &Value {
    match format {
        ToolFormat::Anthropic | ToolFormat::JsonSchema => &definition["input_schema"],
        ToolFormat::Mcp => &definition["inputSchema"],
        ToolFormat::OpenAiChat | ToolFormat::OpenAiChatStrict => {
            &definition["function"]["parameters"]
        }
        _ => &definition["parameters"],
    }
}

/// Returns the input schema as JSON Schema; the OpenAPI subset of the
/// `gemini` format is translated with the semantics of the Gemini API.
pub(crate) fn as_json_schema(format: ToolFormat, schema: &Value) -> Value {
    if format == ToolFormat::Gemini {
        from_openapi(schema)
    } else {
        schema.clone()
    }
}

fn from_openapi(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => {
            let mut out = Map::new();
            for (keyword, value) in map {
                match keyword.as_str() {
                    "type" => {
                        let name = value.as_str().unwrap().to_ascii_lowercase();
                        out.insert(keyword.clone(), Value::from(name));
                    }
                    "format" | "nullable" => {}
                    "properties" => {
                        let properties = value
                            .as_object()
                            .unwrap()
                            .iter()
                            .map(|(name, inner)| (name.clone(), from_openapi(inner)))
                            .collect();
                        out.insert(keyword.clone(), Value::Object(properties));
                    }
                    "items" => {
                        out.insert(keyword.clone(), from_openapi(value));
                    }
                    _ => {
                        out.insert(keyword.clone(), value.clone());
                    }
                }
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// Writes neutral arguments the way a model writes them for a format: `ast`
/// as JSON text where the format requires it, and every absent argument as
/// `null` in the strict formats.
pub(crate) fn encode(format: ToolFormat, schema: &Value, arguments: &Value) -> Value {
    let Value::Object(arguments) = arguments else {
        return arguments.clone();
    };
    let mut encoded = Map::new();
    for (name, value) in arguments {
        let value = match value {
            Value::Object(_) if passes_objects_as_text(format) => Value::from(value.to_string()),
            _ => value.clone(),
        };
        encoded.insert(name.clone(), value);
    }
    if is_strict(format) {
        for name in schema["properties"].as_object().unwrap().keys() {
            encoded.entry(name.clone()).or_insert(Value::Null);
        }
    }
    Value::Object(encoded)
}
