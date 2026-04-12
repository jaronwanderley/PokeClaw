// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

//! Per-task token budget with soft and hard limits.

/// Budget status returned by [`TaskBudget::check`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Within normal operating range.
    Ok,
    /// ≥ 80% of budget consumed — inject warning prompt.
    SoftLimit,
    /// ≥ 100% of budget — force finish.
    HardLimit,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Status::Ok => write!(f, "OK"),
            Status::SoftLimit => write!(f, "SOFT_LIMIT"),
            Status::HardLimit => write!(f, "HARD_LIMIT"),
        }
    }
}

/// Per-task budget with configurable soft and hard limits on tokens and cost.
pub struct TaskBudget {
    pub max_tokens: u32,
    pub max_cost_usd: f64,
    soft_limit_percent: f64,
}

/// Default values matching the Kotlin implementation.
impl Default for TaskBudget {
    fn default() -> Self {
        Self {
            max_tokens: 250_000,
            max_cost_usd: 1.00,
            soft_limit_percent: 0.80,
        }
    }
}

impl TaskBudget {
    /// Create a budget with default limits (250K tokens, $1.00 cost, 80% soft limit).
    pub fn from_defaults() -> Self {
        Self::default()
    }

    /// Create a budget with custom limits.
    pub fn new(max_tokens: u32, max_cost_usd: f64, soft_limit_percent: f64) -> Self {
        Self {
            max_tokens,
            max_cost_usd,
            soft_limit_percent,
        }
    }

    /// Create an unlimited budget (no constraints).
    pub fn unlimited() -> Self {
        Self {
            max_tokens: u32::MAX,
            max_cost_usd: 0.0,
            soft_limit_percent: 0.80,
        }
    }

    /// Check current token/cost usage against the budget.
    ///
    /// Returns the most severe status: hard limit takes precedence over soft limit,
    /// cost is checked alongside tokens.
    pub fn check(&self, current_tokens: u32, current_cost_usd: f64) -> Status {
        let token_limit_enabled = self.max_tokens < u32::MAX;
        let cost_limit_enabled = self.max_cost_usd > 0.0;

        // Hard limit check (either tokens or cost)
        if token_limit_enabled && current_tokens >= self.max_tokens {
            log::warn!(
                "TaskBudget HARD LIMIT: tokens {} >= max {}",
                current_tokens,
                self.max_tokens
            );
            return Status::HardLimit;
        }
        if cost_limit_enabled && current_cost_usd >= self.max_cost_usd {
            log::warn!(
                "TaskBudget HARD LIMIT: cost ${:.4} >= max ${:.2}",
                current_cost_usd,
                self.max_cost_usd
            );
            return Status::HardLimit;
        }

        // Soft limit check
        if token_limit_enabled {
            let token_percent = current_tokens as f64 / self.max_tokens as f64;
            if token_percent >= self.soft_limit_percent {
                log::info!(
                    "TaskBudget SOFT LIMIT: tokens at {:.0}% of budget",
                    token_percent * 100.0
                );
                return Status::SoftLimit;
            }
        }
        if cost_limit_enabled {
            let cost_percent = current_cost_usd / self.max_cost_usd;
            if cost_percent >= self.soft_limit_percent {
                log::info!(
                    "TaskBudget SOFT LIMIT: cost at {:.0}% of budget",
                    cost_percent * 100.0
                );
                return Status::SoftLimit;
            }
        }

        Status::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_default() -> TaskBudget {
        TaskBudget::from_defaults()
    }

    // ── Default budget ───────────────────────────────────────────────

    #[test]
    fn default_values() {
        let b = TaskBudget::from_defaults();
        assert_eq!(b.max_tokens, 250_000);
        assert!((b.max_cost_usd - 1.0).abs() < f64::EPSILON);
        assert!((b.soft_limit_percent - 0.80).abs() < f64::EPSILON);
    }

    // ── Ok status ────────────────────────────────────────────────────

    #[test]
    fn check_ok_under_limits() {
        let b = make_default();
        assert_eq!(b.check(100_000, 0.50), Status::Ok);
    }

    #[test]
    fn check_ok_zero_usage() {
        let b = make_default();
        assert_eq!(b.check(0, 0.0), Status::Ok);
    }

    // ── Soft limit ───────────────────────────────────────────────────

    #[test]
    fn check_soft_limit_tokens() {
        let b = make_default();
        // 80% of 250K = 200K
        assert_eq!(b.check(200_000, 0.0), Status::SoftLimit);
    }

    #[test]
    fn check_soft_limit_just_under() {
        let b = make_default();
        // 199_999 is < 80% of 250_000
        assert_eq!(b.check(199_999, 0.0), Status::Ok);
    }

    #[test]
    fn check_soft_limit_cost() {
        let b = make_default();
        // 80% of $1.00 = $0.80
        assert_eq!(b.check(0, 0.80), Status::SoftLimit);
    }

    #[test]
    fn check_soft_limit_cost_just_under() {
        let b = make_default();
        assert_eq!(b.check(0, 0.79), Status::Ok);
    }

    // ── Hard limit ───────────────────────────────────────────────────

    #[test]
    fn check_hard_limit_tokens() {
        let b = make_default();
        assert_eq!(b.check(250_000, 0.0), Status::HardLimit);
    }

    #[test]
    fn check_hard_limit_exceeds_tokens() {
        let b = make_default();
        assert_eq!(b.check(300_000, 0.0), Status::HardLimit);
    }

    #[test]
    fn check_hard_limit_cost() {
        let b = make_default();
        assert_eq!(b.check(0, 1.00), Status::HardLimit);
    }

    #[test]
    fn check_hard_limit_exceeds_cost() {
        let b = make_default();
        assert_eq!(b.check(0, 2.00), Status::HardLimit);
    }

    // ── Hard limit takes precedence over soft ────────────────────────

    #[test]
    fn hard_limit_overrides_soft() {
        let b = make_default();
        // Both over: tokens at hard limit, cost at soft limit
        assert_eq!(b.check(250_000, 0.80), Status::HardLimit);
    }

    // ── Unlimited budget ─────────────────────────────────────────────

    #[test]
    fn unlimited_always_ok() {
        let b = TaskBudget::unlimited();
        assert_eq!(b.check(u32::MAX, 0.0), Status::Ok);
    }

    #[test]
    fn unlimited_with_zero_cost_max() {
        let b = TaskBudget::unlimited();
        // max_cost_usd = 0 means cost limit disabled
        assert_eq!(b.check(0, 999.0), Status::Ok);
    }

    // ── Custom budget ────────────────────────────────────────────────

    #[test]
    fn custom_budget() {
        let b = TaskBudget::new(100_000, 5.0, 0.90);
        // 90% of 100K = 90K
        assert_eq!(b.check(89_999, 0.0), Status::Ok);
        assert_eq!(b.check(90_000, 0.0), Status::SoftLimit);
        assert_eq!(b.check(100_000, 0.0), Status::HardLimit);
    }

    #[test]
    fn custom_budget_cost_soft() {
        let b = TaskBudget::new(100_000, 5.0, 0.50);
        // 50% of $5.00 = $2.50
        assert_eq!(b.check(0, 2.49), Status::Ok);
        assert_eq!(b.check(0, 2.50), Status::SoftLimit);
    }

    // ── Edge cases ───────────────────────────────────────────────────

    #[test]
    fn zero_max_tokens_no_limit() {
        let b = TaskBudget::new(0, 1.0, 0.8);
        // max_tokens = 0: current (even 0) >= 0 → hard limit immediately
        assert_eq!(b.check(0, 0.0), Status::HardLimit);
    }

    #[test]
    fn soft_limit_exact_boundary() {
        let b = make_default();
        // 80% of 250K = 200_000 exactly
        assert_eq!(b.check(200_000, 0.0), Status::SoftLimit);
    }
}
