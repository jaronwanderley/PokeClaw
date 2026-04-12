// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

//! Configuration for the agent loop runner.

/// Default system prompt for local device tasks (ported from Kotlin LOCAL_TASK_PROMPT).
pub const LOCAL_TASK_PROMPT: &str = r#"You are PokeClaw, an AI agent that controls mobile devices to complete tasks for the user.

## Your capabilities
You can tap, swipe, type text, press system keys, take screenshots, open apps, and interact with any UI element on the device.

## Guidelines
1. Before acting, always check the current screen state using get_screen_info or find_node_info.
2. Break complex tasks into small steps. Complete one step at a time.
3. After each action, verify the result by checking the screen again.
4. If an action fails, try an alternative approach rather than repeating the same action.
5. Use the "finish" tool when the task is complete, providing a summary of what was accomplished.
6. If you cannot complete the task, use "finish" with success=false and explain why.

## Important rules
- Never assume what's on screen. Always verify before acting.
- Do not repeat the same action more than twice without trying something different.
- Keep track of your progress and adjust your strategy as needed.
- Be efficient — minimize unnecessary actions."#;

/// Configuration for the agent loop.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Model name for the LLM provider.
    pub model_name: String,

    /// Maximum number of loop iterations before forced termination.
    pub max_iterations: u32,

    /// System prompt to prepend to the conversation.
    pub system_prompt: String,

    /// Maximum token budget per task.
    pub max_tokens: u32,

    /// Maximum cost (USD) per task.
    pub max_cost_usd: f64,

    /// Soft limit percentage (0.0–1.0) at which a budget warning is injected.
    pub soft_limit_percent: f64,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model_name: "gpt-4o".to_string(),
            max_iterations: 20,
            system_prompt: LOCAL_TASK_PROMPT.to_string(),
            max_tokens: 250_000,
            max_cost_usd: 1.00,
            soft_limit_percent: 0.80,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let config = AgentConfig::default();
        assert_eq!(config.model_name, "gpt-4o");
        assert_eq!(config.max_iterations, 20);
        assert_eq!(config.max_tokens, 250_000);
        assert!((config.max_cost_usd - 1.0).abs() < f64::EPSILON);
        assert!((config.soft_limit_percent - 0.80).abs() < f64::EPSILON);
        assert!(!config.system_prompt.is_empty());
    }

    #[test]
    fn local_task_prompt_not_empty() {
        assert!(!LOCAL_TASK_PROMPT.is_empty());
        assert!(LOCAL_TASK_PROMPT.contains("PokeClaw"));
        assert!(LOCAL_TASK_PROMPT.contains("finish"));
    }

    #[test]
    fn custom_config() {
        let config = AgentConfig {
            model_name: "gpt-4o-mini".to_string(),
            max_iterations: 10,
            system_prompt: "Custom prompt".to_string(),
            max_tokens: 100_000,
            max_cost_usd: 0.50,
            soft_limit_percent: 0.90,
        };
        assert_eq!(config.model_name, "gpt-4o-mini");
        assert_eq!(config.max_iterations, 10);
        assert_eq!(config.max_tokens, 100_000);
    }
}
