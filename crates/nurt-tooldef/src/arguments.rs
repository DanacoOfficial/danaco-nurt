// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-tooldef::arguments
// Responsibility:   Arguments of a tool call turned into a request envelope.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Arguments of a tool call turned into a request envelope.

use nurt_contracts::envelope::{ENVELOPE_VERSION, Request};
use serde_json::{Map, Value};

use crate::error::ToolDefError;
use crate::format::ToolFormat;
use crate::rules::{self, Profile};
use crate::source::{OPTIONS_FIELD, Source};

/// Turns the arguments of a `nurt` tool call into a request `nurt-request/1`.
///
/// The arguments are the JSON object that the model produced for the tool
/// rendered in `format`. The function undoes the adaptations of the format:
/// in the strict OpenAI formats a `null` argument means an absent one, and
/// in the formats that pass `ast` as JSON text the text is parsed. Inputs
/// (`program`, `ast`, `cursor`, `accept`) go to the top level of the
/// envelope and every other argument to `options`.
///
/// The rules of the envelope that the schema of a format cannot express,
/// such as exactly one input or the range of `limit`, are not checked here:
/// the instance checks them and reports a violation as the language error
/// `E_ARG`, so the model receives a correction it can act on.
///
/// # Errors
///
/// [`ToolDefError::InvalidArguments`] when the arguments are not an
/// object, name an unknown argument, hold `ast` text that is not JSON, or
/// have a value of the wrong JSON type.
///
/// # Examples
///
/// ```
/// use nurt_tooldef::{ToolFormat, request_from_arguments};
/// use serde_json::json;
///
/// let request = request_from_arguments(
///     ToolFormat::OpenAiChatStrict,
///     &json!({"program": "list \"/home/user\"", "limit": 20, "cursor": null}),
/// )?;
/// assert_eq!(request.program.as_deref(), Some("list \"/home/user\""));
/// assert_eq!(request.options.limit(), 20);
/// assert!(request.cursor.is_none());
/// # Ok::<(), nurt_tooldef::ToolDefError>(())
/// ```
pub fn request_from_arguments(
    format: ToolFormat,
    arguments: &Value,
) -> Result<Request, ToolDefError> {
    let source = Source::published()?;
    let profile = rules::profile(format);
    let arguments = arguments
        .as_object()
        .ok_or_else(|| invalid("arguments are not a JSON object; pass an object with `program`"))?;
    let mut envelope = Map::new();
    envelope.insert("nurt".to_owned(), Value::from(ENVELOPE_VERSION));
    let mut options = Map::new();
    for (name, value) in arguments {
        if !source.inputs.contains(name) && !source.options.contains(name) {
            return Err(invalid(&format!("unknown argument `{name}`; remove it")));
        }
        if profile.strict && value.is_null() {
            continue;
        }
        let value = decode(&source, profile, name, value)?;
        if source.inputs.contains(name) {
            envelope.insert(name.clone(), value);
        } else if source.options.contains(name) {
            options.insert(name.clone(), value);
        } else {
            return Err(invalid(&format!("unknown argument `{name}`; remove it")));
        }
    }
    if !options.is_empty() {
        envelope.insert(OPTIONS_FIELD.to_owned(), Value::Object(options));
    }
    serde_json::from_value(Value::Object(envelope)).map_err(|error| invalid(&error.to_string()))
}

fn decode(
    source: &Source,
    profile: Profile,
    name: &str,
    value: &Value,
) -> Result<Value, ToolDefError> {
    let passed_as_text = profile.objects_as_text
        && source
            .schema
            .get("properties")
            .and_then(|properties| properties.get(name))
            .is_some_and(rules::is_free_object);
    match value {
        Value::String(text) if passed_as_text => serde_json::from_str(text)
            .map_err(|error| invalid(&format!("`{name}` is not valid JSON text: {error}"))),
        _ => Ok(value.clone()),
    }
}

fn invalid(reason: &str) -> ToolDefError {
    ToolDefError::InvalidArguments {
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn reason(result: Result<Request, ToolDefError>) -> String {
        match result {
            Err(ToolDefError::InvalidArguments { reason }) => reason,
            other => panic!("expected invalid arguments, got {other:?}"),
        }
    }

    #[test]
    fn splits_inputs_and_options() {
        let request = request_from_arguments(
            ToolFormat::Anthropic,
            &json!({"program": "help", "preview": true, "format": "json"}),
        )
        .unwrap();
        assert_eq!(request.nurt, ENVELOPE_VERSION);
        assert_eq!(request.program.as_deref(), Some("help"));
        assert_eq!(request.options.preview, Some(true));
    }

    /// A minimal syntax tree from the published examples.
    const TREE: &str = include_str!("../../../spec/golden/ast/example-37-journal-space.json");

    #[test]
    fn parses_ast_text_in_formats_that_pass_objects_as_text() {
        let tree: Value = serde_json::from_str(TREE).unwrap();
        for format in [ToolFormat::Gemini, ToolFormat::OpenAiChatStrict] {
            let request =
                request_from_arguments(format, &json!({"ast": TREE, "program": null})).unwrap();
            assert_eq!(serde_json::to_value(request.ast).unwrap(), tree, "{format}");
        }
        let request = request_from_arguments(ToolFormat::Anthropic, &json!({"ast": tree})).unwrap();
        assert!(request.ast.is_some());
    }

    #[test]
    fn rejects_ast_text_that_is_not_json() {
        let error = reason(request_from_arguments(
            ToolFormat::Gemini,
            &json!({"ast": "{\"nurt\": "}),
        ));
        assert!(error.starts_with("`ast` is not valid JSON text"), "{error}");
    }

    #[test]
    fn rejects_ast_text_in_formats_that_pass_objects() {
        assert!(
            !reason(request_from_arguments(
                ToolFormat::Anthropic,
                &json!({"ast": TREE})
            ))
            .is_empty()
        );
    }

    #[test]
    fn rejects_arguments_that_are_not_an_object() {
        assert_eq!(
            reason(request_from_arguments(ToolFormat::Mcp, &json!("help"))),
            "arguments are not a JSON object; pass an object with `program`"
        );
    }

    #[test]
    fn rejects_unknown_argument() {
        assert_eq!(
            reason(request_from_arguments(
                ToolFormat::Mcp,
                &json!({"program": "help", "command": "ls"})
            )),
            "unknown argument `command`; remove it"
        );
    }

    #[test]
    fn an_unknown_argument_is_refused_even_when_it_is_null_in_a_strict_format() {
        for format in ToolFormat::ALL {
            assert_eq!(
                reason(request_from_arguments(
                    *format,
                    &json!({"program": "help", "ścieżka": null})
                )),
                "unknown argument `ścieżka`; remove it",
                "{format}"
            );
        }
    }

    #[test]
    fn nulls_of_known_arguments_are_dropped_in_strict_formats_only_where_absent() {
        let request = request_from_arguments(
            ToolFormat::OpenAiChatStrict,
            &json!({"program": "help", "cursor": null, "accept": null, "ast": null,
                    "preview": null, "timeout": null, "fields": null}),
        )
        .unwrap();
        assert_eq!(request.program.as_deref(), Some("help"));
        assert!(request.cursor.is_none() && request.accept.is_none());
        assert!(request.options.preview.is_none());
    }

    #[test]
    fn an_empty_object_gives_a_request_without_inputs_or_options() {
        let request = request_from_arguments(ToolFormat::Mcp, &json!({})).unwrap();
        assert!(request.program.is_none() && request.ast.is_none());
        assert!(request.cursor.is_none() && request.accept.is_none());
        assert!(request.options.preview.is_none());
    }

    #[test]
    fn rejects_value_of_the_wrong_type() {
        assert!(
            !reason(request_from_arguments(
                ToolFormat::Mcp,
                &json!({"program": 1})
            ))
            .is_empty()
        );
    }

    #[test]
    fn null_is_absence_only_in_strict_formats() {
        let request = request_from_arguments(
            ToolFormat::OpenAiResponsesStrict,
            &json!({"program": "help", "limit": null}),
        )
        .unwrap();
        assert!(request.options.limit.is_none());
        assert!(
            request_from_arguments(
                ToolFormat::OpenAiChat,
                &json!({"program": "help", "limit": null})
            )
            .is_ok()
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
