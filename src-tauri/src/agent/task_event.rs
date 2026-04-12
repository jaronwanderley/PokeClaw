// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

//! TaskEvent enum for streaming agent loop progress to the frontend.
//!
//! Serialized as a TypeScript-compatible discriminated union:
//! ```json
//! { "event": "loopStart", "data": { "round": 1 } }
//! ```

use serde::{Deserialize, Serialize};

/// Events emitted by the agent loop during task execution.
///
/// Each variant serializes with `event` tag and `data` content,
/// matching the TypeScript discriminated-union shape consumed by the Vue frontend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum TaskEvent {
    /// A new loop iteration is starting.
    LoopStart { round: u32 },

    /// The agent is about to invoke a tool.
    ToolAction { tool_name: String },

    /// A tool execution completed (or failed).
    ToolResult {
        tool_name: String,
        success: bool,
        detail: String,
    },

    /// The agent's reasoning / thinking text from the LLM response.
    Thinking { content: String },

    /// Token usage and cost update after an LLM call.
    TokenUpdate {
        step: u32,
        formatted_tokens: String,
        formatted_cost: String,
    },

    /// The task completed successfully.
    Completed { answer: String, model_name: String },

    /// The task failed with an error.
    Failed { error: String },

    /// The task was cancelled by the user.
    Cancelled,

    /// General progress update.
    Progress { step: u32, description: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Serialization tests — verify TypeScript discriminated-union shape ──

    #[test]
    fn serialize_loop_start() {
        let event = TaskEvent::LoopStart { round: 1 };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["event"], "loopStart");
        assert_eq!(json["data"]["round"], 1);
    }

    #[test]
    fn serialize_tool_action() {
        let event = TaskEvent::ToolAction {
            tool_name: "tap".to_string(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["event"], "toolAction");
        assert_eq!(json["data"]["toolName"], "tap");
    }

    #[test]
    fn serialize_tool_result() {
        let event = TaskEvent::ToolResult {
            tool_name: "tap".to_string(),
            success: true,
            detail: "Tapped at (100, 200)".to_string(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["event"], "toolResult");
        assert_eq!(json["data"]["toolName"], "tap");
        assert_eq!(json["data"]["success"], true);
        assert_eq!(json["data"]["detail"], "Tapped at (100, 200)");
    }

    #[test]
    fn serialize_thinking() {
        let event = TaskEvent::Thinking {
            content: "I should tap the button".to_string(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["event"], "thinking");
        assert_eq!(json["data"]["content"], "I should tap the button");
    }

    #[test]
    fn serialize_token_update() {
        let event = TaskEvent::TokenUpdate {
            step: 3,
            formatted_tokens: "5.0K".to_string(),
            formatted_cost: "$0.05".to_string(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["event"], "tokenUpdate");
        assert_eq!(json["data"]["step"], 3);
        assert_eq!(json["data"]["formattedTokens"], "5.0K");
        assert_eq!(json["data"]["formattedCost"], "$0.05");
    }

    #[test]
    fn serialize_completed() {
        let event = TaskEvent::Completed {
            answer: "Sent message to John".to_string(),
            model_name: "gpt-4o".to_string(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["event"], "completed");
        assert_eq!(json["data"]["answer"], "Sent message to John");
        assert_eq!(json["data"]["modelName"], "gpt-4o");
    }

    #[test]
    fn serialize_failed() {
        let event = TaskEvent::Failed {
            error: "API timeout".to_string(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["event"], "failed");
        assert_eq!(json["data"]["error"], "API timeout");
    }

    #[test]
    fn serialize_cancelled() {
        let event = TaskEvent::Cancelled;
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["event"], "cancelled");
        // Cancelled has no data content
        assert!(json.get("data").is_none() || json["data"].is_null());
    }

    #[test]
    fn serialize_progress() {
        let event = TaskEvent::Progress {
            step: 2,
            description: "Opening WhatsApp".to_string(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["event"], "progress");
        assert_eq!(json["data"]["step"], 2);
        assert_eq!(json["data"]["description"], "Opening WhatsApp");
    }

    // ── Clone and PartialEq ──────────────────────────────────────────

    #[test]
    fn clone_equality() {
        let a = TaskEvent::LoopStart { round: 1 };
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn different_variants_not_equal() {
        let a = TaskEvent::LoopStart { round: 1 };
        let b = TaskEvent::Cancelled;
        assert_ne!(a, b);
    }

    // ── Roundtrip serialization ──────────────────────────────────────

    #[test]
    fn roundtrip_all_variants() {
        let events = vec![
            TaskEvent::LoopStart { round: 5 },
            TaskEvent::ToolAction { tool_name: "tap".into() },
            TaskEvent::ToolResult {
                tool_name: "tap".into(),
                success: true,
                detail: "ok".into(),
            },
            TaskEvent::Thinking { content: "thinking...".into() },
            TaskEvent::TokenUpdate {
                step: 1,
                formatted_tokens: "1K".into(),
                formatted_cost: "$0.01".into(),
            },
            TaskEvent::Completed {
                answer: "done".into(),
                model_name: "gpt-4o".into(),
            },
            TaskEvent::Failed { error: "oops".into() },
            TaskEvent::Cancelled,
            TaskEvent::Progress {
                step: 3,
                description: "working".into(),
            },
        ];

        for original in &events {
            let json = serde_json::to_string(original).unwrap();
            let parsed: TaskEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(&parsed, original, "Roundtrip failed for {:?}", original);
        }
    }
}
