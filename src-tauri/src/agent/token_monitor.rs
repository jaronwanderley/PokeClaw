// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

//! Tracks token usage and estimated cost during agent task execution.
//! Updated after each LLM call in the agent loop.

use crate::agent::model_pricing;

/// Token usage severity state based on cumulative totals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Normal,   // 0 – 30K tokens
    Caution,  // 30K – 100K
    Warning,  // 100K – 200K
    Critical, // 200K+
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            State::Normal => write!(f, "NORMAL"),
            State::Caution => write!(f, "CAUTION"),
            State::Warning => write!(f, "WARNING"),
            State::Critical => write!(f, "CRITICAL"),
        }
    }
}

/// Snapshot of current token usage and cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub step: u32,
    pub total_tokens: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub estimated_cost_usd: f64,
    pub state: State,
    pub formatted_tokens: String,
    pub formatted_cost: String,
}

/// Cumulative token tracker with cost estimation.
pub struct TokenMonitor {
    model_name: String,
    total_input_tokens: u32,
    total_output_tokens: u32,
    total_tokens: u32,
    current_step: u32,
}

impl TokenMonitor {
    pub fn new(model_name: &str) -> Self {
        Self {
            model_name: model_name.to_string(),
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_tokens: 0,
            current_step: 0,
        }
    }

    /// Record token usage from one LLM call.
    /// Call this after each agent loop iteration.
    pub fn record(
        &mut self,
        step: u32,
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
        total_token_count: Option<u32>,
    ) {
        self.current_step = step;
        if let Some(v) = input_tokens {
            self.total_input_tokens += v;
        }
        if let Some(v) = output_tokens {
            self.total_output_tokens += v;
        }
        if let Some(v) = total_token_count {
            self.total_tokens += v;
        } else {
            self.total_tokens = self.total_input_tokens + self.total_output_tokens;
        }

        let status = self.get_status();
        log::info!(
            "TokenMonitor: Step {}: {} tokens, {} [{}]",
            step,
            status.formatted_tokens,
            status.formatted_cost,
            status.state,
        );
    }

    /// Get current token status snapshot.
    pub fn get_status(&self) -> Status {
        let cost =
            model_pricing::estimate_cost(&self.model_name, self.total_input_tokens, self.total_output_tokens);
        let state = if self.total_tokens >= 200_000 {
            State::Critical
        } else if self.total_tokens >= 100_000 {
            State::Warning
        } else if self.total_tokens >= 30_000 {
            State::Caution
        } else {
            State::Normal
        };
        Status {
            step: self.current_step,
            total_tokens: self.total_tokens,
            input_tokens: self.total_input_tokens,
            output_tokens: self.total_output_tokens,
            estimated_cost_usd: cost,
            state,
            formatted_tokens: model_pricing::format_tokens(self.total_tokens),
            formatted_cost: model_pricing::format_cost(cost),
        }
    }

    /// Reset all counters.
    pub fn reset(&mut self) {
        self.total_input_tokens = 0;
        self.total_output_tokens = 0;
        self.total_tokens = 0;
        self.current_step = 0;
    }

    /// The model name this monitor is tracking.
    pub fn model_name(&self) -> &str {
        &self.model_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_monitor() -> TokenMonitor {
        TokenMonitor::new("gpt-4o")
    }

    #[test]
    fn initial_status() {
        let m = make_monitor();
        let s = m.get_status();
        assert_eq!(s.step, 0);
        assert_eq!(s.total_tokens, 0);
        assert_eq!(s.input_tokens, 0);
        assert_eq!(s.output_tokens, 0);
        assert_eq!(s.state, State::Normal);
        assert_eq!(s.formatted_tokens, "0");
    }

    #[test]
    fn record_with_all_fields() {
        let mut m = make_monitor();
        m.record(1, Some(1000), Some(500), Some(1500));
        let s = m.get_status();
        assert_eq!(s.step, 1);
        assert_eq!(s.input_tokens, 1000);
        assert_eq!(s.output_tokens, 500);
        assert_eq!(s.total_tokens, 1500);
    }

    #[test]
    fn record_without_total_recomputes() {
        let mut m = make_monitor();
        m.record(1, Some(800), Some(200), None);
        let s = m.get_status();
        assert_eq!(s.total_tokens, 1000); // 800 + 200
    }

    #[test]
    fn record_accumulates() {
        let mut m = make_monitor();
        m.record(1, Some(1000), Some(500), Some(1500));
        m.record(2, Some(2000), Some(1000), Some(3000));
        let s = m.get_status();
        assert_eq!(s.input_tokens, 3000);
        assert_eq!(s.output_tokens, 1500);
        assert_eq!(s.total_tokens, 4500);
        assert_eq!(s.step, 2);
    }

    #[test]
    fn state_transitions() {
        let mut m = make_monitor();

        // Normal: < 30K
        m.record(1, Some(10_000), Some(10_000), None);
        assert_eq!(m.get_status().state, State::Normal);

        // Caution: >= 30K
        m.reset();
        m.record(1, Some(20_000), Some(15_000), None);
        assert_eq!(m.get_status().state, State::Caution);

        // Warning: >= 100K
        m.reset();
        m.record(1, Some(60_000), Some(50_000), None);
        assert_eq!(m.get_status().state, State::Warning);

        // Critical: >= 200K
        m.reset();
        m.record(1, Some(150_000), Some(100_000), None);
        assert_eq!(m.get_status().state, State::Critical);
    }

    #[test]
    fn cost_estimation() {
        let mut m = make_monitor();
        // gpt-4o: $2.50/M in, $10.00/M out
        // 1000*2.5/1M + 500*10/1M = 0.0025 + 0.005 = 0.0075
        m.record(1, Some(1000), Some(500), Some(1500));
        let cost = m.get_status().estimated_cost_usd;
        assert!((cost - 0.0075).abs() < 1e-10);
    }

    #[test]
    fn cost_estimation_unknown_model() {
        let mut m = TokenMonitor::new("local-llama");
        m.record(1, Some(1000), Some(500), Some(1500));
        assert_eq!(m.get_status().estimated_cost_usd, 0.0);
    }

    #[test]
    fn reset_clears_everything() {
        let mut m = make_monitor();
        m.record(1, Some(1000), Some(500), Some(1500));
        m.reset();
        let s = m.get_status();
        assert_eq!(s.step, 0);
        assert_eq!(s.total_tokens, 0);
        assert_eq!(s.input_tokens, 0);
        assert_eq!(s.output_tokens, 0);
        assert_eq!(s.state, State::Normal);
    }

    #[test]
    fn formatted_output() {
        let mut m = make_monitor();
        m.record(1, Some(5000), Some(3000), Some(8000));
        let s = m.get_status();
        assert_eq!(s.formatted_tokens, "8.0K");
        // 5000*2.5/1M + 3000*10/1M = 0.0125 + 0.03 = 0.0425
        assert_eq!(s.formatted_cost, "$0.04");
    }

    #[test]
    fn partial_input_only() {
        let mut m = make_monitor();
        m.record(1, Some(1000), None, None);
        let s = m.get_status();
        assert_eq!(s.input_tokens, 1000);
        assert_eq!(s.output_tokens, 0);
        assert_eq!(s.total_tokens, 1000);
    }

    #[test]
    fn state_boundary_normal_to_caution() {
        let mut m = make_monitor();
        // Exactly 30_000 → Caution (>= 30K)
        m.record(1, Some(30_000), Some(0), Some(30_000));
        assert_eq!(m.get_status().state, State::Caution);
    }

    #[test]
    fn state_boundary_just_under_caution() {
        let mut m = make_monitor();
        // 29_999 → Normal
        m.record(1, Some(29_999), Some(0), Some(29_999));
        assert_eq!(m.get_status().state, State::Normal);
    }

    #[test]
    fn state_boundary_warning() {
        let mut m = make_monitor();
        m.record(1, Some(100_000), Some(0), Some(100_000));
        assert_eq!(m.get_status().state, State::Warning);
    }

    #[test]
    fn state_boundary_critical() {
        let mut m = make_monitor();
        m.record(1, Some(200_000), Some(0), Some(200_000));
        assert_eq!(m.get_status().state, State::Critical);
    }

    #[test]
    fn model_name_accessor() {
        let m = make_monitor();
        assert_eq!(m.model_name(), "gpt-4o");
    }
}
