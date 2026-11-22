// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Adapter of the Google Gemini API (`generateContent`).

use serde_json::{Value, json};

use super::http::HttpClient;
use super::{
    ChatRequest, ChatResponse, ModelClient, ModelConfig, Part, Role, StopReason, ToolCall, Usage,
};
use crate::error::Result;

pub(crate) struct GeminiClient {
    config: ModelConfig,
    base: String,
    key: Option<String>,
    http: HttpClient,
    calls: u64,
}

impl GeminiClient {
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
            calls: 0,
        }
    }

    fn headers(&self) -> Vec<(&'static str, String)> {
        self.key
            .iter()
            .map(|key| ("x-goog-api-key", key.clone()))
            .collect()
    }
}

fn encode_part(part: &Part, tool_name: &str) -> Value {
    match part {
        Part::Text { text } => json!({"text": text}),
        Part::ToolCall { id, input } => {
            json!({"functionCall": {"id": id, "name": tool_name, "args": input}})
        }
        Part::ToolResult {
            id,
            output,
            is_error,
        } => json!({"functionResponse": {
            "id": id,
            "name": tool_name,
            "response": {"output": output, "is_error": is_error},
        }}),
    }
}

/// Builds the body of a `generateContent` request.
pub(crate) fn encode(config: &ModelConfig, request: &ChatRequest<'_>) -> Value {
    let contents: Vec<Value> = request
        .messages
        .iter()
        .map(|message| {
            let role = match message.role {
                Role::User => "user",
                Role::Model => "model",
            };
            let parts: Vec<Value> = message
                .parts
                .iter()
                .map(|part| encode_part(part, &request.tool.name))
                .collect();
            json!({"role": role, "parts": parts})
        })
        .collect();
    json!({
        "systemInstruction": {"parts": [{"text": request.system}]},
        "contents": contents,
        "tools": [{"functionDeclarations": [{
            "name": request.tool.name,
            "description": request.tool.description,
            "parameters": request.tool.input_schema,
        }]}],
        "generationConfig": {
            "temperature": config.temperature,
            "seed": request.seed,
            "maxOutputTokens": config.max_output_tokens,
        },
    })
}

/// Reads the response of `generateContent`. Calls without an identifier get
/// `call-<n>`, numbered by `next_call`.
pub(crate) fn decode(
    body: &Value,
    next_call: &mut u64,
) -> std::result::Result<ChatResponse, String> {
    let candidate = body["candidates"]
        .get(0)
        .ok_or_else(|| "the response has no candidates".to_owned())?;
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for part in candidate["content"]["parts"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if let Some(fragment) = part["text"].as_str() {
            text.push_str(fragment);
        }
        if let Some(call) = part.get("functionCall") {
            *next_call += 1;
            let id = call["id"]
                .as_str()
                .map_or_else(|| format!("call-{next_call}"), str::to_owned);
            tool_calls.push(ToolCall {
                id,
                input: call["args"].clone(),
            });
        }
    }
    let stop = match candidate["finishReason"].as_str() {
        Some("STOP") if !tool_calls.is_empty() => StopReason::ToolUse,
        Some("STOP") => StopReason::End,
        Some("MAX_TOKENS") => StopReason::Length,
        _ => StopReason::Other,
    };
    let usage = &body["usageMetadata"];
    Ok(ChatResponse {
        text,
        tool_calls,
        usage: Usage {
            input: usage["promptTokenCount"].as_u64().unwrap_or_default(),
            output: usage["candidatesTokenCount"].as_u64().unwrap_or_default(),
        },
        stop,
    })
}

impl ModelClient for GeminiClient {
    fn complete(&mut self, request: &ChatRequest<'_>) -> Result<ChatResponse> {
        let url = format!(
            "{}/v1beta/models/{}:generateContent",
            self.base, self.config.model
        );
        let body = self
            .http
            .post(&url, &self.headers(), &encode(&self.config, request))?;
        decode(&body, &mut self.calls).map_err(|message| self.http.error(message, false))
    }

    fn count_tokens(&mut self, text: &str) -> Result<Option<u64>> {
        let url = format!(
            "{}/v1beta/models/{}:countTokens",
            self.base, self.config.model
        );
        let body = json!({"contents": [{"role": "user", "parts": [{"text": text}]}]});
        let response = self.http.post(&url, &self.headers(), &body)?;
        Ok(response["totalTokens"].as_u64())
    }
}

#[cfg(test)]
mod tests {
    use super::super::http::testserver;
    use super::super::tests::config;
    use super::super::{Message, ToolSpec};
    use super::*;

    #[test]
    fn talks_to_the_generate_content_api() {
        let reply = json!({
            "candidates": [{
                "content": {"role": "model", "parts": [
                    {"text": "Running."},
                    {"functionCall": {"name": "pwsh", "args": {"command": "Get-ChildItem"}}}
                ]},
                "finishReason": "STOP"
            }],
            "usageMetadata": {"promptTokenCount": 90, "candidatesTokenCount": 9}
        });
        let (base, log) = testserver::serve(vec![
            (200, reply.to_string()),
            (200, r#"{"totalTokens": 11}"#.into()),
        ]);
        let model = config("gemini", &base);
        let http = HttpClient::new(model.http_settings());
        let mut client = GeminiClient::new(model, base, Some("g".into()), http);
        let tool = ToolSpec {
            name: "pwsh".into(),
            description: "Runs PowerShell.".into(),
            input_schema: json!({"type": "object"}),
        };
        let messages = vec![
            Message::user_text("List."),
            Message {
                role: Role::Model,
                parts: vec![Part::ToolCall {
                    id: "call-0".into(),
                    input: json!({"command": "pwd"}),
                }],
            },
            Message {
                role: Role::User,
                parts: vec![Part::ToolResult {
                    id: "call-0".into(),
                    output: "C:/Users/user".into(),
                    is_error: false,
                }],
            },
        ];
        let response = client
            .complete(&ChatRequest {
                system: "System.",
                tool: &tool,
                messages: &messages,
                seed: 2,
            })
            .unwrap();
        assert_eq!(response.text, "Running.");
        assert_eq!(response.tool_calls[0].id, "call-1");
        assert_eq!(response.tool_calls[0].input["command"], "Get-ChildItem");
        assert_eq!(response.stop, StopReason::ToolUse);
        assert_eq!(
            response.usage,
            Usage {
                input: 90,
                output: 9
            }
        );
        assert_eq!(client.count_tokens("x").unwrap(), Some(11));
        let log = log.lock().unwrap();
        assert_eq!(log[0].path, "/v1beta/models/m-1:generateContent");
        assert_eq!(log[0].header("x-goog-api-key"), Some("g"));
        let body = &log[0].body;
        assert_eq!(body["generationConfig"]["seed"], 2);
        assert_eq!(body["contents"][1]["role"], "model");
        assert_eq!(
            body["contents"][2]["parts"][0]["functionResponse"]["name"],
            "pwsh"
        );
        assert_eq!(log[1].path, "/v1beta/models/m-1:countTokens");
    }

    #[test]
    fn maps_finish_reasons() {
        let mut calls = 0;
        let end = json!({"candidates": [{"content": {"parts": [{"text": "x"}]}, "finishReason": "STOP"}]});
        assert_eq!(decode(&end, &mut calls).unwrap().stop, StopReason::End);
        let length =
            json!({"candidates": [{"content": {"parts": []}, "finishReason": "MAX_TOKENS"}]});
        assert_eq!(
            decode(&length, &mut calls).unwrap().stop,
            StopReason::Length
        );
        let safety = json!({"candidates": [{"finishReason": "SAFETY"}]});
        assert_eq!(decode(&safety, &mut calls).unwrap().stop, StopReason::Other);
        let with_id = json!({"candidates": [{"content": {"parts": [
            {"functionCall": {"id": "own", "args": {}}}
        ]}, "finishReason": "STOP"}]});
        assert_eq!(
            decode(&with_id, &mut calls).unwrap().tool_calls[0].id,
            "own"
        );
        assert!(decode(&json!({}), &mut calls).is_err());
    }
}
