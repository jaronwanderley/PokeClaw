// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// ParsedTask — Tier 1 deterministic parse output
// ---------------------------------------------------------------------------

/// Result of deterministic Tier 1 task parsing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParsedTask {
    /// A single tool invocation with parameters — bypasses LLM entirely.
    DirectTool {
        tool_name: String,
        params: HashMap<String, Value>,
        description: String,
    },
}

// ---------------------------------------------------------------------------
// Compiled regex patterns
// ---------------------------------------------------------------------------

static SCREENSHOT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(take\s+)?screenshot(s)?\b").unwrap());

static SCREENCAP_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bscreencap(ture)?\b").unwrap());

static GO_BACK_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^go\s+back$|^back$").unwrap());

static GO_HOME_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^go\s+home$|^home$").unwrap());

static OPEN_APP_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^open\s+(.+)$").unwrap());

// ---------------------------------------------------------------------------
// parse — deterministic Tier 1 parser
// ---------------------------------------------------------------------------

/// Parse user input through deterministic regex patterns.
/// Returns `Some(ParsedTask)` for instant tool execution,
/// or `None` to fall through to Tier 1.5 / Tier 3.
pub fn parse(input: &str) -> Option<ParsedTask> {
    let trimmed = input.trim();

    // --- screenshot / screencap ---
    if SCREENSHOT_RE.is_match(trimmed) || SCREENCAP_RE.is_match(trimmed) {
        return Some(ParsedTask::DirectTool {
            tool_name: "take_screenshot".into(),
            params: HashMap::new(),
            description: "Take a screenshot".into(),
        });
    }

    // --- go back / back ---
    if GO_BACK_RE.is_match(trimmed) {
        return Some(ParsedTask::DirectTool {
            tool_name: "system_key".into(),
            params: HashMap::from([("key".into(), Value::String("back".into()))]),
            description: "Press back button".into(),
        });
    }

    // --- go home / home ---
    if GO_HOME_RE.is_match(trimmed) {
        return Some(ParsedTask::DirectTool {
            tool_name: "system_key".into(),
            params: HashMap::from([("key".into(), Value::String("home".into()))]),
            description: "Press home button".into(),
        });
    }

    // --- open <app> ---
    if let Some(caps) = OPEN_APP_RE.captures(trimmed) {
        if let Some(app_name) = caps.get(1) {
            let app = app_name.as_str().trim().to_string();
            if !app.is_empty() {
                return Some(ParsedTask::DirectTool {
                    tool_name: "open_app".into(),
                    params: HashMap::from([("app_name".into(), Value::String(app))]),
                    description: "Open application".into(),
                });
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- screenshot patterns ---

    #[test]
    fn test_screenshot_simple() {
        let result = parse("screenshot");
        assert!(matches!(result, Some(ParsedTask::DirectTool { tool_name, .. }) if tool_name == "take_screenshot"));
    }

    #[test]
    fn test_take_screenshot() {
        let result = parse("take a screenshot");
        assert!(matches!(result, Some(ParsedTask::DirectTool { tool_name, .. }) if tool_name == "take_screenshot"));
    }

    #[test]
    fn test_screencap() {
        let result = parse("screencap");
        assert!(matches!(result, Some(ParsedTask::DirectTool { tool_name, .. }) if tool_name == "take_screenshot"));
    }

    #[test]
    fn test_screencapture() {
        let result = parse("screencapture");
        assert!(matches!(result, Some(ParsedTask::DirectTool { tool_name, .. }) if tool_name == "take_screenshot"));
    }

    #[test]
    fn test_screenshots_plural() {
        let result = parse("take screenshots");
        assert!(matches!(result, Some(ParsedTask::DirectTool { tool_name, .. }) if tool_name == "take_screenshot"));
    }

    #[test]
    fn test_screenshot_case_insensitive() {
        let result = parse("SCREENSHOT");
        assert!(matches!(result, Some(ParsedTask::DirectTool { tool_name, .. }) if tool_name == "take_screenshot"));
    }

    // --- back patterns ---

    #[test]
    fn test_go_back() {
        let result = parse("go back");
        assert!(matches!(result, Some(ParsedTask::DirectTool { ref tool_name, ref params, .. })
            if tool_name == "system_key" && params.get("key").unwrap().as_str() == Some("back")));
    }

    #[test]
    fn test_back_alone() {
        let result = parse("back");
        assert!(matches!(result, Some(ParsedTask::DirectTool { ref tool_name, .. }) if tool_name == "system_key"));
    }

    #[test]
    fn test_back_case_insensitive() {
        let result = parse("BACK");
        assert!(matches!(result, Some(ParsedTask::DirectTool { ref tool_name, .. }) if tool_name == "system_key"));
    }

    #[test]
    fn test_go_back_no_word_boundary_leak() {
        // "go backward" should NOT match
        let result = parse("go backward");
        assert!(result.is_none());
    }

    // --- home patterns ---

    #[test]
    fn test_go_home() {
        let result = parse("go home");
        assert!(matches!(result, Some(ParsedTask::DirectTool { ref tool_name, ref params, .. })
            if tool_name == "system_key" && params.get("key").unwrap().as_str() == Some("home")));
    }

    #[test]
    fn test_home_alone() {
        let result = parse("home");
        assert!(matches!(result, Some(ParsedTask::DirectTool { ref tool_name, .. }) if tool_name == "system_key"));
    }

    #[test]
    fn test_home_case_insensitive() {
        let result = parse("HOME");
        assert!(matches!(result, Some(ParsedTask::DirectTool { ref tool_name, .. }) if tool_name == "system_key"));
    }

    // --- open app patterns ---

    #[test]
    fn test_open_single_app() {
        let result = parse("open WhatsApp");
        assert!(matches!(result, Some(ParsedTask::DirectTool { ref tool_name, ref params, .. })
            if tool_name == "open_app" && params.get("app_name").unwrap().as_str() == Some("WhatsApp")));
    }

    #[test]
    fn test_open_multi_word_app() {
        let result = parse("open Google Chrome");
        assert!(matches!(result, Some(ParsedTask::DirectTool { ref tool_name, ref params, .. })
            if tool_name == "open_app" && params.get("app_name").unwrap().as_str() == Some("Google Chrome")));
    }

    #[test]
    fn test_open_case_insensitive() {
        let result = parse("OPEN telegram");
        assert!(matches!(result, Some(ParsedTask::DirectTool { ref tool_name, ref params, .. })
            if tool_name == "open_app" && params.get("app_name").unwrap().as_str() == Some("telegram")));
    }

    #[test]
    fn test_open_trims_whitespace() {
        let result = parse("open  WhatsApp  ");
        assert!(matches!(result, Some(ParsedTask::DirectTool { ref tool_name, ref params, .. })
            if tool_name == "open_app" && params.get("app_name").unwrap().as_str() == Some("WhatsApp")));
    }

    // --- non-matches ---

    #[test]
    fn test_non_match_complex_task() {
        let result = parse("send a message to Mom on WhatsApp");
        assert!(result.is_none());
    }

    #[test]
    fn test_non_match_empty() {
        let result = parse("");
        assert!(result.is_none());
    }

    #[test]
    fn test_non_match_whitespace() {
        let result = parse("   ");
        assert!(result.is_none());
    }

    #[test]
    fn test_non_match_open_without_app() {
        let result = parse("open");
        assert!(result.is_none());
    }

    #[test]
    fn test_non_match_back_in_sentence() {
        // "bring back the old menu" should NOT match go back
        let result = parse("bring back the old menu");
        assert!(result.is_none());
    }

    #[test]
    fn test_non_match_home_in_sentence() {
        // "check my homework" should NOT match home
        let result = parse("check my homework");
        assert!(result.is_none());
    }
}
