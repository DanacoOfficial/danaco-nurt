// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Rendered definitions against the limits of tool-calling APIs: names,
//! nesting, the keywords each platform rejects, and the types of the values.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

// Each test crate uses a different part of the shared helpers.
#[allow(dead_code)]
mod common;

use nurt_contracts::envelope::Language;
use nurt_tooldef::{RenderOptions, TOOL_NAME, ToolFormat, definition, render};
use serde_json::{Map, Value};

/// Longest name of a tool or of a property accepted by the platforms.
const NAME_LIMIT: usize = 64;

/// Deepest nesting of objects accepted by the strict formats.
const NESTING_LIMIT: usize = 5;

fn allowed_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= NAME_LIMIT
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn tool_name(format: ToolFormat, tool: &Value) -> &str {
    let name = match format {
        ToolFormat::OpenAiChat | ToolFormat::OpenAiChatStrict => &tool["function"]["name"],
        _ => &tool["name"],
    };
    name.as_str().unwrap()
}

/// Calls `visit` for every schema object of the tree with its object depth.
fn walk(schema: &Value, depth: usize, visit: &mut dyn FnMut(&Map<String, Value>, usize)) {
    let Value::Object(map) = schema else {
        return;
    };
    visit(map, depth);
    if let Some(Value::Object(properties)) = map.get("properties") {
        for property in properties.values() {
            walk(property, depth + 1, visit);
        }
    }
    if let Some(items) = map.get("items") {
        walk(items, depth, visit);
    }
}

fn schema_of(format: ToolFormat) -> Value {
    let tool = definition(format, &common::options()).unwrap();
    common::input_schema(format, &tool).clone()
}

#[test]
fn the_tool_name_and_every_property_name_fit_the_limits_of_the_platforms() {
    for format in ToolFormat::ALL {
        let tool = definition(*format, &common::options()).unwrap();
        assert_eq!(tool_name(*format, &tool), TOOL_NAME);
        assert!(allowed_name(TOOL_NAME), "{format}");
        let schema = common::input_schema(*format, &tool);
        let mut count = 0;
        walk(schema, 0, &mut |map, _| {
            if let Some(Value::Object(properties)) = map.get("properties") {
                for name in properties.keys() {
                    count += 1;
                    assert!(allowed_name(name), "{format}: `{name}`");
                }
            }
        });
        assert!(count > 0 && count <= 100, "{format}: {count} properties");
    }
}

#[test]
fn objects_nest_within_the_limit_and_the_root_is_an_object_without_combinators() {
    for format in ToolFormat::ALL {
        let schema = schema_of(*format);
        let root_type = schema["type"].as_str().unwrap().to_ascii_lowercase();
        assert_eq!(root_type, "object", "{format}");
        for keyword in ["oneOf", "anyOf", "allOf", "not", "enum"] {
            assert!(schema.get(keyword).is_none(), "{format}: root `{keyword}`");
        }
        let mut deepest = 0;
        walk(&schema, 0, &mut |_, depth| deepest = deepest.max(depth));
        assert!(deepest <= NESTING_LIMIT, "{format}: depth {deepest}");
    }
}

#[test]
fn strict_formats_close_every_object_require_every_property_and_drop_unsupported_keywords() {
    for format in [
        ToolFormat::OpenAiChatStrict,
        ToolFormat::OpenAiResponsesStrict,
    ] {
        let schema = schema_of(format);
        walk(&schema, 0, &mut |map, _| {
            for keyword in ["default", "minLength", "oneOf", "$schema"] {
                assert!(!map.contains_key(keyword), "{format}: `{keyword}`");
            }
            if let Some(Value::Object(properties)) = map.get("properties") {
                assert_eq!(map["additionalProperties"], false, "{format}");
                let mut required: Vec<&str> = map["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|name| name.as_str().unwrap())
                    .collect();
                let mut declared: Vec<&str> = properties.keys().map(String::as_str).collect();
                required.sort_unstable();
                declared.sort_unstable();
                assert_eq!(required, declared, "{format}");
            }
        });
    }
}

#[test]
fn gemini_uses_upper_case_types_and_none_of_the_rejected_keywords() {
    let schema = schema_of(ToolFormat::Gemini);
    walk(&schema, 0, &mut |map, _| {
        for keyword in [
            "default",
            "additionalProperties",
            "$schema",
            "oneOf",
            "$ref",
        ] {
            assert!(!map.contains_key(keyword), "`{keyword}`");
        }
        if let Some(kind) = map.get("type") {
            let name = kind.as_str().unwrap();
            assert_eq!(name, name.to_ascii_uppercase());
        }
        if map.contains_key("enum") {
            assert_eq!(map["format"], "enum");
            assert_eq!(map["type"], "STRING");
        }
    });
}

#[test]
fn plain_formats_keep_defaults_ranges_patterns_and_enumerations() {
    for format in [
        ToolFormat::Anthropic,
        ToolFormat::OpenAiChat,
        ToolFormat::OpenAiResponses,
        ToolFormat::Mcp,
        ToolFormat::JsonSchema,
    ] {
        let schema = schema_of(format);
        let properties = &schema["properties"];
        assert_eq!(properties["limit"]["type"], "integer", "{format}");
        assert_eq!(properties["limit"]["minimum"], 1);
        assert_eq!(properties["limit"]["maximum"], 5000);
        assert_eq!(properties["limit"]["default"], 50);
        assert_eq!(properties["timeout"]["type"], "string");
        assert_eq!(properties["timeout"]["default"], "120s");
        assert!(properties["timeout"]["pattern"].is_string());
        assert_eq!(properties["preview"]["type"], "boolean");
        assert_eq!(properties["base"]["type"], "string");
        assert_eq!(properties["save_to"]["type"], "string");
    }
}

#[test]
fn plain_formats_keep_list_and_enumeration_members_and_omit_envelope_members() {
    for format in [
        ToolFormat::Anthropic,
        ToolFormat::OpenAiChat,
        ToolFormat::OpenAiResponses,
        ToolFormat::Mcp,
        ToolFormat::JsonSchema,
    ] {
        let schema = schema_of(format);
        let properties = &schema["properties"];
        assert_eq!(properties["fields"]["type"], "array", "{format}");
        assert_eq!(properties["fields"]["items"]["type"], "string");
        assert_eq!(
            properties["verbosity"]["enum"],
            serde_json::json!(["quiet", "normal", "detailed"])
        );
        assert!(properties.get("nurt").is_none());
        assert!(properties.get("variant").is_none());
        assert!(properties.get("options").is_none());
    }
}

#[test]
fn the_language_of_the_options_is_the_default_of_the_language_argument() {
    for language in [Language::En, Language::Pl] {
        let options = RenderOptions::new(common::CORE_TEXT).with_language(language);
        let tool = definition(ToolFormat::Anthropic, &options).unwrap();
        assert_eq!(
            tool["input_schema"]["properties"]["language"]["default"],
            language.as_str()
        );
    }
}

#[test]
fn the_output_is_byte_for_byte_the_same_on_every_call_and_ends_without_a_line_feed() {
    for format in ToolFormat::ALL {
        let first = render(*format, &common::options()).unwrap();
        let second = render(*format, &common::options()).unwrap();
        assert_eq!(first, second, "{format}");
        assert!(!first.ends_with('\n'), "{format}");
        let parsed: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(parsed, definition(*format, &common::options()).unwrap());
    }
}

#[test]
fn the_description_carries_multibyte_text_unchanged() {
    let options = RenderOptions::new("Uruchamia programy: zażółć gęślą jaźń \u{1F600}.");
    for format in ToolFormat::ALL {
        let tool = definition(*format, &options).unwrap();
        let description = match format {
            ToolFormat::OpenAiChat | ToolFormat::OpenAiChatStrict => {
                &tool["function"]["description"]
            }
            _ => &tool["description"],
        };
        assert_eq!(
            description, "Uruchamia programy: zażółć gęślą jaźń \u{1F600}.",
            "{format}"
        );
    }
}
