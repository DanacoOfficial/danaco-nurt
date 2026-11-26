// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Adapter of the OpenAI Chat Completions API, also used for servers with a
//! compatible API.

use serde_json::{Value, json};

use super::http::HttpClient;
use super::{
    ChatRequest, ChatResponse, Message, ModelClient, ModelConfig, Part, Provider, Role, StopReason,
    ToolCall, Usage, input_text,
};
use crate::error::Result;

pub(crate) struct OpenAiClient {
    config: ModelConfig,
    base: String,
    key: Option<String>,
    http: HttpClient,
}

impl OpenAiClient {
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
}

fn encode_message(message: &Message, tool_name: &str, out: &mut Vec<Value>) {
    let mut text = String::new();
    let mut calls = Vec::new();
    for part in &message.parts {
        match part {
            Part::Text { text: part } => text.push_str(part),
            Part::ToolCall { id, input } => calls.push(json!({
                "id": id,
                "type": "function",
                "function": {"name": tool_name, "arguments": input_text(input)},
            })),
            Part::ToolResult { id, output, .. } => {
                out.push(json!({"role": "tool", "tool_call_id": id, "content": output}));
            }
        }
    }
    match message.role {
        Role::Model => {
            let mut entry = json!({"role": "assistant", "content": text});
            if !calls.is_empty() {
                entry["tool_calls"] = Value::Array(calls);
            }
            out.push(entry);
        }
        Role::User if !text.is_empty() => out.push(json!({"role": "user", "content": text})),
        Role::User => {}
    }
}

/// Builds the body of a Chat Completions request.
pub(crate) fn encode(config: &ModelConfig, request: &ChatRequest<'_>) -> Value {
    let mut messages = vec![json!({"role": "system", "content": request.system})];
    for message in request.messages {
        encode_message(message, &request.tool.name, &mut messages);
    }
    let limit_field = match config.provider {
        Provider::Openai => "max_completion_tokens",
        _ => "max_tokens",
    };
    let mut body = json!({
        "model": config.model,
        "temperature": config.temperature,
        "seed": request.seed,
        "messages": messages,
        "tools": [{
            "type": "function",
            "function": {
                "name": request.tool.name,
                "description": request.tool.description,
                "parameters": request.tool.input_schema,
            },
        }],
    });
    body[limit_field] = json!(config.max_output_tokens);
    body
}

fn decode_arguments(raw: &Value) -> Value {
    match raw {
        Value::String(text) => {
            serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.clone()))
        }
        other => other.clone(),
    }
}

/// Reads the response of the Chat Completions API.
pub(crate) fn decode(body: &Value) -> std::result::Result<ChatResponse, String> {
    let choice = body["choices"]
        .get(0)
        .ok_or_else(|| "the response has no choices".to_owned())?;
    let message = &choice["message"];
    let tool_calls = message["tool_calls"]
        .as_array()
        .map(|calls| {
            calls
                .iter()
                .map(|call| ToolCall {
                    id: call["id"].as_str().unwrap_or_default().to_owned(),
                    input: decode_arguments(&call["function"]["arguments"]),
                })
                .collect()
        })
        .unwrap_or_default();
    let stop = match choice["finish_reason"].as_str() {
        Some("tool_calls" | "function_call") => StopReason::ToolUse,
        Some("stop") => StopReason::End,
        Some("length") => StopReason::Length,
        _ => StopReason::Other,
    };
    Ok(ChatResponse {
        text: message["content"].as_str().unwrap_or_default().to_owned(),
        tool_calls,
        usage: Usage {
            input: body["usage"]["prompt_tokens"].as_u64().unwrap_or_default(),
            output: body["usage"]["completion_tokens"]
                .as_u64()
                .unwrap_or_default(),
        },
        stop,
    })
}

impl ModelClient for OpenAiClient {
    fn complete(&mut self, request: &ChatRequest<'_>) -> Result<ChatResponse> {
        let url = format!("{}/v1/chat/completions", self.base);
        let headers: Vec<(&str, String)> = self
            .key
            .iter()
            .map(|key| ("authorization", format!("Bearer {key}")))
            .collect();
        let body = self
            .http
            .post(&url, &headers, &encode(&self.config, request))?;
        decode(&body).map_err(|message| self.http.error(message, false))
    }

    fn count_tokens(&mut self, _text: &str) -> Result<Option<u64>> {
        Ok(None)
    }

    fn set_time_limit(&mut self, remaining: std::time::Duration) {
        self.http.set_time_limit(Some(remaining));
    }
}

#[cfg(test)]
mod tests {
    use super::super::ToolSpec;
    use super::super::http::testserver;
    use super::super::tests::config;
    use super::*;

    #[test]
    fn talks_to_the_chat_completions_api() {
        let reply = json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1", "type": "function",
                        "function": {"name": "nurt", "arguments": "{\"program\":\"find \\\".\\\"\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 300, "completion_tokens": 20}
        });
        let (base, log) = testserver::serve(vec![(200, reply.to_string())]);
        let model = config("openai", &base);
        let http = HttpClient::new(model.http_settings());
        let mut client = OpenAiClient::new(model, base, Some("k".into()), http);
        let tool = ToolSpec {
            name: "nurt".into(),
            description: "Runs a program.".into(),
            input_schema: json!({"type": "object"}),
        };
        let messages = vec![
            Message::user_text("Find files."),
            Message {
                role: Role::Model,
                parts: vec![
                    Part::Text {
                        text: "Looking.".into(),
                    },
                    Part::ToolCall {
                        id: "call_0".into(),
                        input: json!({"program": "info \".\""}),
                    },
                ],
            },
            Message {
                role: Role::User,
                parts: vec![Part::ToolResult {
                    id: "call_0".into(),
                    output: "dir".into(),
                    is_error: false,
                }],
            },
        ];
        let response = client
            .complete(&ChatRequest {
                system: "System.",
                tool: &tool,
                messages: &messages,
                seed: 3,
            })
            .unwrap();
        assert_eq!(response.text, "");
        assert_eq!(response.tool_calls[0].input["program"], "find \".\"");
        assert_eq!(
            response.usage,
            Usage {
                input: 300,
                output: 20
            }
        );
        assert_eq!(response.stop, StopReason::ToolUse);
        assert_eq!(client.count_tokens("x").unwrap(), None);
        let log = log.lock().unwrap();
        assert_eq!(log[0].path, "/v1/chat/completions");
        assert_eq!(log[0].header("authorization"), Some("Bearer k"));
        let body = &log[0].body;
        assert_eq!(body["seed"], 3);
        assert_eq!(body["max_completion_tokens"], 512);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(
            body["messages"][2]["tool_calls"][0]["function"]["name"],
            "nurt"
        );
        assert_eq!(body["messages"][3]["role"], "tool");
        assert_eq!(body["messages"][3]["tool_call_id"], "call_0");
    }

    #[test]
    fn compatible_servers_use_max_tokens() {
        let model = config("openai-compatible", "http://127.0.0.1");
        let tool = ToolSpec {
            name: "bash".into(),
            description: String::new(),
            input_schema: json!({}),
        };
        let body = encode(
            &model,
            &ChatRequest {
                system: "",
                tool: &tool,
                messages: &[],
                seed: 1,
            },
        );
        assert_eq!(body["max_tokens"], 512);
        assert!(body.get("max_completion_tokens").is_none());
    }

    #[test]
    fn maps_finish_reasons_and_keeps_invalid_arguments_as_text() {
        let reply = json!({"choices": [{"message": {"content": "done", "tool_calls": [
            {"id": "c", "function": {"arguments": "not json"}}
        ]}, "finish_reason": "stop"}]});
        let response = decode(&reply).unwrap();
        assert_eq!(response.stop, StopReason::End);
        assert_eq!(response.tool_calls[0].input, json!("not json"));
        let length = decode(&json!({"choices": [{"message": {}, "finish_reason": "length"}]}));
        assert_eq!(length.unwrap().stop, StopReason::Length);
        let other = decode(&json!({"choices": [{"message": {}}]})).unwrap();
        assert_eq!(other.stop, StopReason::Other);
        assert!(decode(&json!({"choices": []})).is_err());
    }
}
