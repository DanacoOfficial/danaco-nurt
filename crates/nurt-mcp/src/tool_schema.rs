// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Definitions of the tools and conversion of tool arguments.
//!
//! The `nurt` tool is defined by `nurt-tooldef` in its `mcp` format, the same
//! definition that `nurt schema tool --format mcp` prints; its arguments are
//! turned into a request envelope by the same crate. The `nurt_help` tool
//! exists only in this server.

use nurt_contracts::envelope::Request;
use nurt_tooldef::{RenderOptions, ToolFormat};
use rmcp::model::{JsonObject, Tool, ToolAnnotations};
use serde_json::{Map, Value, json};

use crate::backend::{HelpKind, HelpQuery};

/// Name of the tool that runs programs.
pub(crate) const RUN_TOOL: &str = nurt_tooldef::TOOL_NAME;

/// Name of the tool that returns built-in help.
pub(crate) const HELP_TOOL: &str = "nurt_help";

/// Title of the `nurt` tool.
const RUN_TITLE: &str = "Danaco Nurt";

/// Title of the `nurt_help` tool.
const HELP_TITLE: &str = "Danaco Nurt help";

/// Description of the `nurt_help` tool.
const HELP_DESCRIPTION: &str = "Returns built-in help of Danaco Nurt as plain text: a command, \
a group of commands, an error code, a domain guide, the syntax or the corrections. Reading help \
is optional; the `nurt` tool accepts programs without it.";

/// Builds the `nurt` tool with the core description layer of the engine.
///
/// The program of a call may change files and run external programs, so
/// the tool is annotated as destructive, not idempotent and open-world. A
/// definition that cannot be built, for example for an empty core
/// description, is reported as text.
pub(crate) fn run_tool(core_description: &str) -> Result<Tool, String> {
    let definition =
        nurt_tooldef::definition(ToolFormat::Mcp, &RenderOptions::new(core_description))
            .map_err(|error| format!("tool definition: {error}"))?;
    let tool: Tool = serde_json::from_value(definition)
        .map_err(|error| format!("tool definition does not match the protocol: {error}"))?;
    let annotations = ToolAnnotations::with_title(RUN_TITLE)
        .read_only(false)
        .destructive(true)
        .idempotent(false)
        .open_world(true);
    Ok(tool.with_title(RUN_TITLE).with_annotations(annotations))
}

/// Builds the `nurt_help` tool, which only reads.
pub(crate) fn help_tool() -> Tool {
    let annotations = ToolAnnotations::with_title(HELP_TITLE)
        .read_only(true)
        .destructive(false)
        .idempotent(true)
        .open_world(false);
    Tool::new(HELP_TOOL, HELP_DESCRIPTION, help_input_schema())
        .with_title(HELP_TITLE)
        .with_annotations(annotations)
}

/// Builds the input schema of the `nurt_help` tool.
fn help_input_schema() -> JsonObject {
    let kinds: Vec<&str> = HelpKind::ALL.iter().map(|kind| kind.as_str()).collect();
    let mut properties = Map::new();
    properties.insert(
        "topic".to_owned(),
        json!({
            "type": "string",
            "description": "Command, group, error code, `syntax` or `corrections`; omit for the index of topics."
        }),
    );
    properties.insert(
        "kind".to_owned(),
        json!({
            "enum": kinds,
            "description": "Kind of the topic, when the name alone is ambiguous."
        }),
    );
    properties.insert(
        "examples".to_owned(),
        json!({
            "type": "integer",
            "minimum": 0,
            "default": 3,
            "description": "Number of examples to include."
        }),
    );
    properties.insert(
        "language".to_owned(),
        json!({
            "enum": ["en", "pl"],
            "default": "en",
            "description": "Language of the text."
        }),
    );

    let mut input = Map::new();
    input.insert("type".to_owned(), json!("object"));
    input.insert("properties".to_owned(), Value::Object(properties));
    input.insert("additionalProperties".to_owned(), Value::Bool(false));
    input
}

/// Converts the arguments of the `nurt` tool into a request envelope. The
/// default session applies when the arguments do not name a session. An
/// argument that does not match the tool definition is reported as text.
pub(crate) fn request_from_arguments(
    arguments: Option<JsonObject>,
    session: Option<&str>,
) -> Result<Request, String> {
    let arguments = Value::Object(arguments.unwrap_or_default());
    let mut request = nurt_tooldef::request_from_arguments(ToolFormat::Mcp, &arguments)
        .map_err(|error| format!("invalid arguments of the `{RUN_TOOL}` tool: {error}"))?;
    if request.options.session.is_none() {
        request.options.session = session.map(str::to_owned);
    }
    Ok(request)
}

/// Converts the arguments of the `nurt_help` tool into a help query. An
/// argument that does not match the input schema is reported as text.
pub(crate) fn help_query_from_arguments(
    arguments: Option<JsonObject>,
) -> Result<HelpQuery, String> {
    serde_json::from_value(Value::Object(arguments.unwrap_or_default()))
        .map_err(|error| format!("invalid arguments of the `{HELP_TOOL}` tool: {error}"))
}

#[cfg(test)]
mod tests {
    use nurt_contracts::envelope::{Language, OutputFormat};
    use nurt_tooldef::{RenderOptions, ToolFormat};
    use serde_json::{Map, Value, json};

    use super::{help_query_from_arguments, help_tool, request_from_arguments, run_tool};
    use crate::backend::HelpKind;

    fn object(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            _ => Map::new(),
        }
    }

    #[test]
    fn run_tool_matches_the_mcp_definition() {
        let tool = run_tool("Runs programs of the Nurt language.").unwrap();
        let expected = nurt_tooldef::definition(
            ToolFormat::Mcp,
            &RenderOptions::new("Runs programs of the Nurt language."),
        )
        .unwrap();
        let actual = serde_json::to_value(&tool).unwrap();
        assert_eq!(actual["name"], expected["name"]);
        assert_eq!(actual["description"], expected["description"]);
        assert_eq!(actual["inputSchema"], expected["inputSchema"]);
        assert_eq!(
            actual["annotations"],
            json!({
                "title": "Danaco Nurt",
                "readOnlyHint": false,
                "destructiveHint": true,
                "idempotentHint": false,
                "openWorldHint": true
            })
        );
    }

    #[test]
    fn run_tool_rejects_an_empty_description() {
        let error = run_tool("  ").unwrap_err();
        assert!(error.starts_with("tool definition: "), "{error}");
    }

    #[test]
    fn help_tool_only_reads_and_lists_every_kind() {
        let tool = serde_json::to_value(help_tool()).unwrap();
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert_eq!(tool["annotations"]["destructiveHint"], false);
        assert_eq!(
            tool["inputSchema"]["properties"]["kind"]["enum"],
            json!([
                "command",
                "group",
                "error",
                "skill",
                "syntax",
                "corrections"
            ])
        );
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
    }

    #[test]
    fn arguments_split_into_inputs_and_options() {
        let arguments = object(json!({
            "program": "find \"/srv/app\"",
            "format": "json",
            "limit": 20
        }));
        let request = request_from_arguments(Some(arguments), None).unwrap();
        assert_eq!(request.nurt, "1.0");
        assert_eq!(request.program.as_deref(), Some("find \"/srv/app\""));
        assert_eq!(request.options.format, Some(OutputFormat::Json));
        assert_eq!(request.options.limit, Some(20));
        assert_eq!(request.options.session, None);
    }

    #[test]
    fn default_session_applies_only_when_absent() {
        let without = request_from_arguments(Some(object(json!({"program": "ok"}))), Some("s1"));
        assert_eq!(without.unwrap().options.session.as_deref(), Some("s1"));
        let with = request_from_arguments(
            Some(object(json!({"program": "ok", "session": "agent-7"}))),
            Some("s1"),
        );
        assert_eq!(with.unwrap().options.session.as_deref(), Some("agent-7"));
    }

    #[test]
    fn unknown_argument_is_rejected_with_its_name() {
        let message =
            request_from_arguments(Some(object(json!({"program": "ok", "colour": 1}))), None)
                .unwrap_err();
        assert!(
            message.starts_with("invalid arguments of the `nurt` tool: "),
            "{message}"
        );
        assert!(message.contains("colour"), "{message}");
    }

    #[test]
    fn help_arguments_are_parsed() {
        let query = help_query_from_arguments(Some(object(json!({
            "topic": "replace",
            "kind": "command",
            "examples": 6,
            "language": "pl"
        }))))
        .unwrap();
        assert_eq!(query.topic.as_deref(), Some("replace"));
        assert_eq!(query.kind, Some(HelpKind::Command));
        assert_eq!(query.examples, Some(6));
        assert_eq!(query.language, Some(Language::Pl));
        assert_eq!(help_query_from_arguments(None), Ok(Default::default()));
        assert!(help_query_from_arguments(Some(object(json!({"kind": "page"})))).is_err());
    }
}
