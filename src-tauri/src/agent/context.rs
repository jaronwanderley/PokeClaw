// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

//! Context compression for the agent loop's message history.
//!
//! For S02 this module provides:
//! - A placeholder map for observation tools (large-output tools whose results
//!   can be replaced with short placeholders in older rounds).
//! - A `compress_tool_result` function that replaces observation tool results
//!   with placeholders and truncates long results from non-observation tools.
//!
//! The full protected-zone logic (keeping recent N rounds intact, compressing
//! older rounds) depends on the message history type that T02 defines, so this
//! module exports the compression primitives that T02 will orchestrate.

use std::collections::HashMap;

/// Number of most-recent rounds kept intact during compression.
pub const KEEP_RECENT_ROUNDS: usize = 3;

/// Observation tools whose output can be replaced with short placeholders
/// during context compression (older rounds only).
pub static OBSERVATION_PLACEHOLDERS: &[(&str, &str)] = &[
    ("get_screen_info", "[screen info omitted]"),
    ("take_screenshot", "[screenshot result omitted]"),
    ("find_node_info", "[node find result omitted]"),
    ("get_installed_apps", "[app list omitted]"),
    ("scroll_to_find", "[scroll find result omitted]"),
];

/// Build a HashMap from the static placeholder list.
pub fn placeholder_map() -> HashMap<&'static str, &'static str> {
    OBSERVATION_PLACEHOLDERS.iter().copied().collect()
}

/// Maximum length for a tool result before it's considered "long" and eligible
/// for compression.
const SHORT_THRESHOLD: usize = 100;

/// Compress a tool result string.
///
/// - If the tool name has a placeholder entry and the result is long enough,
///   returns the placeholder.
/// - If the result is short (≤ `SHORT_THRESHOLD`), returns it unchanged.
/// - Otherwise returns a truncated summary: `"✓ " + first 80 chars + "..."`.
pub fn compress_tool_result(tool_name: &str, result: &str) -> String {
    // Short results are never compressed
    if result.len() <= SHORT_THRESHOLD {
        return result.to_string();
    }

    // Check for observation tool placeholder
    let placeholders = placeholder_map();
    if let Some(&placeholder) = placeholders.get(tool_name) {
        return placeholder.to_string();
    }

    // Generic truncation
    summarize_tool_result(result)
}

/// Summarize a tool result into a one-line summary.
///
/// Tries to parse as JSON and extract success/data or error fields.
/// Falls back to plain truncation if parsing fails.
pub fn summarize_tool_result(result: &str) -> String {
    // Try JSON extraction
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(result) {
        let is_success = val
            .get("isSuccess")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if is_success {
            let data = val
                .get("data")
                .map(|v| v.to_string())
                .unwrap_or_else(|| "ok".to_string());
            let truncated = if data.len() > 80 {
                format!("{}...", &data[..80])
            } else {
                data
            };
            format!("✓ {}", truncated)
        } else {
            let error = val
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("failed");
            let truncated = if error.len() > 80 {
                format!("{}...", &error[..80])
            } else {
                error.to_string()
            };
            format!("✗ {}", truncated)
        }
    } else {
        // Not JSON — plain truncation
        let truncated = if result.len() > 80 {
            format!("{}...", &result[..80])
        } else {
            result.to_string()
        };
        format!("✓ {}", truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Placeholder map ──────────────────────────────────────────────

    #[test]
    fn placeholder_map_contains_all_tools() {
        let map = placeholder_map();
        assert_eq!(map.len(), 5);
        assert!(map.contains_key("get_screen_info"));
        assert!(map.contains_key("take_screenshot"));
        assert!(map.contains_key("find_node_info"));
        assert!(map.contains_key("get_installed_apps"));
        assert!(map.contains_key("scroll_to_find"));
    }

    #[test]
    fn placeholder_values() {
        let map = placeholder_map();
        assert_eq!(map.get("get_screen_info"), Some(&"[screen info omitted]"));
        assert_eq!(map.get("take_screenshot"), Some(&"[screenshot result omitted]"));
    }

    // ── compress_tool_result ─────────────────────────────────────────

    #[test]
    fn short_result_unchanged() {
        let result = compress_tool_result("any_tool", "short");
        assert_eq!(result, "short");
    }

    #[test]
    fn short_result_exactly_at_threshold() {
        let input = "x".repeat(100);
        let result = compress_tool_result("any_tool", &input);
        assert_eq!(result, input); // 100 <= 100 → unchanged
    }

    #[test]
    fn observation_tool_compressed() {
        let long_result = "x".repeat(500);
        let result = compress_tool_result("get_screen_info", &long_result);
        assert_eq!(result, "[screen info omitted]");
    }

    #[test]
    fn observation_tool_short_not_compressed() {
        let result = compress_tool_result("get_screen_info", "ok");
        assert_eq!(result, "ok"); // short → unchanged
    }

    #[test]
    fn all_observation_tools_compressed() {
        let long = "x".repeat(500);
        for &(tool, placeholder) in OBSERVATION_PLACEHOLDERS {
            let result = compress_tool_result(tool, &long);
            assert_eq!(result, placeholder, "tool {} should compress to {:?}", tool, placeholder);
        }
    }

    #[test]
    fn non_observation_tool_long_result_truncated() {
        let long = "a".repeat(200);
        let result = compress_tool_result("some_tool", &long);
        // Not JSON, so plain truncation: "✓ " + first 80 + "..."
        assert!(result.starts_with("✓ "));
        assert!(result.len() < 200);
    }

    // ── summarize_tool_result ────────────────────────────────────────

    #[test]
    fn summarize_success_json() {
        let json = r#"{"isSuccess": true, "data": "button clicked"}"#;
        let summary = summarize_tool_result(json);
        assert_eq!(summary, "✓ \"button clicked\"");
    }

    #[test]
    fn summarize_failure_json() {
        let json = r#"{"isSuccess": false, "error": "node not found"}"#;
        let summary = summarize_tool_result(json);
        assert_eq!(summary, "✗ node not found");
    }

    #[test]
    fn summarize_long_data_json() {
        let long_data = "x".repeat(200);
        let json = format!(r#"{{"isSuccess": true, "data": "{}"}}"#, long_data);
        let summary = summarize_tool_result(&json);
        assert!(summary.starts_with("✓ "));
        assert!(summary.contains("..."));
        assert!(summary.len() < 200);
    }

    #[test]
    fn summarize_non_json() {
        let text = "Just plain text that is quite long and goes on and on and on and should be truncated";
        let summary = summarize_tool_result(text);
        assert!(summary.starts_with("✓ "));
    }

    #[test]
    fn summarize_json_without_success_field() {
        let json = r#"{"someField": "value"}"#;
        let summary = summarize_tool_result(json);
        // isSuccess defaults to false → failure path
        assert!(summary.starts_with("✗ "));
    }

    #[test]
    fn summarize_long_error_json() {
        let long_error = "e".repeat(200);
        let json = format!(r#"{{"isSuccess": false, "error": "{}"}}"#, long_error);
        let summary = summarize_tool_result(&json);
        assert!(summary.starts_with("✗ "));
        assert!(summary.contains("..."));
    }

    // ── KEEP_RECENT_ROUNDS ───────────────────────────────────────────

    #[test]
    fn keep_recent_rounds_value() {
        assert_eq!(KEEP_RECENT_ROUNDS, 3);
    }
}
