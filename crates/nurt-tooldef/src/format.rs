// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Output formats of the tool definition.

use std::fmt;
use std::str::FromStr;

use crate::error::ToolDefError;

/// Format of a rendered tool definition.
///
/// Each format is the tool object that the platform accepts in its list of
/// tools. The names returned by [`ToolFormat::name`] are the values of
/// `nurt schema tool --format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ToolFormat {
    /// Anthropic Messages API: an element of `tools` with `input_schema`.
    Anthropic,
    /// OpenAI Chat Completions API: an element of `tools` with
    /// `type: "function"` and the nested `function` object.
    OpenAiChat,
    /// OpenAI Chat Completions API with `strict: true` (structured outputs).
    OpenAiChatStrict,
    /// OpenAI Responses API: a flat function tool.
    OpenAiResponses,
    /// OpenAI Responses API with `strict: true` (structured outputs).
    OpenAiResponsesStrict,
    /// Gemini API: an element of `functionDeclarations` with `parameters`
    /// in the OpenAPI schema subset.
    Gemini,
    /// Model Context Protocol: an element of the `tools/list` result.
    Mcp,
    /// Neutral definition: name, description and a JSON Schema 2020-12
    /// document of the arguments.
    JsonSchema,
}

impl ToolFormat {
    /// Every format, in a stable order.
    pub const ALL: &'static [Self] = &[
        Self::Anthropic,
        Self::OpenAiChat,
        Self::OpenAiChatStrict,
        Self::OpenAiResponses,
        Self::OpenAiResponsesStrict,
        Self::Gemini,
        Self::Mcp,
        Self::JsonSchema,
    ];

    /// Returns the stable name of the format.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_tooldef::ToolFormat;
    ///
    /// assert_eq!(ToolFormat::OpenAiChatStrict.name(), "openai-strict");
    /// assert_eq!("gemini".parse::<ToolFormat>(), Ok(ToolFormat::Gemini));
    /// ```
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAiChat => "openai",
            Self::OpenAiChatStrict => "openai-strict",
            Self::OpenAiResponses => "openai-responses",
            Self::OpenAiResponsesStrict => "openai-responses-strict",
            Self::Gemini => "gemini",
            Self::Mcp => "mcp",
            Self::JsonSchema => "json-schema",
        }
    }
}

impl fmt::Display for ToolFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for ToolFormat {
    type Err = ToolDefError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|format| format.name() == name)
            .ok_or_else(|| ToolDefError::UnknownFormat {
                name: name.to_owned(),
                expected: Self::ALL
                    .iter()
                    .map(|format| format.name())
                    .collect::<Vec<_>>()
                    .join(", "),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for format in ToolFormat::ALL {
            assert_eq!(format.name().parse::<ToolFormat>(), Ok(*format));
            assert_eq!(format.to_string(), format.name());
        }
    }

    #[test]
    fn rejects_unknown_name_with_the_list_of_known_names() {
        let error = "claude".parse::<ToolFormat>().unwrap_err();
        assert_eq!(
            error.to_string(),
            "unknown tool format `claude`; expected one of anthropic, openai, openai-strict, \
             openai-responses, openai-responses-strict, gemini, mcp, json-schema"
        );
    }
}
