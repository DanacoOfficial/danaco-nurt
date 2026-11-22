// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Rendered definitions against the schemas of the platforms.
//!
//! Every format is rendered with the core group, with an optional group,
//! with both languages and with and without a catalog version, and each
//! result is validated against the schema of its platform in
//! `tests/provider-schemas/`; the `json-schema` format is validated against
//! the published schema `nurt-tool/1`. The formats based on JSON Schema are
//! also validated against the input schema vocabulary of `nurt-tool/1`.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

// Each test crate uses a different part of the shared helpers.
#[allow(dead_code)]
mod common;

use nurt_contracts::envelope::Language;
use nurt_tooldef::{GroupSelection, RenderOptions, ToolFormat, definition};
use serde_json::{Value, json};

use common::{errors, input_schema, provider_schema, tool_schema, validator};

fn variants() -> Vec<RenderOptions> {
    vec![
        RenderOptions::new(common::CORE_TEXT),
        common::options(),
        common::options().with_language(Language::Pl),
        RenderOptions::new(common::CORE_TEXT)
            .with_group_description("git", common::GIT_TEXT)
            .with_groups(GroupSelection::Only(vec!["git".into()])),
    ]
}

/// Returns the schema that a definition in a format must match.
fn platform_schema(format: ToolFormat) -> Value {
    let file = match format {
        ToolFormat::Anthropic => "anthropic-tool.schema.json",
        ToolFormat::OpenAiChat | ToolFormat::OpenAiChatStrict => "openai-chat-tool.schema.json",
        ToolFormat::OpenAiResponses | ToolFormat::OpenAiResponsesStrict => {
            "openai-responses-tool.schema.json"
        }
        ToolFormat::Gemini => "gemini-function-declaration.schema.json",
        ToolFormat::Mcp => "mcp-tool.schema.json",
        _ => return tool_schema(),
    };
    provider_schema(file)
}

fn assert_valid(validator: &jsonschema::Validator, instance: &Value, what: &str) {
    let errors = errors(validator, instance);
    assert!(errors.is_empty(), "{what}: {errors:#?}");
}

#[test]
fn every_format_matches_the_schema_of_its_platform() {
    for format in ToolFormat::ALL {
        let validator = validator(&platform_schema(*format));
        for options in variants() {
            let definition = definition(*format, &options).unwrap();
            assert_valid(&validator, &definition, format.name());
        }
    }
}

#[test]
fn json_schema_format_matches_the_published_schema() {
    let published: Value = serde_json::from_str(
        nurt_contracts::schema::by_name("tool").expect("the schema `tool` is published"),
    )
    .unwrap();
    assert_eq!(published["title"], "nurt-tool/1");
    let validator = validator(&published);
    for options in variants() {
        let definition = definition(ToolFormat::JsonSchema, &options).unwrap();
        assert_valid(&validator, &definition, "json-schema");
    }
}

#[test]
fn strict_formats_match_the_structured_outputs_subset() {
    let validator = validator(&provider_schema("openai-strict-parameters.schema.json"));
    for format in [
        ToolFormat::OpenAiChatStrict,
        ToolFormat::OpenAiResponsesStrict,
    ] {
        let definition = definition(format, &common::options()).unwrap();
        let parameters = input_schema(format, &definition);
        assert_valid(&validator, parameters, format.name());
        let properties: Vec<&String> = parameters["properties"]
            .as_object()
            .unwrap()
            .keys()
            .collect();
        let required: Vec<&str> = parameters["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| name.as_str().unwrap())
            .collect();
        assert_eq!(properties, required, "{format}: every property is required");
    }
}

#[test]
fn json_schema_formats_use_the_neutral_vocabulary() {
    let vocabulary = validator(&json!({
        "$ref": "#/$defs/inputSchema",
        "$defs": tool_schema()["$defs"],
    }));
    for format in ToolFormat::ALL.iter().filter(|f| **f != ToolFormat::Gemini) {
        let definition = definition(*format, &common::options()).unwrap();
        let schema = input_schema(*format, &definition);
        assert_valid(&vocabulary, schema, format.name());
        assert!(
            jsonschema::meta::is_valid(schema),
            "{format}: the input schema is not a valid JSON Schema"
        );
    }
}

#[test]
fn platform_schemas_reject_known_invalid_definitions() {
    let cases = [
        (
            "anthropic-tool.schema.json",
            json!({"name": "nurt", "input_schema": {"type": "object", "oneOf": []}}),
        ),
        (
            "openai-strict-parameters.schema.json",
            json!({"type": "object", "properties": {"a": {"type": "string", "default": "x"}},
                   "required": ["a"], "additionalProperties": false}),
        ),
        (
            "openai-strict-parameters.schema.json",
            json!({"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"]}),
        ),
        (
            "gemini-function-declaration.schema.json",
            json!({"name": "nurt", "description": "d", "parameters": {
                "type": "OBJECT", "properties": {"a": {"type": "STRING"}},
                "additionalProperties": false}}),
        ),
        (
            "gemini-function-declaration.schema.json",
            json!({"name": "nurt", "description": "d", "parameters": {
                "type": "OBJECT", "properties": {"ast": {"type": "OBJECT"}}}}),
        ),
        (
            "mcp-tool.schema.json",
            json!({"name": "nurt tool", "inputSchema": {"type": "object"}}),
        ),
    ];
    for (file, instance) in cases {
        assert!(
            !validator(&provider_schema(file)).is_valid(&instance),
            "{file} accepts {instance}"
        );
    }
}

#[test]
fn published_schema_rejects_known_invalid_definitions() {
    let input_schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object", "description": "d", "additionalProperties": false,
        "properties": {"program": {"type": "string"}},
    });
    let mut combinator = input_schema.clone();
    combinator["oneOf"] = json!([{"required": ["program"]}]);
    let mut without_dialect = input_schema.clone();
    without_dialect.as_object_mut().unwrap().remove("$schema");
    let cases = [
        json!({"name": "nurt", "description": "d", "input_schema": combinator}),
        json!({"name": "nurt", "description": "d", "input_schema": without_dialect}),
        json!({"name": "shell", "description": "d", "input_schema": input_schema}),
        json!({"name": "nurt", "description": "", "input_schema": input_schema}),
        json!({"name": "nurt", "description": "d", "input_schema": input_schema, "extra": 1}),
    ];
    let validator = validator(&tool_schema());
    for instance in cases {
        assert!(
            !validator.is_valid(&instance),
            "nurt-tool/1 accepts {instance}"
        );
    }
}

#[test]
fn metadata_follows_the_options() {
    let mcp = definition(ToolFormat::Mcp, &common::options()).unwrap();
    assert_eq!(mcp["_meta"][nurt_tooldef::MCP_CATALOG_META_KEY], "1.0");
    let neutral = definition(ToolFormat::JsonSchema, &common::options()).unwrap();
    assert_eq!(neutral["catalog_version"], "1.0");
    assert_eq!(
        neutral["input_schema"]["$schema"],
        nurt_tooldef::JSON_SCHEMA_DIALECT
    );
    let plain = RenderOptions::new(common::CORE_TEXT);
    assert!(
        definition(ToolFormat::Mcp, &plain)
            .unwrap()
            .get("_meta")
            .is_none()
    );
    let polish = common::options().with_language(Language::Pl);
    let anthropic = definition(ToolFormat::Anthropic, &polish).unwrap();
    assert_eq!(
        anthropic["input_schema"]["properties"]["language"]["default"],
        "pl"
    );
    let gemini = definition(ToolFormat::Gemini, &polish).unwrap();
    let language = gemini["parameters"]["properties"]["language"]["description"]
        .as_str()
        .unwrap();
    assert!(language.ends_with("Default: `pl`."), "{language}");
}

#[test]
fn description_holds_the_selected_groups_in_every_format() {
    let expected = format!("{}\n\n{}", common::CORE_TEXT, common::GIT_TEXT);
    for format in ToolFormat::ALL {
        let definition = definition(*format, &common::options()).unwrap();
        let description = match format {
            ToolFormat::OpenAiChat | ToolFormat::OpenAiChatStrict => {
                &definition["function"]["description"]
            }
            _ => &definition["description"],
        };
        assert_eq!(description, &Value::from(expected.as_str()), "{format}");
    }
}
