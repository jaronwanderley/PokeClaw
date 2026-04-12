// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

//! Detects stuck agent loops using 5 signals and a sliding window.
//!
//! Recovery is 3-level:
//!   Level 1 (Hint): inject recovery hint into prompt
//!   Level 2 (StrategySwitch): suggest different tool
//!   Level 3 (AutoKill): force finish

use std::collections::VecDeque;

/// Which stuck signal was detected.
#[derive(Debug, Clone, PartialEq)]
pub enum Signal {
    /// Same action repeated 3+ times consecutively.
    SameAction {
        action: String,
        count: usize,
    },
    /// Screen unchanged for 3+ consecutive steps.
    ScreenUnchanged {
        steps: usize,
    },
    /// Zero screen text diff for 3+ consecutive steps.
    ZeroDiff {
        steps: usize,
    },
    /// An action appeared 3+ times within the full window.
    HighRepetition {
        action: String,
        count: usize,
        window: usize,
    },
    /// Same error repeated 3+ times consecutively.
    RepeatedError {
        error: String,
        count: usize,
    },
}

impl Signal {
    pub fn description(&self) -> String {
        match self {
            Signal::SameAction { action, count } => {
                format!("Same action '{}' repeated {} times consecutively", action, count)
            }
            Signal::ScreenUnchanged { steps } => {
                format!("Screen unchanged for {} consecutive steps", steps)
            }
            Signal::ZeroDiff { steps } => {
                format!("Zero screen text diff for {} consecutive steps", steps)
            }
            Signal::HighRepetition {
                action,
                count,
                window,
            } => {
                format!(
                    "Action '{}' appeared {} times in last {} steps",
                    action, count, window
                )
            }
            Signal::RepeatedError { error, count } => {
                format!("Same error repeated {} times consecutively: {}", count, error)
            }
        }
    }
}

/// Recovery escalation level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryLevel {
    /// Level 1: inject recovery hint.
    Hint,
    /// Level 2: suggest different approach.
    StrategySwitch,
    /// Level 3: force finish.
    AutoKill,
}

impl std::fmt::Display for RecoveryLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecoveryLevel::Hint => write!(f, "HINT"),
            RecoveryLevel::StrategySwitch => write!(f, "STRATEGY_SWITCH"),
            RecoveryLevel::AutoKill => write!(f, "AUTO_KILL"),
        }
    }
}

/// Result of a stuck detection check.
#[derive(Debug, Clone)]
pub struct Detection {
    pub signal: Signal,
    pub level: RecoveryLevel,
    pub recovery_hint: String,
}

/// Sliding-window stuck detector checking 5 signals.
pub struct StuckDetector {
    window_size: usize,
    actions: VecDeque<String>,
    screen_hashes: VecDeque<u64>,
    screen_diff_counts: VecDeque<u32>,
    errors: VecDeque<String>,
    consecutive_stuck_steps: u32,
}

impl StuckDetector {
    pub fn new(window_size: usize) -> Self {
        Self {
            window_size,
            actions: VecDeque::with_capacity(window_size + 1),
            screen_hashes: VecDeque::with_capacity(window_size + 1),
            screen_diff_counts: VecDeque::with_capacity(window_size + 1),
            errors: VecDeque::with_capacity(window_size + 1),
            consecutive_stuck_steps: 0,
        }
    }

    /// Default detector with window size 8.
    pub fn with_defaults() -> Self {
        Self::new(8)
    }

    /// Record one agent loop step and check for stuck patterns.
    ///
    /// Returns `Some(Detection)` if a stuck signal is found, `None` if OK.
    pub fn record(
        &mut self,
        action: &str,
        screen_hash: u64,
        screen_diff_count: u32,
        error: Option<&str>,
    ) -> Option<Detection> {
        // Add to sliding windows
        self.actions.push_back(action.to_string());
        if self.actions.len() > self.window_size {
            self.actions.pop_front();
        }

        self.screen_hashes.push_back(screen_hash);
        if self.screen_hashes.len() > self.window_size {
            self.screen_hashes.pop_front();
        }

        self.screen_diff_counts.push_back(screen_diff_count);
        if self.screen_diff_counts.len() > self.window_size {
            self.screen_diff_counts.pop_front();
        }

        if let Some(err) = error {
            self.errors.push_back(err.to_string());
            if self.errors.len() > self.window_size {
                self.errors.pop_front();
            }
        } else {
            // Non-error step breaks consecutive error chain
            self.errors.clear();
        }

        // Check all 5 signals
        let signal = self
            .check_same_action()
            .or_else(|| self.check_screen_unchanged())
            .or_else(|| self.check_zero_diff())
            .or_else(|| self.check_high_repetition())
            .or_else(|| self.check_repeated_error());

        if let Some(signal) = signal {
            self.consecutive_stuck_steps += 1;
            let level = if self.consecutive_stuck_steps >= 5 {
                RecoveryLevel::AutoKill
            } else if self.consecutive_stuck_steps >= 3 {
                RecoveryLevel::StrategySwitch
            } else {
                RecoveryLevel::Hint
            };
            let hint = generate_recovery_hint(&signal, level);
            log::warn!(
                "[StuckDetector] {} → Level {}",
                signal.description(),
                level
            );
            Some(Detection {
                signal,
                level,
                recovery_hint: hint,
            })
        } else {
            self.consecutive_stuck_steps = 0;
            None
        }
    }

    /// Reset all internal state.
    pub fn reset(&mut self) {
        self.actions.clear();
        self.screen_hashes.clear();
        self.screen_diff_counts.clear();
        self.errors.clear();
        self.consecutive_stuck_steps = 0;
    }

    fn check_same_action(&self) -> Option<Signal> {
        if self.actions.len() < 3 {
            return None;
        }
        let n = self.actions.len();
        let a = &self.actions[n - 3];
        let b = &self.actions[n - 2];
        let c = &self.actions[n - 1];
        if a == b && b == c {
            Some(Signal::SameAction {
                action: truncate_str(a, 50),
                count: 3,
            })
        } else {
            None
        }
    }

    fn check_screen_unchanged(&self) -> Option<Signal> {
        if self.screen_hashes.len() < 3 {
            return None;
        }
        let n = self.screen_hashes.len();
        let a = self.screen_hashes[n - 3];
        let b = self.screen_hashes[n - 2];
        let c = self.screen_hashes[n - 1];
        if a == b && b == c {
            Some(Signal::ScreenUnchanged { steps: 3 })
        } else {
            None
        }
    }

    fn check_zero_diff(&self) -> Option<Signal> {
        if self.screen_diff_counts.len() < 3 {
            return None;
        }
        let n = self.screen_diff_counts.len();
        let a = self.screen_diff_counts[n - 3];
        let b = self.screen_diff_counts[n - 2];
        let c = self.screen_diff_counts[n - 1];
        if a == 0 && b == 0 && c == 0 {
            Some(Signal::ZeroDiff { steps: 3 })
        } else {
            None
        }
    }

    fn check_high_repetition(&self) -> Option<Signal> {
        if self.actions.len() < self.window_size {
            return None;
        }
        let mut counts = std::collections::HashMap::new();
        for a in &self.actions {
            *counts.entry(a.as_str()).or_insert(0usize) += 1;
        }
        let max_entry = counts.into_iter().max_by_key(|(_, v)| *v)?;
        if max_entry.1 >= 3 {
            Some(Signal::HighRepetition {
                action: truncate_str(max_entry.0, 50),
                count: max_entry.1,
                window: self.window_size,
            })
        } else {
            None
        }
    }

    fn check_repeated_error(&self) -> Option<Signal> {
        if self.errors.len() < 3 {
            return None;
        }
        let n = self.errors.len();
        let a = &self.errors[n - 3];
        let b = &self.errors[n - 2];
        let c = &self.errors[n - 1];
        if a == b && b == c {
            Some(Signal::RepeatedError {
                error: truncate_str(a, 80),
                count: 3,
            })
        } else {
            None
        }
    }
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        s[..max_len].to_string()
    }
}

fn generate_recovery_hint(signal: &Signal, level: RecoveryLevel) -> String {
    let base = match signal {
        Signal::SameAction { action, .. } => {
            if action.contains("find_and_tap") {
                "Your find_and_tap action is not working. Try using tap_node with a specific node ID from get_screen_info, or use system_key(key=\"enter\") to submit.".to_string()
            } else if action.contains("scroll") {
                "You may have reached the end of scrollable content. Try a different approach or press back.".to_string()
            } else if action.contains("tap") {
                "Your tap action may not be hitting the right target. Call get_screen_info to refresh the screen state and try a different element.".to_string()
            } else {
                format!("Your last action '{}' is not producing results. Try a completely different approach.", action)
            }
        }
        Signal::ScreenUnchanged { steps } => {
            format!(
                "The screen has not changed for {} steps. Your actions may not be having any effect. Try pressing system_key(key=\"back\") or system_key(key=\"home\") and restart from a different angle.",
                steps
            )
        }
        Signal::ZeroDiff { steps } => {
            format!(
                "No new content has appeared on screen for {} steps. You may be stuck. Try navigating away and back, or use a different tool.",
                steps
            )
        }
        Signal::HighRepetition { action, .. } => {
            format!(
                "You are repeating '{}' too frequently. This approach is not working. Try something fundamentally different.",
                action
            )
        }
        Signal::RepeatedError { error, .. } => {
            format!(
                "The same error keeps occurring: '{}'. Do not retry the same approach. Try a different tool or strategy.",
                error
            )
        }
    };

    match level {
        RecoveryLevel::Hint => format!("[System Notice] {}", base),
        RecoveryLevel::StrategySwitch => {
            format!(
                "[System Warning] You have been stuck for multiple rounds. {} If you cannot make progress, call finish and explain what went wrong.",
                base
            )
        }
        RecoveryLevel::AutoKill => String::new(), // caller handles auto-kill
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_detector() -> StuckDetector {
        StuckDetector::new(8)
    }

    // ── No signal ────────────────────────────────────────────────────

    #[test]
    fn no_signal_normal_operation() {
        let mut d = make_detector();
        assert!(d.record("tap_button1", 100, 5, None).is_none());
        assert!(d.record("tap_button2", 200, 3, None).is_none());
        assert!(d.record("scroll_down", 300, 10, None).is_none());
    }

    #[test]
    fn no_signal_with_different_errors() {
        let mut d = make_detector();
        // Use different actions to avoid SameAction triggering
        assert!(d.record("action_a", 100, 5, Some("error A")).is_none());
        assert!(d.record("action_b", 200, 3, Some("error B")).is_none());
        // Third step with different action, different screen, nonzero diff → no signal
        assert!(d.record("action_c", 300, 10, Some("error C")).is_none());
    }

    // ── SameAction signal ────────────────────────────────────────────

    #[test]
    fn same_action_3_times() {
        let mut d = make_detector();
        let result = d.record("tap_home", 100, 5, None);
        assert!(result.is_none());
        let result = d.record("tap_home", 200, 3, None);
        assert!(result.is_none());
        let result = d.record("tap_home", 300, 10, None);
        assert!(result.is_some());
        let det = result.unwrap();
        assert!(matches!(det.signal, Signal::SameAction { ref action, count: 3 } if action == "tap_home"));
    }

    #[test]
    fn same_action_with_varying_screen() {
        let mut d = make_detector();
        // Different screens but same action
        assert!(d.record("scroll_down", 100, 5, None).is_none());
        assert!(d.record("scroll_down", 200, 10, None).is_none());
        let result = d.record("scroll_down", 300, 8, None);
        assert!(result.is_some());
    }

    // ── ScreenUnchanged signal ───────────────────────────────────────

    #[test]
    fn screen_unchanged_3_times() {
        let mut d = make_detector();
        assert!(d.record("action_a", 42, 5, None).is_none());
        assert!(d.record("action_b", 42, 3, None).is_none());
        let result = d.record("action_c", 42, 0, None);
        assert!(result.is_some());
        let det = result.unwrap();
        assert!(matches!(det.signal, Signal::ScreenUnchanged { steps: 3 }));
    }

    // ── ZeroDiff signal ──────────────────────────────────────────────

    #[test]
    fn zero_diff_3_times() {
        let mut d = make_detector();
        assert!(d.record("action_a", 100, 0, None).is_none());
        assert!(d.record("action_b", 200, 0, None).is_none());
        let result = d.record("action_c", 300, 0, None);
        assert!(result.is_some());
        let det = result.unwrap();
        assert!(matches!(det.signal, Signal::ZeroDiff { steps: 3 }));
    }

    // ── HighRepetition signal ────────────────────────────────────────

    #[test]
    fn high_repetition_fills_window() {
        let mut d = StuckDetector::new(5);
        // Fill window: after 5 entries, window is full.
        d.record("scroll_down", 100, 5, None);
        d.record("tap_btn", 200, 3, None);
        d.record("scroll_down", 300, 8, None);
        d.record("other", 400, 2, None);
        // 5th entry fills window: actions=[scroll_down, tap_btn, scroll_down, other, scroll_down]
        // scroll_down appears 3 times in window of 5 → HighRepetition fires
        let result = d.record("scroll_down", 500, 1, None);
        assert!(result.is_some());
        if let Some(det) = result {
            assert!(matches!(det.signal, Signal::HighRepetition { ref action, count: 3, window: 5 } if action == "scroll_down"));
        }
    }

    // ── RepeatedError signal ─────────────────────────────────────────

    #[test]
    fn repeated_error_3_times() {
        let mut d = make_detector();
        // Use different actions so SameAction doesn't fire first
        assert!(d.record("action_a", 100, 5, Some("node not found")).is_none());
        assert!(d.record("action_b", 200, 3, Some("node not found")).is_none());
        let result = d.record("action_c", 300, 10, Some("node not found"));
        assert!(result.is_some());
        let det = result.unwrap();
        assert!(matches!(det.signal, Signal::RepeatedError { ref error, count: 3 } if error == "node not found"));
    }

    #[test]
    fn error_chain_broken_by_success() {
        let mut d = make_detector();
        d.record("a", 100, 5, Some("err"));
        d.record("a", 200, 3, Some("err"));
        // Non-error clears the error deque
        d.record("a", 300, 10, None);
        // Need 3 consecutive errors again from scratch
        d.record("a", 400, 5, Some("err"));
        d.record("a", 500, 3, Some("err"));
        assert!(d.record("a", 600, 10, Some("err")).is_some());
    }

    // ── Recovery level escalation ────────────────────────────────────

    #[test]
    fn recovery_escalation_hint_to_auto_kill() {
        let mut d = make_detector();

        // Every record with same action, same hash, zero diff triggers a signal
        // once we have >= 3 entries. Each firing increments consecutive_stuck_steps.

        // Steps 1-3: first stuck detection (consecutive=1 → Hint)
        d.record("tap_x", 1, 0, None);
        d.record("tap_x", 1, 0, None);
        let r1 = d.record("tap_x", 1, 0, None);
        assert_eq!(r1.as_ref().map(|d| d.level), Some(RecoveryLevel::Hint));
        assert_eq!(d.consecutive_stuck_steps, 1);

        // Step 4: consecutive=2 → Hint
        d.record("tap_x", 1, 0, None);
        assert_eq!(d.consecutive_stuck_steps, 2);

        // Step 5: consecutive=3 → StrategySwitch
        let r3 = d.record("tap_x", 1, 0, None);
        assert_eq!(r3.as_ref().map(|d| d.level), Some(RecoveryLevel::StrategySwitch));
        assert_eq!(d.consecutive_stuck_steps, 3);

        // Step 6: consecutive=4 → StrategySwitch
        d.record("tap_x", 1, 0, None);
        assert_eq!(d.consecutive_stuck_steps, 4);

        // Step 7: consecutive=5 → AutoKill
        let r5 = d.record("tap_x", 1, 0, None);
        assert_eq!(r5.as_ref().map(|d| d.level), Some(RecoveryLevel::AutoKill));
        assert_eq!(d.consecutive_stuck_steps, 5);
    }

    #[test]
    fn non_stuck_step_resets_consecutive_counter() {
        let mut d = make_detector();
        // Trigger one stuck
        d.record("tap_x", 1, 0, None);
        d.record("tap_x", 1, 0, None);
        let _ = d.record("tap_x", 1, 0, None);
        assert_eq!(d.consecutive_stuck_steps, 1);

        // Non-stuck step resets
        d.record("different_action", 999, 50, None);
        assert_eq!(d.consecutive_stuck_steps, 0);
    }

    // ── Recovery hints ───────────────────────────────────────────────

    #[test]
    fn recovery_hint_format() {
        let det = Detection {
            signal: Signal::SameAction {
                action: "tap_btn".to_string(),
                count: 3,
            },
            level: RecoveryLevel::Hint,
            recovery_hint: String::new(),
        };
        let hint = generate_recovery_hint(&det.signal, det.level);
        assert!(hint.starts_with("[System Notice]"));
    }

    #[test]
    fn recovery_hint_strategy_switch() {
        let det = Detection {
            signal: Signal::ScreenUnchanged { steps: 3 },
            level: RecoveryLevel::StrategySwitch,
            recovery_hint: String::new(),
        };
        let hint = generate_recovery_hint(&det.signal, det.level);
        assert!(hint.starts_with("[System Warning]"));
        assert!(hint.contains("stuck for multiple rounds"));
    }

    #[test]
    fn recovery_hint_auto_kill_is_empty() {
        let det = Detection {
            signal: Signal::ZeroDiff { steps: 3 },
            level: RecoveryLevel::AutoKill,
            recovery_hint: String::new(),
        };
        let hint = generate_recovery_hint(&det.signal, det.level);
        assert!(hint.is_empty());
    }

    #[test]
    fn recovery_hint_scroll_action() {
        let hint = generate_recovery_hint(
            &Signal::SameAction {
                action: "scroll_down".to_string(),
                count: 3,
            },
            RecoveryLevel::Hint,
        );
        assert!(hint.contains("end of scrollable content"));
    }

    #[test]
    fn recovery_hint_find_and_tap() {
        let hint = generate_recovery_hint(
            &Signal::SameAction {
                action: "find_and_tap:cat".to_string(),
                count: 3,
            },
            RecoveryLevel::Hint,
        );
        assert!(hint.contains("tap_node"));
    }

    #[test]
    fn recovery_hint_repeated_error() {
        let hint = generate_recovery_hint(
            &Signal::RepeatedError {
                error: "node not found".to_string(),
                count: 3,
            },
            RecoveryLevel::Hint,
        );
        assert!(hint.contains("node not found"));
    }

    // ── Reset ────────────────────────────────────────────────────────

    #[test]
    fn reset_clears_state() {
        let mut d = make_detector();
        d.record("a", 1, 0, None);
        d.record("a", 1, 0, None);
        d.record("a", 1, 0, None);
        assert_eq!(d.consecutive_stuck_steps, 1);

        d.reset();
        assert_eq!(d.consecutive_stuck_steps, 0);
        assert!(d.actions.is_empty());
        assert!(d.screen_hashes.is_empty());
        assert!(d.screen_diff_counts.is_empty());
        assert!(d.errors.is_empty());
    }

    // ── Signal description ───────────────────────────────────────────

    #[test]
    fn signal_descriptions() {
        assert!(matches!(
            &Signal::SameAction { action: "x".into(), count: 3 }.description(),
            s if s.contains("Same action 'x' repeated 3 times")
        ));
        assert!(matches!(
            &Signal::ScreenUnchanged { steps: 5 }.description(),
            s if s.contains("Screen unchanged for 5")
        ));
        assert!(matches!(
            &Signal::ZeroDiff { steps: 3 }.description(),
            s if s.contains("Zero screen text diff")
        ));
    }

    // ── Window sliding ───────────────────────────────────────────────

    #[test]
    fn window_size_respected() {
        let mut d = StuckDetector::new(4);
        d.record("a", 1, 1, None);
        d.record("b", 2, 2, None);
        d.record("c", 3, 3, None);
        d.record("d", 4, 4, None);
        assert_eq!(d.actions.len(), 4);
        d.record("e", 5, 5, None);
        assert_eq!(d.actions.len(), 4); // oldest evicted
        assert_eq!(d.actions[0], "b");
    }

    #[test]
    fn high_repetition_needs_full_window() {
        let mut d = StuckDetector::new(5);
        d.record("x", 1, 1, None);
        // Only 1 entry, window not full → no HighRepetition
        d.record("x", 2, 2, None);
        d.record("x", 3, 3, None);
        d.record("x", 4, 4, None);
        // 4 entries < window_size 5 → no high repetition
        // But SameAction triggers (3 consecutive x)
        let result = d.record("x", 5, 5, None);
        // SameAction fires (3 consecutive x) OR HighRepetition would if window full
        // Window size is 5, we've added 5 entries now, so HighRepetition could fire
        assert!(result.is_some());
    }

    // ── Truncation ───────────────────────────────────────────────────

    #[test]
    fn long_action_truncated() {
        let long_action = "a".repeat(100);
        let mut d = make_detector();
        d.record(&long_action, 1, 0, None);
        d.record(&long_action, 1, 0, None);
        let result = d.record(&long_action, 1, 0, None);
        if let Some(det) = result {
            if let Signal::SameAction { action, .. } = &det.signal {
                assert_eq!(action.len(), 50);
            }
        }
    }
}
