// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Models: configuration, a provider-neutral conversation and the adapters
//! of the providers.
//!
//! A model is selected by a configuration file `nurt-bench-model/1` (TOML)
//! or by `mock:<file>` for the mock model. API keys are read only from the
//! environment variable named in the configuration.

mod anthropic;
mod gemini;
mod http;
pub mod mock;
mod openai;

use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::condition::Condition;
use crate::error::{BenchError, Result};

/// Format identifier of a model configuration.
pub const MODEL_FORMAT: &str = "nurt-bench-model/1";

/// Prefix of the `--model` argument that selects the mock model.
pub const MOCK_PREFIX: &str = "mock:";

/// Author of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// The user: the task and the tool results.
    User,
    /// The model.
    Model,
}

/// Part of a message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    /// Text.
    Text {
        /// The text.
        text: String,
    },
    /// Call of the tool by the model.
    ToolCall {
        /// Identifier of the call.
        id: String,
        /// Arguments of the call.
        input: Value,
    },
    /// Result of a tool call.
    ToolResult {
        /// Identifier of the call.
        id: String,
        /// Text returned to the model.
        output: String,
        /// Whether the call failed.
        is_error: bool,
    },
}

/// Message of the conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// Author.
    pub role: Role,
    /// Parts in order.
    pub parts: Vec<Part>,
}

impl Message {
    /// Creates a user message with one text part.
    #[must_use]
    pub fn user_text(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            parts: vec![Part::Text { text: text.into() }],
        }
    }
}

/// Definition of the single tool offered to the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    /// Name of the tool.
    pub name: String,
    /// Description shown to the model.
    pub description: String,
    /// JSON Schema of the arguments.
    pub input_schema: Value,
}

/// One request to a model.
#[derive(Debug, Clone, Copy)]
pub struct ChatRequest<'a> {
    /// System message.
    pub system: &'a str,
    /// The tool.
    pub tool: &'a ToolSpec,
    /// Conversation so far, starting with a user message.
    pub messages: &'a [Message],
    /// Seed of the run, passed to providers that accept one.
    pub seed: u64,
}

/// Call of the tool requested by the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Identifier of the call.
    pub id: String,
    /// Arguments.
    pub input: Value,
}

/// Token counts reported by the provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Input tokens.
    pub input: u64,
    /// Output tokens.
    pub output: u64,
}

/// Reason the model stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The model requested tool calls.
    ToolUse,
    /// The model finished its answer.
    End,
    /// The output token limit was reached.
    Length,
    /// Any other reason reported by the provider.
    Other,
}

/// Response of a model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatResponse {
    /// Text of the response.
    pub text: String,
    /// Requested tool calls, in order.
    pub tool_calls: Vec<ToolCall>,
    /// Token counts.
    pub usage: Usage,
    /// Reason the model stopped.
    pub stop: StopReason,
}

impl ChatResponse {
    /// The model message of the response for the conversation.
    #[must_use]
    pub fn to_message(&self) -> Message {
        let mut parts = Vec::new();
        if !self.text.is_empty() {
            parts.push(Part::Text {
                text: self.text.clone(),
            });
        }
        parts.extend(self.tool_calls.iter().map(|call| Part::ToolCall {
            id: call.id.clone(),
            input: call.input.clone(),
        }));
        Message {
            role: Role::Model,
            parts,
        }
    }
}

/// Client of one model for one run.
pub trait ModelClient: Send {
    /// Sends the conversation and returns the response of the model.
    ///
    /// # Errors
    ///
    /// Returns an error when the provider fails or returns an unreadable
    /// response.
    fn complete(&mut self, request: &ChatRequest<'_>) -> Result<ChatResponse>;

    /// Counts the tokens of a text with the tokenizer of the model, when the
    /// provider offers a counting interface.
    ///
    /// # Errors
    ///
    /// Returns an error when the provider fails.
    fn count_tokens(&mut self, text: &str) -> Result<Option<u64>>;
}

/// Identification of a run, used by the mock model to select its script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunKey {
    /// Task identifier.
    pub task: String,
    /// Condition.
    pub condition: Condition,
    /// Seed.
    pub seed: u64,
}

/// API of a model provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    /// Anthropic Messages API.
    Anthropic,
    /// OpenAI Chat Completions API.
    Openai,
    /// Google Gemini API.
    Gemini,
    /// Server with the OpenAI Chat Completions API, typically serving a
    /// model with open weights.
    OpenaiCompatible,
    /// Mock model of the benchmark.
    Mock,
}

impl Provider {
    /// Name of the provider as written in a configuration.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Openai => "openai",
            Self::Gemini => "gemini",
            Self::OpenaiCompatible => "openai-compatible",
            Self::Mock => "mock",
        }
    }

    const fn default_base_url(self) -> Option<&'static str> {
        match self {
            Self::Anthropic => Some("https://api.anthropic.com"),
            Self::Openai => Some("https://api.openai.com"),
            Self::Gemini => Some("https://generativelanguage.googleapis.com"),
            Self::OpenaiCompatible | Self::Mock => None,
        }
    }
}

/// Availability of the weights of a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Weights {
    /// Weights are published.
    Open,
    /// Weights are not published.
    Closed,
}

/// Configuration of a model, `nurt-bench-model/1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConfig {
    /// Format identifier, [`MODEL_FORMAT`].
    pub format: String,
    /// Label of the model in run logs and results.
    pub id: String,
    /// API of the provider.
    pub provider: Provider,
    /// Model name in the API of the provider.
    pub model: String,
    /// Organization that trained the model.
    pub producer: String,
    /// Availability of the weights.
    pub weights: Weights,
    /// Base address of the API; required for `openai-compatible`.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Environment variable holding the API key; may be absent only for
    /// `openai-compatible` servers without authentication.
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// Sampling temperature.
    pub temperature: f64,
    /// Largest number of output tokens of one response.
    pub max_output_tokens: u32,
    /// Time limit of one request in seconds.
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    /// Number of repeated attempts after a retryable failure.
    #[serde(default = "default_retries")]
    pub retries: u32,
    /// Delay before the first repeated attempt in milliseconds; doubled for
    /// every further attempt.
    #[serde(default = "default_retry_delay_ms")]
    pub retry_delay_ms: u64,
}

const fn default_timeout_seconds() -> u64 {
    300
}

const fn default_retries() -> u32 {
    4
}

const fn default_retry_delay_ms() -> u64 {
    2000
}

impl ModelConfig {
    /// Reads a configuration file.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read or is invalid.
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).map_err(|error| BenchError::io(path, error))?;
        let config: Self = toml::from_str(&text)
            .map_err(|error| BenchError::format(path, "model configuration", error.to_string()))?;
        config
            .validate()
            .map_err(|message| BenchError::format(path, "model configuration", message))?;
        Ok(config)
    }

    fn validate(&self) -> std::result::Result<(), String> {
        if self.format != MODEL_FORMAT {
            return Err(format!("format must be {MODEL_FORMAT}"));
        }
        if self.provider == Provider::Mock {
            return Err("select the mock model with --model mock:<file>".to_owned());
        }
        if self.base_url().is_none() {
            return Err(format!(
                "base_url is required for provider {}",
                self.provider.as_str()
            ));
        }
        if self.api_key_env.is_none() && self.provider != Provider::OpenaiCompatible {
            return Err("api_key_env is required".to_owned());
        }
        if !(0.0..=2.0).contains(&self.temperature) {
            return Err("temperature must be between 0 and 2".to_owned());
        }
        Ok(())
    }

    /// Base address of the API without a trailing `/`.
    #[must_use]
    pub fn base_url(&self) -> Option<String> {
        self.base_url
            .as_deref()
            .or(self.provider.default_base_url())
            .map(|url| url.trim_end_matches('/').to_owned())
    }

    fn api_key(&self) -> Result<Option<String>> {
        let Some(name) = &self.api_key_env else {
            return Ok(None);
        };
        std::env::var(name)
            .map(Some)
            .map_err(|_| BenchError::MissingKey(name.clone()))
    }

    fn http_settings(&self) -> http::Settings {
        http::Settings {
            provider: self.provider.as_str(),
            timeout: Duration::from_secs(self.timeout_seconds),
            retries: self.retries,
            retry_delay: Duration::from_millis(self.retry_delay_ms),
        }
    }
}

/// Description of a model recorded in run logs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Label of the model.
    pub id: String,
    /// API of the provider.
    pub provider: Provider,
    /// Model name in the API.
    pub model: String,
    /// Organization that trained the model.
    pub producer: String,
    /// Availability of the weights.
    pub weights: Weights,
    /// Sampling temperature.
    pub temperature: f64,
    /// Largest number of output tokens of one response.
    pub max_output_tokens: u32,
}

/// Model selected for a series of runs.
#[derive(Debug, Clone)]
pub enum ModelSpec {
    /// Model of a provider.
    Remote(ModelConfig),
    /// Mock model.
    Mock(mock::MockModel),
}

impl ModelSpec {
    /// Reads the model given by a `--model` argument: a configuration file
    /// or `mock:<file>`.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read or is invalid.
    pub fn from_argument(argument: &str) -> Result<Self> {
        match argument.strip_prefix(MOCK_PREFIX) {
            Some(path) => Ok(Self::Mock(mock::MockModel::load(Path::new(path))?)),
            None => Ok(Self::Remote(ModelConfig::load(Path::new(argument))?)),
        }
    }

    /// Description of the model for run logs.
    #[must_use]
    pub fn info(&self) -> ModelInfo {
        match self {
            Self::Remote(config) => ModelInfo {
                id: config.id.clone(),
                provider: config.provider,
                model: config.model.clone(),
                producer: config.producer.clone(),
                weights: config.weights,
                temperature: config.temperature,
                max_output_tokens: config.max_output_tokens,
            },
            Self::Mock(model) => model.info(),
        }
    }

    /// Opens a client for one run.
    ///
    /// # Errors
    ///
    /// Returns an error when the API key is missing.
    pub fn open(&self, key: &RunKey) -> Result<Box<dyn ModelClient>> {
        match self {
            Self::Mock(model) => Ok(Box::new(model.client(key))),
            Self::Remote(config) => {
                let key = config.api_key()?;
                let base = config.base_url().unwrap_or_default();
                let http = http::HttpClient::new(config.http_settings());
                Ok(match config.provider {
                    Provider::Anthropic => Box::new(anthropic::AnthropicClient::new(
                        config.clone(),
                        base,
                        key,
                        http,
                    )),
                    Provider::Gemini => {
                        Box::new(gemini::GeminiClient::new(config.clone(), base, key, http))
                    }
                    Provider::Openai | Provider::OpenaiCompatible | Provider::Mock => {
                        Box::new(openai::OpenAiClient::new(config.clone(), base, key, http))
                    }
                })
            }
        }
    }
}

/// Text of a tool call argument object as one line of JSON.
pub(crate) fn input_text(input: &Value) -> String {
    match input {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn config(provider: &str, base_url: &str) -> ModelConfig {
        toml::from_str(&format!(
            r#"
            format = "nurt-bench-model/1"
            id = "test-model"
            provider = "{provider}"
            model = "m-1"
            producer = "Example"
            weights = "open"
            base_url = "{base_url}"
            api_key_env = "NURT_BENCH_TEST_KEY"
            temperature = 0.0
            max_output_tokens = 512
            retries = 1
            retry_delay_ms = 0
            "#
        ))
        .unwrap()
    }

    #[test]
    fn validates_configuration() {
        let mut ok = config("anthropic", "https://example.com/");
        assert!(ok.validate().is_ok());
        assert_eq!(ok.base_url().as_deref(), Some("https://example.com"));
        ok.format = "x".into();
        assert!(ok.validate().is_err());
        let mut no_key = config("openai", "https://example.com");
        no_key.api_key_env = None;
        assert!(no_key.validate().is_err());
        let mut compatible = config("openai-compatible", "https://example.com");
        compatible.api_key_env = None;
        assert!(compatible.validate().is_ok());
        compatible.base_url = None;
        assert!(compatible.validate().is_err());
        let mut hot = config("gemini", "https://example.com");
        hot.temperature = 3.0;
        assert!(hot.validate().is_err());
        assert!(config("mock", "https://example.com").validate().is_err());
    }

    #[test]
    fn default_base_urls_of_providers() {
        let mut anthropic = config("anthropic", "x");
        anthropic.base_url = None;
        assert_eq!(
            anthropic.base_url().as_deref(),
            Some("https://api.anthropic.com")
        );
    }

    #[test]
    fn loads_configuration_file_and_reports_missing_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.toml");
        let mut model = config("anthropic", "https://example.com");
        model.api_key_env = Some("NURT_BENCH_TEST_UNSET_KEY".into());
        fs::write(&path, toml::to_string(&model).unwrap()).unwrap();
        let spec = ModelSpec::from_argument(path.to_str().unwrap()).unwrap();
        assert_eq!(spec.info().id, "test-model");
        let key = RunKey {
            task: "t".into(),
            condition: Condition::Bash,
            seed: 1,
        };
        let error = spec.open(&key).err().unwrap();
        assert!(matches!(error, BenchError::MissingKey(_)));
        fs::write(&path, "format = 1").unwrap();
        assert!(ModelSpec::from_argument(path.to_str().unwrap()).is_err());
    }

    #[test]
    fn response_becomes_model_message() {
        let response = ChatResponse {
            text: "ok".into(),
            tool_calls: vec![ToolCall {
                id: "c1".into(),
                input: serde_json::json!({"command": "ls"}),
            }],
            usage: Usage::default(),
            stop: StopReason::ToolUse,
        };
        let message = response.to_message();
        assert_eq!(message.role, Role::Model);
        assert_eq!(message.parts.len(), 2);
        assert_eq!(input_text(&serde_json::json!("x")), "x");
        assert_eq!(input_text(&serde_json::json!({"a": 1})), r#"{"a":1}"#);
    }
}
