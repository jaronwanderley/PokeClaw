// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

//! Narrow execution guard for explicit "search in app" tasks.
//!
//! Ported from `InAppSearchGuard.kt`. Only activates for clear patterns like:
//! - "search [app] for [query]"
//! - "search for [query] on [app]"
//!
//! The guard prevents the agent from claiming success before it has actually
//! typed the query into the target app's search field.

use once_cell::sync::Lazy;
use regex::Regex;

/// Static mapping of common app names to Android package names.
/// Ported from `OpenAppTool.resolveAppNameStatic`.
fn resolve_app_name(name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();
    Some(match lower.as_str() {
        "whatsapp" => "com.whatsapp",
        "telegram" => "org.telegram.messenger",
        "instagram" => "com.instagram.android",
        "youtube" => "com.google.android.youtube",
        "chrome" => "com.android.chrome",
        "camera" => "com.android.camera2",
        "settings" => "com.android.settings",
        "messages" => "com.google.android.apps.messaging",
        "gmail" => "com.google.android.gm",
        "maps" => "com.google.android.apps.maps",
        "phone" => "com.google.android.dialer",
        "contacts" => "com.google.android.contacts",
        "calendar" => "com.google.android.calendar",
        "clock" => "com.google.android.deskclock",
        "calculator" => "com.google.android.calculator",
        "files" => "com.google.android.documentsui",
        "photos" => "com.google.android.apps.photos",
        "spotify" => "com.spotify.music",
        "twitter" | "x" => "com.twitter.android",
        "facebook" => "com.facebook.katana",
        "tiktok" => "com.zhiliaoapp.musically",
        "snapchat" => "com.snapchat.android",
        "reddit" => "com.reddit.frontpage",
        "discord" => "com.discord",
        "slack" => "com.Slack",
        "wechat" => "com.tencent.mm",
        "line" => "jp.naver.line.android",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Compiled regex patterns
// ---------------------------------------------------------------------------

static SEARCH_APP_FOR_QUERY: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\s*search\s+(.+?)\s+for\s+(.+?)\s*$").unwrap()
});

static SEARCH_QUERY_ON_APP: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\s*search\s+for\s+(.+?)\s+(?:on|in)\s+(.+?)\s*$").unwrap()
});

/// Regex to extract node IDs like `[n12]` from screen info lines.
static NODE_ID_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\[(n\d+)]").unwrap()
});

// ---------------------------------------------------------------------------
// Match data
// ---------------------------------------------------------------------------

/// Parsed match from a task text that activates the guard.
#[derive(Debug, Clone)]
pub struct SearchMatch {
    pub app_name: String,
    pub query: String,
    pub resolved_package: String,
}

// ---------------------------------------------------------------------------
// InAppSearchGuard
// ---------------------------------------------------------------------------

/// Execution guard that ensures in-app search tasks actually open the target
/// app and type the search query before finishing.
pub struct InAppSearchGuard {
    match_data: Option<SearchMatch>,
    opened_target_app: bool,
    typed_query: bool,
}

impl InAppSearchGuard {
    /// Create a guard from task text. Returns a no-op guard if the task
    /// doesn't match an in-app search pattern.
    pub fn from_task(task: &str) -> Self {
        let match_data = Self::parse(task);
        Self {
            match_data,
            opened_target_app: false,
            typed_query: false,
        }
    }

    /// Whether this guard is active (i.e., the task matched a search pattern).
    pub fn is_active(&self) -> bool {
        self.match_data.is_some()
    }

    /// Build the prompt section to inject into the system prompt when active.
    pub fn build_prompt_section(&self) -> String {
        let task = match &self.match_data {
            Some(m) => m,
            None => return String::new(),
        };

        format!(
            "\n\n## Task Guard: In-App Search\n\
             This request means: open {app} and search for \"{query}\" inside that app.\n\
             Required execution steps before completion:\n\
             1. Open the target app with open_app(app_name=\"{app}\") if it is not already open.\n\
             2. Find the app's search field or search icon.\n\
             3. Call input_text(text=\"{query}\") to actually type the query.\n\
             4. Submit the search and inspect the visible results with get_screen_info.\n\
             5. Only then call finish(summary=\"what is visible in the results\").\n\
             Never claim the search succeeded from memory alone. If you cannot type the query, \
             explain the blocker instead of finishing.",
            app = task.app_name,
            query = task.query
        )
    }

    /// Whether to block a text-only completion (no tool usage) for this task.
    pub fn should_block_text_only_completion(&self) -> bool {
        self.match_data.is_some() && !self.typed_query
    }

    /// Build the correction message when the agent tries to finish prematurely.
    pub fn build_completion_correction(&self) -> String {
        match &self.match_data {
            Some(task) => format!(
                "[System Guard] This is an in-app search task for {}. \
                 Do not stop yet. You must use input_text(text=\"{}\") to type the search query, \
                 submit it, and inspect the visible results before completing.",
                task.app_name, task.query
            ),
            None => "[System Guard] Continue the task instead of stopping.".to_string(),
        }
    }

    /// Check whether to block a `finish` call. Returns `Some(reason)` if the
    /// agent should not finish yet, or `None` if finishing is allowed.
    pub fn maybe_block_finish(&self, screen_info: Option<&str>) -> Option<String> {
        let task = self.match_data.as_ref()?;
        if self.typed_query {
            return None;
        }

        let open_hint = if self.opened_target_app {
            String::new()
        } else {
            format!(" Open {} first if needed.", task.app_name)
        };

        let node_hint = Self::build_node_hint(screen_info, &task.query);

        Some(format!(
            "[System Guard] Do not call finish yet for this {} search task.{} \
             You still need to type the query with input_text(text=\"{}\"), \
             submit it, and then inspect the results screen.{}",
            task.app_name, open_hint, task.query, node_hint
        ))
    }

    /// Record that a tool was successfully executed, updating internal state.
    pub fn record_successful_tool(&mut self, tool_name: &str, params: &serde_json::Value) {
        let task = match &self.match_data {
            Some(m) => m,
            None => return,
        };

        match tool_name {
            "open_app" => {
                let candidate = params
                    .get("package_name")
                    .or_else(|| params.get("app_name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let resolved = if candidate.contains('.') {
                    candidate.to_string()
                } else {
                    resolve_app_name(candidate)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| candidate.to_string())
                };
                if resolved == task.resolved_package {
                    self.opened_target_app = true;
                }
            }
            "input_text" => {
                let text = match params.get("text").and_then(|v| v.as_str()) {
                    Some(t) => t,
                    None => return,
                };
                let normalized_text = Self::normalize(text);
                let normalized_query = Self::normalize(&task.query);
                if normalized_text == normalized_query
                    || normalized_text.contains(&normalized_query)
                {
                    self.typed_query = true;
                }
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn parse(task: &str) -> Option<SearchMatch> {
        let trimmed = task.trim();

        // Try "search {app} for {query}" first
        if let Some(caps) = SEARCH_APP_FOR_QUERY.captures(trimmed) {
            let app_name = Self::sanitize_app_name(&caps[1]);
            let query = Self::sanitize_query(&caps[2]);
            if !app_name.is_empty() && !query.is_empty() {
                if let Some(resolved) = resolve_app_name(&app_name) {
                    return Some(SearchMatch {
                        app_name,
                        query,
                        resolved_package: resolved.to_string(),
                    });
                }
            }
        }

        // Try "search for {query} on {app}"
        if let Some(caps) = SEARCH_QUERY_ON_APP.captures(trimmed) {
            let query = Self::sanitize_query(&caps[1]);
            let app_name = Self::sanitize_app_name(&caps[2]);
            if !app_name.is_empty() && !query.is_empty() {
                if let Some(resolved) = resolve_app_name(&app_name) {
                    return Some(SearchMatch {
                        app_name,
                        query,
                        resolved_package: resolved.to_string(),
                    });
                }
            }
        }

        None
    }

    fn sanitize_app_name(raw: &str) -> String {
        let s = raw.trim();
        let s = s.strip_prefix("the ").unwrap_or(s);
        let s = s.strip_prefix("The ").unwrap_or(s);
        let s = s.strip_suffix(" app").unwrap_or(s);
        let s = s.strip_suffix(" App").unwrap_or(s);
        s.trim().to_string()
    }

    fn sanitize_query(raw: &str) -> String {
        let s = raw.trim();
        let s = s.strip_prefix('"').unwrap_or(s);
        let s = s.strip_suffix('"').unwrap_or(s);
        let s = s.strip_prefix('\'').unwrap_or(s);
        let s = s.strip_suffix('\'').unwrap_or(s);
        s.trim().to_string()
    }

    fn normalize(value: &str) -> String {
        value
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string()
    }

    fn build_node_hint(screen_info: Option<&str>, query: &str) -> String {
        let info = match screen_info {
            Some(s) if !s.trim().is_empty() => s,
            _ => return String::new(),
        };

        // First: look for a search-specific edit node
        for line in info.lines() {
            let trimmed = line.trim();
            if trimmed.to_lowercase().contains("search") && trimmed.contains(" edit") {
                if let Some(node_id) = Self::extract_node_id(trimmed) {
                    return format!(
                        " Current screen shows a search field at node_id=\"{}\". \
                         Use input_text(text=\"{}\", node_id=\"{}\") next.",
                        node_id, query, node_id
                    );
                }
            }
        }

        // Fallback: any edit node
        for line in info.lines() {
            let trimmed = line.trim();
            if trimmed.contains(" edit") {
                if let Some(node_id) = Self::extract_node_id(trimmed) {
                    return format!(
                        " Current screen has an editable field at node_id=\"{}\". \
                         Use input_text(text=\"{}\", node_id=\"{}\") next.",
                        node_id, query, node_id
                    );
                }
            }
        }

        String::new()
    }

    fn extract_node_id(line: &str) -> Option<String> {
        NODE_ID_RE
            .captures(line)
            .and_then(|caps| caps.get(1))
            .map(|m| m.as_str().to_string())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // === Regex matching tests ===

    #[test]
    fn test_match_search_app_for_query() {
        let guard = InAppSearchGuard::from_task("search WhatsApp for hello world");
        assert!(guard.is_active());
        let m = guard.match_data.as_ref().unwrap();
        assert_eq!(m.app_name, "WhatsApp");
        assert_eq!(m.query, "hello world");
        assert_eq!(m.resolved_package, "com.whatsapp");
    }

    #[test]
    fn test_match_search_query_on_app() {
        let guard = InAppSearchGuard::from_task("search for cats on YouTube");
        assert!(guard.is_active());
        let m = guard.match_data.as_ref().unwrap();
        assert_eq!(m.app_name, "YouTube");
        assert_eq!(m.query, "cats");
        assert_eq!(m.resolved_package, "com.google.android.youtube");
    }

    #[test]
    fn test_match_search_query_in_app() {
        let guard = InAppSearchGuard::from_task("search for flights in Chrome");
        assert!(guard.is_active());
        let m = guard.match_data.as_ref().unwrap();
        assert_eq!(m.app_name, "Chrome");
        assert_eq!(m.query, "flights");
    }

    #[test]
    fn test_match_case_insensitive() {
        let guard = InAppSearchGuard::from_task("Search WhatsApp For test query");
        assert!(guard.is_active());
    }

    #[test]
    fn test_match_with_leading_trailing_whitespace() {
        let guard = InAppSearchGuard::from_task("  search WhatsApp for test  ");
        assert!(guard.is_active());
    }

    #[test]
    fn test_no_match_non_search_task() {
        let guard = InAppSearchGuard::from_task("send a message to Mom");
        assert!(!guard.is_active());
    }

    #[test]
    fn test_no_match_unknown_app() {
        let guard = InAppSearchGuard::from_task("search ObscureApp for stuff");
        assert!(!guard.is_active());
    }

    #[test]
    fn test_no_match_empty_query() {
        let guard = InAppSearchGuard::from_task("search WhatsApp for ");
        assert!(!guard.is_active());
    }

    // === Sanitize helpers ===

    #[test]
    fn test_sanitize_app_name_the_prefix() {
        let guard = InAppSearchGuard::from_task("search the WhatsApp for cats");
        assert!(guard.is_active());
        let m = guard.match_data.as_ref().unwrap();
        assert_eq!(m.app_name, "WhatsApp");
    }

    #[test]
    fn test_sanitize_app_name_app_suffix() {
        let guard = InAppSearchGuard::from_task("search WhatsApp app for dogs");
        assert!(guard.is_active());
        let m = guard.match_data.as_ref().unwrap();
        assert_eq!(m.app_name, "WhatsApp");
    }

    #[test]
    fn test_sanitize_query_quoted() {
        let guard = InAppSearchGuard::from_task("search Gmail for \"important emails\"");
        assert!(guard.is_active());
        let m = guard.match_data.as_ref().unwrap();
        assert_eq!(m.query, "important emails");
    }

    // === Prompt generation ===

    #[test]
    fn test_prompt_section_active() {
        let guard = InAppSearchGuard::from_task("search Gmail for urgent");
        let prompt = guard.build_prompt_section();
        assert!(prompt.contains("In-App Search"));
        assert!(prompt.contains("Gmail"));
        assert!(prompt.contains("urgent"));
        assert!(prompt.contains("input_text"));
    }

    #[test]
    fn test_prompt_section_inactive() {
        let guard = InAppSearchGuard::from_task("open WhatsApp");
        assert_eq!(guard.build_prompt_section(), "");
    }

    // === Completion blocking ===

    #[test]
    fn test_should_block_text_only_initially() {
        let guard = InAppSearchGuard::from_task("search WhatsApp for hello");
        assert!(guard.should_block_text_only_completion());
    }

    #[test]
    fn test_should_not_block_after_typing() {
        let mut guard = InAppSearchGuard::from_task("search WhatsApp for hello");
        guard.record_successful_tool("input_text", &json!({"text": "hello"}));
        assert!(!guard.should_block_text_only_completion());
    }

    #[test]
    fn test_should_not_block_when_inactive() {
        let guard = InAppSearchGuard::from_task("open WhatsApp");
        assert!(!guard.should_block_text_only_completion());
    }

    // === Completion correction ===

    #[test]
    fn test_correction_message() {
        let guard = InAppSearchGuard::from_task("search Gmail for test");
        let correction = guard.build_completion_correction();
        assert!(correction.contains("Gmail"));
        assert!(correction.contains("test"));
        assert!(correction.contains("input_text"));
    }

    // === maybe_block_finish ===

    #[test]
    fn test_block_finish_initially() {
        let guard = InAppSearchGuard::from_task("search WhatsApp for test");
        let block = guard.maybe_block_finish(None);
        assert!(block.is_some());
        assert!(block.unwrap().contains("WhatsApp"));
    }

    #[test]
    fn test_block_finish_with_open_hint() {
        let guard = InAppSearchGuard::from_task("search WhatsApp for test");
        let block = guard.maybe_block_finish(None);
        let msg = block.unwrap();
        assert!(msg.contains("Open WhatsApp first"));
    }

    #[test]
    fn test_no_block_after_typing() {
        let mut guard = InAppSearchGuard::from_task("search WhatsApp for test");
        guard.record_successful_tool("input_text", &json!({"text": "test"}));
        assert!(guard.maybe_block_finish(None).is_none());
    }

    #[test]
    fn test_no_block_when_inactive() {
        let guard = InAppSearchGuard::from_task("open WhatsApp");
        assert!(guard.maybe_block_finish(None).is_none());
    }

    #[test]
    fn test_block_finish_with_screen_info_search_node() {
        let guard = InAppSearchGuard::from_task("search Gmail for urgent");
        let screen = "some text [n5] search edit field";
        let block = guard.maybe_block_finish(Some(screen));
        let msg = block.unwrap();
        assert!(msg.contains("node_id=\"n5\""));
    }

    #[test]
    fn test_block_finish_with_screen_info_any_edit_node() {
        let guard = InAppSearchGuard::from_task("search Gmail for urgent");
        let screen = "some text [n8] compose edit field";
        let block = guard.maybe_block_finish(Some(screen));
        let msg = block.unwrap();
        assert!(msg.contains("node_id=\"n8\""));
    }

    #[test]
    fn test_block_finish_no_edit_node() {
        let guard = InAppSearchGuard::from_task("search Gmail for urgent");
        let screen = "some text without any edit node";
        let block = guard.maybe_block_finish(Some(screen));
        let msg = block.unwrap();
        assert!(!msg.contains("node_id"));
    }

    // === record_successful_tool ===

    #[test]
    fn test_record_open_app_by_app_name() {
        let mut guard = InAppSearchGuard::from_task("search WhatsApp for test");
        guard.record_successful_tool("open_app", &json!({"app_name": "WhatsApp"}));
        // Now maybe_block_finish should not contain "Open WhatsApp first"
        let block = guard.maybe_block_finish(None);
        let msg = block.unwrap();
        assert!(!msg.contains("Open WhatsApp first"));
    }

    #[test]
    fn test_record_open_app_by_package_name() {
        let mut guard = InAppSearchGuard::from_task("search WhatsApp for test");
        guard.record_successful_tool(
            "open_app",
            &json!({"package_name": "com.whatsapp"}),
        );
        let block = guard.maybe_block_finish(None);
        let msg = block.unwrap();
        assert!(!msg.contains("Open WhatsApp first"));
    }

    #[test]
    fn test_record_open_app_wrong_package() {
        let mut guard = InAppSearchGuard::from_task("search WhatsApp for test");
        guard.record_successful_tool("open_app", &json!({"app_name": "Gmail"}));
        let block = guard.maybe_block_finish(None);
        let msg = block.unwrap();
        assert!(msg.contains("Open WhatsApp first"));
    }

    #[test]
    fn test_record_input_text_exact_match() {
        let mut guard = InAppSearchGuard::from_task("search WhatsApp for hello");
        guard.record_successful_tool("input_text", &json!({"text": "hello"}));
        assert!(!guard.should_block_text_only_completion());
    }

    #[test]
    fn test_record_input_text_case_insensitive() {
        let mut guard = InAppSearchGuard::from_task("search WhatsApp for Hello World");
        guard.record_successful_tool("input_text", &json!({"text": "hello world"}));
        assert!(!guard.should_block_text_only_completion());
    }

    #[test]
    fn test_record_input_text_partial_match() {
        let mut guard = InAppSearchGuard::from_task("search WhatsApp for hello world");
        // The text typed contains the query
        guard.record_successful_tool("input_text", &json!({"text": "hello world today"}));
        assert!(!guard.should_block_text_only_completion());
    }

    #[test]
    fn test_record_input_text_no_match() {
        let mut guard = InAppSearchGuard::from_task("search WhatsApp for hello");
        guard.record_successful_tool("input_text", &json!({"text": "goodbye"}));
        assert!(guard.should_block_text_only_completion());
    }

    #[test]
    fn test_record_ignores_other_tools() {
        let mut guard = InAppSearchGuard::from_task("search WhatsApp for test");
        guard.record_successful_tool("tap", &json!({"x": 100, "y": 200}));
        assert!(guard.should_block_text_only_completion());
    }

    #[test]
    fn test_full_flow_open_then_type() {
        let mut guard = InAppSearchGuard::from_task("search Gmail for important emails");
        assert!(guard.should_block_text_only_completion());

        guard.record_successful_tool("open_app", &json!({"app_name": "Gmail"}));
        assert!(guard.should_block_text_only_completion());

        guard.record_successful_tool("input_text", &json!({"text": "important emails"}));
        assert!(!guard.should_block_text_only_completion());
        assert!(guard.maybe_block_finish(None).is_none());
    }

    // === Node extraction ===

    #[test]
    fn test_extract_node_id_basic() {
        let id = InAppSearchGuard::extract_node_id("some text [n5] more text");
        assert_eq!(id, Some("n5".to_string()));
    }

    #[test]
    fn test_extract_node_id_none() {
        let id = InAppSearchGuard::extract_node_id("no node id here");
        assert_eq!(id, None);
    }
}
