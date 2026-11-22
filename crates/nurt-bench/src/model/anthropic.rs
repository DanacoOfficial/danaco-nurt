// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Adapter of the Anthropic Messages API.

use serde_json::{Value, json};

use super::http::HttpClient;
use super::{
    ChatRequest, ChatResponse, Message, ModelClient, ModelConfig, Part, Role, StopReason, ToolCall,
    Usage,
};
use crate::error::Result;

/// Version of the Messages API.
const API_VERSION: &str = "2023-06-01";

pub(crate) struct AnthropicClient {
    config: ModelConfig,
    base: String,
    key: Option<String>,
    http: HttpClient,
}

impl AnthropicClient {
    pub(crate) fn new(
        config: ModelConfig,
        base: String,
        key: Option<String>,
        http: HttpClient,
    ) -> Self {
        Self {
            config,
            base,
            key,
            http,
        }
    }

    fn headers(&self) -> Vec<(&'static str, String)> {
        let mut headers = vec![("anthropic-version", API_VERSION.to_owned())];
        if let Some(key) = &self.key {
            headers.push(("x-api-key", key.clone()));
        }
        headers
    }
}

fn encode_message(message: &Message, tool_name: &str) -> Value {
    let content: Vec<Value> = message
        .parts
        .iter()
        .map(|part| match part {
            Part::Text { text } => json!({"type": "text", "text": text}),
            Part::ToolCall { id, input } => {
                json!({"type": "tool_use", "id": id, "name": tool_name, "input": input})
            }
            Part::ToolResult {
                id,
                output,
                is_error,
            } => json!({
                "type": "tool_result", "tool_use_id": id, "content": output, "is_error": is_error
            }),
        })
        .collect();
    let role = match message.role {
        Role::User => "user",
        Role::Model => "assistant",
    };
    json!({"role": role, "content": content})
}

/// Builds the body of a Messages API request.
pub(crate) fn encode(config: &ModelConfig, request: &ChatRequest<'_>) -> Value {
    let messages: Vec<Value> = request
        .messages
        .iter()
        .map(|message| encode_message(message, &request.tool.name))
        .collect();
    json!({
        "model": config.model,
        "max_tokens": config.max_output_tokens,
        "temperature": config.temperature,
        "system": request.system,
        "tools": [{
            "name": request.tool.name,
            "description": request.tool.description,
            "input_schema": request.tool.input_schema,
        }],
        "messages": messages,
    })
}

/// Reads the response of the Messages API.
pub(crate) fn decode(body: &Value) -> std::result::Result<ChatResponse, String> {
    let content = body["content"]
        .as_array()
        .ok_or_else(|| "the response has no content".to_owned())?;
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for part in content {
        match part["type"].as_str() {
            Some("text") => text.push_str(part["text"].as_str().unwrap_or_default()),
            Some("tool_use") => tool_calls.push(ToolCall {
                id: part["id"].as_str().unwrap_or_default().to_owned(),
                input: part["input"].clone(),
            }),
            _ => {}
        }
    }
    let stop = match body["stop_reason"].as_str() {
        Some("tool_use") => StopReason::ToolUse,
        Some("end_turn" | "stop_sequence") => StopReason::End,
        Some("max_tokens") => StopReason::Length,
        _ => StopReason::Other,
    };
    Ok(ChatResponse {
        text,
        tool_calls,
        usage: Usage {
            input: body["usage"]["input_tokens"].as_u64().unwrap_or_default(),
            output: body["usage"]["output_tokens"].as_u64().unwrap_or_default(),
        },
        stop,
    })
}

impl ModelClient for AnthropicClient {
    fn complete(&mut self, request: &ChatRequest<'_>) -> Result<ChatResponse> {
        let url = format!("{}/v1/messages", self.base);
        let body = self
            .http
            .post(&url, &self.headers(), &encode(&self.config, request))?;
        decode(&body).map_err(|message| self.http.error(message, false))
    }

    fn count_tokens(&mut self, text: &str) -> Result<Option<u64>> {
        let url = format!("{}/v1/messages/count_tokens", self.base);
        let body = json!({
            "model": self.config.model,
            "messages": [{"role": "user", "content": text}],
        });
        let response = self.http.post(&url, &self.headers(), &body)?;
        Ok(response["input_tokens"].as_u64())
    }
}

#[cfg(test)]
mod tests {
    use super::super::http::testserver;
    use super::super::tests::config;
    use super::super::{ModelSpec, RunKey, ToolSpec};
    use super::*;
    use crate::condition::Condition;

    fn tool() -> ToolSpec {
        ToolSpec {
            name: "bash".into(),
            description: "Runs a command.".into(),
            input_schema: json!({"type": "object", "properties": {"command": {"type": "string"}}}),
        }
    }

    fn conversation() -> Vec<Message> {
        vec![
            Message::user_text("List files."),
            Message {
                role: Role::Model,
                parts: vec![Part::ToolCall {
                    id: "t1".into(),
                    input: json!({"command": "ls"}),
                }],
            },
            Message {
                role: Role::User,
                parts: vec![Part::ToolResult {
                    id: "t1".into(),
                    output: "a.txt".into(),
                    is_error: false,
                }],
            },
        ]
    }

    #[test]
    fn talks_to_the_messages_api() {
        let reply = json!({
            "content": [
                {"type": "text", "text": "Reading."},
                {"type": "tool_use", "id": "t2", "name": "bash", "input": {"command": "cat a.txt"}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 120, "output_tokens": 15}
        });
        let (base, log) = testserver::serve(vec![
            (200, reply.to_string()),
            (200, r#"{"input_tokens": 42}"#.into()),
        ]);
        let mut model = config("anthropic", &base);
        model.api_key_env = None;
        let spec = ModelSpec::Remote(model);
        let key = RunKey {
            task: "t".into(),
            condition: Condition::Bash,
            seed: 7,
        };
        let mut client = spec.open(&key).unwrap();
        let tool = tool();
        let messages = conversation();
        let response = client
            .complete(&ChatRequest {
                system: "System.",
                tool: &tool,
                messages: &messages,
                seed: 7,
            })
            .unwrap();
        assert_eq!(response.text, "Reading.");
        assert_eq!(response.tool_calls[0].input["command"], "cat a.txt");
        assert_eq!(
            response.usage,
            Usage {
                input: 120,
                output: 15
            }
        );
        assert_eq!(response.stop, StopReason::ToolUse);
        assert_eq!(client.count_tokens("text").unwrap(), Some(42));
        let log = log.lock().unwrap();
        assert_eq!(log[0].path, "/v1/messages");
        assert_eq!(log[0].header("anthropic-version"), Some(API_VERSION));
        let body = &log[0].body;
        assert_eq!(body["system"], "System.");
        assert_eq!(body["tools"][0]["name"], "bash");
        assert_eq!(body["messages"][1]["content"][0]["name"], "bash");
        assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "t1");
        assert_eq!(log[1].path, "/v1/messages/count_tokens");
    }

    #[test]
    fn maps_stop_reasons_and_rejects_missing_content() {
        let end = decode(&json!({"content": [], "stop_reason": "end_turn"})).unwrap();
        assert_eq!(end.stop, StopReason::End);
        let length = decode(&json!({"content": [], "stop_reason": "max_tokens"})).unwrap();
        assert_eq!(length.stop, StopReason::Length);
        let other = decode(&json!({"content": [{"type": "thinking"}]})).unwrap();
        assert_eq!(other.stop, StopReason::Other);
        assert!(decode(&json!({"error": {}})).is_err());
    }

    #[test]
    fn sends_the_api_key_header() {
        let reply = json!({"content": [], "stop_reason": "end_turn", "usage": {}});
        let (base, log) = testserver::serve(vec![(200, reply.to_string())]);
        let http = HttpClient::new(config("anthropic", &base).http_settings());
        let mut client = AnthropicClient::new(
            config("anthropic", &base),
            base,
            Some("secret".into()),
            http,
        );
        let tool = tool();
        let messages = vec![Message::user_text("x")];
        client
            .complete(&ChatRequest {
                system: "",
                tool: &tool,
                messages: &messages,
                seed: 1,
            })
            .unwrap();
        assert_eq!(log.lock().unwrap()[0].header("x-api-key"), Some("secret"));
    }
}
