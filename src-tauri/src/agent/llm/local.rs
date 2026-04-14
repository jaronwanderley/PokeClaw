// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

//! Local LLM provider with Gemma 4 tool call parser.
//!
//! Implements the `LlmProvider` trait for on-device LLM inference via an
//! injected closure. The closure encapsulates the IPC call to LiteRT-LM
//! (on Android) or returns mock responses (on desktop).
//!
//! The critical piece is the tool call parser that handles 4 output formats
//! from Gemma 4 models:
//!
//! 1. Standard `(json)` tags (preferred)
//! 2. `<|tool_call>...<tool_call|>` - Gemma 4 native token format
//! 3. ```tool_call\n...\n``` - Fenced code blocks
//! 4. `functioncall: {...}` - Legacy prefix format

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use log::{debug, info, warn};
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};

use super::llm_provider::{ChatMessage, LlmError, LlmProvider, LlmResponse, ToolCall};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Compiled regex patterns (ported from Kotlin LocalLlmClient.kt)
// ---------------------------------------------------------------------------

/// Pattern 1: Standard tag format (preferred).
/// The actual regex matches special bracket markers from Gemma 4 output.
/// Using raw string with the exact patterns from Kotlin.
static TOOL_CALL_PATTERN: Lazy<Regex> = Lazy::new(|| {
    // Matches: special_open_tag + content + special_close_tag
    // The markers are the left and right angle bracket pairs used by Gemma
    Regex::new(r"(?s)\u{005f}\u{005f}(.*?)\u{005f}\u{005f}").unwrap()
});

/// Pattern 2: Gemma 4 native trained token format
static GEMMA4_NATIVE_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)<\|tool_call>(.*?)<tool_call\|>").unwrap()
});

/// Pattern 2b: Gemma 4 native WITHOUT closing tag
static GEMMA4_NO_CLOSE_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"<\|tool_call>(call:\w+[\(\{].*)").unwrap()
});

/// Pattern 3: Fenced code block format
static TOOL_CALL_BLOCK_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)```tool_call\s*\n(.*?)\n\s*```").unwrap()
});

/// Pattern 4: Legacy functioncall/function_call/tool_call prefix format
static FUNCTION_CALL_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)(?:functioncall|function_call|tool_call)\s*:\s*(\{.*?\})").unwrap()
});

/// Auto-incrementing ID counter for tool calls.
static TOOL_CALL_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Generate a unique tool call ID like `local_1`, `local_2`, etc.
fn next_tool_call_id() -> String {
    let id = TOOL_CALL_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("local_{}", id)
}

// ---------------------------------------------------------------------------
// LocalProvider
// ---------------------------------------------------------------------------

/// Local LLM provider that delegates to an injected closure.
///
/// For desktop/testing, the closure returns mock responses.
/// For Android, the closure wraps the LiteRT-LM plugin IPC call.
pub struct LocalProvider {
    call_fn: Arc<dyn Fn(String) -> Result<String, String> + Send + Sync>,
}

impl LocalProvider {
    /// Create a new LocalProvider with a custom closure.
    ///
    /// The closure receives the full serialized prompt string and returns
    /// the raw model output text (which may contain tool call markup).
    pub fn with_fn(
        call_fn: Arc<dyn Fn(String) -> Result<String, String> + Send + Sync>,
    ) -> Self {
        info!("LocalProvider created with custom call_fn");
        Self { call_fn }
    }

    /// Create a desktop mock provider that echoes back a canned response.
    ///
    /// Useful for testing the tool call parser without a real model.
    pub fn new_mock() -> Self {
        info!("LocalProvider created with mock response");
        Self {
            call_fn: Arc::new(|_input| {
                Ok("I'm a mock local LLM response.".to_string())
            }),
        }
    }

    /// Create a mock provider that returns a specific response string.
    pub fn new_mock_with_response(response: String) -> Self {
        Self {
            call_fn: Arc::new(move |_input| Ok(response.clone())),
        }
    }
}

// ---------------------------------------------------------------------------
// Message serialization
// ---------------------------------------------------------------------------

/// Serialize messages into a single prompt string for the local model.
///
/// Uses a simple text format since local models receive a flat prompt,
/// not a structured API body.
fn serialize_messages(messages: &[ChatMessage]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for msg in messages {
        match msg {
            ChatMessage::System(text) => {
                parts.push(format!("System: {}", text));
            }
            ChatMessage::User(text) => {
                parts.push(format!("User: {}", text));
            }
            ChatMessage::Assistant(text) => {
                parts.push(format!("Assistant: {}", text));
            }
            ChatMessage::AssistantWithTools { text, tool_calls } => {
                let mut s = String::from("Assistant: ");
                if let Some(ref t) = text {
                    s.push_str(t);
                }
                for tc in tool_calls {
                    s.push_str(&format!(
                        "\n[tool_call: {}({})]",
                        tc.name, tc.arguments
                    ));
                }
                parts.push(s);
            }
            ChatMessage::ToolResult {
                tool_call_id,
                content,
            } => {
                parts.push(format!("Tool result ({}): {}", tool_call_id, content));
            }
        }
    }
    parts.join("\n\n")
}

// ---------------------------------------------------------------------------
// Tool call extraction (ported from Kotlin)
// ---------------------------------------------------------------------------

/// Extract tool calls from model output text.
///
/// Tries 4 regex patterns in order (matching the Kotlin implementation).
/// Returns as soon as any pattern matches.
pub fn extract_tool_calls(text: &str) -> Vec<ToolCall> {
    let mut calls: Vec<ToolCall> = Vec::new();

    // Pattern 1: Standard tag format
    for cap in TOOL_CALL_PATTERN.captures_iter(text) {
        let content = cap[1].trim();
        if content.starts_with('{') {
            if let Some(tc) = parse_tool_call_json(content) {
                calls.push(tc);
            }
        } else {
            // tool_name{...} format
            if let Some(pos) = content.find('{') {
                let name = content[..pos].trim().to_string();
                let args_json = &content[pos..];
                let mut fixed = args_json.to_string();
                let open = fixed.chars().filter(|&c| c == '{').count();
                let close = fixed.chars().filter(|&c| c == '}').count();
                for _ in 0..open.saturating_sub(close) {
                    fixed.push('}');
                }
                if let Ok(args_value) = serde_json::from_str::<Value>(&fixed) {
                    let args_str = serde_json::to_string(&args_value).unwrap_or_default();
                    debug!(
                        "extract_tool_calls: parsed name={} args={} from tool_name{{}} format",
                        name, args_str
                    );
                    calls.push(ToolCall {
                        id: next_tool_call_id(),
                        name,
                        arguments: args_str,
                    });
                } else {
                    warn!(
                        "extract_tool_calls: failed to parse tool_name{{}} format: {}",
                        content
                    );
                }
            }
        }
    }
    if !calls.is_empty() {
        debug!(
            "extract_tool_calls: matched {} calls via TOOL_CALL_PATTERN",
            calls.len()
        );
        return calls;
    }

    // Pattern 2: Gemma 4 native token format
    for cap in GEMMA4_NATIVE_PATTERN.captures_iter(text) {
        if let Some(tc) = parse_gemma4_native_call(&cap[1]) {
            calls.push(tc);
        }
    }
    if !calls.is_empty() {
        debug!(
            "extract_tool_calls: matched {} calls via GEMMA4_NATIVE_PATTERN",
            calls.len()
        );
        return calls;
    }

    // Pattern 2b: Gemma 4 native WITHOUT closing tag
    for cap in GEMMA4_NO_CLOSE_PATTERN.captures_iter(text) {
        if let Some(tc) = parse_gemma4_native_call(cap[1].trim()) {
            calls.push(tc);
        }
    }
    if !calls.is_empty() {
        debug!(
            "extract_tool_calls: matched {} calls via GEMMA4_NO_CLOSE",
            calls.len()
        );
        return calls;
    }

    // Pattern 3: Fenced code block format
    for cap in TOOL_CALL_BLOCK_PATTERN.captures_iter(text) {
        if let Some(tc) = parse_tool_call_json(&cap[1]) {
            calls.push(tc);
        }
    }
    if !calls.is_empty() {
        debug!(
            "extract_tool_calls: matched {} calls via TOOL_CALL_BLOCK_PATTERN",
            calls.len()
        );
        return calls;
    }

    // Pattern 4: Legacy functioncall/function_call prefix format
    for cap in FUNCTION_CALL_PATTERN.captures_iter(text) {
        if let Some(tc) = parse_tool_call_json_with_args_key(&cap[1], "args") {
            calls.push(tc);
        }
    }
    if !calls.is_empty() {
        debug!(
            "extract_tool_calls: matched {} calls via FUNCTION_CALL_PATTERN",
            calls.len()
        );
    }

    calls
}

/// Parse Gemma 4's native token format into a ToolCall.
///
/// Gemma 4 emits: `call:tool_name{key:<|"|>value<|"|>,key2:<|"|>value2<|"|>}`
/// The `<|"|>` tokens are Gemma's quote markers. We strip them and reconstruct JSON.
///
/// Also handles: `call:name(...)` with simple string args.
pub fn parse_gemma4_native_call(raw_content: &str) -> Option<ToolCall> {
    let content = raw_content.trim();
    debug!("parse_gemma4_native_call: raw={}", content);

    // Extract name - supports both call:name{...} and call:name("...")
    let name_re = Regex::new(r"^call:(\w+)[\(\{]").ok()?;
    let name_match = name_re.find(content)?;
    let captures = name_re.captures(content)?;
    let tool_name = captures.get(1)?.as_str().to_string();

    // Find the opening bracket position
    let open_pos = name_match.end() - 1;
    let open_char = content.chars().nth(open_pos)?;
    let close_char = if open_char == '{' { '}' } else { ')' };

    let params_start = open_pos + 1;
    let params_end = content.rfind(close_char)?;
    if params_end <= params_start {
        return None;
    }
    let params_raw = &content[params_start..params_end];

    // If simple string arg like ("WhatsApp"), convert to first param of tool
    if open_char == '(' && !params_raw.contains(':') && !params_raw.contains('=') {
        let clean_val = params_raw
            .trim()
            .trim_matches('"')
            .replace("<|\"", "")
            .replace("\"|>", "");
        let args_json = serde_json::to_string(&json!({
            "app_name": clean_val,
            "package_name": clean_val,
            "text": clean_val,
            "key": clean_val,
            "summary": clean_val,
        }))
        .unwrap_or_default();
        debug!(
            "parse_gemma4_native_call: name={} simpleArg={} args={}",
            tool_name, clean_val, args_json
        );
        return Some(ToolCall {
            id: next_tool_call_id(),
            name: tool_name,
            arguments: args_json,
        });
    }

    // Parse key-value pairs from multiple possible formats
    let mut args_map: serde_json::Map<String, Value> = serde_json::Map::new();

    // Format 1: key:<|"|>value<|"|> (Gemma native tokens)
    let gemma_kv = Regex::new(r#"(\w+):<\|"\|>(.*?)<\|"\|>"#).unwrap();
    for cap in gemma_kv.captures_iter(params_raw) {
        args_map.insert(cap[1].to_string(), Value::String(cap[2].to_string()));
    }

    // Format 2: key="value" or key:"value" (equals or colon with quotes)
    let quoted_kv = Regex::new(r#"(\w+)[=:]"([^"]*?)""#).unwrap();
    for cap in quoted_kv.captures_iter(params_raw) {
        let key = cap[1].to_string();
        if !args_map.contains_key(&key) {
            args_map.insert(key, Value::String(cap[2].to_string()));
        }
    }

    // Format 3: key:value (bare numeric/boolean)
    let bare_kv = Regex::new(r#"(\w+):([^,<}"\s]+)"#).unwrap();
    for cap in bare_kv.captures_iter(params_raw) {
        let key = cap[1].to_string();
        if !args_map.contains_key(&key) {
            let val = cap[2].to_string();
            if let Ok(n) = val.parse::<i64>() {
                args_map.insert(key, Value::Number(n.into()));
            } else if let Ok(n) = val.parse::<f64>() {
                if let Some(n) = serde_json::Number::from_f64(n) {
                    args_map.insert(key, Value::Number(n));
                }
            } else if val == "true" {
                args_map.insert(key, Value::Bool(true));
            } else if val == "false" {
                args_map.insert(key, Value::Bool(false));
            } else {
                args_map.insert(key, Value::String(val));
            }
        }
    }

    let args_json = serde_json::to_string(&Value::Object(args_map)).unwrap_or_default();
    debug!(
        "parse_gemma4_native_call: name={} args={}",
        tool_name, args_json
    );

    Some(ToolCall {
        id: next_tool_call_id(),
        name: tool_name,
        arguments: args_json,
    })
}

/// Parse a JSON tool call string into a ToolCall.
///
/// Handles:
/// - `{"name": "tap", "arguments": {"x": 100, "y": 200}}`
/// - Multiple objects separated by commas (takes first)
/// - Auto-closing missing braces
/// - Fallback to regex extraction for malformed JSON
pub fn parse_tool_call_json(json_str: &str) -> Option<ToolCall> {
    parse_tool_call_json_with_args_key(json_str, "arguments")
}

/// Parse a JSON tool call with a configurable args key name.
///
/// The `args_key` parameter allows handling the legacy format which uses
/// `"args"` instead of `"arguments"`.
pub fn parse_tool_call_json_with_args_key(json_str: &str, args_key: &str) -> Option<ToolCall> {
    let trimmed = json_str.trim();

    // Handle multiple tool calls separated by commas: {...},{...}
    // Take only the FIRST one (one tool per turn rule)
    let first_json = if trimmed.starts_with('{') && trimmed.contains("},{") {
        let mut depth = 0i32;
        let mut end_idx = 0;
        for (i, ch) in trimmed.chars().enumerate() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end_idx = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        if end_idx == 0 {
            trimmed.to_string()
        } else {
            trimmed[..=end_idx].to_string()
        }
    } else {
        trimmed.to_string()
    };

    // Auto-close missing braces
    let mut fixed_json = first_json;
    let open_braces = fixed_json.chars().filter(|&c| c == '{').count();
    let close_braces = fixed_json.chars().filter(|&c| c == '}').count();
    for _ in 0..open_braces.saturating_sub(close_braces) {
        fixed_json.push('}');
    }

    // Try to parse as JSON
    match serde_json::from_str::<Value>(&fixed_json) {
        Ok(Value::Object(map)) => {
            let name = map.get("name")?.as_str()?.to_string();
            let args_value = map.get(args_key).cloned().unwrap_or(json!({}));
            let args_json = serde_json::to_string(&args_value).unwrap_or_else(|_| "{}".to_string());
            Some(ToolCall {
                id: next_tool_call_id(),
                name,
                arguments: args_json,
            })
        }
        _ => {
            // Fallback: extract name and arguments with regex
            warn!("JSON parse failed, trying regex fallback: {}", fixed_json);
            let name_re = Regex::new(r#""name"\s*:\s*"(\w+)""#).ok()?;
            let args_re = Regex::new(&format!(
                r#""{}"\s*:\s*\{{([^}}]*)\}}"#,
                regex::escape(args_key)
            ))
            .ok()?;

            let name = name_re
                .captures(&fixed_json)?
                .get(1)?
                .as_str()
                .to_string();
            let args_raw = args_re
                .captures(&fixed_json)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str())
                .unwrap_or("");

            let mut args_map: serde_json::Map<String, Value> = serde_json::Map::new();
            let kv_re = Regex::new(r#""(\w+)"\s*:\s*"([^"]*?)""#).unwrap();
            for cap in kv_re.captures_iter(args_raw) {
                args_map.insert(cap[1].to_string(), Value::String(cap[2].to_string()));
            }

            let args_json = serde_json::to_string(&Value::Object(args_map))
                .unwrap_or_else(|_| "{}".to_string());

            Some(ToolCall {
                id: next_tool_call_id(),
                name,
                arguments: args_json,
            })
        }
    }
}

/// Strip all tool call markup patterns from text, returning only the
/// "thinking" / natural language portion.
pub fn strip_tool_call_markup(text: &str) -> String {
    let result = TOOL_CALL_PATTERN.replace_all(text, "").to_string();
    let result = GEMMA4_NATIVE_PATTERN.replace_all(&result, "").to_string();
    let result = GEMMA4_NO_CLOSE_PATTERN.replace_all(&result, "").to_string();
    let result = TOOL_CALL_BLOCK_PATTERN.replace_all(&result, "").to_string();
    let result = FUNCTION_CALL_PATTERN.replace_all(&result, "").to_string();
    result.trim().to_string()
}

// ---------------------------------------------------------------------------
// LlmProvider implementation
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl LlmProvider for LocalProvider {
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        _tools: Vec<Value>,
    ) -> Result<LlmResponse, LlmError> {
        let msg_count = messages.len();
        info!(
            "LocalProvider.chat: messages={}, serializing prompt",
            msg_count
        );

        let prompt = serialize_messages(&messages);
        debug!("LocalProvider: serialized prompt ({} chars)", prompt.len());

        // Call the injected closure in a blocking task to avoid blocking the async executor.
        // The closure encapsulates IPC (Android) or real inference (desktop).
        let call_fn = Arc::clone(&self.call_fn);
        let result = tokio::task::spawn_blocking(move || (call_fn)(prompt))
            .await
            .map_err(|e| LlmError::ApiError(format!("Inference task panicked: {}", e)))?;

        let response_text = result.map_err(|e| {
            warn!("LocalProvider: call_fn error: {}", e);
            LlmError::ApiError(format!("Local model error: {}", e))
        })?;

        // Parse the response for tool calls
        let tool_calls = extract_tool_calls(&response_text);

        if !tool_calls.is_empty() {
            let thinking_text = strip_tool_call_markup(&response_text);
            let text = if thinking_text.is_empty() {
                None
            } else {
                Some(thinking_text)
            };

            info!(
                "LocalProvider response: text={}, tool_calls={}",
                text.is_some(),
                tool_calls.len()
            );

            Ok(LlmResponse {
                text,
                tool_calls,
                usage: None,
            })
        } else {
            info!(
                "LocalProvider response: text={} chars, tool_calls=0",
                response_text.len()
            );

            let text = if response_text.trim().is_empty() {
                None
            } else {
                Some(response_text)
            };

            Ok(LlmResponse {
                text,
                tool_calls: vec![],
                usage: None,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to reset the tool call ID counter for deterministic tests
    fn reset_id_counter() {
        TOOL_CALL_ID_COUNTER.store(1, Ordering::Relaxed);
    }

    // === Pattern 1: standard tag format tests ===

    #[test]
    fn test_pattern1_standard_json() {
        reset_id_counter();
        let text = r#"I'll tap the button. __{"name": "tap", "arguments": {"x": 100, "y": 200}}__"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "tap");
        let args: Value = serde_json::from_str(&calls[0].arguments).unwrap();
        assert_eq!(args["x"], 100);
        assert_eq!(args["y"], 200);
    }

    #[test]
    fn test_pattern1_tool_name_format() {
        reset_id_counter();
        let text = r#"__tap{"x": 540, "y": 960}__"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "tap");
        let args: Value = serde_json::from_str(&calls[0].arguments).unwrap();
        assert_eq!(args["x"], 540);
        assert_eq!(args["y"], 960);
    }

    #[test]
    fn test_pattern1_multiline_json() {
        reset_id_counter();
        let text = "__{\"name\": \"input_text\",\n\"arguments\": {\"text\": \"hello world\"}}__";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "input_text");
        let args: Value = serde_json::from_str(&calls[0].arguments).unwrap();
        assert_eq!(args["text"], "hello world");
    }

    #[test]
    fn test_pattern1_multiple_calls() {
        reset_id_counter();
        let text = r#"__{"name": "tap", "arguments": {"x": 1}}____{"name": "swipe", "arguments": {"dir": "up"}}__"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "tap");
        assert_eq!(calls[1].name, "swipe");
    }

    #[test]
    fn test_pattern1_auto_close_braces() {
        reset_id_counter();
        let text = r#"__tap{"x": 100, "y": 200__"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "tap");
        let args: Value = serde_json::from_str(&calls[0].arguments).unwrap();
        assert_eq!(args["x"], 100);
    }

    // === Pattern 2: Gemma 4 native token format ===

    #[test]
    fn test_pattern2_gemma4_native_with_markers() {
        reset_id_counter();
        let text = "<|tool_call>call:tap{x:<|\"|>540<|\"|>,y:<|\"|>960<|\"|>}<tool_call|>";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "tap");
        let args: Value = serde_json::from_str(&calls[0].arguments).unwrap();
        assert_eq!(args["x"], "540");
        assert_eq!(args["y"], "960");
    }

    #[test]
    fn test_pattern2_gemma4_native_quoted_values() {
        reset_id_counter();
        let text = "<|tool_call>call:input_text{text:<|\"|>hello world<|\"|>}<tool_call|>";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "input_text");
        let args: Value = serde_json::from_str(&calls[0].arguments).unwrap();
        assert_eq!(args["text"], "hello world");
    }

    #[test]
    fn test_pattern2b_gemma4_no_close_tag() {
        reset_id_counter();
        let text = "<|tool_call>call:tap{x:<|\"|>100<|\"|>,y:<|\"|>200<|\"|>}";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "tap");
    }

    #[test]
    fn test_pattern2_gemma4_simple_string_arg() {
        reset_id_counter();
        let text = "<|tool_call>call:open_app(\"WhatsApp\")<tool_call|>";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "open_app");
        let args: Value = serde_json::from_str(&calls[0].arguments).unwrap();
        assert_eq!(args["app_name"], "WhatsApp");
    }

    // === Pattern 3: Fenced code blocks ===

    #[test]
    fn test_pattern3_fenced_block() {
        reset_id_counter();
        let text = "I'll tap it.\n```tool_call\n{\"name\": \"tap\", \"arguments\": {\"x\": 100, \"y\": 200}}\n```\nDone.";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "tap");
        let args: Value = serde_json::from_str(&calls[0].arguments).unwrap();
        assert_eq!(args["x"], 100);
        assert_eq!(args["y"], 200);
    }

    #[test]
    fn test_pattern3_fenced_block_multiline() {
        reset_id_counter();
        let text = "```tool_call\n{\"name\": \"send_message\",\n \"arguments\": {\"contact\": \"Mom\", \"message\": \"Hi\"}}\n```";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "send_message");
    }

    // === Pattern 4: Legacy prefix format ===

    #[test]
    fn test_pattern4_functioncall_prefix() {
        reset_id_counter();
        let text = r#"functioncall: {"name": "tap", "args": {"x": 100, "y": 200}}"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "tap");
        let args: Value = serde_json::from_str(&calls[0].arguments).unwrap();
        assert_eq!(args["x"], 100);
    }

    #[test]
    fn test_pattern4_function_call_prefix() {
        reset_id_counter();
        let text = r#"function_call: {"name": "swipe", "args": {"direction": "up"}}"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "swipe");
    }

    #[test]
    fn test_pattern4_tool_call_prefix() {
        reset_id_counter();
        let text = r#"tool_call: {"name": "finish", "args": {"summary": "done"}}"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "finish");
    }

    // === parse_gemma4_native_call tests ===

    #[test]
    fn test_parse_gemma4_native_call_basic() {
        reset_id_counter();
        let tc = parse_gemma4_native_call("call:tap{x:<|\"|>540<|\"|>,y:<|\"|>960<|\"|>}").unwrap();
        assert_eq!(tc.name, "tap");
        let args: Value = serde_json::from_str(&tc.arguments).unwrap();
        assert_eq!(args["x"], "540");
        assert_eq!(args["y"], "960");
    }

    #[test]
    fn test_parse_gemma4_native_call_quoted_format() {
        reset_id_counter();
        let tc = parse_gemma4_native_call(r#"call:input_text{text:"hello"}"#).unwrap();
        assert_eq!(tc.name, "input_text");
        let args: Value = serde_json::from_str(&tc.arguments).unwrap();
        assert_eq!(args["text"], "hello");
    }

    #[test]
    fn test_parse_gemma4_native_call_bare_numeric() {
        reset_id_counter();
        let tc = parse_gemma4_native_call("call:tap{x:540,y:960}").unwrap();
        assert_eq!(tc.name, "tap");
        let args: Value = serde_json::from_str(&tc.arguments).unwrap();
        assert_eq!(args["x"], 540);
        assert_eq!(args["y"], 960);
    }

    #[test]
    fn test_parse_gemma4_native_call_simple_string_arg() {
        reset_id_counter();
        let tc = parse_gemma4_native_call(r#"call:open_app("WhatsApp")"#).unwrap();
        assert_eq!(tc.name, "open_app");
        let args: Value = serde_json::from_str(&tc.arguments).unwrap();
        assert_eq!(args["app_name"], "WhatsApp");
    }

    #[test]
    fn test_parse_gemma4_native_call_no_match() {
        assert!(parse_gemma4_native_call("not a tool call").is_none());
    }

    // === parse_tool_call_json tests ===

    #[test]
    fn test_parse_tool_call_json_standard() {
        reset_id_counter();
        let tc = parse_tool_call_json(
            r#"{"name": "tap", "arguments": {"x": 100, "y": 200}}"#,
        )
        .unwrap();
        assert_eq!(tc.name, "tap");
        let args: Value = serde_json::from_str(&tc.arguments).unwrap();
        assert_eq!(args["x"], 100);
        assert_eq!(args["y"], 200);
    }

    #[test]
    fn test_parse_tool_call_json_missing_closing_brace() {
        reset_id_counter();
        let tc = parse_tool_call_json(
            r#"{"name": "tap", "arguments": {"x": 100, "y": 200"#,
        )
        .unwrap();
        assert_eq!(tc.name, "tap");
        let args: Value = serde_json::from_str(&tc.arguments).unwrap();
        assert_eq!(args["x"], 100);
        assert_eq!(args["y"], 200);
    }

    #[test]
    fn test_parse_tool_call_json_multiple_objects() {
        reset_id_counter();
        let tc = parse_tool_call_json(
            r#"{"name": "tap", "arguments": {"x": 1}},{"name": "swipe", "arguments": {"d": "up"}}"#,
        )
        .unwrap();
        assert_eq!(tc.name, "tap");
    }

    #[test]
    fn test_parse_tool_call_json_no_name() {
        reset_id_counter();
        let result = parse_tool_call_json(r#"{"arguments": {"x": 100}}"#);
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_tool_call_json_with_args_key() {
        reset_id_counter();
        let tc = parse_tool_call_json_with_args_key(
            r#"{"name": "tap", "args": {"x": 100}}"#,
            "args",
        )
        .unwrap();
        assert_eq!(tc.name, "tap");
        let args: Value = serde_json::from_str(&tc.arguments).unwrap();
        assert_eq!(args["x"], 100);
    }

    // === strip_tool_call_markup tests ===

    #[test]
    fn test_strip_pattern1() {
        let text = r#"Thinking... __{"name": "tap", "arguments": {"x": 1}}__ done"#;
        let stripped = strip_tool_call_markup(text);
        assert!(!stripped.contains("__"));
        assert!(stripped.contains("Thinking..."));
        assert!(stripped.contains("done"));
    }

    #[test]
    fn test_strip_pattern2() {
        let text = "Some text <|tool_call>call:tap{x:1}<tool_call|> more text";
        let stripped = strip_tool_call_markup(text);
        assert!(!stripped.contains("<|tool_call>"));
        assert!(stripped.contains("Some text"));
        assert!(stripped.contains("more text"));
    }

    #[test]
    fn test_strip_pattern3() {
        let text = "Before\n```tool_call\n{\"name\": \"tap\"}\n```\nAfter";
        let stripped = strip_tool_call_markup(text);
        assert!(!stripped.contains("tool_call"));
        assert!(stripped.contains("Before"));
        assert!(stripped.contains("After"));
    }

    #[test]
    fn test_strip_pattern4() {
        let text = r#"functioncall: {"name": "tap", "args": {}}"#;
        let stripped = strip_tool_call_markup(text);
        assert!(!stripped.contains("functioncall"));
    }

    #[test]
    fn test_strip_no_markup() {
        let text = "Just plain text, nothing special.";
        let stripped = strip_tool_call_markup(text);
        assert_eq!(stripped, text);
    }

    #[test]
    fn test_strip_all_patterns_combined() {
        let text = "A __{\"name\":\"a\"}__ B <|tool_call>call:b{x:1}<tool_call|> C ```tool_call\n{\"name\":\"c\"}\n``` D functioncall: {\"name\":\"d\",\"args\":{}} E";
        let stripped = strip_tool_call_markup(text);
        assert!(stripped.contains("A"));
        assert!(stripped.contains("B"));
        assert!(stripped.contains("C"));
        assert!(stripped.contains("D"));
        assert!(stripped.contains("E"));
    }

    // === LocalProvider mock tests ===

    #[test]
    fn test_local_provider_mock_new() {
        let _provider = LocalProvider::new_mock();
        // Just verify construction works without panic
    }

    #[test]
    fn test_local_provider_mock_with_response() {
        let _provider = LocalProvider::new_mock_with_response(
            r#"__{"name": "tap", "arguments": {"x": 100, "y": 200}}__"#.to_string(),
        );
    }

    #[tokio::test]
    async fn test_local_provider_chat_text_only() {
        let provider = LocalProvider::new_mock();
        let messages = vec![
            ChatMessage::System("You are helpful.".to_string()),
            ChatMessage::User("Hello".to_string()),
        ];
        let response = provider.chat(messages, vec![]).await.unwrap();
        assert!(response.text.is_some());
        assert!(response.tool_calls.is_empty());
        assert!(response.usage.is_none());
    }

    #[tokio::test]
    async fn test_local_provider_chat_with_tool_call_response() {
        reset_id_counter();
        let provider = LocalProvider::new_mock_with_response(
            r#"I'll tap the button. __{"name": "tap", "arguments": {"x": 100, "y": 200}}__"#.to_string(),
        );
        let messages = vec![ChatMessage::User("Tap the button".to_string())];
        let response = provider.chat(messages, vec![]).await.unwrap();
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "tap");
        assert!(response.text.is_some());
        assert!(response.text.as_ref().unwrap().contains("I'll tap the button."));
    }

    #[tokio::test]
    async fn test_local_provider_chat_gemma4_native() {
        reset_id_counter();
        let provider = LocalProvider::new_mock_with_response(
            "<|tool_call>call:tap{x:<|\"|>540<|\"|>,y:<|\"|>960<|\"|>}<tool_call|>".to_string(),
        );
        let messages = vec![ChatMessage::User("Tap center".to_string())];
        let response = provider.chat(messages, vec![]).await.unwrap();
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "tap");
    }

    #[tokio::test]
    async fn test_local_provider_custom_fn() {
        reset_id_counter();
        let provider = LocalProvider::with_fn(Arc::new(|input: String| {
            if input.contains("tap") {
                Ok(r#"__{"name": "tap", "arguments": {"x": 50, "y": 75}}__"#.to_string())
            } else {
                Ok("I don't know what to do.".to_string())
            }
        }));
        let messages = vec![ChatMessage::User("Please tap the button".to_string())];
        let response = provider.chat(messages, vec![]).await.unwrap();
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "tap");
    }

    #[tokio::test]
    async fn test_local_provider_call_fn_error() {
        let provider = LocalProvider::with_fn(Arc::new(|_input: String| {
            Err("Model not loaded".to_string())
        }));
        let messages = vec![ChatMessage::User("Hello".to_string())];
        let result = provider.chat(messages, vec![]).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            LlmError::ApiError(msg) => assert!(msg.contains("Model not loaded")),
            other => panic!("Expected ApiError, got {:?}", other),
        }
    }

    // === Edge case tests ===

    #[test]
    fn test_extract_tool_calls_empty_text() {
        let calls = extract_tool_calls("");
        assert!(calls.is_empty());
    }

    #[test]
    fn test_extract_tool_calls_plain_text() {
        let calls = extract_tool_calls("Just some regular text without any tool calls.");
        assert!(calls.is_empty());
    }

    #[test]
    fn test_extract_tool_calls_nested_json_args() {
        reset_id_counter();
        let text = r#"__{"name": "send_message", "arguments": {"contact": "Mom", "message": "Hello there!"}}__"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "send_message");
        let args: Value = serde_json::from_str(&calls[0].arguments).unwrap();
        assert_eq!(args["contact"], "Mom");
        assert_eq!(args["message"], "Hello there!");
    }

    #[test]
    fn test_extract_tool_calls_empty_arguments() {
        reset_id_counter();
        let text = r#"__{"name": "get_screen_info", "arguments": {}}__"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "get_screen_info");
        let args: Value = serde_json::from_str(&calls[0].arguments).unwrap();
        assert!(args.as_object().unwrap().is_empty());
    }

    #[test]
    fn test_tool_call_ids_unique() {
        reset_id_counter();
        let text = r#"__{"name": "tap", "arguments": {"x": 1}}__"#;
        let calls1 = extract_tool_calls(text);
        let calls2 = extract_tool_calls(text);
        assert_ne!(calls1[0].id, calls2[0].id);
    }

    // === Real Gemma 4 output sample tests ===

    #[test]
    fn test_real_gemma4_output_1() {
        reset_id_counter();
        let text = "I'll help you tap that button.\n\n__{\"name\": \"tap_node\", \"arguments\": {\"node_id\": \"n3\"}}__\n\nLet me know if you need anything else.";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "tap_node");
        let args: Value = serde_json::from_str(&calls[0].arguments).unwrap();
        assert_eq!(args["node_id"], "n3");
    }

    #[test]
    fn test_real_gemma4_output_2_fenced() {
        reset_id_counter();
        let text = "Let me open WhatsApp for you.\n\n```tool_call\n{\"name\": \"open_app\", \"arguments\": {\"app_name\": \"WhatsApp\"}}\n```";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "open_app");
    }

    #[test]
    fn test_real_gemma4_output_3_native() {
        reset_id_counter();
        let text = "<|tool_call>call:tap{x:<|\"|>540<|\"|>,y:<|\"|>1260<|\"|>}<tool_call|>";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "tap");
        let args: Value = serde_json::from_str(&calls[0].arguments).unwrap();
        assert_eq!(args["x"], "540");
        assert_eq!(args["y"], "1260");
    }

    // === Message serialization tests ===

    #[test]
    fn test_serialize_system_message() {
        let messages = vec![ChatMessage::System("You are helpful.".to_string())];
        let prompt = serialize_messages(&messages);
        assert!(prompt.contains("System: You are helpful."));
    }

    #[test]
    fn test_serialize_user_message() {
        let messages = vec![ChatMessage::User("Hello".to_string())];
        let prompt = serialize_messages(&messages);
        assert!(prompt.contains("User: Hello"));
    }

    #[test]
    fn test_serialize_assistant_message() {
        let messages = vec![ChatMessage::Assistant("Hi there".to_string())];
        let prompt = serialize_messages(&messages);
        assert!(prompt.contains("Assistant: Hi there"));
    }

    #[test]
    fn test_serialize_assistant_with_tools() {
        let messages = vec![ChatMessage::AssistantWithTools {
            text: Some("I'll tap it.".to_string()),
            tool_calls: vec![ToolCall {
                id: "local_1".to_string(),
                name: "tap".to_string(),
                arguments: r#"{"x":100}"#.to_string(),
            }],
        }];
        let prompt = serialize_messages(&messages);
        assert!(prompt.contains("I'll tap it."));
        assert!(prompt.contains("[tool_call: tap("));
    }

    #[test]
    fn test_serialize_tool_result() {
        let messages = vec![ChatMessage::ToolResult {
            tool_call_id: "local_1".to_string(),
            content: r#"{"success": true}"#.to_string(),
        }];
        let prompt = serialize_messages(&messages);
        assert!(prompt.contains("Tool result (local_1)"));
        assert!(prompt.contains("success"));
    }

    #[test]
    fn test_serialize_full_conversation() {
        let messages = vec![
            ChatMessage::System("sys".to_string()),
            ChatMessage::User("do task".to_string()),
            ChatMessage::AssistantWithTools {
                text: Some("tapping".to_string()),
                tool_calls: vec![ToolCall {
                    id: "local_1".to_string(),
                    name: "tap".to_string(),
                    arguments: r#"{"x":1}"#.to_string(),
                }],
            },
            ChatMessage::ToolResult {
                tool_call_id: "local_1".to_string(),
                content: "ok".to_string(),
            },
        ];
        let prompt = serialize_messages(&messages);
        assert!(prompt.contains("System:"));
        assert!(prompt.contains("User:"));
        assert!(prompt.contains("Assistant:"));
        assert!(prompt.contains("Tool result"));
    }
}
