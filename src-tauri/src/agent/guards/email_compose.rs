// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

//! Narrow execution guard for explicit email-compose tasks.
//!
//! Ported from `EmailComposeGuard.kt`. Only activates for requests that clearly
//! mean "open an email composer and draft something", not for generic chat
//! requests about writing. The goal is to stop the agent from claiming success
//! with a text-only draft before it has attempted any UI work inside an email app.

use once_cell::sync::Lazy;
use regex::Regex;

// ---------------------------------------------------------------------------
// Compiled regex patterns
// ---------------------------------------------------------------------------

static EXPLICIT_EMAIL_TASK: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\s*(?:write|compose|draft|send)\s+(?:an?\s+)?email\b.*$").unwrap()
});

/// Regex to extract node IDs like `[n12]` from screen info lines.
static NODE_ID_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\[(n\d+)]").unwrap()
});

/// Tools that indicate the agent is making progress in a compose flow.
const UI_COMPOSE_TOOLS: &[&str] = &[
    "open_app",
    "tap",
    "tap_node",
    "find_and_tap",
    "input_text",
    "type_text",
    "scroll_to_find",
    "system_key",
];

// ---------------------------------------------------------------------------
// Match data
// ---------------------------------------------------------------------------

/// Parsed match from task text that activates the email compose guard.
#[derive(Debug, Clone)]
pub struct EmailMatch {
    pub task_text: String,
}

// ---------------------------------------------------------------------------
// EmailComposeGuard
// ---------------------------------------------------------------------------

/// Execution guard that ensures email-compose tasks actually open an email
/// app and attempt a compose flow before finishing.
pub struct EmailComposeGuard {
    match_data: Option<EmailMatch>,
    attempted_compose_flow: bool,
}

impl EmailComposeGuard {
    /// Create a guard from task text. Returns a no-op guard if the task
    /// doesn't match an email-compose pattern.
    pub fn from_task(task: &str) -> Self {
        let trimmed = task.trim();
        let match_data = if EXPLICIT_EMAIL_TASK.is_match(trimmed) {
            Some(EmailMatch {
                task_text: trimmed.to_string(),
            })
        } else {
            None
        };
        Self {
            match_data,
            attempted_compose_flow: false,
        }
    }

    /// Whether this guard is active (i.e., the task matched an email pattern).
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
            "\n\n## Task Guard: Compose Email Draft\n\
             This task means: create an email draft in an email app, not just reply with draft text in chat.\n\
             Required execution steps before completion:\n\
             1. Open an email app (for example Gmail) and start a compose/new-draft flow.\n\
             2. Fill the visible fields one at a time with input_text.\n\
             3. If the task does not name a recipient, leave the recipient field blank but still fill a subject and body.\n\
             4. Inspect the compose screen to confirm the draft is visible.\n\
             5. Only then call finish(summary=\"draft ready to review\").\n\
             Never press Send unless the user explicitly asked you to send the email.\n\
             Never satisfy this task by returning draft text alone without opening an email composer.\n\
             Current email-draft task: \"{}\"",
            task.task_text
        )
    }

    /// Whether to block a text-only completion (no tool usage) for this task.
    pub fn should_block_text_only_completion(&self) -> bool {
        self.match_data.is_some() && !self.attempted_compose_flow
    }

    /// Build the correction message when the agent tries to finish prematurely.
    pub fn build_completion_correction(&self) -> String {
        "[System Guard] This is an email-compose task. \
         Do not stop with draft text only. Open an email app, start a compose flow, \
         fill the visible draft fields, inspect the draft screen, and only then finish."
            .to_string()
    }

    /// Check whether to block a `finish` call. Returns `Some(reason)` if the
    /// agent should not finish yet, or `None` if finishing is allowed.
    pub fn maybe_block_finish(&self, screen_info: Option<&str>) -> Option<String> {
        if self.match_data.is_none() || self.attempted_compose_flow {
            return None;
        }

        let node_hint = Self::build_node_hint(screen_info);

        Some(format!(
            "[System Guard] Do not call finish yet for this email-compose task. \
             You have not attempted any in-app compose action yet. \
             Open an email app, enter compose mode, fill the draft fields, then finish.{}",
            node_hint
        ))
    }

    /// Record that a tool was used (even if not successful), to track
    /// that the agent has attempted some compose-flow UI action.
    pub fn record_tool_attempt(&mut self, tool_name: &str) {
        if self.match_data.is_none() {
            return;
        }
        if UI_COMPOSE_TOOLS.contains(&tool_name) {
            self.attempted_compose_flow = true;
        }
    }

    /// Record that a tool was successfully executed.
    pub fn record_successful_tool(&mut self, tool_name: &str) {
        if self.match_data.is_none() {
            return;
        }
        if tool_name == "input_text" || tool_name == "type_text" {
            self.attempted_compose_flow = true;
        }
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn build_node_hint(screen_info: Option<&str>) -> String {
        let info = match screen_info {
            Some(s) if !s.trim().is_empty() => s,
            _ => return String::new(),
        };

        // Look for compose/subject/to nodes with a node ID
        for line in info.lines() {
            let trimmed = line.trim();
            let lower = trimmed.to_lowercase();
            if (lower.contains("compose")
                || lower.contains("subject")
                || lower.contains("to"))
                && trimmed.contains("[n")
            {
                if let Some(node_id) = Self::extract_node_id(trimmed) {
                    return format!(
                        " Current screen suggests a relevant compose element at node_id=\"{}\".",
                        node_id
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
                        " Current screen has an editable field at node_id=\"{}\".",
                        node_id
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

    // === Regex matching tests ===

    #[test]
    fn test_match_write_email() {
        let guard = EmailComposeGuard::from_task("write an email to boss about meeting");
        assert!(guard.is_active());
        assert_eq!(
            guard.match_data.as_ref().unwrap().task_text,
            "write an email to boss about meeting"
        );
    }

    #[test]
    fn test_match_compose_email() {
        let guard = EmailComposeGuard::from_task("compose email");
        assert!(guard.is_active());
    }

    #[test]
    fn test_match_draft_email() {
        let guard = EmailComposeGuard::from_task("draft an email to the team");
        assert!(guard.is_active());
    }

    #[test]
    fn test_match_send_email() {
        let guard = EmailComposeGuard::from_task("send a email to John");
        assert!(guard.is_active());
    }

    #[test]
    fn test_match_case_insensitive() {
        let guard = EmailComposeGuard::from_task("Write An Email to Mom");
        assert!(guard.is_active());
    }

    #[test]
    fn test_match_with_whitespace() {
        let guard = EmailComposeGuard::from_task("  write an email to someone  ");
        assert!(guard.is_active());
    }

    #[test]
    fn test_no_match_non_email_task() {
        let guard = EmailComposeGuard::from_task("open WhatsApp");
        assert!(!guard.is_active());
    }

    #[test]
    fn test_no_match_write_message() {
        let guard = EmailComposeGuard::from_task("write a message to Mom");
        assert!(!guard.is_active());
    }

    #[test]
    fn test_no_match_search_task() {
        let guard = EmailComposeGuard::from_task("search WhatsApp for cats");
        assert!(!guard.is_active());
    }

    #[test]
    fn test_no_match_email_but_wrong_prefix() {
        let guard = EmailComposeGuard::from_task("check email inbox");
        assert!(!guard.is_active());
    }

    // === Prompt generation ===

    #[test]
    fn test_prompt_section_active() {
        let guard = EmailComposeGuard::from_task("write an email to boss");
        let prompt = guard.build_prompt_section();
        assert!(prompt.contains("Compose Email Draft"));
        assert!(prompt.contains("write an email to boss"));
        assert!(prompt.contains("input_text"));
    }

    #[test]
    fn test_prompt_section_inactive() {
        let guard = EmailComposeGuard::from_task("open WhatsApp");
        assert_eq!(guard.build_prompt_section(), "");
    }

    // === Completion blocking ===

    #[test]
    fn test_should_block_initially() {
        let guard = EmailComposeGuard::from_task("write an email");
        assert!(guard.should_block_text_only_completion());
    }

    #[test]
    fn test_should_not_block_after_attempt() {
        let mut guard = EmailComposeGuard::from_task("write an email");
        guard.record_tool_attempt("open_app");
        assert!(!guard.should_block_text_only_completion());
    }

    #[test]
    fn test_should_not_block_when_inactive() {
        let guard = EmailComposeGuard::from_task("open WhatsApp");
        assert!(!guard.should_block_text_only_completion());
    }

    // === Completion correction ===

    #[test]
    fn test_correction_message() {
        let guard = EmailComposeGuard::from_task("write an email");
        let correction = guard.build_completion_correction();
        assert!(correction.contains("email-compose"));
        assert!(correction.contains("email app"));
        assert!(correction.contains("compose flow"));
    }

    // === maybe_block_finish ===

    #[test]
    fn test_block_finish_initially() {
        let guard = EmailComposeGuard::from_task("write an email");
        let block = guard.maybe_block_finish(None);
        assert!(block.is_some());
        assert!(block.unwrap().contains("email-compose"));
    }

    #[test]
    fn test_no_block_after_compose_attempt() {
        let mut guard = EmailComposeGuard::from_task("write an email");
        guard.record_tool_attempt("open_app");
        assert!(guard.maybe_block_finish(None).is_none());
    }

    #[test]
    fn test_no_block_when_inactive() {
        let guard = EmailComposeGuard::from_task("open WhatsApp");
        assert!(guard.maybe_block_finish(None).is_none());
    }

    #[test]
    fn test_block_finish_with_compose_node() {
        let guard = EmailComposeGuard::from_task("write an email");
        let screen = "some text [n3] Compose button";
        let block = guard.maybe_block_finish(Some(screen));
        let msg = block.unwrap();
        assert!(msg.contains("node_id=\"n3\""));
    }

    #[test]
    fn test_block_finish_with_subject_node() {
        let guard = EmailComposeGuard::from_task("write an email");
        let screen = "some text [n7] Subject edit field";
        let block = guard.maybe_block_finish(Some(screen));
        let msg = block.unwrap();
        assert!(msg.contains("node_id=\"n7\""));
    }

    #[test]
    fn test_block_finish_with_to_node() {
        let guard = EmailComposeGuard::from_task("write an email");
        let screen = "some text [n2] To field";
        let block = guard.maybe_block_finish(Some(screen));
        let msg = block.unwrap();
        assert!(msg.contains("node_id=\"n2\""));
    }

    #[test]
    fn test_block_finish_with_any_edit_node() {
        let guard = EmailComposeGuard::from_task("write an email");
        let screen = "some text [n10] random edit field";
        let block = guard.maybe_block_finish(Some(screen));
        let msg = block.unwrap();
        assert!(msg.contains("node_id=\"n10\""));
    }

    #[test]
    fn test_block_finish_no_relevant_node() {
        let guard = EmailComposeGuard::from_task("write an email");
        let screen = "some text without any relevant nodes";
        let block = guard.maybe_block_finish(Some(screen));
        let msg = block.unwrap();
        assert!(!msg.contains("node_id"));
    }

    // === record_tool_attempt ===

    #[test]
    fn test_record_tool_attempt_compose_tool() {
        let mut guard = EmailComposeGuard::from_task("write an email");
        guard.record_tool_attempt("tap");
        assert!(guard.attempted_compose_flow);
    }

    #[test]
    fn test_record_tool_attempt_non_compose_tool() {
        let mut guard = EmailComposeGuard::from_task("write an email");
        guard.record_tool_attempt("get_screen_info");
        assert!(!guard.attempted_compose_flow);
    }

    // === record_successful_tool ===

    #[test]
    fn test_record_successful_tool_input_text() {
        let mut guard = EmailComposeGuard::from_task("write an email");
        guard.record_successful_tool("input_text");
        assert!(guard.attempted_compose_flow);
    }

    #[test]
    fn test_record_successful_tool_type_text() {
        let mut guard = EmailComposeGuard::from_task("write an email");
        guard.record_successful_tool("type_text");
        assert!(guard.attempted_compose_flow);
    }

    #[test]
    fn test_record_successful_tool_other() {
        let mut guard = EmailComposeGuard::from_task("write an email");
        guard.record_successful_tool("tap");
        assert!(!guard.attempted_compose_flow);
    }

    // === Full flow test ===

    #[test]
    fn test_full_flow_open_type_finish() {
        let mut guard = EmailComposeGuard::from_task("compose an email to John about meeting");
        assert!(guard.should_block_text_only_completion());

        guard.record_tool_attempt("open_app");
        assert!(!guard.should_block_text_only_completion());

        guard.record_successful_tool("input_text");
        assert!(guard.maybe_block_finish(None).is_none());
    }

    // === Node extraction ===

    #[test]
    fn test_extract_node_id_basic() {
        let id = EmailComposeGuard::extract_node_id("some text [n5] more text");
        assert_eq!(id, Some("n5".to_string()));
    }

    #[test]
    fn test_extract_node_id_none() {
        let id = EmailComposeGuard::extract_node_id("no node id here");
        assert_eq!(id, None);
    }
}
