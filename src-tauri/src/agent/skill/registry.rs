// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

use super::{Skill, builtins};

// ---------------------------------------------------------------------------
// SkillRegistry — lookup skills by trigger pattern or ID
// ---------------------------------------------------------------------------

/// Registry of available skills for Tier 1.5 matching.
pub struct SkillRegistry {
    skills: Vec<Skill>,
}

impl SkillRegistry {
    /// Create a new registry with the given skills.
    pub fn new(skills: Vec<Skill>) -> Self {
        Self { skills }
    }

    /// Create a registry pre-loaded with all built-in skills.
    pub fn with_builtins() -> Self {
        Self::new(builtins::builtin_skills())
    }

    /// Find a skill whose trigger patterns match the input (case-insensitive contains).
    /// Returns the first matching skill.
    pub fn find_by_trigger(&self, input: &str) -> Option<&Skill> {
        let lower = input.to_lowercase();
        self.skills.iter().find(|skill| {
            skill.trigger_patterns.iter().any(|pattern| {
                lower.contains(&pattern.to_lowercase())
            })
        })
    }

    /// Find a skill by its exact ID.
    pub fn find_by_id(&self, id: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.id == id)
    }

    /// Return a slice of all registered skills.
    pub fn all_skills(&self) -> &[Skill] {
        &self.skills
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> SkillRegistry {
        SkillRegistry::with_builtins()
    }

    #[test]
    fn test_with_builtins_loads_9_skills() {
        let reg = registry();
        assert_eq!(reg.all_skills().len(), 9);
    }

    #[test]
    fn test_find_by_id_dismiss() {
        let reg = registry();
        let skill = reg.find_by_id("dismiss").unwrap();
        assert_eq!(skill.name, "Dismiss Dialog");
    }

    #[test]
    fn test_find_by_id_not_found() {
        let reg = registry();
        assert!(reg.find_by_id("nonexistent").is_none());
    }

    #[test]
    fn test_find_by_trigger_dismiss() {
        let reg = registry();
        let skill = reg.find_by_trigger("dismiss").unwrap();
        assert_eq!(skill.id, "dismiss");
    }

    #[test]
    fn test_find_by_trigger_close_dialog() {
        let reg = registry();
        let skill = reg.find_by_trigger("close dialog").unwrap();
        assert_eq!(skill.id, "dismiss");
    }

    #[test]
    fn test_find_by_trigger_case_insensitive() {
        let reg = registry();
        let skill = reg.find_by_trigger("CHECK NOTIFICATIONS").unwrap();
        assert_eq!(skill.id, "check_notifications");
    }

    #[test]
    fn test_find_by_trigger_embedded_text() {
        // "I want to check notifications now" contains "check notifications"
        let reg = registry();
        let skill = reg.find_by_trigger("I want to check notifications now").unwrap();
        assert_eq!(skill.id, "check_notifications");
    }

    #[test]
    fn test_find_by_trigger_no_match() {
        let reg = registry();
        assert!(reg.find_by_trigger("fly to the moon").is_none());
    }

    #[test]
    fn test_find_by_trigger_scroll_down() {
        let reg = registry();
        let skill = reg.find_by_trigger("scroll down").unwrap();
        assert_eq!(skill.id, "scroll_to_bottom");
    }

    #[test]
    fn test_find_by_trigger_volume_up() {
        let reg = registry();
        let skill = reg.find_by_trigger("volume up").unwrap();
        assert_eq!(skill.id, "volume_control");
    }

    #[test]
    fn test_find_by_trigger_louder() {
        let reg = registry();
        let skill = reg.find_by_trigger("make it louder").unwrap();
        assert_eq!(skill.id, "volume_control");
    }

    #[test]
    fn test_empty_registry() {
        let reg = SkillRegistry::new(vec![]);
        assert!(reg.find_by_trigger("anything").is_none());
        assert!(reg.find_by_id("dismiss").is_none());
        assert_eq!(reg.all_skills().len(), 0);
    }
}
