// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

//! Execution guards for the agent loop.
//!
//! Guards are narrow-scope monitors that prevent the agent from claiming
//! success before it has actually performed the required UI actions.
//! They inject prompt sections, block premature `finish` calls, and track
//! progress as tools are executed.

pub mod email_compose;
pub mod in_app_search;

use log::info;

use self::email_compose::EmailComposeGuard;
use self::in_app_search::InAppSearchGuard;

// ---------------------------------------------------------------------------
// GuardRegistry
// ---------------------------------------------------------------------------

/// Holds all active guards for a given task. Only guards whose patterns
/// match the task text are activated; the rest are inert.
pub struct GuardRegistry {
    search_guard: Option<InAppSearchGuard>,
    email_guard: Option<EmailComposeGuard>,
}

impl GuardRegistry {
    /// Create a registry by parsing the task text and activating matching guards.
    pub fn from_task(task: &str) -> Self {
        let search_guard = InAppSearchGuard::from_task(task);
        let email_guard = EmailComposeGuard::from_task(task);

        let search_active = search_guard.is_active();
        let email_active = email_guard.is_active();

        if search_active || email_active {
            info!(
                "GuardRegistry: activated guards for task '{}' — search={}, email={}",
                task.split_whitespace().take(8).collect::<Vec<_>>().join(" "),
                search_active,
                email_active
            );
        }

        Self {
            search_guard: if search_active {
                Some(search_guard)
            } else {
                None
            },
            email_guard: if email_active {
                Some(email_guard)
            } else {
                None
            },
        }
    }

    /// Collect prompt sections from all active guards.
    pub fn build_prompt_sections(&self) -> String {
        let mut sections = String::new();
        if let Some(ref g) = self.search_guard {
            sections.push_str(&g.build_prompt_section());
        }
        if let Some(ref g) = self.email_guard {
            sections.push_str(&g.build_prompt_section());
        }
        sections
    }

    /// Whether any active guard wants to block a text-only completion.
    pub fn should_block_text_only_completion(&self) -> bool {
        self.search_guard
            .as_ref()
            .map_or(false, |g| g.should_block_text_only_completion())
            || self
                .email_guard
                .as_ref()
                .map_or(false, |g| g.should_block_text_only_completion())
    }

    /// Collect completion correction messages from all active guards.
    pub fn build_completion_correction(&self) -> String {
        let mut corrections: Vec<String> = Vec::new();
        if let Some(ref g) = self.search_guard {
            let c = g.build_completion_correction();
            if !c.is_empty() {
                corrections.push(c);
            }
        }
        if let Some(ref g) = self.email_guard {
            let c = g.build_completion_correction();
            if !c.is_empty() {
                corrections.push(c);
            }
        }
        corrections.join(" ")
    }

    /// Check whether any guard wants to block a `finish` call.
    /// Returns `Some(reason)` if the agent should not finish, `None` otherwise.
    pub fn maybe_block_finish(&self, screen_info: Option<&str>) -> Option<String> {
        if let Some(ref g) = self.search_guard {
            if let Some(block) = g.maybe_block_finish(screen_info) {
                return Some(block);
            }
        }
        if let Some(ref g) = self.email_guard {
            if let Some(block) = g.maybe_block_finish(screen_info) {
                return Some(block);
            }
        }
        None
    }

    /// Record that a tool was successfully executed, updating all active guards.
    pub fn record_successful_tool(&mut self, name: &str, params: &serde_json::Value) {
        if let Some(ref mut g) = self.search_guard {
            g.record_successful_tool(name, params);
        }
        if let Some(ref mut g) = self.email_guard {
            g.record_successful_tool(name);
        }
    }

    /// Record that a tool was attempted (even if not successful).
    /// Used by email compose guard to track compose flow progress.
    pub fn record_tool_attempt(&mut self, name: &str) {
        if let Some(ref mut g) = self.email_guard {
            g.record_tool_attempt(name);
        }
    }

    /// Whether any guard is active.
    pub fn has_active_guards(&self) -> bool {
        self.search_guard.is_some() || self.email_guard.is_some()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // === GuardRegistry combined tests ===

    #[test]
    fn test_registry_no_guards_for_non_matching_task() {
        let registry = GuardRegistry::from_task("open WhatsApp and send hi to Mom");
        assert!(!registry.has_active_guards());
        assert!(!registry.should_block_text_only_completion());
        assert_eq!(registry.build_prompt_sections(), "");
        assert!(registry.maybe_block_finish(None).is_none());
    }

    #[test]
    fn test_registry_search_guard_active() {
        let registry = GuardRegistry::from_task("search WhatsApp for hello");
        assert!(registry.has_active_guards());
        assert!(registry.should_block_text_only_completion());
        let prompts = registry.build_prompt_sections();
        assert!(prompts.contains("In-App Search"));
    }

    #[test]
    fn test_registry_email_guard_active() {
        let registry = GuardRegistry::from_task("write an email to boss");
        assert!(registry.has_active_guards());
        assert!(registry.should_block_text_only_completion());
        let prompts = registry.build_prompt_sections();
        assert!(prompts.contains("Compose Email"));
    }

    #[test]
    fn test_registry_correction_search() {
        let registry = GuardRegistry::from_task("search Gmail for urgent");
        let correction = registry.build_completion_correction();
        assert!(correction.contains("Gmail"));
        assert!(correction.contains("urgent"));
    }

    #[test]
    fn test_registry_correction_email() {
        let registry = GuardRegistry::from_task("compose an email to team");
        let correction = registry.build_completion_correction();
        assert!(correction.contains("email-compose"));
    }

    #[test]
    fn test_registry_record_tool_search() {
        let mut registry = GuardRegistry::from_task("search WhatsApp for test");
        assert!(registry.should_block_text_only_completion());
        registry.record_successful_tool("input_text", &json!({"text": "test"}));
        assert!(!registry.should_block_text_only_completion());
    }

    #[test]
    fn test_registry_record_tool_email() {
        let mut registry = GuardRegistry::from_task("write an email");
        assert!(registry.should_block_text_only_completion());
        registry.record_tool_attempt("open_app");
        assert!(!registry.should_block_text_only_completion());
    }

    #[test]
    fn test_registry_record_successful_tool_email() {
        let mut registry = GuardRegistry::from_task("compose an email");
        registry.record_successful_tool("input_text", &json!({}));
        assert!(!registry.should_block_text_only_completion());
    }

    #[test]
    fn test_registry_maybe_block_finish_search() {
        let registry = GuardRegistry::from_task("search YouTube for cats");
        let block = registry.maybe_block_finish(None);
        assert!(block.is_some());
        assert!(block.unwrap().contains("YouTube"));
    }

    #[test]
    fn test_registry_maybe_block_finish_email() {
        let registry = GuardRegistry::from_task("draft an email to John");
        let block = registry.maybe_block_finish(None);
        assert!(block.is_some());
        assert!(block.unwrap().contains("email-compose"));
    }

    #[test]
    fn test_registry_maybe_block_finish_after_completion() {
        let mut registry = GuardRegistry::from_task("search WhatsApp for test");
        registry.record_successful_tool("input_text", &json!({"text": "test"}));
        assert!(registry.maybe_block_finish(None).is_none());
    }

    #[test]
    fn test_registry_no_guards_record_is_noop() {
        let mut registry = GuardRegistry::from_task("open WhatsApp");
        registry.record_successful_tool("input_text", &json!({"text": "hi"}));
        registry.record_tool_attempt("tap");
        // Should not panic or change state
        assert!(!registry.has_active_guards());
    }
}
