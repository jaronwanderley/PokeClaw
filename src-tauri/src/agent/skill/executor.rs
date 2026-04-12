// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

use crate::agent::tool_executor::{ToolExecutor, ToolResult};
use super::Skill;
use log::{info, warn};
use std::sync::atomic::{AtomicBool, Ordering};

// ---------------------------------------------------------------------------
// SkillResult — outcome of skill execution
// ---------------------------------------------------------------------------

/// Result of executing a skill's step sequence.
#[derive(Debug, Clone)]
pub enum SkillResult {
    /// All steps completed successfully.
    Completed { answer: String },
    /// A required step failed after retries.
    Failed { error: String, fallback_goal: String },
}

// ---------------------------------------------------------------------------
// SkillExecutor — runs skill steps sequentially
// ---------------------------------------------------------------------------

/// Executes a skill's steps against a tool executor.
pub struct SkillExecutor;

impl SkillExecutor {
    /// Execute all steps of a skill sequentially.
    ///
    /// - Calls `executor.execute()` for each step.
    /// - Checks the cancel flag between steps.
    /// - On optional step failure: skips and continues.
    /// - On required step failure: retries up to `max_retries`, then fails.
    /// - Returns `Completed` with a summary or `Failed` with a fallback goal.
    pub fn execute_skill(
        skill: &Skill,
        executor: &dyn ToolExecutor,
        cancel: &AtomicBool,
    ) -> SkillResult {
        info!("SkillExecutor: executing skill '{}' ({} steps)", skill.id, skill.steps.len());

        let mut step_results: Vec<String> = Vec::new();

        for (i, step) in skill.steps.iter().enumerate() {
            // Check cancellation between steps
            if cancel.load(Ordering::Relaxed) {
                warn!("SkillExecutor: skill '{}' cancelled at step {}", skill.id, i);
                return SkillResult::Failed {
                    error: format!("Cancelled at step {} of {}", i + 1, skill.steps.len()),
                    fallback_goal: skill.fallback_goal.clone(),
                };
            }

            info!(
                "SkillExecutor: step {}/{} — {} (tool: {})",
                i + 1, skill.steps.len(), step.description, step.tool_name
            );

            let params = serde_json::Value::Object(
                step.params.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
            );

            let mut attempt = 0;
            let max_attempts = step.max_retries + 1;
            let mut last_result: Option<ToolResult> = None;

            while attempt < max_attempts {
                if cancel.load(Ordering::Relaxed) {
                    warn!("SkillExecutor: skill '{}' cancelled during retry", skill.id);
                    return SkillResult::Failed {
                        error: format!("Cancelled during step {}", i + 1),
                        fallback_goal: skill.fallback_goal.clone(),
                    };
                }

                let result = executor.execute(&step.tool_name, params.clone());
                attempt += 1;

                if result.success {
                    let summary = result.data
                        .as_ref()
                        .and_then(|d| d.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("OK")
                        .to_string();
                    step_results.push(format!("Step {}: {}", step.description, summary));
                    info!("SkillExecutor: step {} succeeded on attempt {}", i + 1, attempt);
                    last_result = Some(result);
                    break;
                } else {
                    let error_msg = result.error.as_deref().unwrap_or("Unknown error");
                    warn!(
                        "SkillExecutor: step {} attempt {}/{} failed: {}",
                        i + 1, attempt, max_attempts, error_msg
                    );
                    last_result = Some(result);
                }
            }

            // After all retries, check if step succeeded
            let succeeded = last_result.as_ref().map_or(false, |r| r.success);
            if !succeeded {
                if step.optional {
                    warn!("SkillExecutor: optional step '{}' failed, skipping", step.description);
                    step_results.push(format!("Step {}: (skipped - optional step failed)", step.description));
                } else {
                    let error_msg = last_result
                        .and_then(|r| r.error)
                        .unwrap_or_else(|| "Unknown error".into());
                    warn!(
                        "SkillExecutor: required step '{}' failed after {} attempts",
                        step.description, attempt
                    );
                    return SkillResult::Failed {
                        error: format!(
                            "Step '{}' failed after {} attempts: {}",
                            step.description, attempt, error_msg
                        ),
                        fallback_goal: skill.fallback_goal.clone(),
                    };
                }
            }
        }

        let answer = if step_results.is_empty() {
            "Skill completed with no steps".into()
        } else {
            step_results.join("\n")
        };

        info!("SkillExecutor: skill '{}' completed successfully", skill.id);
        SkillResult::Completed { answer }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tool_executor::{ToolResult, ToolExecutor};
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;

    // --- Mock executor for testing ---

    struct MockExecutor {
        results: HashMap<String, ToolResult>,
    }

    impl MockExecutor {
        fn new() -> Self {
            Self { results: HashMap::new() }
        }

        fn with_tool_result(mut self, tool_name: &str, result: ToolResult) -> Self {
            self.results.insert(tool_name.into(), result);
            self
        }
    }

    impl ToolExecutor for MockExecutor {
        fn execute(&self, tool_name: &str, _params: Value) -> ToolResult {
            self.results.get(tool_name).cloned().unwrap_or(ToolResult {
                success: true,
                data: Some(json!({ "message": format!("Mock executed {}", tool_name) })),
                error: None,
            })
        }

        fn available_tools(&self) -> Vec<String> {
            self.results.keys().cloned().collect()
        }
    }

    fn make_skill(steps: Vec<super::super::SkillStep>) -> Skill {
        Skill {
            id: "test_skill".into(),
            name: "Test Skill".into(),
            description: "A test skill".into(),
            category: super::super::SkillCategory::Utility,
            estimated_steps_saved: 1,
            parameters: vec![],
            trigger_patterns: vec!["test".into()],
            steps,
            fallback_goal: "Fallback for test skill".into(),
        }
    }

    fn cancel_flag() -> AtomicBool {
        AtomicBool::new(false)
    }

    // --- Happy path ---

    #[test]
    fn test_single_step_completes() {
        let skill = make_skill(vec![
            super::super::SkillStep {
                tool_name: "get_screen_info".into(),
                params: HashMap::new(),
                description: "Get screen".into(),
                optional: false,
                max_retries: 1,
            },
        ]);
        let executor = MockExecutor::new();
        let cancel = cancel_flag();
        let result = SkillExecutor::execute_skill(&skill, &executor, &cancel);
        assert!(matches!(result, SkillResult::Completed { .. }));
    }

    #[test]
    fn test_multi_step_completes() {
        let skill = make_skill(vec![
            super::super::SkillStep {
                tool_name: "open_app".into(),
                params: HashMap::from([("app_name".into(), json!("WhatsApp"))]),
                description: "Open app".into(),
                optional: false,
                max_retries: 1,
            },
            super::super::SkillStep {
                tool_name: "wait".into(),
                params: HashMap::from([("milliseconds".into(), json!(1000))]),
                description: "Wait".into(),
                optional: true,
                max_retries: 0,
            },
            super::super::SkillStep {
                tool_name: "get_screen_info".into(),
                params: HashMap::new(),
                description: "Get screen".into(),
                optional: false,
                max_retries: 1,
            },
        ]);
        let executor = MockExecutor::new();
        let cancel = cancel_flag();
        let result = SkillExecutor::execute_skill(&skill, &executor, &cancel);
        assert!(matches!(result, SkillResult::Completed { .. }));
        if let SkillResult::Completed { answer } = result {
            assert!(answer.contains("Open app"));
            assert!(answer.contains("Get screen"));
        }
    }

    // --- Cancel ---

    #[test]
    fn test_cancel_before_step() {
        let cancel = AtomicBool::new(true);
        let skill = make_skill(vec![
            super::super::SkillStep {
                tool_name: "get_screen_info".into(),
                params: HashMap::new(),
                description: "Get screen".into(),
                optional: false,
                max_retries: 1,
            },
        ]);
        let executor = MockExecutor::new();
        let result = SkillExecutor::execute_skill(&skill, &executor, &cancel);
        assert!(matches!(result, SkillResult::Failed { .. }));
        if let SkillResult::Failed { error, .. } = result {
            assert!(error.contains("Cancelled"));
        }
    }

    // --- Optional step failure ---

    #[test]
    fn test_optional_step_failure_skips() {
        let skill = make_skill(vec![
            super::super::SkillStep {
                tool_name: "failing_tool".into(),
                params: HashMap::new(),
                description: "Optional step".into(),
                optional: true,
                max_retries: 0,
            },
            super::super::SkillStep {
                tool_name: "get_screen_info".into(),
                params: HashMap::new(),
                description: "Required step".into(),
                optional: false,
                max_retries: 1,
            },
        ]);
        let executor = MockExecutor::new()
            .with_tool_result("failing_tool", ToolResult {
                success: false,
                data: None,
                error: Some("tool failed".into()),
            });
        let cancel = cancel_flag();
        let result = SkillExecutor::execute_skill(&skill, &executor, &cancel);
        assert!(matches!(result, SkillResult::Completed { .. }));
        if let SkillResult::Completed { answer } = result {
            assert!(answer.contains("skipped"));
        }
    }

    // --- Required step failure with retry ---

    #[test]
    fn test_required_step_failure_returns_failed() {
        let skill = make_skill(vec![
            super::super::SkillStep {
                tool_name: "failing_tool".into(),
                params: HashMap::new(),
                description: "Required step".into(),
                optional: false,
                max_retries: 2,
            },
        ]);
        let executor = MockExecutor::new()
            .with_tool_result("failing_tool", ToolResult {
                success: false,
                data: None,
                error: Some("persistent failure".into()),
            });
        let cancel = cancel_flag();
        let result = SkillExecutor::execute_skill(&skill, &executor, &cancel);
        assert!(matches!(result, SkillResult::Failed { .. }));
        if let SkillResult::Failed { error, fallback_goal } = result {
            assert!(error.contains("Required step"));
            assert!(error.contains("persistent failure"));
            assert_eq!(fallback_goal, "Fallback for test skill");
        }
    }

    // --- Empty skill ---

    #[test]
    fn test_empty_skill_completes() {
        let skill = make_skill(vec![]);
        let executor = MockExecutor::new();
        let cancel = cancel_flag();
        let result = SkillExecutor::execute_skill(&skill, &executor, &cancel);
        assert!(matches!(result, SkillResult::Completed { .. }));
        if let SkillResult::Completed { answer } = result {
            assert_eq!(answer, "Skill completed with no steps");
        }
    }
}
