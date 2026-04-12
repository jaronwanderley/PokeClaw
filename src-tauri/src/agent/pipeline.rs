// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

use crate::agent::skill::registry::SkillRegistry;
use crate::agent::task_parser;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Route — the pipeline routing decision
// ---------------------------------------------------------------------------

/// Routing decision from the 3-tier pipeline router.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Route {
    /// Tier 1: Deterministic tool execution — no LLM needed.
    DirectTool {
        tool_name: String,
        params: HashMap<String, Value>,
        description: String,
    },
    /// Tier 1.5: Skill-based sequential execution — no LLM needed.
    Skill {
        skill_id: String,
        description: String,
    },
    /// Tier 3: Full agent loop with LLM reasoning.
    AgentLoop {
        task: String,
    },
}

// ---------------------------------------------------------------------------
// PipelineRouter — 3-tier routing logic
// ---------------------------------------------------------------------------

/// Routes user input through the 3-tier pipeline:
/// 1. Compound check → AgentLoop
/// 2. Tier 1: deterministic parse → DirectTool
/// 3. Tier 1.5: skill trigger match → Skill
/// 4. Fallback → AgentLoop
pub struct PipelineRouter;

impl PipelineRouter {
    /// Route user input to the appropriate pipeline tier.
    pub fn route(input: &str, skill_registry: &SkillRegistry) -> Route {
        let trimmed = input.trim();

        // --- Compound task check ---
        // If the user asks for multiple things, always use the full agent loop.
        let lower = trimmed.to_lowercase();
        if lower.contains(" and ")
            || lower.contains(" then ")
            || lower.contains(" after ")
        {
            return Route::AgentLoop {
                task: trimmed.to_string(),
            };
        }

        // --- Tier 1: Deterministic parse ---
        if let Some(parsed) = task_parser::parse(trimmed) {
            match parsed {
                task_parser::ParsedTask::DirectTool {
                    tool_name,
                    params,
                    description,
                } => {
                    return Route::DirectTool {
                        tool_name,
                        params,
                        description,
                    };
                }
            }
        }

        // --- Tier 1.5: Skill trigger matching ---
        if let Some(skill) = skill_registry.find_by_trigger(trimmed) {
            return Route::Skill {
                skill_id: skill.id.clone(),
                description: skill.description.clone(),
            };
        }

        // --- Tier 3: Full agent loop ---
        Route::AgentLoop {
            task: trimmed.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::skill::registry::SkillRegistry;

    fn registry() -> SkillRegistry {
        SkillRegistry::with_builtins()
    }

    // --- Tier 1: DirectTool routes ---

    #[test]
    fn test_route_screenshot() {
        let reg = registry();
        let route = PipelineRouter::route("screenshot", &reg);
        assert_eq!(
            route,
            Route::DirectTool {
                tool_name: "take_screenshot".into(),
                params: HashMap::new(),
                description: "Take a screenshot".into(),
            }
        );
    }

    #[test]
    fn test_route_go_back() {
        let reg = registry();
        let route = PipelineRouter::route("back", &reg);
        assert!(matches!(route, Route::DirectTool { ref tool_name, .. } if tool_name == "system_key"));
    }

    #[test]
    fn test_route_go_home() {
        let reg = registry();
        let route = PipelineRouter::route("home", &reg);
        assert!(matches!(route, Route::DirectTool { ref tool_name, .. } if tool_name == "system_key"));
    }

    #[test]
    fn test_route_open_app() {
        let reg = registry();
        let route = PipelineRouter::route("open WhatsApp", &reg);
        assert!(matches!(route, Route::DirectTool { ref tool_name, ref params, .. }
            if tool_name == "open_app"
            && params.get("app_name").unwrap().as_str() == Some("WhatsApp")));
    }

    // --- Tier 1.5: Skill routes ---

    #[test]
    fn test_route_check_notifications() {
        let reg = registry();
        let route = PipelineRouter::route("check notifications", &reg);
        assert!(matches!(route, Route::Skill { ref skill_id, .. } if skill_id == "check_notifications"));
    }

    #[test]
    fn test_route_scroll_down() {
        let reg = registry();
        let route = PipelineRouter::route("scroll down", &reg);
        assert!(matches!(route, Route::Skill { ref skill_id, .. } if skill_id == "scroll_to_bottom"));
    }

    #[test]
    fn test_route_close_dialog() {
        let reg = registry();
        let route = PipelineRouter::route("close dialog", &reg);
        assert!(matches!(route, Route::Skill { ref skill_id, .. } if skill_id == "dismiss"));
    }

    #[test]
    fn test_route_volume_up() {
        let reg = registry();
        let route = PipelineRouter::route("volume up", &reg);
        assert!(matches!(route, Route::Skill { ref skill_id, .. } if skill_id == "volume_control"));
    }

    #[test]
    fn test_route_capture_screen_skill() {
        // "capture screen" is a Tier 1.5 trigger, not a Tier 1 regex match
        let reg = registry();
        let route = PipelineRouter::route("capture screen", &reg);
        assert!(matches!(route, Route::Skill { ref skill_id, .. } if skill_id == "take_screenshot"));
    }

    // --- Tier 3: AgentLoop routes ---

    #[test]
    fn test_route_complex_task() {
        let reg = registry();
        let route = PipelineRouter::route("send a message to Mom on WhatsApp", &reg);
        assert!(matches!(route, Route::AgentLoop { .. }));
    }

    #[test]
    fn test_route_unknown_task() {
        let reg = registry();
        let route = PipelineRouter::route("find the nearest pizza place", &reg);
        assert!(matches!(route, Route::AgentLoop { .. }));
    }

    // --- Compound detection ---

    #[test]
    fn test_compound_and() {
        let reg = registry();
        let route = PipelineRouter::route("open WhatsApp and send hi to Mom", &reg);
        assert!(matches!(route, Route::AgentLoop { .. }));
    }

    #[test]
    fn test_compound_then() {
        let reg = registry();
        let route = PipelineRouter::route("take a screenshot then open WhatsApp", &reg);
        assert!(matches!(route, Route::AgentLoop { .. }));
    }

    #[test]
    fn test_compound_after() {
        let reg = registry();
        let route = PipelineRouter::route("open WhatsApp after taking a screenshot", &reg);
        assert!(matches!(route, Route::AgentLoop { .. }));
    }

    #[test]
    fn test_compound_case_insensitive() {
        let reg = registry();
        let route = PipelineRouter::route("open WhatsApp AND send hi", &reg);
        assert!(matches!(route, Route::AgentLoop { .. }));
    }

    // --- Tier 1 takes priority over Tier 1.5 ---

    #[test]
    fn test_tier1_priority_over_skill() {
        // "screenshot" should route to DirectTool (Tier 1), not Skill (Tier 1.5)
        let reg = registry();
        let route = PipelineRouter::route("screenshot", &reg);
        assert!(matches!(route, Route::DirectTool { .. }));
    }

    // --- Boundary cases ---

    #[test]
    fn test_empty_input_routes_to_agent() {
        let reg = registry();
        let route = PipelineRouter::route("", &reg);
        assert!(matches!(route, Route::AgentLoop { .. }));
    }

    #[test]
    fn test_whitespace_input_routes_to_agent() {
        let reg = registry();
        let route = PipelineRouter::route("   ", &reg);
        assert!(matches!(route, Route::AgentLoop { .. }));
    }

    #[test]
    fn test_single_word_nonmatch() {
        let reg = registry();
        let route = PipelineRouter::route("elephant", &reg);
        assert!(matches!(route, Route::AgentLoop { ref task } if task == "elephant"));
    }
}
