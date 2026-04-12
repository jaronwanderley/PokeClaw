// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

pub mod builtins;
pub mod executor;
pub mod registry;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Skill types
// ---------------------------------------------------------------------------

/// Category of a skill for organization and filtering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SkillCategory {
    Navigation,
    Communication,
    Utility,
    Media,
    Productivity,
}

/// A parameter definition for a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillParameter {
    pub name: String,
    pub description: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<Value>,
}

/// A single step within a skill's execution plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillStep {
    pub tool_name: String,
    pub params: HashMap<String, Value>,
    pub description: String,
    pub optional: bool,
    pub max_retries: u32,
}

/// A skill definition — a pre-built execution plan for common tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: SkillCategory,
    pub estimated_steps_saved: u32,
    pub parameters: Vec<SkillParameter>,
    pub trigger_patterns: Vec<String>,
    pub steps: Vec<SkillStep>,
    pub fallback_goal: String,
}
