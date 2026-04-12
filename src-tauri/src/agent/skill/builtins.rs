// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

use super::{Skill, SkillCategory, SkillParameter, SkillStep};
use serde_json::Value;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Built-in skill definitions
// ---------------------------------------------------------------------------

/// Returns the 9 built-in skills for Tier 1.5 matching.
pub fn builtin_skills() -> Vec<Skill> {
    vec![
        // 1. Dismiss — dismiss dialogs and popups
        Skill {
            id: "dismiss".into(),
            name: "Dismiss Dialog".into(),
            description: "Dismiss any active dialog, popup, or overlay".into(),
            category: SkillCategory::Navigation,
            estimated_steps_saved: 3,
            parameters: vec![],
            trigger_patterns: vec![
                "dismiss".into(),
                "close dialog".into(),
                "close popup".into(),
                "dismiss dialog".into(),
            ],
            steps: vec![
                SkillStep {
                    tool_name: "system_key".into(),
                    params: HashMap::from([("key".into(), Value::String("back".into()))]),
                    description: "Press back to dismiss dialog".into(),
                    optional: false,
                    max_retries: 1,
                },
            ],
            fallback_goal: "Dismiss the current dialog or popup on screen".into(),
        },
        // 2. Go Home — navigate to home screen
        Skill {
            id: "go_home".into(),
            name: "Go Home".into(),
            description: "Navigate to the device home screen".into(),
            category: SkillCategory::Navigation,
            estimated_steps_saved: 2,
            parameters: vec![],
            trigger_patterns: vec![
                "go to home".into(),
                "go to home screen".into(),
                "return home".into(),
            ],
            steps: vec![
                SkillStep {
                    tool_name: "system_key".into(),
                    params: HashMap::from([("key".into(), Value::String("home".into()))]),
                    description: "Press home button".into(),
                    optional: false,
                    max_retries: 1,
                },
            ],
            fallback_goal: "Navigate to the home screen".into(),
        },
        // 3. Take Screenshot — capture the screen
        Skill {
            id: "take_screenshot".into(),
            name: "Take Screenshot".into(),
            description: "Capture a screenshot of the current screen".into(),
            category: SkillCategory::Media,
            estimated_steps_saved: 4,
            parameters: vec![],
            trigger_patterns: vec![
                "capture screen".into(),
                "grab screenshot".into(),
                "snap screen".into(),
            ],
            steps: vec![
                SkillStep {
                    tool_name: "take_screenshot".into(),
                    params: HashMap::new(),
                    description: "Capture the screen".into(),
                    optional: false,
                    max_retries: 2,
                },
            ],
            fallback_goal: "Take a screenshot of the current screen".into(),
        },
        // 4. Check Notifications — review all notifications
        Skill {
            id: "check_notifications".into(),
            name: "Check Notifications".into(),
            description: "Check and list all pending notifications".into(),
            category: SkillCategory::Utility,
            estimated_steps_saved: 5,
            parameters: vec![],
            trigger_patterns: vec![
                "check notifications".into(),
                "show notifications".into(),
                "view notifications".into(),
                "read notifications".into(),
                "my notifications".into(),
            ],
            steps: vec![
                SkillStep {
                    tool_name: "get_notifications".into(),
                    params: HashMap::new(),
                    description: "Retrieve all notifications".into(),
                    optional: false,
                    max_retries: 1,
                },
            ],
            fallback_goal: "Check and summarize all pending notifications".into(),
        },
        // 5. Open App — open a specified application
        Skill {
            id: "open_app".into(),
            name: "Open App".into(),
            description: "Open a specific application by name".into(),
            category: SkillCategory::Navigation,
            estimated_steps_saved: 3,
            parameters: vec![
                SkillParameter {
                    name: "app_name".into(),
                    description: "Name of the application to open".into(),
                    required: true,
                    default_value: None,
                },
            ],
            trigger_patterns: vec![
                "launch app".into(),
                "start app".into(),
            ],
            steps: vec![
                SkillStep {
                    tool_name: "open_app".into(),
                    params: HashMap::from([("app_name".into(), Value::String("{{app_name}}".into()))]),
                    description: "Open the specified application".into(),
                    optional: false,
                    max_retries: 1,
                },
            ],
            fallback_goal: "Open the specified application".into(),
        },
        // 6. Send Quick Message — quick message to a contact
        Skill {
            id: "send_quick_message".into(),
            name: "Send Quick Message".into(),
            description: "Send a quick message to a contact via a messaging app".into(),
            category: SkillCategory::Communication,
            estimated_steps_saved: 6,
            parameters: vec![
                SkillParameter {
                    name: "contact".into(),
                    description: "Contact name to send message to".into(),
                    required: true,
                    default_value: None,
                },
                SkillParameter {
                    name: "message".into(),
                    description: "Message content to send".into(),
                    required: true,
                    default_value: None,
                },
                SkillParameter {
                    name: "app".into(),
                    description: "Messaging app to use".into(),
                    required: false,
                    default_value: Some(Value::String("whatsapp".into())),
                },
            ],
            trigger_patterns: vec![
                "quick message".into(),
                "send a quick".into(),
            ],
            steps: vec![
                SkillStep {
                    tool_name: "open_app".into(),
                    params: HashMap::from([("app_name".into(), Value::String("{{app}}".into()))]),
                    description: "Open the messaging app".into(),
                    optional: false,
                    max_retries: 1,
                },
                SkillStep {
                    tool_name: "wait".into(),
                    params: HashMap::from([("milliseconds".into(), Value::Number(2000.into()))]),
                    description: "Wait for app to load".into(),
                    optional: true,
                    max_retries: 0,
                },
                SkillStep {
                    tool_name: "send_message".into(),
                    params: HashMap::from([
                        ("contact".into(), Value::String("{{contact}}".into())),
                        ("message".into(), Value::String("{{message}}".into())),
                        ("app".into(), Value::String("{{app}}".into())),
                    ]),
                    description: "Send the message".into(),
                    optional: false,
                    max_retries: 2,
                },
            ],
            fallback_goal: "Send a message to a contact via a messaging app".into(),
        },
        // 7. Search and Tap — find and tap a UI element
        Skill {
            id: "search_and_tap".into(),
            name: "Search and Tap".into(),
            description: "Find a UI element by text and tap it".into(),
            category: SkillCategory::Utility,
            estimated_steps_saved: 4,
            parameters: vec![
                SkillParameter {
                    name: "text".into(),
                    description: "Text to search for".into(),
                    required: true,
                    default_value: None,
                },
            ],
            trigger_patterns: vec![
                "tap on".into(),
                "click on".into(),
                "find and tap".into(),
                "press the".into(),
            ],
            steps: vec![
                SkillStep {
                    tool_name: "find_and_tap".into(),
                    params: HashMap::from([("text".into(), Value::String("{{text}}".into()))]),
                    description: "Find the element and tap it".into(),
                    optional: false,
                    max_retries: 2,
                },
            ],
            fallback_goal: "Find and tap a specific UI element on screen".into(),
        },
        // 8. Scroll to Bottom — scroll down the current view
        Skill {
            id: "scroll_to_bottom".into(),
            name: "Scroll to Bottom".into(),
            description: "Scroll down to the bottom of the current view".into(),
            category: SkillCategory::Navigation,
            estimated_steps_saved: 3,
            parameters: vec![],
            trigger_patterns: vec![
                "scroll down".into(),
                "scroll to bottom".into(),
                "go to bottom".into(),
                "page down".into(),
            ],
            steps: vec![
                SkillStep {
                    tool_name: "swipe".into(),
                    params: HashMap::from([
                        ("start_x".into(), Value::Number(540.into())),
                        ("start_y".into(), Value::Number(1800.into())),
                        ("end_x".into(), Value::Number(540.into())),
                        ("end_y".into(), Value::Number(300.into())),
                    ]),
                    description: "Swipe down to scroll".into(),
                    optional: false,
                    max_retries: 1,
                },
            ],
            fallback_goal: "Scroll to the bottom of the current view".into(),
        },
        // 9. Volume Control — set volume level
        Skill {
            id: "volume_control".into(),
            name: "Volume Control".into(),
            description: "Adjust device volume level".into(),
            category: SkillCategory::Utility,
            estimated_steps_saved: 3,
            parameters: vec![
                SkillParameter {
                    name: "direction".into(),
                    description: "Volume direction: up or down".into(),
                    required: true,
                    default_value: None,
                },
            ],
            trigger_patterns: vec![
                "volume up".into(),
                "volume down".into(),
                "turn up volume".into(),
                "turn down volume".into(),
                "louder".into(),
                "quieter".into(),
            ],
            steps: vec![
                SkillStep {
                    tool_name: "system_key".into(),
                    params: HashMap::from([("key".into(), Value::String("{{direction}}".into()))]),
                    description: "Press volume key".into(),
                    optional: false,
                    max_retries: 1,
                },
            ],
            fallback_goal: "Adjust the device volume".into(),
        },
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_skills_count() {
        let skills = builtin_skills();
        assert_eq!(skills.len(), 9, "Should have exactly 9 built-in skills");
    }

    #[test]
    fn test_all_skills_have_ids() {
        let skills = builtin_skills();
        for skill in &skills {
            assert!(!skill.id.is_empty(), "Skill '{}' has empty id", skill.name);
        }
    }

    #[test]
    fn test_all_skills_have_unique_ids() {
        let skills = builtin_skills();
        let ids: Vec<&str> = skills.iter().map(|s| s.id.as_str()).collect();
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(ids.len(), unique.len(), "All skill IDs must be unique");
    }

    #[test]
    fn test_all_skills_have_trigger_patterns() {
        let skills = builtin_skills();
        for skill in &skills {
            assert!(!skill.trigger_patterns.is_empty(),
                "Skill '{}' has no trigger patterns", skill.id);
        }
    }

    #[test]
    fn test_all_skills_have_steps() {
        let skills = builtin_skills();
        for skill in &skills {
            assert!(!skill.steps.is_empty(),
                "Skill '{}' has no execution steps", skill.id);
        }
    }

    #[test]
    fn test_all_skills_have_fallback_goals() {
        let skills = builtin_skills();
        for skill in &skills {
            assert!(!skill.fallback_goal.is_empty(),
                "Skill '{}' has no fallback goal", skill.id);
        }
    }

    #[test]
    fn test_all_step_tool_names_valid() {
        let skills = builtin_skills();
        let valid_tools = [
            "system_key", "take_screenshot", "get_notifications", "open_app",
            "wait", "send_message", "find_and_tap", "swipe",
        ];
        for skill in &skills {
            for step in &skill.steps {
                assert!(valid_tools.contains(&step.tool_name.as_str()),
                    "Skill '{}' step '{}' has invalid tool_name '{}'",
                    skill.id, step.description, step.tool_name);
            }
        }
    }

    #[test]
    fn test_dismiss_skill_structure() {
        let skills = builtin_skills();
        let dismiss = skills.iter().find(|s| s.id == "dismiss").unwrap();
        assert_eq!(dismiss.category, SkillCategory::Navigation);
        assert_eq!(dismiss.steps.len(), 1);
        assert_eq!(dismiss.steps[0].tool_name, "system_key");
    }

    #[test]
    fn test_send_quick_message_has_multiple_params() {
        let skills = builtin_skills();
        let sqm = skills.iter().find(|s| s.id == "send_quick_message").unwrap();
        assert_eq!(sqm.parameters.len(), 3);
        assert_eq!(sqm.steps.len(), 3);
    }
}
