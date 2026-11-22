// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Protocol files: settings, system message and tool definitions.
//!
//! The directory `bench/protocol/` holds `protocol.toml`,
//! `system-message.txt` and one tool definition per tool in `tools/`. The
//! system message is the same in every condition; only the tool differs.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::condition::Condition;
use crate::error::{BenchError, Result};
use crate::task::HostOs;

/// Format identifier of `protocol.toml`.
pub const PROTOCOL_FORMAT: &str = "nurt-bench-protocol/1";

/// Format identifier of a tool definition.
pub const TOOL_FORMAT: &str = "nurt-bench-tool/1";

/// Settings of `protocol.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Format identifier, [`PROTOCOL_FORMAT`].
    pub format: String,
    /// Version of the protocol.
    pub version: String,
    /// Largest number of characters of a tool result shown to the model;
    /// longer results keep their beginning and end.
    pub max_tool_output_chars: usize,
}

/// Tool definition of `tools/<name>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDefinition {
    /// Format identifier, [`TOOL_FORMAT`].
    pub format: String,
    /// Name of the tool shown to the model.
    pub name: String,
    /// Description shown to the model; absent for Danaco Nurt, whose
    /// description comes from the server (`describe`).
    #[serde(default)]
    pub description: Option<String>,
    /// JSON Schema of the arguments.
    pub input_schema: Value,
    /// Name of the argument that carries the command or the program.
    pub argument: String,
}

/// Loaded protocol.
#[derive(Debug, Clone, PartialEq)]
pub struct Protocol {
    /// Settings.
    pub settings: Settings,
    /// System message template with the placeholders `{os}` and
    /// `{max_steps}`.
    pub system_message: String,
    /// Tool definitions by file name.
    pub tools: BTreeMap<String, ToolDefinition>,
}

impl Protocol {
    /// Reads the protocol directory.
    ///
    /// # Errors
    ///
    /// Returns an error when a file is missing or invalid.
    pub fn load(directory: &Path) -> Result<Self> {
        let path = directory.join("protocol.toml");
        let text = fs::read_to_string(&path).map_err(|error| BenchError::io(&path, error))?;
        let settings: Settings = toml::from_str(&text)
            .map_err(|error| BenchError::format(&path, "protocol settings", error.to_string()))?;
        if settings.format != PROTOCOL_FORMAT {
            return Err(BenchError::format(
                &path,
                "protocol settings",
                format!("format must be {PROTOCOL_FORMAT}"),
            ));
        }
        let message = directory.join("system-message.txt");
        let system_message =
            fs::read_to_string(&message).map_err(|error| BenchError::io(&message, error))?;
        let mut tools = BTreeMap::new();
        for condition in Condition::ALL {
            let name = condition.tool_file();
            if tools.contains_key(name) {
                continue;
            }
            let path = directory.join("tools").join(format!("{name}.json"));
            let value = crate::snapshot::read_json(&path)?;
            let tool: ToolDefinition = serde_json::from_value(value)
                .map_err(|error| BenchError::format(&path, "tool definition", error.to_string()))?;
            if tool.format != TOOL_FORMAT {
                return Err(BenchError::format(
                    &path,
                    "tool definition",
                    format!("format must be {TOOL_FORMAT}"),
                ));
            }
            if tool.description.is_none() != condition.is_nurt() {
                return Err(BenchError::format(
                    &path,
                    "tool definition",
                    "a shell tool needs a description; the Danaco Nurt tool takes it from the server",
                ));
            }
            tools.insert(name.to_owned(), tool);
        }
        Ok(Self {
            settings,
            system_message,
            tools,
        })
    }

    /// Tool definition of a condition.
    ///
    /// # Errors
    ///
    /// Returns [`BenchError::Argument`] when the definition is missing.
    pub fn tool(&self, condition: Condition) -> Result<&ToolDefinition> {
        self.tools
            .get(condition.tool_file())
            .ok_or_else(|| BenchError::Argument(format!("no tool definition for {condition}")))
    }

    /// System message for a system and a step limit.
    #[must_use]
    pub fn system_message(&self, host: HostOs, max_steps: u32) -> String {
        self.system_message
            .replace("{os}", host.display_name())
            .replace("{max_steps}", &max_steps.to_string())
    }

    /// Shortens a tool result to the limit of the protocol, keeping its
    /// beginning and its end and naming the number of omitted characters.
    #[must_use]
    pub fn clip_output(&self, text: &str) -> String {
        clip(text, self.settings.max_tool_output_chars)
    }
}

/// Shortens a text to `limit` characters, keeping the first and the last
/// half and naming the number of omitted characters.
#[must_use]
pub fn clip(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.to_owned();
    }
    let head = limit / 2;
    let tail = limit - head;
    let start: String = text.chars().take(head).collect();
    let end: String = text.chars().skip(count - tail).collect();
    format!("{start}\n[{} characters omitted]\n{end}", count - limit)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    /// Writes a minimal protocol directory.
    pub(crate) fn write_protocol(directory: &Path) {
        fs::create_dir_all(directory.join("tools")).unwrap();
        fs::write(
            directory.join("protocol.toml"),
            "format = \"nurt-bench-protocol/1\"\nversion = \"1.0.0\"\nmax_tool_output_chars = 2000\n",
        )
        .unwrap();
        fs::write(
            directory.join("system-message.txt"),
            "You work on {os}. At most {max_steps} steps.",
        )
        .unwrap();
        for (name, argument, description) in [
            ("bash", "command", Some("Runs a bash command.")),
            ("pwsh", "command", Some("Runs a PowerShell command.")),
            ("nurt", "program", None),
        ] {
            let tool = json!({
                "format": "nurt-bench-tool/1",
                "name": name,
                "description": description,
                "input_schema": {"type": "object", "properties": {argument: {"type": "string"}}, "required": [argument]},
                "argument": argument,
            });
            fs::write(
                directory.join("tools").join(format!("{name}.json")),
                tool.to_string(),
            )
            .unwrap();
        }
    }

    #[test]
    fn loads_the_protocol_directory() {
        let dir = tempfile::tempdir().unwrap();
        write_protocol(dir.path());
        let protocol = Protocol::load(dir.path()).unwrap();
        assert_eq!(protocol.settings.version, "1.0.0");
        assert_eq!(
            protocol.tool(Condition::NurtGroups).unwrap().argument,
            "program"
        );
        assert_eq!(
            protocol.system_message(HostOs::Windows, 15),
            "You work on Windows. At most 15 steps."
        );
    }

    #[test]
    fn rejects_a_nurt_tool_with_a_fixed_description() {
        let dir = tempfile::tempdir().unwrap();
        write_protocol(dir.path());
        let path = dir.path().join("tools/nurt.json");
        let mut tool = crate::snapshot::read_json(&path).unwrap();
        tool["description"] = json!("fixed");
        fs::write(&path, tool.to_string()).unwrap();
        assert!(Protocol::load(dir.path()).is_err());
    }

    #[test]
    fn rejects_foreign_formats_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Protocol::load(dir.path()).is_err());
        write_protocol(dir.path());
        fs::write(
            dir.path().join("protocol.toml"),
            "format = \"x\"\nversion = \"1\"\nmax_tool_output_chars = 1\n",
        )
        .unwrap();
        assert!(Protocol::load(dir.path()).is_err());
        write_protocol(dir.path());
        let path = dir.path().join("tools/bash.json");
        let mut tool = crate::snapshot::read_json(&path).unwrap();
        tool["format"] = json!("x");
        fs::write(&path, tool.to_string()).unwrap();
        assert!(Protocol::load(dir.path()).is_err());
    }

    #[test]
    fn clips_long_output() {
        assert_eq!(clip("short", 10), "short");
        assert_eq!(clip("abcdefghij", 4), "ab\n[6 characters omitted]\nij");
        assert_eq!(clip("αβγδεζηθικ", 4), "αβ\n[6 characters omitted]\nικ");
    }
}
