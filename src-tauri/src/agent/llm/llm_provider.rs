// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

use std::time::Duration;

use async_openai::config::OpenAIConfig;
use async_openai::types::chat::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestToolMessage, ChatCompletionRequestUserMessageArgs,
    ChatCompletionRequestAssistantMessageArgs, ChatCompletionTool,
    ChatCompletionTools, CreateChatCompletionRequestArgs, FunctionObjectArgs,
};
use async_openai::Client;
use log::{debug, info, warn};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Public types — no async-openai leakage beyond this module
// ---------------------------------------------------------------------------

/// Messages in the conversation.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatMessage {
    System(String),
    User(String),
    Assistant(String),
    /// Assistant message that includes tool calls — critical for multi-round
    /// history so the LLM knows what tools it previously invoked.
    AssistantWithTools {
        text: Option<String>,
        tool_calls: Vec<ToolCall>,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
    },
}

/// A single tool call returned by the LLM.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Raw JSON arguments string from the API.
    pub arguments: String,
}

/// Token usage statistics from the API response.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

/// Response from the LLM.
#[derive(Debug, Clone, PartialEq)]
pub struct LlmResponse {
    /// Text content, if any.
    pub text: Option<String>,
    /// Tool calls, if the model decided to invoke tools.
    pub tool_calls: Vec<ToolCall>,
    /// Token usage from the API.
    pub usage: Option<TokenUsage>,
}

/// Errors from the LLM provider.
#[derive(Debug, Clone, PartialEq)]
pub enum LlmError {
    ApiError(String),
    RateLimit,
    Timeout,
    AuthFailed,
    Network(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::ApiError(msg) => write!(f, "API error: {}", msg),
            LlmError::RateLimit => write!(f, "Rate limited"),
            LlmError::Timeout => write!(f, "Request timed out"),
            LlmError::AuthFailed => write!(f, "Authentication failed"),
            LlmError::Network(msg) => write!(f, "Network error: {}", msg),
        }
    }
}

impl std::error::Error for LlmError {}

// ---------------------------------------------------------------------------
// LlmProvider trait
// ---------------------------------------------------------------------------

/// Abstraction over LLM chat completion providers.
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    /// Send a chat completion request with optional tool specs.
    ///
    /// `tools` is a vec of serde_json::Value objects, each being a JSON object
    /// with `name` and `description` fields plus a `parameters` sub-schema.
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<Value>,
    ) -> Result<LlmResponse, LlmError>;
}

// ---------------------------------------------------------------------------
// OpenAiProvider
// ---------------------------------------------------------------------------

const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 1000;
const REQUEST_TIMEOUT_SECS: u64 = 30;

/// OpenAI-compatible LLM provider using the async-openai crate.
pub struct OpenAiProvider {
    client: Client<OpenAIConfig>,
    model: String,
}

impl OpenAiProvider {
    /// Create a new provider targeting the default OpenAI API endpoint.
    pub fn new(api_key: String, model: String) -> Self {
        let config = OpenAIConfig::new().with_api_key(api_key);
        let client = Client::with_config(config);
        info!("OpenAiProvider created: model={}, endpoint=default", model);
        Self { client, model }
    }

    /// Create a provider targeting a custom base URL (e.g. Gemini OpenAI-compat).
    pub fn with_base_url(api_key: String, model: String, base_url: String) -> Self {
        let config = OpenAIConfig::new()
            .with_api_key(api_key)
            .with_api_base(base_url);
        let client = Client::with_config(config);
        info!("OpenAiProvider created: model={}, endpoint=custom", model);
        Self { client, model }
    }
}

// ---------------------------------------------------------------------------
// Conversion helpers (private)
// ---------------------------------------------------------------------------

/// Convert our ChatMessage into async-openai request messages.
fn convert_messages(messages: &[ChatMessage]) -> Vec<ChatCompletionRequestMessage> {

    messages
        .iter()
        .map(|msg| match msg {
            ChatMessage::System(text) => {
                ChatCompletionRequestMessage::System(
                    ChatCompletionRequestSystemMessageArgs::default()
                        .content(text.clone())
                        .build()
                        .expect("system message build should not fail"),
                )
            }
            ChatMessage::User(text) => {
                ChatCompletionRequestMessage::User(
                    ChatCompletionRequestUserMessageArgs::default()
                        .content(text.clone())
                        .build()
                        .expect("user message build should not fail"),
                )
            }
            ChatMessage::Assistant(text) => {
                ChatCompletionRequestMessage::Assistant(
                    ChatCompletionRequestAssistantMessageArgs::default()
                        .content(text.clone())
                        .build()
                        .expect("assistant message build should not fail"),
                )
            }
            ChatMessage::AssistantWithTools { text, tool_calls } => {
                let api_tool_calls: Vec<_> = tool_calls
                    .iter()
                    .map(|tc| {
                        async_openai::types::chat::ChatCompletionMessageToolCalls::Function(
                            async_openai::types::chat::ChatCompletionMessageToolCall {
                                id: tc.id.clone(),
                                function: async_openai::types::chat::FunctionCall {
                                    name: tc.name.clone(),
                                    arguments: tc.arguments.clone(),
                                },
                            },
                        )
                    })
                    .collect();

                let mut args = ChatCompletionRequestAssistantMessageArgs::default();
                if let Some(ref t) = text {
                    args.content(t.clone());
                }
                if !api_tool_calls.is_empty() {
                    args.tool_calls(api_tool_calls);
                }
                ChatCompletionRequestMessage::Assistant(
                    args.build()
                        .expect("assistant-with-tools message build should not fail"),
                )
            }
            ChatMessage::ToolResult {
                tool_call_id,
                content,
            } => {
                ChatCompletionRequestMessage::Tool(ChatCompletionRequestToolMessage {
                    content: content.clone().into(),
                    tool_call_id: tool_call_id.clone(),
                })
            }
        })
        .collect()
}

/// Convert a ToolSpec's JSON Schema (from ToolRegistry) into a
/// ChatCompletionTool for the async-openai request.
///
/// Expected `schema` format:
/// ```json
/// {
///   "name": "tap",
///   "description": "...",
///   "parameters": { "type": "object", "properties": {...}, "required": [...] }
/// }
/// ```
fn convert_tool_schema(schema: &Value) -> Option<ChatCompletionTool> {
    let name = schema.get("name")?.as_str()?.to_string();
    let description = schema
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let parameters = schema.get("parameters").cloned();

    let mut binding = FunctionObjectArgs::default();
    let mut func_builder = binding
        .name(&name)
        .description(&description);

    if let Some(params) = parameters {
        func_builder = func_builder.parameters(params);
    }

    Some(ChatCompletionTool {
        function: func_builder
            .build()
            .expect("function object build should not fail"),
    })
}

/// Convert all tool JSON Schema values into ChatCompletionTools objects
/// wrapped in the enum variant expected by the API.
fn convert_tools(tool_schemas: &[Value]) -> Vec<ChatCompletionTools> {
    tool_schemas
        .iter()
        .filter_map(|s| convert_tool_schema(s))
        .map(ChatCompletionTools::Function)
        .collect()
}

/// Classify an async-openai error into our LlmError.
fn classify_error(err: &async_openai::error::OpenAIError) -> LlmError {
    let msg = err.to_string();
    // async-openai wraps HTTP errors as ApiError with status information
    if msg.contains("401") || msg.contains("403") || msg.contains("authentication") || msg.contains("Invalid API") {
        LlmError::AuthFailed
    } else if msg.contains("429") || msg.contains("rate limit") || msg.contains("Rate limit") {
        LlmError::RateLimit
    } else if msg.contains("timed out") || msg.contains("timeout") || msg.contains("deadline") {
        LlmError::Timeout
    } else if msg.contains("connect") || msg.contains("dns") || msg.contains("connection") {
        LlmError::Network(msg)
    } else {
        LlmError::ApiError(msg)
    }
}

/// Check if an error is retryable (429, 500, 502, 503).
fn is_retryable(err: &async_openai::error::OpenAIError) -> bool {
    let msg = err.to_string();
    msg.contains("429")
        || msg.contains("500")
        || msg.contains("502")
        || msg.contains("503")
        || msg.contains("rate limit")
        || msg.contains("Rate limit")
        || msg.contains("server error")
        || msg.contains("Server Error")
}

// ---------------------------------------------------------------------------
// LlmProvider impl for OpenAiProvider
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl LlmProvider for OpenAiProvider {
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<Value>,
    ) -> Result<LlmResponse, LlmError> {
        let msg_count = messages.len();
        let tool_count = tools.len();
        info!(
            "LlmProvider.chat: model={}, messages={}, tools={}",
            self.model, msg_count, tool_count
        );

        let api_messages = convert_messages(&messages);
        let api_tools = convert_tools(&tools);

        let mut req_binding = CreateChatCompletionRequestArgs::default();
        let mut request_builder = req_binding
            .model(&self.model)
            .messages(api_messages);

        if !api_tools.is_empty() {
            request_builder = request_builder.tools(api_tools);
        }

        let request = request_builder
            .build()
            .map_err(|e: async_openai::error::OpenAIError| LlmError::ApiError(e.to_string()))?;

        debug!(
            "LlmProvider: sending request to model '{}' with {} messages",
            self.model, msg_count
        );

        // Retry loop for transient errors
        let mut last_error: Option<async_openai::error::OpenAIError> = None;
        for attempt in 0..=MAX_RETRIES {
            let result = tokio::time::timeout(
                Duration::from_secs(REQUEST_TIMEOUT_SECS),
                self.client.chat().create(request.clone()),
            )
            .await;

            match result {
                Ok(Ok(response)) => {
                    // Parse response
                    let choice = response.choices.first();
                    let (text, tool_calls) = if let Some(choice) = choice {
                        let msg = &choice.message;
                        let text = msg.content.clone();
                        let calls: Vec<ToolCall> = msg
                            .tool_calls
                            .as_ref()
                            .map(|tc| {
                                tc.iter()
                                    .filter_map(|c| match c {
                                        async_openai::types::chat::ChatCompletionMessageToolCalls::Function(f) => {
                                            Some(ToolCall {
                                                id: f.id.clone(),
                                                name: f.function.name.clone(),
                                                arguments: f.function.arguments.clone(),
                                            })
                                        }
                                        // Skip custom tool calls — not used in our agent loop
                                        _ => None,
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();

                        (text, calls)
                    } else {
                        (None, vec![])
                    };

                    let usage = response.usage.map(|u| TokenUsage {
                        prompt_tokens: u.prompt_tokens as u32,
                        completion_tokens: u.completion_tokens as u32,
                    });

                    info!(
                        "LlmProvider response: text={}, tool_calls={}, usage={:?}",
                        text.is_some(),
                        tool_calls.len(),
                        usage
                    );

                    return Ok(LlmResponse {
                        text,
                        tool_calls,
                        usage,
                    });
                }
                Ok(Err(api_err)) => {
                    let classified = classify_error(&api_err);
                    if !is_retryable(&api_err) || attempt >= MAX_RETRIES {
                        warn!(
                            "LlmProvider error (attempt {}/{}): {}",
                            attempt + 1,
                            MAX_RETRIES + 1,
                            classified
                        );
                        return Err(classified);
                    }
                    let backoff_ms = INITIAL_BACKOFF_MS * 2u64.pow(attempt);
                    warn!(
                        "LlmProvider retryable error (attempt {}/{}): {}, retrying in {}ms",
                        attempt + 1,
                        MAX_RETRIES + 1,
                        classified,
                        backoff_ms
                    );
                    last_error = Some(api_err);
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                }
                Err(_) => {
                    // Timeout
                    if attempt >= MAX_RETRIES {
                        warn!(
                            "LlmProvider timeout after {} attempts",
                            MAX_RETRIES + 1
                        );
                        return Err(LlmError::Timeout);
                    }
                    let backoff_ms = INITIAL_BACKOFF_MS * 2u64.pow(attempt);
                    warn!(
                        "LlmProvider timeout (attempt {}/{}), retrying in {}ms",
                        attempt + 1,
                        MAX_RETRIES + 1,
                        backoff_ms
                    );
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                }
            }
        }

        // Should be unreachable, but just in case
        Err(last_error.map(|e| classify_error(&e)).unwrap_or(LlmError::ApiError(
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

    // --- ChatMessage conversion tests ---

    #[test]
    fn test_convert_system_message() {
        let messages = vec![ChatMessage::System("You are helpful.".to_string())];
        let api_msgs = convert_messages(&messages);
        assert_eq!(api_msgs.len(), 1);
        match &api_msgs[0] {
            ChatCompletionRequestMessage::System(m) => match &m.content {
                async_openai::types::chat::ChatCompletionRequestSystemMessageContent::Text(t) => {
                    assert_eq!(t, "You are helpful.");
                }
                other => panic!("Expected Text content, got {:?}", other),
            },
            _ => panic!("Expected System message"),
        }
    }

    #[test]
    fn test_convert_user_message() {
        let messages = vec![ChatMessage::User("Hello".to_string())];
        let api_msgs = convert_messages(&messages);
        assert_eq!(api_msgs.len(), 1);
        match &api_msgs[0] {
            ChatCompletionRequestMessage::User(m) => match &m.content {
                async_openai::types::chat::ChatCompletionRequestUserMessageContent::Text(t) => {
                    assert_eq!(t, "Hello");
                }
                other => panic!("Expected Text content, got {:?}", other),
            },
            _ => panic!("Expected User message"),
        }
    }

    #[test]
    fn test_convert_assistant_message() {
        let messages = vec![ChatMessage::Assistant("Hi there".to_string())];
        let api_msgs = convert_messages(&messages);
        assert_eq!(api_msgs.len(), 1);
        match &api_msgs[0] {
            ChatCompletionRequestMessage::Assistant(m) => match &m.content {
                Some(async_openai::types::chat::ChatCompletionRequestAssistantMessageContent::Text(t)) => {
                    assert_eq!(t, "Hi there");
                }
                other => panic!("Expected Text content, got {:?}", other),
            },
            _ => panic!("Expected Assistant message"),
        }
    }

    #[test]
    fn test_convert_assistant_with_tools_message() {
        let messages = vec![ChatMessage::AssistantWithTools {
            text: Some("Let me tap that.".to_string()),
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "tap".to_string(),
                arguments: r#"{"x":100,"y":200}"#.to_string(),
            }],
        }];
        let api_msgs = convert_messages(&messages);
        assert_eq!(api_msgs.len(), 1);
        match &api_msgs[0] {
            ChatCompletionRequestMessage::Assistant(m) => {
                // Verify content is set
                assert!(m.content.is_some());
                // Verify tool_calls are set
                assert!(m.tool_calls.is_some());
                let tc = m.tool_calls.as_ref().unwrap();
                assert_eq!(tc.len(), 1);
            },
            _ => panic!("Expected Assistant message"),
        }
    }

    #[test]
    fn test_convert_assistant_with_tools_no_text() {
        let messages = vec![ChatMessage::AssistantWithTools {
            text: None,
            tool_calls: vec![ToolCall {
                id: "call_2".to_string(),
                name: "finish".to_string(),
                arguments: r#"{"result":"done"}"#.to_string(),
            }],
        }];
        let api_msgs = convert_messages(&messages);
        assert_eq!(api_msgs.len(), 1);
        match &api_msgs[0] {
            ChatCompletionRequestMessage::Assistant(m) => {
                assert!(m.tool_calls.is_some());
            },
            _ => panic!("Expected Assistant message"),
        }
    }

    #[test]
    fn test_convert_assistant_with_tools_multiple_calls() {
        let messages = vec![ChatMessage::AssistantWithTools {
            text: Some("I'll do both.".to_string()),
            tool_calls: vec![
                ToolCall {
                    id: "call_a".to_string(),
                    name: "tap".to_string(),
                    arguments: r#"{"x":100,"y":200}"#.to_string(),
                },
                ToolCall {
                    id: "call_b".to_string(),
                    name: "input_text".to_string(),
                    arguments: r#"{"text":"hello"}"#.to_string(),
                },
            ],
        }];
        let api_msgs = convert_messages(&messages);
        match &api_msgs[0] {
            ChatCompletionRequestMessage::Assistant(m) => {
                let tc = m.tool_calls.as_ref().unwrap();
                assert_eq!(tc.len(), 2);
            },
            _ => panic!("Expected Assistant message"),
        }
    }

    #[test]
    fn test_convert_full_conversation_with_assistant_with_tools() {
        // Simulate a full multi-round conversation
        let messages = vec![
            ChatMessage::System("You are helpful.".to_string()),
            ChatMessage::User("Tap the button".to_string()),
            ChatMessage::AssistantWithTools {
                text: Some("I'll tap it.".to_string()),
                tool_calls: vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "tap".to_string(),
                    arguments: r#"{"x":100,"y":200}"#.to_string(),
                }],
            },
            ChatMessage::ToolResult {
                tool_call_id: "call_1".to_string(),
                content: r#"{"success":true}"#.to_string(),
            },
        ];
        let api_msgs = convert_messages(&messages);
        assert_eq!(api_msgs.len(), 4);
    }

    #[test]
    fn test_convert_tool_result_message() {
        let messages = vec![ChatMessage::ToolResult {
            tool_call_id: "call_123".to_string(),
            content: "{\"temp\": 72}".to_string(),
        }];
        let api_msgs = convert_messages(&messages);
        assert_eq!(api_msgs.len(), 1);
        match &api_msgs[0] {
            ChatCompletionRequestMessage::Tool(m) => match &m.content {
                async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(t) => {
                    assert_eq!(t, "{\"temp\": 72}");
                }
                other => panic!("Expected Text content, got {:?}", other),
            },
            _ => panic!("Expected Tool message"),
        }
    }

    #[test]
    fn test_convert_mixed_messages() {
        let messages = vec![
            ChatMessage::System("sys".to_string()),
            ChatMessage::User("usr".to_string()),
            ChatMessage::Assistant("asst".to_string()),
            ChatMessage::ToolResult {
                tool_call_id: "id1".to_string(),
                content: "result".to_string(),
            },
        ];
        let api_msgs = convert_messages(&messages);
        assert_eq!(api_msgs.len(), 4);
    }

    #[test]
    fn test_convert_empty_messages() {
        let messages: Vec<ChatMessage> = vec![];
        let api_msgs = convert_messages(&messages);
        assert!(api_msgs.is_empty());
    }

    // --- Tool schema conversion tests ---

    #[test]
    fn test_convert_tool_schema_basic() {
        let schema = json!({
            "name": "tap",
            "description": "Tap the screen",
            "parameters": {
                "type": "object",
                "properties": {
                    "x": { "type": "integer", "description": "X coordinate" },
                    "y": { "type": "integer", "description": "Y coordinate" }
                },
                "required": ["x", "y"]
            }
        });

        let tool = convert_tool_schema(&schema).expect("should convert");
        assert_eq!(tool.function.name, "tap");
        assert_eq!(tool.function.description, Some("Tap the screen".to_string()));
        assert!(tool.function.parameters.is_some());
    }

    #[test]
    fn test_convert_tool_schema_no_params() {
        let schema = json!({
            "name": "get_screen_info",
            "description": "Get screen info"
        });

        let tool = convert_tool_schema(&schema).expect("should convert");
        assert_eq!(tool.function.name, "get_screen_info");
        assert!(tool.function.parameters.is_none());
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
        assert_eq!(tool.function.description, Some("".to_string()));
    }

    #[test]
    fn test_convert_tools_multiple() {
        let schemas = vec![
            json!({"name": "tap", "description": "Tap"}),
            json!({"name": "swipe", "description": "Swipe", "parameters": {"type": "object", "properties": {}}}),
        ];
        let tools = convert_tools(&schemas);
        assert_eq!(tools.len(), 2);
        // Verify they're wrapped in the Function variant
        match &tools[0] {
            ChatCompletionTools::Function(f) => assert_eq!(f.function.name, "tap"),
            _ => panic!("Expected Function variant"),
        }
        match &tools[1] {
            ChatCompletionTools::Function(f) => assert_eq!(f.function.name, "swipe"),
            _ => panic!("Expected Function variant"),
        }
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

    // --- Error classification tests ---

    #[test]
    fn test_classify_auth_error_401() {
        let err = async_openai::error::OpenAIError::ApiError(
            async_openai::error::ApiError {
                message: "Incorrect API key provided".to_string(),
                r#type: Some("invalid_request_error".to_string()),
                param: None,
                code: Some("401".to_string()),
            },
        );
        assert_eq!(classify_error(&err), LlmError::AuthFailed);
    }

    #[test]
    fn test_classify_auth_error_403() {
        let err = async_openai::error::OpenAIError::ApiError(
            async_openai::error::ApiError {
                message: "Forbidden".to_string(),
                r#type: None,
                param: None,
                code: Some("403".to_string()),
            },
        );
        assert_eq!(classify_error(&err), LlmError::AuthFailed);
    }

    #[test]
    fn test_classify_rate_limit_429() {
        let err = async_openai::error::OpenAIError::ApiError(
            async_openai::error::ApiError {
                message: "Rate limit reached".to_string(),
                r#type: None,
                param: None,
                code: Some("429".to_string()),
            },
        );
        assert_eq!(classify_error(&err), LlmError::RateLimit);
    }

    #[test]
    fn test_classify_timeout() {
        let err = async_openai::error::OpenAIError::ApiError(
            async_openai::error::ApiError {
                message: "request timed out".to_string(),
                r#type: None,
                param: None,
                code: None,
            },
        );
        assert_eq!(classify_error(&err), LlmError::Timeout);
    }

    #[test]
    fn test_classify_network_connect() {
        let err = async_openai::error::OpenAIError::ApiError(
            async_openai::error::ApiError {
                message: "connection refused".to_string(),
                r#type: None,
                param: None,
                code: None,
            },
        );
        match classify_error(&err) {
            LlmError::Network(_) => {}
            other => panic!("Expected Network, got {:?}", other),
        }
    }

    #[test]
    fn test_classify_generic_api_error() {
        let err = async_openai::error::OpenAIError::ApiError(
            async_openai::error::ApiError {
                message: "Bad request".to_string(),
                r#type: None,
                param: None,
                code: Some("400".to_string()),
            },
        );
        match classify_error(&err) {
            LlmError::ApiError(msg) => assert!(msg.contains("Bad request")),
            other => panic!("Expected ApiError, got {:?}", other),
        }
    }

    // --- Retryability tests ---

    #[test]
    fn test_retryable_429() {
        let err = async_openai::error::OpenAIError::ApiError(
            async_openai::error::ApiError {
                message: "rate limit".to_string(),
                r#type: None,
                param: None,
                code: Some("429".to_string()),
            },
        );
        assert!(is_retryable(&err));
    }

    #[test]
    fn test_retryable_500() {
        let err = async_openai::error::OpenAIError::ApiError(
            async_openai::error::ApiError {
                message: "server error".to_string(),
                r#type: None,
                param: None,
                code: Some("500".to_string()),
            },
        );
        assert!(is_retryable(&err));
    }

    #[test]
    fn test_retryable_503() {
        let err = async_openai::error::OpenAIError::ApiError(
            async_openai::error::ApiError {
                message: "Service unavailable".to_string(),
                r#type: None,
                param: None,
                code: Some("503".to_string()),
            },
        );
        assert!(is_retryable(&err));
    }

    #[test]
    fn test_not_retryable_401() {
        let err = async_openai::error::OpenAIError::ApiError(
            async_openai::error::ApiError {
                message: "Unauthorized".to_string(),
                r#type: None,
                param: None,
                code: Some("401".to_string()),
            },
        );
        assert!(!is_retryable(&err));
    }

    // --- LlmResponse parsing tests (mock JSON) ---

    #[test]
    fn test_llm_response_text_only() {
        let response = LlmResponse {
            text: Some("Hello!".to_string()),
            tool_calls: vec![],
            usage: Some(TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
            }),
        };
        assert_eq!(response.text, Some("Hello!".to_string()));
        assert!(response.tool_calls.is_empty());
        assert!(response.usage.is_some());
    }

    #[test]
    fn test_llm_response_tool_calls_only() {
        let response = LlmResponse {
            text: None,
            tool_calls: vec![ToolCall {
                id: "call_abc".to_string(),
                name: "tap".to_string(),
                arguments: r#"{"x": 100, "y": 200}"#.to_string(),
            }],
            usage: Some(TokenUsage {
                prompt_tokens: 50,
                completion_tokens: 20,
            }),
        };
        assert!(response.text.is_none());
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "tap");
        assert_eq!(response.tool_calls[0].id, "call_abc");
    }

    #[test]
    fn test_llm_response_empty_tool_call_args() {
        let response = LlmResponse {
            text: None,
            tool_calls: vec![ToolCall {
                id: "call_empty".to_string(),
                name: "finish".to_string(),
                arguments: "".to_string(),
            }],
            usage: None,
        };
        assert_eq!(response.tool_calls[0].arguments, "");
        assert!(response.usage.is_none());
    }

    #[test]
    fn test_llm_response_no_choices() {
        let response = LlmResponse {
            text: None,
            tool_calls: vec![],
            usage: None,
        };
        assert!(response.text.is_none());
        assert!(response.tool_calls.is_empty());
        assert!(response.usage.is_none());
    }

    // --- LlmError display tests ---

    #[test]
    fn test_llm_error_display() {
        assert_eq!(
            LlmError::ApiError("bad".to_string()).to_string(),
            "API error: bad"
        );
        assert_eq!(LlmError::RateLimit.to_string(), "Rate limited");
        assert_eq!(LlmError::Timeout.to_string(), "Request timed out");
        assert_eq!(LlmError::AuthFailed.to_string(), "Authentication failed");
        assert_eq!(
            LlmError::Network("dns failed".to_string()).to_string(),
            "Network error: dns failed"
        );
    }

    // --- Full tool schema from ToolRegistry integration ---

    #[test]
    fn test_convert_full_tap_tool_from_registry() {
        use crate::agent::tool_registry::ToolRegistry;

        let registry = ToolRegistry::default();
        let tap = registry.get("tap").expect("tap should exist");

        // Build the full schema as it would be passed to the provider
        let schema = json!({
            "name": tap.name,
            "description": tap.description_en,
            "parameters": tap.parameters_json_schema()
        });

        let tool = convert_tool_schema(&schema).expect("should convert");
        assert_eq!(tool.function.name, "tap");
        assert!(tool.function.parameters.is_some());

        let params = tool.function.parameters.unwrap();
        assert_eq!(params["type"], "object");
        let props = params["properties"].as_object().unwrap();
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
    }
}
