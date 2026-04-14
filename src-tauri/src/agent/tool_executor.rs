// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

use log::{info, warn};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Desktop real-OS imports (gated same as DesktopToolExecutor)
// ---------------------------------------------------------------------------
#[cfg(not(target_os = "android"))]
use tauri_plugin_pokeclaw::desktop::{automation, system, screen, kb};

// ---------------------------------------------------------------------------
// ToolResult — mirrors plugin ToolResult for agent-internal use
// ---------------------------------------------------------------------------

/// Result of a tool execution, matching the plugin's ToolResult shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// ToolCallResult — used in AgentRoundResult
// ---------------------------------------------------------------------------

/// Parsed tool call with its execution result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallResult {
    pub name: String,
    pub arguments: Value,
    pub result: ToolResult,
}

// ---------------------------------------------------------------------------
// TokenUsage — returned inside AgentRoundResult
// ---------------------------------------------------------------------------

/// Token usage from the LLM API response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

// ---------------------------------------------------------------------------
// AgentRoundResult — end-to-end agent round result
// ---------------------------------------------------------------------------

/// Full result of one agent round: prompt → LLM → (optional) tool → result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentRoundResult {
    pub prompt: String,
    pub model: String,
    pub tool_call: Option<ToolCallResult>,
    pub response_text: Option<String>,
    pub latency_ms: u64,
    pub token_usage: Option<TokenUsage>,
}

// ---------------------------------------------------------------------------
// ToolExecutor trait
// ---------------------------------------------------------------------------

/// Abstraction for executing tool calls by name.
pub trait ToolExecutor: Send + Sync {
    /// Execute a tool by name with the given JSON parameters.
    fn execute(&self, tool_name: &str, params: Value) -> ToolResult;

    /// Return the names of tools this executor can dispatch.
    fn available_tools(&self) -> Vec<String>;
}

// ---------------------------------------------------------------------------
// Desktop real-OS ToolResult conversion
// ---------------------------------------------------------------------------

/// Convert the plugin's ToolResult to the agent-internal ToolResult.
/// Both structs have identical public fields (success, data, error).
#[cfg(not(target_os = "android"))]
fn convert_tool_result(plugin_result: tauri_plugin_pokeclaw::ToolResult) -> ToolResult {
    ToolResult {
        success: plugin_result.success,
        data: plugin_result.data,
        error: plugin_result.error,
    }
}

// ---------------------------------------------------------------------------
// DesktopToolExecutor — dispatches real OS calls for input/clipboard/device tools
// ---------------------------------------------------------------------------

/// Executes tool calls using real OS-level implementations for desktop input,
/// clipboard, and device-info tools. Other tools remain as mock implementations.
pub struct DesktopToolExecutor;

impl DesktopToolExecutor {
    pub fn new() -> Self {
        info!("DesktopToolExecutor created");
        Self
    }
}

impl Default for DesktopToolExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolExecutor for DesktopToolExecutor {
    fn execute(&self, tool_name: &str, params: Value) -> ToolResult {
        let start = std::time::Instant::now();
        info!("DesktopToolExecutor: executing tool '{}' with params: {}", tool_name, params);

        let result = match tool_name {
            // --- Observation tools ---
            "get_screen_info" => convert_tool_result(screen::do_get_screen_info()),
            "find_node_info" => {
                let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
                convert_tool_result(screen::do_find_node_info(text))
            }
            "input_text" => {
                let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let node_id = params.get("node_id").and_then(|v| v.as_str());
                let clear_first = params.get("clear_first").and_then(|v| v.as_bool());
                convert_tool_result(automation::do_input_text(text, node_id, clear_first))
            }
            "system_key" => {
                let key = params.get("key").and_then(|v| v.as_str()).unwrap_or("");
                convert_tool_result(automation::do_system_key(key))
            }
            "open_app" => {
                let app_name = params.get("app_name").and_then(|v| v.as_str()).unwrap_or("");
                convert_tool_result(automation::do_open_app(app_name))
            }
            "get_installed_apps" => {
                let filter = params.get("filter").and_then(|v| v.as_str());
                convert_tool_result(screen::do_get_installed_apps(filter))
            }
            "take_screenshot" => {
                let file_path = params.get("file_path").and_then(|v| v.as_str());
                convert_tool_result(screen::do_take_screenshot(file_path))
            }
            "wait" => self.mock_wait(&params),
            "repeat_actions" => self.mock_repeat_actions(&params),
            "clipboard" => {
                let action = params.get("action").and_then(|v| v.as_str()).unwrap_or("get");
                match action {
                    "get" => convert_tool_result(system::do_clipboard_get()),
                    "set" => {
                        let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        convert_tool_result(system::do_clipboard_set(text))
                    }
                    _ => ToolResult {
                        success: false,
                        data: None,
                        error: Some(format!("Unknown clipboard action: {}", action)),
                    },
                }
            }
            "send_file" => self.mock_send_file(&params),
            "get_device_info" => {
                let category = params.get("category").and_then(|v| v.as_str()).unwrap_or("device");
                convert_tool_result(system::do_get_device_info(category))
            }
            "get_notifications" => self.mock_get_notifications(),
            "make_call" => self.mock_make_call(&params),
            "finish" => self.mock_finish(&params),
            "kb_write" => {
                let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");
                convert_tool_result(kb::do_kb_write(path, content))
            }
            "kb_read" => {
                let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
                convert_tool_result(kb::do_kb_read(path))
            }
            "kb_search" => {
                let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
                convert_tool_result(kb::do_kb_search(query))
            }
            "kb_append" => {
                let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");
                convert_tool_result(kb::do_kb_append(path, content))
            }
            "kb_add_todo" => {
                let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("");
                convert_tool_result(kb::do_kb_add_todo(text))
            }

            // --- Mobile-only tools (real OS on desktop) ---
            "tap" => {
                let x = params.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let y = params.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                convert_tool_result(automation::do_tap(x, y))
            }
            "tap_node" => self.mock_tap_node(&params),
            "long_press" => {
                let x = params.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let y = params.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let duration_ms = params.get("duration_ms").and_then(|v| v.as_i64()).map(|d| d as i32);
                convert_tool_result(automation::do_long_press(x, y, duration_ms))
            }
            "swipe" => {
                let sx = params.get("start_x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let sy = params.get("start_y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let ex = params.get("end_x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let ey = params.get("end_y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let duration_ms = params.get("duration_ms").and_then(|v| v.as_i64()).map(|d| d as i32);
                convert_tool_result(automation::do_swipe(sx, sy, ex, ey, duration_ms))
            }
            "scroll_to_find" => self.mock_scroll_to_find(&params),
            "find_and_tap" => self.mock_find_and_tap(&params),
            "send_message" => self.mock_send_message(&params),
            "auto_reply" => self.mock_auto_reply(&params),

            _ => {
                warn!("DesktopToolExecutor: unknown tool '{}'", tool_name);
                ToolResult {
                    success: false,
                    data: None,
                    error: Some(format!("Unknown tool: {}", tool_name)),
                }
            }
        };

        let elapsed = start.elapsed();
        info!(
            "DesktopToolExecutor: tool '{}' completed in {:?} — success={}",
            tool_name, elapsed, result.success
        );
        result
    }

    fn available_tools(&self) -> Vec<String> {
        vec![
            "get_screen_info".into(),
            "find_node_info".into(),
            "input_text".into(),
            "system_key".into(),
            "open_app".into(),
            "get_installed_apps".into(),
            "take_screenshot".into(),
            "wait".into(),
            "repeat_actions".into(),
            "clipboard".into(),
            "send_file".into(),
            "get_device_info".into(),
            "get_notifications".into(),
            "make_call".into(),
            "finish".into(),
            "kb_write".into(),
            "kb_read".into(),
            "kb_search".into(),
            "kb_append".into(),
            "kb_add_todo".into(),
            "tap".into(),
            "tap_node".into(),
            "long_press".into(),
            "swipe".into(),
            "scroll_to_find".into(),
            "find_and_tap".into(),
            "send_message".into(),
            "auto_reply".into(),
        ]
    }
}

// ---------------------------------------------------------------------------
// Mock implementations — mirrors the desktop_commands logic
// ---------------------------------------------------------------------------

impl DesktopToolExecutor {
    fn mock_wait(&self, params: &Value) -> ToolResult {
        let ms = params.get("milliseconds").and_then(|v| v.as_u64()).unwrap_or(1000);
        ToolResult {
            success: true,
            data: Some(serde_json::json!({ "message": format!("Waited {}ms", ms) })),
            error: None,
        }
    }

    fn mock_repeat_actions(&self, params: &Value) -> ToolResult {
        let count = params.get("count").and_then(|v| v.as_u64()).unwrap_or(1);
        ToolResult {
            success: true,
            data: Some(serde_json::json!({ "message": format!("Repeated {} actions", count) })),
            error: None,
        }
    }

    fn mock_send_file(&self, params: &Value) -> ToolResult {
        let contact = params.get("contact").and_then(|v| v.as_str()).unwrap_or("");
        let app = params.get("app").and_then(|v| v.as_str()).unwrap_or("");
        ToolResult {
            success: true,
            data: Some(serde_json::json!({
                "message": format!("Sent file to '{}' via '{}'", contact, app)
            })),
            error: None,
        }
    }

    fn mock_get_notifications(&self) -> ToolResult {
        ToolResult {
            success: true,
            data: Some(serde_json::json!({
                "notifications": [
                    { "package": "com.whatsapp", "text": "John: Hey!" },
                ]
            })),
            error: None,
        }
    }

    fn mock_make_call(&self, params: &Value) -> ToolResult {
        let contact = params.get("contact").and_then(|v| v.as_str()).unwrap_or("");
        if contact.trim().is_empty() {
            return ToolResult {
                success: false,
                data: None,
                error: Some("contact must not be empty".into()),
            };
        }
        ToolResult {
            success: true,
            data: Some(serde_json::json!({ "message": format!("Calling '{}'", contact) })),
            error: None,
        }
    }

    fn mock_finish(&self, params: &Value) -> ToolResult {
        let result = params.get("result").and_then(|v| v.as_str()).unwrap_or("Done");
        ToolResult {
            success: true,
            data: Some(serde_json::json!({ "result": result })),
            error: None,
        }
    }

    // --- Mobile tool mocks ---

    fn mock_tap_node(&self, params: &Value) -> ToolResult {
        let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("");
        if text.trim().is_empty() {
            return ToolResult {
                success: false,
                data: None,
                error: Some("No identifier provided (text, resource_id, or description required)".into()),
            };
        }
        ToolResult {
            success: true,
            data: Some(serde_json::json!({ "message": format!("Tapped node with text '{}' at (540,160)", text) })),
            error: None,
        }
    }

    fn mock_scroll_to_find(&self, params: &Value) -> ToolResult {
        let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("");
        if text.trim().is_empty() {
            return ToolResult {
                success: false,
                data: None,
                error: Some("text must not be empty".into()),
            };
        }
        ToolResult {
            success: true,
            data: Some(serde_json::json!({ "message": format!("Found '{}' after scrolling", text) })),
            error: None,
        }
    }

    fn mock_find_and_tap(&self, params: &Value) -> ToolResult {
        let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("");
        if text.trim().is_empty() {
            return ToolResult {
                success: false,
                data: None,
                error: Some("text must not be empty".into()),
            };
        }
        ToolResult {
            success: true,
            data: Some(serde_json::json!({ "message": format!("Found and tapped '{}'", text) })),
            error: None,
        }
    }

    fn mock_send_message(&self, params: &Value) -> ToolResult {
        let contact = params.get("contact").and_then(|v| v.as_str()).unwrap_or("");
        let message = params.get("message").and_then(|v| v.as_str()).unwrap_or("");
        let app = params.get("app").and_then(|v| v.as_str()).unwrap_or("");
        if contact.trim().is_empty() || message.trim().is_empty() || app.trim().is_empty() {
            return ToolResult {
                success: false,
                data: None,
                error: Some("contact, message, and app must not be empty".into()),
            };
        }
        ToolResult {
            success: true,
            data: Some(serde_json::json!({
                "message": format!("Sent '{}' to {} via {}", message, contact, app)
            })),
            error: None,
        }
    }

    fn mock_auto_reply(&self, params: &Value) -> ToolResult {
        let notification_id = params.get("notification_id").and_then(|v| v.as_str()).unwrap_or("");
        ToolResult {
            success: true,
            data: Some(serde_json::json!({
                "message": format!("Auto-replied to notification '{}'", notification_id)
            })),
            error: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn executor() -> DesktopToolExecutor {
        DesktopToolExecutor::new()
    }

    // --- Trait contract ---

    #[test]
    fn test_available_tools_count() {
        let exec = executor();
        let tools = exec.available_tools();
        assert_eq!(tools.len(), 28, "Should list all 28 tools");
    }

    #[test]
    fn test_unknown_tool_returns_error() {
        let exec = executor();
        let result = exec.execute("nonexistent_tool", json!({}));
        assert!(!result.success);
        assert!(result.error.unwrap().contains("Unknown tool"));
    }

    // --- Observation tool mocks ---

    #[test]
    fn test_get_screen_info() {
        let exec = executor();
        let result = exec.execute("get_screen_info", json!({}));
        assert!(result.success);
        assert!(result.data.unwrap().get("tree").is_some());
    }

    #[test]
    fn test_find_node_info_with_text() {
        let exec = executor();
        let result = exec.execute("find_node_info", json!({ "text": "John" }));
        assert!(result.success);
    }

    #[test]
    fn test_find_node_info_empty_text() {
        let exec = executor();
        let result = exec.execute("find_node_info", json!({ "text": "" }));
        assert!(!result.success);
    }

    #[test]
    fn test_input_text_valid() {
        let exec = executor();
        let result = exec.execute("input_text", json!({ "text": "hello" }));
        assert!(result.success);
    }

    #[test]
    fn test_input_text_empty() {
        let exec = executor();
        let result = exec.execute("input_text", json!({ "text": "" }));
        assert!(!result.success);
    }

    #[test]
    fn test_system_key_valid() {
        let exec = executor();
        let result = exec.execute("system_key", json!({ "key": "home" }));
        assert!(result.success);
    }

    #[test]
    fn test_system_key_invalid() {
        let exec = executor();
        let result = exec.execute("system_key", json!({ "key": "fly" }));
        assert!(!result.success);
    }

    #[test]
    fn test_open_app_valid() {
        let exec = executor();
        let result = exec.execute("open_app", json!({ "app_name": "Chrome" }));
        assert!(result.success);
    }

    #[test]
    fn test_open_app_empty() {
        let exec = executor();
        let result = exec.execute("open_app", json!({ "app_name": "" }));
        assert!(!result.success);
    }

    #[test]
    fn test_get_device_info() {
        let exec = executor();
        let result = exec.execute("get_device_info", json!({}));
        assert!(result.success);
    }

    #[test]
    fn test_get_notifications() {
        let exec = executor();
        let result = exec.execute("get_notifications", json!({}));
        assert!(result.success);
    }

    #[test]
    fn test_make_call_valid() {
        let exec = executor();
        let result = exec.execute("make_call", json!({ "contact": "John" }));
        assert!(result.success);
    }

    #[test]
    fn test_make_call_empty() {
        let exec = executor();
        let result = exec.execute("make_call", json!({ "contact": "" }));
        assert!(!result.success);
    }

    #[test]
    fn test_finish() {
        let exec = executor();
        let result = exec.execute("finish", json!({ "result": "Done", "success": true }));
        assert!(result.success);
    }

    // --- KB tool tests ---

    #[test]
    fn test_kb_write() {
        let kb_dir = std::env::temp_dir().join("pokeclaw_test_kb_write");
        let _ = std::fs::remove_dir_all(&kb_dir);
        std::env::set_var("POKECLAW_KB_DIR", &kb_dir);
        
        let exec = executor();
        let result = exec.execute("kb_write", json!({ "path": "test.md", "content": "Hello" }));
        assert!(result.success);
        assert!(kb_dir.join("test.md").exists());
    }

    #[test]
    fn test_kb_read() {
        let kb_dir = std::env::temp_dir().join("pokeclaw_test_kb_read");
        let _ = std::fs::remove_dir_all(&kb_dir);
        std::fs::create_dir_all(&kb_dir).unwrap();
        std::fs::write(kb_dir.join("test.md"), "Hello read").unwrap();
        std::env::set_var("POKECLAW_KB_DIR", &kb_dir);

        let exec = executor();
        let result = exec.execute("kb_read", json!({ "path": "test.md" }));
        assert!(result.success);
        assert_eq!(result.data.unwrap()["content"], "Hello read");
    }

    #[test]
    fn test_kb_search() {
        let kb_dir = std::env::temp_dir().join("pokeclaw_test_kb_search");
        let _ = std::fs::remove_dir_all(&kb_dir);
        std::fs::create_dir_all(&kb_dir).unwrap();
        std::fs::write(kb_dir.join("note.md"), "Project X secret").unwrap();
        std::env::set_var("POKECLAW_KB_DIR", &kb_dir);

        let exec = executor();
        let result = exec.execute("kb_search", json!({ "query": "project x" }));
        assert!(result.success);
        let results = result.data.unwrap()["results"].as_array().unwrap().clone();
        assert!(!results.is_empty());
    }

    // --- Mobile tool mocks ---

    #[test]
    fn test_tap() {
        let exec = executor();
        let result = exec.execute("tap", json!({ "x": 100, "y": 200 }));
        assert!(result.success);
    }

    #[test]
    fn test_swipe() {
        let exec = executor();
        let result = exec.execute("swipe", json!({
            "start_x": 500, "start_y": 1000, "end_x": 500, "end_y": 200
        }));
        assert!(result.success);
    }

    #[test]
    fn test_long_press() {
        let exec = executor();
        let result = exec.execute("long_press", json!({ "x": 300, "y": 400 }));
        assert!(result.success);
    }

    #[test]
    fn test_send_message_valid() {
        let exec = executor();
        let result = exec.execute("send_message", json!({
            "contact": "John", "message": "Hi!", "app": "whatsapp"
        }));
        assert!(result.success);
    }

    #[test]
    fn test_send_message_missing_fields() {
        let exec = executor();
        let result = exec.execute("send_message", json!({ "contact": "", "message": "Hi!", "app": "whatsapp" }));
        assert!(!result.success);
    }

    // --- AgentRoundResult serialization ---

    #[test]
    fn test_agent_round_result_text_only() {
        let result = AgentRoundResult {
            prompt: "Hello".into(),
            model: "gpt-4".into(),
            tool_call: None,
            response_text: Some("Hi there!".into()),
            latency_ms: 500,
            token_usage: Some(TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
            }),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["prompt"], "Hello");
        assert_eq!(json["model"], "gpt-4");
        assert!(json["tool_call"].is_null());
        assert_eq!(json["response_text"], "Hi there!");
    }

    #[test]
    fn test_agent_round_result_with_tool_call() {
        let result = AgentRoundResult {
            prompt: "Tap the button".into(),
            model: "gpt-4".into(),
            tool_call: Some(ToolCallResult {
                name: "tap".into(),
                arguments: json!({ "x": 100, "y": 200 }),
                result: ToolResult {
                    success: true,
                    data: Some(json!({ "message": "Tapped at (100, 200)" })),
                    error: None,
                },
            }),
            response_text: None,
            latency_ms: 1500,
            token_usage: Some(TokenUsage {
                prompt_tokens: 50,
                completion_tokens: 20,
            }),
        };
        let json = serde_json::to_value(&result).unwrap();
        let tc = json["tool_call"].as_object().unwrap();
        assert_eq!(tc["name"], "tap");
        assert!(tc["result"]["success"].as_bool().unwrap());
    }

    #[test]
    fn test_tool_result_serialization_roundtrip() {
        let original = ToolResult {
            success: true,
            data: Some(json!({ "key": "value" })),
            error: None,
        };
        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: ToolResult = serde_json::from_str(&serialized).unwrap();
        assert_eq!(original, deserialized);
    }

    // --- Empty/boundary arguments ---

    #[test]
    fn test_execute_with_empty_params_object() {
        let exec = executor();
        let result = exec.execute("get_screen_info", json!({}));
        assert!(result.success);
    }

    #[test]
    fn test_execute_with_null_params() {
        let exec = executor();
        let result = exec.execute("get_screen_info", Value::Null);
        assert!(result.success);
    }

    #[test]
    fn test_execute_all_28_tools_succeed_with_valid_params() {
        let kb_dir = std::env::temp_dir().join("pokeclaw_test_all_tools");
        let _ = std::fs::remove_dir_all(&kb_dir);
        std::env::set_var("POKECLAW_KB_DIR", &kb_dir);

        let exec = executor();
        let all_tools_with_params = [
            ("get_screen_info", json!({})),
            ("find_node_info", json!({ "text": "test" })),
            ("input_text", json!({ "text": "hello" })),
            ("system_key", json!({ "key": "home" })),
            ("open_app", json!({ "app_name": "Chrome" })),
            ("get_installed_apps", json!({})),
            ("take_screenshot", json!({})),
            ("wait", json!({ "milliseconds": 100 })),
            ("repeat_actions", json!({ "actions": "[]", "count": 1 })),
            ("clipboard", json!({ "action": "get" })),
            ("send_file", json!({ "contact": "A", "file_path": "/tmp/f", "app": "whatsapp" })),
            ("get_device_info", json!({})),
            ("get_notifications", json!({})),
            ("make_call", json!({ "contact": "John" })),
            ("finish", json!({ "result": "done" })),
            ("kb_write", json!({ "path": "a.md", "content": "hi" })),
            ("kb_read", json!({ "path": "a.md" })),
            ("kb_search", json!({ "query": "test" })),
            ("kb_append", json!({ "path": "a.md", "content": "more" })),
            ("kb_add_todo", json!({ "text": "todo" })),
            ("tap", json!({ "x": 100, "y": 200 })),
            ("tap_node", json!({ "text": "button" })),
            ("long_press", json!({ "x": 100, "y": 200 })),
            ("swipe", json!({ "start_x": 500, "start_y": 1000, "end_x": 500, "end_y": 200 })),
            ("scroll_to_find", json!({ "text": "target" })),
            ("find_and_tap", json!({ "text": "button" })),
            ("send_message", json!({ "contact": "A", "message": "Hi", "app": "whatsapp" })),
            ("auto_reply", json!({ "notification_id": "n1" })),
        ];

        for (name, params) in &all_tools_with_params {
            let result = exec.execute(name, params.clone());
            assert!(result.success, "Tool '{}' should succeed with valid params, got error: {:?}", name, result.error);
        }
        assert_eq!(all_tools_with_params.len(), 28);
    }
}
