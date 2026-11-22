// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Definitions of the `nurt` tool for model providers and the Model Context
//! Protocol.
//!
//! An AI agent calls Danaco Nurt through one tool named `nurt`. The
//! arguments of the tool are the inputs and call options of the request
//! envelope `nurt-request/1`; [`render`] writes the tool definition in the
//! format that a platform accepts in its list of tools, and
//! [`request_from_arguments`] turns the arguments of a tool call back into a
//! request. Every format accepts the same calls: a constraint that a format
//! cannot express is written into a description and checked by the instance.
//!
//! The output is deterministic: the same format and [`RenderOptions`] give
//! the same bytes.
//!
//! # Examples
//!
//! ```
//! use nurt_tooldef::{RenderOptions, ToolFormat, render};
//!
//! let options = RenderOptions::new("Runs programs of the Nurt language.");
//! let definition = render(ToolFormat::Anthropic, &options)?;
//! let definition: serde_json::Value = serde_json::from_str(&definition).unwrap();
//! assert_eq!(definition["name"], "nurt");
//! assert_eq!(definition["input_schema"]["type"], "object");
//! # Ok::<(), nurt_tooldef::ToolDefError>(())
//! ```

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod arguments;
mod error;
mod format;
mod options;
mod rules;
mod source;

use serde_json::{Map, Value};

pub use crate::arguments::request_from_arguments;
pub use crate::error::ToolDefError;
pub use crate::format::ToolFormat;
pub use crate::options::{CORE_GROUP, GroupSelection, RenderOptions};

/// Name of the tool in every format.
pub const TOOL_NAME: &str = "nurt";

/// Dialect of the input schema of the `json-schema` format.
pub const JSON_SCHEMA_DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

/// Key of the catalog version in the `_meta` object of the `mcp` format.
pub const MCP_CATALOG_META_KEY: &str = "pl.danaco-group/nurt-catalog";

/// Name of the argument whose default follows the language of the options.
const LANGUAGE_ARGUMENT: &str = "language";

/// Renders the tool definition in a format as pretty-printed JSON.
///
/// # Errors
///
/// The errors of [`definition`].
///
/// # Examples
///
/// ```
/// use nurt_tooldef::{RenderOptions, ToolFormat, render};
///
/// let options = RenderOptions::new("Runs programs of the Nurt language.");
/// let first = render(ToolFormat::Gemini, &options)?;
/// assert_eq!(first, render(ToolFormat::Gemini, &options)?);
/// # Ok::<(), nurt_tooldef::ToolDefError>(())
/// ```
pub fn render(format: ToolFormat, options: &RenderOptions) -> Result<String, ToolDefError> {
    definition(format, options).map(|value| format!("{value:#}"))
}

/// Returns the tool definition in a format as a JSON value.
///
/// | Format | Value |
/// |---|---|
/// | `anthropic` | `{name, description, input_schema}` |
/// | `openai`, `openai-strict` | `{type: "function", function: {name, description, parameters, strict}}` |
/// | `openai-responses`, `openai-responses-strict` | `{type: "function", name, description, parameters, strict}` |
/// | `gemini` | `{name, description, parameters}` |
/// | `mcp` | `{name, description, inputSchema, _meta}`; `_meta` only with a catalog version |
/// | `json-schema` | `{name, description, input_schema, catalog_version}`; `catalog_version` only when given |
///
/// # Errors
///
/// - the errors of [`RenderOptions::description`];
/// - [`ToolDefError::SourceSchema`] when the request schema has a construct
///   that the generator does not translate.
pub fn definition(format: ToolFormat, options: &RenderOptions) -> Result<Value, ToolDefError> {
    let description = options.description()?;
    let mut schema = source::Source::published()?.schema;
    if let Some(argument) = schema
        .get_mut("properties")
        .and_then(|properties| properties.get_mut(LANGUAGE_ARGUMENT))
        .and_then(Value::as_object_mut)
    {
        argument.insert(
            "default".to_owned(),
            Value::from(options.language().as_str()),
        );
    }
    if format == ToolFormat::JsonSchema {
        schema.insert("$schema".to_owned(), Value::from(JSON_SCHEMA_DIALECT));
    }
    let schema = Value::Object(rules::adapt(schema, rules::profile(format))?);
    Ok(wrap(format, description, schema, options.catalog_version()))
}

fn wrap(
    format: ToolFormat,
    description: String,
    schema: Value,
    catalog_version: Option<&str>,
) -> Value {
    let mut tool = Map::new();
    let strict = matches!(
        format,
        ToolFormat::OpenAiChatStrict | ToolFormat::OpenAiResponsesStrict
    );
    if matches!(
        format,
        ToolFormat::OpenAiResponses | ToolFormat::OpenAiResponsesStrict
    ) {
        tool.insert("type".to_owned(), Value::from("function"));
    }
    tool.insert("name".to_owned(), Value::from(TOOL_NAME));
    tool.insert("description".to_owned(), Value::from(description));
    let schema_key = match format {
        ToolFormat::Anthropic | ToolFormat::JsonSchema => "input_schema",
        ToolFormat::Mcp => "inputSchema",
        ToolFormat::OpenAiChat
        | ToolFormat::OpenAiChatStrict
        | ToolFormat::OpenAiResponses
        | ToolFormat::OpenAiResponsesStrict
        | ToolFormat::Gemini => "parameters",
    };
    tool.insert(schema_key.to_owned(), schema);
    match format {
        ToolFormat::OpenAiChat | ToolFormat::OpenAiChatStrict => {
            tool.insert("strict".to_owned(), Value::Bool(strict));
            let mut outer = Map::new();
            outer.insert("type".to_owned(), Value::from("function"));
            outer.insert("function".to_owned(), Value::Object(tool));
            return Value::Object(outer);
        }
        ToolFormat::OpenAiResponses | ToolFormat::OpenAiResponsesStrict => {
            tool.insert("strict".to_owned(), Value::Bool(strict));
        }
        ToolFormat::Mcp => {
            if let Some(version) = catalog_version {
                let mut meta = Map::new();
                meta.insert(MCP_CATALOG_META_KEY.to_owned(), Value::from(version));
                tool.insert("_meta".to_owned(), Value::Object(meta));
            }
        }
        ToolFormat::JsonSchema => {
            if let Some(version) = catalog_version {
                tool.insert("catalog_version".to_owned(), Value::from(version));
            }
        }
        ToolFormat::Anthropic | ToolFormat::Gemini => {}
    }
    Value::Object(tool)
}
