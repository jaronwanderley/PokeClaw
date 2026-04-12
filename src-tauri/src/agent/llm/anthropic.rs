// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

//! Anthropic Messages API provider.
//!
//! Implements the `LlmProvider` trait for Anthropic's Messages API, which has a
//! fundamentally different wire format from OpenAI:
//!
//! - Endpoint: `POST https://api.anthropic.com/v1/messages`
//! - Auth: `x-api-key` header + `anthropic-version: 2023-06-01`
//! - Tool defs use `input_schema` instead of `parameters`
//! - Tool calls are `content[]` blocks with `type: "tool_use"`
//! - Tool results are `user` role messages with `tool_result` content blocks
//! - `max_tokens` is **required** (default 4096)
//! - Stop reason: `"tool_use"` instead of `"tool_calls"`

use std::time::Duration;

use log::{debug, info, warn};
use reqwest::StatusCode;
use serde_json::{json, Value};

use super::llm_provider::{ChatMessage, LlmError, LlmProvider, LlmResponse, TokenUsage, ToolCall};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u32 = 4096;
const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 1000;
const REQUEST_TIMEOUT_SECS: u64 = 30;

// ---------------------------------------------------------------------------
// AnthropicProvider
// ---------------------------------------------------------------------------

/// Anthropic Messages API LLM provider using `reqwest`.
pub struct AnthropicProvider {
    api_key: String,
    model: String,
    client: reqwest::Client,
    max_tokens: u32,
}

impl AnthropicProvider {
    /// Create a new Anthropic provider with the given API key and model name.
    pub fn new(api_key: String, model: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .expect("reqwest client build should not fail");
        info!(
            "AnthropicProvider created: model={}, max_tokens={}",
            model, DEFAULT_MAX_TOKENS
        );
        Self {
            api_key,
            model,
            client,
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }

    /// Create a provider with a custom `max_tokens` value.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }
}

// ---------------------------------------------------------------------------
// Message conversion (ChatMessage → Anthropic JSON)
// ---------------------------------------------------------------------------

/// Convert our `ChatMessage` sequence into the Anthropic Messages API format.
///
/// Returns `(system_prompt, messages)` where:
/// - `system_prompt` is an optional `String` (Anthropic uses a top-level field).
/// - `messages` is a `Vec<Value>` of role/content objects.
///
/// Anthropic requires that tool results be sent as `user` role messages with
/// `tool_result` content blocks, and that assistant messages with tool calls
/// use `tool_use` content blocks.
fn convert_messages(messages: &[ChatMessage]) -> (Option<String>, Vec<Value>) {
    let mut system_prompt: Option<String> = None;
    let mut api_messages: Vec<Value> = Vec::new();

    for msg in messages {
        match msg {
            ChatMessage::System(text) => {
                // Anthropic uses a top-level `system` parameter, not a message.
                system_prompt = Some(text.clone());
            }
            ChatMessage::User(text) => {
                api_messages.push(json!({
                    "role": "user",
                    "content": text,
                }));
            }
            ChatMessage::Assistant(text) => {
                api_messages.push(json!({
                    "role": "assistant",
                    "content": text,
                }));
            }
            ChatMessage::AssistantWithTools { text, tool_calls } => {
                let mut content: Vec<Value> = Vec::new();

                // Add text block if present
                if let Some(ref t) = text {
                    content.push(json!({
                        "type": "text",
                        "text": t,
                    }));
                }

                // Add tool_use blocks
                for tc in tool_calls {
                    // Parse arguments as JSON Value; fall back to raw string
                    let args_value: Value = serde_json::from_str(&tc.arguments)
                        .unwrap_or_else(|_| Value::String(tc.arguments.clone()));
                    content.push(json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.name,
                        "input": args_value,
                    }));
                }

                api_messages.push(json!({
                    "role": "assistant",
                    "content": content,
                }));
            }
            ChatMessage::ToolResult {
                tool_call_id,
                content,
            } => {
                // Anthropic requires tool results as user messages with tool_result blocks
                // Try to parse content as JSON; otherwise send as plain text
                let result_content: Value = serde_json::from_str(content)
                    .unwrap_or_else(|_| Value::String(content.clone()));
                api_messages.push(json!({
                    "role": "user",
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": tool_call_id,
                            "content": result_content,
                        }
                    ],
                }));
            }
        }
    }

    (system_prompt, api_messages)
}

// ---------------------------------------------------------------------------
// Tool schema conversion
// ---------------------------------------------------------------------------

/// Convert a tool JSON Schema (from ToolRegistry) into the Anthropic tool format.
///
/// The key difference from OpenAI: Anthropic uses `input_schema` instead of
/// `parameters` for the JSON Schema of the tool's arguments.
///
/// Expected input format:
/// ```json
/// {
///   "name": "tap",
///   "description": "...",
///   "parameters": { "type": "object", "properties": {...} }
/// }
/// ```
///
/// Output format:
/// ```json
/// {
///   "name": "tap",
///   "description": "...",
///   "input_schema": { "type": "object", "properties": {...} }
/// }
/// ```
fn convert_tool_schema(schema: &Value) -> Option<Value> {
    let name = schema.get("name")?.as_str()?;
    let description = schema
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Rename "parameters" → "input_schema", defaulting to empty object
    let input_schema = schema
        .get("parameters")
        .cloned()
        .unwrap_or(json!({"type": "object", "properties": {}}));

    Some(json!({
        "name": name,
        "description": description,
        "input_schema": input_schema,
    }))
}

/// Convert all tool schemas to Anthropic format.
fn convert_tools(tool_schemas: &[Value]) -> Vec<Value> {
    tool_schemas
        .iter()
        .filter_map(|s| convert_tool_schema(s))
        .collect()
}

// ---------------------------------------------------------------------------
// Response parsing
// ---------------------------------------------------------------------------

/// Parse the Anthropic Messages API response into our `LlmResponse`.
fn parse_response(body: &Value) -> Result<LlmResponse, LlmError> {
    let content_blocks = body
        .get("content")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();

    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    for block in &content_blocks {
        let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match block_type {
            "text" => {
                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                    text_parts.push(text.to_string());
                }
            }
            "tool_use" => {
                let id = block
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                // Serialize `input` back to a JSON string for compatibility
                let arguments = block
                    .get("input")
                    .map(|v| serde_json::to_string(v).unwrap_or_default())
                    .unwrap_or_default();

                tool_calls.push(ToolCall {
                    id,
                    name,
                    arguments,
                });
            }
            _ => {
                // Skip unknown block types
                debug!("Anthropic: skipping unknown content block type: {}", block_type);
            }
        }
    }

    let text = if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join(""))
    };

    let usage = body.get("usage").and_then(|u| {
        let prompt = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let completion = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        if prompt > 0 || completion > 0 {
            Some(TokenUsage {
                prompt_tokens: prompt,
                completion_tokens: completion,
            })
        } else {
            None
        }
    });

    Ok(LlmResponse {
        text,
        tool_calls,
        usage,
    })
}

// ---------------------------------------------------------------------------
// Error classification
// ---------------------------------------------------------------------------

/// Classify an HTTP status code + error body into our `LlmError` variants.
fn classify_http_error(status: StatusCode, body: &str) -> LlmError {
    match status.as_u16() {
        401 | 403 => LlmError::AuthFailed,
        429 => LlmError::RateLimit,
        408 | 504 => LlmError::Timeout,
        _ if body.contains("timeout") || body.contains("timed out") => LlmError::Timeout,
        _ if body.contains("rate limit") || body.contains("Rate limit") => LlmError::RateLimit,
        _ if status.is_server_error() => {
            LlmError::ApiError(format!("Server error {}: {}", status, body))
        }
        _ => LlmError::ApiError(format!("HTTP {}: {}", status, body)),
    }
}

/// Check if an HTTP error is retryable (429, 500, 502, 503).
fn is_retryable_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 500 | 502 | 503)
}

// ---------------------------------------------------------------------------
// LlmProvider implementation
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl LlmProvider for AnthropicProvider {
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<Value>,
    ) -> Result<LlmResponse, LlmError> {
        let msg_count = messages.len();
        let tool_count = tools.len();
        info!(
            "AnthropicProvider.chat: model={}, messages={}, tools={}",
            self.model, msg_count, tool_count
        );

        let (system_prompt, api_messages) = convert_messages(&messages);
        let anthropic_tools = convert_tools(&tools);

        // Build the request body
        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "messages": api_messages,
        });

        if let Some(sys) = system_prompt {
            body["system"] = json!(sys);
        }

        if !anthropic_tools.is_empty() {
            body["tools"] = json!(anthropic_tools);
        }

        debug!(
            "AnthropicProvider: sending request to model '{}' with {} messages",
            self.model, msg_count
        );

        // Retry loop for transient errors
        let mut last_error: Option<LlmError> = None;
        for attempt in 0..=MAX_RETRIES {
            let result = self
                .client
                .post(ANTHROPIC_API_URL)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await;

            match result {
                Ok(response) => {
                    let status = response.status();

                    if status.is_success() {
                        let resp_body: Value = response
                            .json()
                            .await
                            .map_err(|e| LlmError::ApiError(format!("Failed to parse response: {}", e)))?;

                        let parsed = parse_response(&resp_body)?;

                        info!(
                            "AnthropicProvider response: text={}, tool_calls={}, usage={:?}",
                            parsed.text.is_some(),
                            parsed.tool_calls.len(),
                            parsed.usage
                        );

                        return Ok(parsed);
                    }

                    // HTTP error
                    let error_text = response.text().await.unwrap_or_else(|e| {
                        format!("<failed to read error body: {}>", e)
                    });

                    let classified = classify_http_error(status, &error_text);

                    if !is_retryable_status(status) || attempt >= MAX_RETRIES {
                        warn!(
                            "AnthropicProvider error (attempt {}/{}): {} - {}",
                            attempt + 1,
                            MAX_RETRIES + 1,
                            status,
                            error_text
                        );
                        return Err(classified);
                    }

                    let backoff_ms = INITIAL_BACKOFF_MS * 2u64.pow(attempt);
                    warn!(
                        "AnthropicProvider retryable error (attempt {}/{}): {}, retrying in {}ms",
                        attempt + 1,
                        MAX_RETRIES + 1,
                        classified,
                        backoff_ms
                    );
                    last_error = Some(classified);
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                }
                Err(reqwest_err) => {
                    // Network / timeout error from reqwest
                    if reqwest_err.is_timeout() {
                        if attempt >= MAX_RETRIES {
                            warn!(
                                "AnthropicProvider timeout after {} attempts",
                                MAX_RETRIES + 1
                            );
                            return Err(LlmError::Timeout);
                        }
                        let backoff_ms = INITIAL_BACKOFF_MS * 2u64.pow(attempt);
                        warn!(
                            "AnthropicProvider timeout (attempt {}/{}), retrying in {}ms",
                            attempt + 1,
                            MAX_RETRIES + 1,
                            backoff_ms
                        );
                        last_error = Some(LlmError::Timeout);
                        tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                    } else if reqwest_err.is_connect() {
                        let msg = reqwest_err.to_string();
                        return Err(LlmError::Network(msg));
                    } else {
                        let msg = reqwest_err.to_string();
                        return Err(LlmError::Network(msg));
                    }
                }
            }
        }

        // Should be unreachable, but just in case
        Err(last_error.unwrap_or(LlmError::ApiError(
            "Max retries exceeded with no error recorded".to_string(),
        )))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // === Message conversion tests ===

    #[test]
    fn test_convert_system_message() {
        let messages = vec![ChatMessage::System("You are helpful.".to_string())];
        let (system, api_msgs) = convert_messages(&messages);
        assert_eq!(system, Some("You are helpful.".to_string()));
        assert!(api_msgs.is_empty());
    }

    #[test]
    fn test_convert_user_message() {
        let messages = vec![ChatMessage::User("Hello".to_string())];
        let (system, api_msgs) = convert_messages(&messages);
        assert!(system.is_none());
        assert_eq!(api_msgs.len(), 1);
        assert_eq!(api_msgs[0]["role"], "user");
        assert_eq!(api_msgs[0]["content"], "Hello");
    }

    #[test]
    fn test_convert_assistant_message() {
        let messages = vec![ChatMessage::Assistant("Hi there".to_string())];
        let (_, api_msgs) = convert_messages(&messages);
        assert_eq!(api_msgs.len(), 1);
        assert_eq!(api_msgs[0]["role"], "assistant");
        assert_eq!(api_msgs[0]["content"], "Hi there");
    }

    #[test]
    fn test_convert_assistant_with_tools_message() {
        let messages = vec![ChatMessage::AssistantWithTools {
            text: Some("I'll tap it.".to_string()),
            tool_calls: vec![ToolCall {
                id: "toolu_1".to_string(),
                name: "tap".to_string(),
                arguments: r#"{"x":100,"y":200}"#.to_string(),
            }],
        }];
        let (_, api_msgs) = convert_messages(&messages);
        assert_eq!(api_msgs.len(), 1);
        assert_eq!(api_msgs[0]["role"], "assistant");

        let content = api_msgs[0]["content"].as_array().expect("should be array");
        assert_eq!(content.len(), 2); // text block + tool_use block

        // First block: text
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "I'll tap it.");

        // Second block: tool_use
        assert_eq!(content[1]["type"], "tool_use");
        assert_eq!(content[1]["id"], "toolu_1");
        assert_eq!(content[1]["name"], "tap");
        assert_eq!(content[1]["input"]["x"], 100);
        assert_eq!(content[1]["input"]["y"], 200);
    }

    #[test]
    fn test_convert_assistant_with_tools_no_text() {
        let messages = vec![ChatMessage::AssistantWithTools {
            text: None,
            tool_calls: vec![ToolCall {
                id: "toolu_2".to_string(),
                name: "finish".to_string(),
                arguments: r#"{"result":"done"}"#.to_string(),
            }],
        }];
        let (_, api_msgs) = convert_messages(&messages);
        let content = api_msgs[0]["content"].as_array().expect("should be array");
        assert_eq!(content.len(), 1); // only tool_use block
        assert_eq!(content[0]["type"], "tool_use");
    }

    #[test]
    fn test_convert_assistant_with_tools_multiple_calls() {
        let messages = vec![ChatMessage::AssistantWithTools {
            text: Some("I'll do both.".to_string()),
            tool_calls: vec![
                ToolCall {
                    id: "toolu_a".to_string(),
                    name: "tap".to_string(),
                    arguments: r#"{"x":100,"y":200}"#.to_string(),
                },
                ToolCall {
                    id: "toolu_b".to_string(),
                    name: "input_text".to_string(),
                    arguments: r#"{"text":"hello"}"#.to_string(),
                },
            ],
        }];
        let (_, api_msgs) = convert_messages(&messages);
        let content = api_msgs[0]["content"].as_array().expect("should be array");
        assert_eq!(content.len(), 3); // text + 2 tool_use
    }

    #[test]
    fn test_convert_tool_result_message() {
        let messages = vec![ChatMessage::ToolResult {
            tool_call_id: "toolu_123".to_string(),
            content: r#"{"success":true}"#.to_string(),
        }];
        let (_, api_msgs) = convert_messages(&messages);
        assert_eq!(api_msgs.len(), 1);

        // Must be a user message with tool_result content block
        assert_eq!(api_msgs[0]["role"], "user");
        let content = api_msgs[0]["content"].as_array().expect("should be array");
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[0]["tool_use_id"], "toolu_123");
        assert_eq!(content[0]["content"]["success"], true);
    }

    #[test]
    fn test_convert_tool_result_non_json_content() {
        let messages = vec![ChatMessage::ToolResult {
            tool_call_id: "toolu_456".to_string(),
            content: "plain text result".to_string(),
        }];
        let (_, api_msgs) = convert_messages(&messages);
        let content = api_msgs[0]["content"].as_array().expect("should be array");
        // Non-JSON content should be sent as a string value
        assert_eq!(content[0]["content"], "plain text result");
    }

    #[test]
    fn test_convert_full_conversation() {
        let messages = vec![
            ChatMessage::System("You are helpful.".to_string()),
            ChatMessage::User("Tap the button".to_string()),
            ChatMessage::AssistantWithTools {
                text: Some("I'll tap it.".to_string()),
                tool_calls: vec![ToolCall {
                    id: "toolu_1".to_string(),
                    name: "tap".to_string(),
                    arguments: r#"{"x":100,"y":200}"#.to_string(),
                }],
            },
            ChatMessage::ToolResult {
                tool_call_id: "toolu_1".to_string(),
                content: r#"{"success":true}"#.to_string(),
            },
        ];
        let (system, api_msgs) = convert_messages(&messages);
        assert_eq!(system, Some("You are helpful.".to_string()));
        // System is separate, so only 3 messages
        assert_eq!(api_msgs.len(), 3);
        assert_eq!(api_msgs[0]["role"], "user");
        assert_eq!(api_msgs[1]["role"], "assistant");
        assert_eq!(api_msgs[2]["role"], "user"); // tool result
    }

    #[test]
    fn test_convert_empty_messages() {
        let messages: Vec<ChatMessage> = vec![];
        let (system, api_msgs) = convert_messages(&messages);
        assert!(system.is_none());
        assert!(api_msgs.is_empty());
    }

    // === Tool schema conversion tests ===

    #[test]
    fn test_convert_tool_schema_basic() {
        let schema = json!({
            "name": "tap",
            "description": "Tap the screen",
            "parameters": {
                "type": "object",
                "properties": {
                    "x": { "type": "integer" },
                    "y": { "type": "integer" }
                },
                "required": ["x", "y"]
            }
        });

        let tool = convert_tool_schema(&schema).expect("should convert");
        assert_eq!(tool["name"], "tap");
        assert_eq!(tool["description"], "Tap the screen");
        // Key difference: "input_schema" instead of "parameters"
        assert!(tool.get("input_schema").is_some());
        assert!(tool.get("parameters").is_none());
        assert_eq!(tool["input_schema"]["type"], "object");
    }

    #[test]
    fn test_convert_tool_schema_no_params() {
        let schema = json!({
            "name": "get_screen_info",
            "description": "Get screen info"
        });

        let tool = convert_tool_schema(&schema).expect("should convert");
        assert_eq!(tool["name"], "get_screen_info");
        // Should get a default empty object schema
        assert_eq!(tool["input_schema"]["type"], "object");
    }

    #[test]
    fn test_convert_tool_schema_missing_name() {
        let schema = json!({
            "description": "no name"
        });
        assert!(convert_tool_schema(&schema).is_none());
    }

    #[test]
    fn test_convert_tool_schema_missing_description() {
        let schema = json!({
            "name": "some_tool"
        });
        let tool = convert_tool_schema(&schema).expect("should convert");
        assert_eq!(tool["description"], "");
    }

    #[test]
    fn test_convert_tools_multiple() {
        let schemas = vec![
            json!({"name": "tap", "description": "Tap", "parameters": {"type": "object", "properties": {}}}),
            json!({"name": "swipe", "description": "Swipe", "parameters": {"type": "object", "properties": {}}}),
        ];
        let tools = convert_tools(&schemas);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["name"], "tap");
        assert_eq!(tools[1]["name"], "swipe");
    }

    #[test]
    fn test_convert_tools_filters_invalid() {
        let schemas = vec![
            json!({"name": "valid", "description": "ok"}),
            json!({"description": "no name"}),
            json!({"name": "also_valid", "description": "ok2"}),
        ];
        let tools = convert_tools(&schemas);
        assert_eq!(tools.len(), 2);
    }

    #[test]
    fn test_convert_tools_empty() {
        let tools = convert_tools(&[]);
        assert!(tools.is_empty());
    }

    // === Response parsing tests ===

    #[test]
    fn test_parse_response_text_only() {
        let body = json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [
                { "type": "text", "text": "Hello! How can I help?" }
            ],
            "model": "claude-sonnet-4-20250514",
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 25,
                "output_tokens": 10
            }
        });

        let response = parse_response(&body).expect("should parse");
        assert_eq!(response.text, Some("Hello! How can I help?".to_string()));
        assert!(response.tool_calls.is_empty());
        assert!(response.usage.is_some());
        assert_eq!(response.usage.as_ref().unwrap().prompt_tokens, 25);
        assert_eq!(response.usage.as_ref().unwrap().completion_tokens, 10);
    }

    #[test]
    fn test_parse_response_tool_use() {
        let body = json!({
            "id": "msg_456",
            "type": "message",
            "role": "assistant",
            "content": [
                {
                    "type": "tool_use",
                    "id": "toolu_abc",
                    "name": "tap",
                    "input": { "x": 100, "y": 200 }
                }
            ],
            "model": "claude-sonnet-4-20250514",
            "stop_reason": "tool_use",
            "usage": {
                "input_tokens": 50,
                "output_tokens": 20
            }
        });

        let response = parse_response(&body).expect("should parse");
        assert!(response.text.is_none());
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].id, "toolu_abc");
        assert_eq!(response.tool_calls[0].name, "tap");
        // Arguments should be serialized JSON string of the input
        let args: Value = serde_json::from_str(&response.tool_calls[0].arguments).unwrap();
        assert_eq!(args["x"], 100);
        assert_eq!(args["y"], 200);
    }

    #[test]
    fn test_parse_response_mixed_text_and_tools() {
        let body = json!({
            "id": "msg_789",
            "type": "message",
            "role": "assistant",
            "content": [
                { "type": "text", "text": "I'll tap that for you." },
                {
                    "type": "tool_use",
                    "id": "toolu_xyz",
                    "name": "tap",
                    "input": { "x": 50, "y": 75 }
                }
            ],
            "model": "claude-sonnet-4-20250514",
            "stop_reason": "tool_use",
            "usage": {
                "input_tokens": 60,
                "output_tokens": 30
            }
        });

        let response = parse_response(&body).expect("should parse");
        assert_eq!(response.text, Some("I'll tap that for you.".to_string()));
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.usage.as_ref().unwrap().prompt_tokens, 60);
    }

    #[test]
    fn test_parse_response_multiple_tool_calls() {
        let body = json!({
            "id": "msg_multi",
            "type": "message",
            "role": "assistant",
            "content": [
                {
                    "type": "tool_use",
                    "id": "toolu_1",
                    "name": "tap",
                    "input": { "x": 100, "y": 200 }
                },
                {
                    "type": "tool_use",
                    "id": "toolu_2",
                    "name": "input_text",
                    "input": { "text": "hello" }
                }
            ],
            "model": "claude-sonnet-4-20250514",
            "stop_reason": "tool_use",
            "usage": {
                "input_tokens": 80,
                "output_tokens": 40
            }
        });

        let response = parse_response(&body).expect("should parse");
        assert!(response.text.is_none());
        assert_eq!(response.tool_calls.len(), 2);
        assert_eq!(response.tool_calls[0].name, "tap");
        assert_eq!(response.tool_calls[1].name, "input_text");
    }

    #[test]
    fn test_parse_response_empty_content() {
        let body = json!({
            "id": "msg_empty",
            "type": "message",
            "role": "assistant",
            "content": [],
            "model": "claude-sonnet-4-20250514",
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 10,
                "output_tokens": 0
            }
        });

        let response = parse_response(&body).expect("should parse");
        assert!(response.text.is_none());
        assert!(response.tool_calls.is_empty());
    }

    #[test]
    fn test_parse_response_no_usage() {
        let body = json!({
            "id": "msg_no_usage",
            "type": "message",
            "role": "assistant",
            "content": [
                { "type": "text", "text": "Hi" }
            ],
            "stop_reason": "end_turn",
        });

        let response = parse_response(&body).expect("should parse");
        assert!(response.usage.is_none());
    }

    #[test]
    fn test_parse_response_zero_tokens() {
        let body = json!({
            "content": [
                { "type": "text", "text": "Hi" }
            ],
            "usage": {
                "input_tokens": 0,
                "output_tokens": 0
            },
        });

        let response = parse_response(&body).expect("should parse");
        assert!(response.usage.is_none()); // 0 tokens → None
    }

    // === Error classification tests ===

    #[test]
    fn test_classify_auth_error_401() {
        let err = classify_http_error(StatusCode::UNAUTHORIZED, "Invalid API key");
        assert_eq!(err, LlmError::AuthFailed);
    }

    #[test]
    fn test_classify_auth_error_403() {
        let err = classify_http_error(StatusCode::FORBIDDEN, "Forbidden");
        assert_eq!(err, LlmError::AuthFailed);
    }

    #[test]
    fn test_classify_rate_limit_429() {
        let err = classify_http_error(StatusCode::TOO_MANY_REQUESTS, "Slow down");
        assert_eq!(err, LlmError::RateLimit);
    }

    #[test]
    fn test_classify_timeout_408() {
        let err = classify_http_error(StatusCode::REQUEST_TIMEOUT, "Timeout");
        assert_eq!(err, LlmError::Timeout);
    }

    #[test]
    fn test_classify_timeout_504() {
        let err = classify_http_error(StatusCode::GATEWAY_TIMEOUT, "Gateway timeout");
        assert_eq!(err, LlmError::Timeout);
    }

    #[test]
    fn test_classify_server_error_500() {
        let err = classify_http_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal error");
        match err {
            LlmError::ApiError(msg) => assert!(msg.contains("500")),
            other => panic!("Expected ApiError, got {:?}", other),
        }
    }

    #[test]
    fn test_classify_generic_client_error_400() {
        let err = classify_http_error(StatusCode::BAD_REQUEST, "Bad request");
        match err {
            LlmError::ApiError(msg) => assert!(msg.contains("400")),
            other => panic!("Expected ApiError, got {:?}", other),
        }
    }

    // === Retryability tests ===

    #[test]
    fn test_retryable_429() {
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
    }

    #[test]
    fn test_retryable_500() {
        assert!(is_retryable_status(StatusCode::INTERNAL_SERVER_ERROR));
    }

    #[test]
    fn test_retryable_502() {
        assert!(is_retryable_status(StatusCode::BAD_GATEWAY));
    }

    #[test]
    fn test_retryable_503() {
        assert!(is_retryable_status(StatusCode::SERVICE_UNAVAILABLE));
    }

    #[test]
    fn test_not_retryable_401() {
        assert!(!is_retryable_status(StatusCode::UNAUTHORIZED));
    }

    #[test]
    fn test_not_retryable_400() {
        assert!(!is_retryable_status(StatusCode::BAD_REQUEST));
    }

    // === AnthropicProvider construction tests ===

    #[test]
    fn test_anthropic_provider_new() {
        let provider = AnthropicProvider::new("sk-test-key".to_string(), "claude-sonnet-4-20250514".to_string());
        assert_eq!(provider.model, "claude-sonnet-4-20250514");
        assert_eq!(provider.api_key, "sk-test-key");
        assert_eq!(provider.max_tokens, DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn test_anthropic_provider_custom_max_tokens() {
        let provider = AnthropicProvider::new("sk-test-key".to_string(), "claude-sonnet-4-20250514".to_string())
            .with_max_tokens(8192);
        assert_eq!(provider.max_tokens, 8192);
    }

    // === Full tool schema from ToolRegistry integration ===

    #[test]
    fn test_convert_full_tap_tool_from_registry() {
        use crate::agent::tool_registry::ToolRegistry;

        let registry = ToolRegistry::default();
        let tap = registry.get("tap").expect("tap should exist");

        let schema = json!({
            "name": tap.name,
            "description": tap.description_en,
            "parameters": tap.parameters_json_schema()
        });

        let tool = convert_tool_schema(&schema).expect("should convert");
        assert_eq!(tool["name"], "tap");
        assert!(tool.get("input_schema").is_some());
        assert_eq!(tool["input_schema"]["type"], "object");
        let props = tool["input_schema"]["properties"].as_object().unwrap();
        assert!(props.contains_key("x"));
        assert!(props.contains_key("y"));
    }

    #[test]
    fn test_convert_all_28_tools_from_registry() {
        use crate::agent::tool_registry::ToolRegistry;

        let registry = ToolRegistry::default();
        let schemas: Vec<Value> = registry
            .all_specs()
            .iter()
            .map(|spec| {
                json!({
                    "name": spec.name,
                    "description": spec.description_en,
                    "parameters": spec.parameters_json_schema()
                })
            })
            .collect();

        assert_eq!(schemas.len(), 28);
        let tools = convert_tools(&schemas);
        assert_eq!(tools.len(), 28, "All 28 tools should convert successfully");
        // Verify all use input_schema not parameters
        for tool in &tools {
            assert!(tool.get("input_schema").is_some(), "Tool should have input_schema");
            assert!(tool.get("parameters").is_none(), "Tool should not have parameters");
        }
    }
}
