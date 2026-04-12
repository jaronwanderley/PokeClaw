// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

//! Model pricing table and cost estimation.
//!
//! Prices are in USD per 1 million tokens.
//! Source: official provider pricing pages as of 2026-04.

use std::collections::HashMap;
use once_cell::sync::Lazy;

/// Per-model pricing: cost in USD per 1M tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub input_per_million: f64,
    pub output_per_million: f64,
}

static PRICES: Lazy<HashMap<&'static str, Price>> = Lazy::new(|| {
    let mut m = HashMap::new();
    // OpenAI
    m.insert("gpt-4o", Price { input_per_million: 2.50, output_per_million: 10.00 });
    m.insert("gpt-4o-mini", Price { input_per_million: 0.15, output_per_million: 0.60 });
    m.insert("gpt-4.1", Price { input_per_million: 2.00, output_per_million: 8.00 });
    m.insert("gpt-4.1-mini", Price { input_per_million: 0.40, output_per_million: 1.60 });
    m.insert("gpt-4.1-nano", Price { input_per_million: 0.10, output_per_million: 0.40 });
    m.insert("gpt-4-turbo", Price { input_per_million: 10.00, output_per_million: 30.00 });
    m.insert("gpt-3.5-turbo", Price { input_per_million: 0.50, output_per_million: 1.50 });
    m.insert("o4-mini", Price { input_per_million: 1.10, output_per_million: 4.40 });

    // Anthropic
    m.insert("claude-opus-4-6", Price { input_per_million: 15.00, output_per_million: 75.00 });
    m.insert("claude-sonnet-4-6", Price { input_per_million: 3.00, output_per_million: 15.00 });
    m.insert("claude-haiku-4-5", Price { input_per_million: 0.80, output_per_million: 4.00 });

    // Google
    m.insert("gemini-2.5-flash", Price { input_per_million: 0.15, output_per_million: 0.60 });
    m.insert("gemini-2.5-pro", Price { input_per_million: 1.25, output_per_million: 10.00 });
    m.insert("gemini-2.0-flash", Price { input_per_million: 0.10, output_per_million: 0.40 });

    // Open-source via OpenRouter/Groq
    m.insert("llama-3.3-70b-versatile", Price { input_per_million: 0.59, output_per_million: 0.79 });
    m.insert("llama-4-maverick", Price { input_per_million: 0.50, output_per_million: 0.70 });
    m.insert("deepseek-chat", Price { input_per_million: 0.27, output_per_million: 1.10 });
    m.insert("deepseek-reasoner", Price { input_per_million: 0.55, output_per_million: 2.19 });
    m.insert("qwen-2.5-72b", Price { input_per_million: 0.29, output_per_million: 0.39 });

    m
});

/// Estimate cost in USD for a given model and token counts.
/// Returns 0.0 if model is not found (e.g. local models).
pub fn estimate_cost(model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
    match find_price(model) {
        Some(price) => {
            (input_tokens as f64 * price.input_per_million / 1_000_000.0)
                + (output_tokens as f64 * price.output_per_million / 1_000_000.0)
        }
        None => 0.0,
    }
}

/// Get the price entry for a model, with fuzzy matching for dated variants.
///
/// Matching order:
/// 1. Direct match
/// 2. Strip OpenRouter prefix (`openai/gpt-4o` → `gpt-4o`)
/// 3. Strip date suffixes (`gpt-4o-2025-03-01` → `gpt-4o-2025-03` → … → `gpt-4o`)
pub fn find_price(model: &str) -> Option<Price> {
    if model.is_empty() {
        return None;
    }

    // Direct match
    if let Some(&price) = PRICES.get(model) {
        return Some(price);
    }

    // Strip common prefixes (OpenRouter format: "openai/gpt-4o")
    let stripped = if let Some(pos) = model.rfind('/') {
        &model[pos + 1..]
    } else {
        model
    };

    if let Some(&price) = PRICES.get(stripped) {
        return Some(price);
    }

    // Strip date suffixes: "gpt-4o-2025-03-01" → "gpt-4o-2025-03" → "gpt-4o-2025" → "gpt-4o"
    let mut candidate = stripped.to_string();
    let re = regex_lazy();
    loop {
        if let Some(mat) = re.find(&candidate) {
            if mat.start() > 0 {
                candidate.truncate(mat.start());
                if let Some(&price) = PRICES.get(candidate.as_str()) {
                    return Some(price);
                }
                continue;
            }
        }
        break;
    }

    None
}

/// Format cost as a human-readable string.
///
/// - `< $0.001` → `"< $0.001"`
/// - `< $0.01`  → `"$0.003"` (3 decimals)
/// - `>= $0.01` → `"$0.02"`  (2 decimals)
pub fn format_cost(cost_usd: f64) -> String {
    if cost_usd < 0.001 {
        "< $0.001".to_string()
    } else if cost_usd < 0.01 {
        format!("${:.3}", cost_usd)
    } else {
        format!("${:.2}", cost_usd)
    }
}

/// Format token count as human-readable.
///
/// - `< 1K`   → `"500"`
/// - `< 1M`   → `"8.2K"`
/// - `>= 1M`  → `"1.2M"`
pub fn format_tokens(tokens: u32) -> String {
    if tokens < 1000 {
        tokens.to_string()
    } else if tokens < 1_000_000 {
        format!("{:.1}K", tokens as f64 / 1000.0)
    } else {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    }
}

/// Estimate how many agent steps a budget allows for a given model.
/// Assumes ~5000 tokens per step (4000 input + 1000 output).
pub fn estimate_steps(model: &str, budget_usd: f64) -> u32 {
    let price = match find_price(model) {
        Some(p) => p,
        None => return 0,
    };
    let cost_per_step = (4000.0 * price.input_per_million / 1_000_000.0)
        + (1000.0 * price.output_per_million / 1_000_000.0);
    if cost_per_step > 0.0 {
        (budget_usd / cost_per_step) as u32
    } else {
        0
    }
}

use std::sync::OnceLock;
static DATE_RE: OnceLock<regex::Regex> = OnceLock::new();

fn regex_lazy() -> &'static regex::Regex {
    DATE_RE.get_or_init(|| regex::Regex::new(r"-\d{2,4}$").unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Price table ──────────────────────────────────────────────────

    #[test]
    fn prices_table_has_all_models() {
        assert!(PRICES.contains_key("gpt-4o"));
        assert!(PRICES.contains_key("gpt-4o-mini"));
        assert!(PRICES.contains_key("gpt-4.1"));
        assert!(PRICES.contains_key("gpt-4.1-mini"));
        assert!(PRICES.contains_key("gpt-4.1-nano"));
        assert!(PRICES.contains_key("gpt-4-turbo"));
        assert!(PRICES.contains_key("gpt-3.5-turbo"));
        assert!(PRICES.contains_key("o4-mini"));
        assert!(PRICES.contains_key("claude-opus-4-6"));
        assert!(PRICES.contains_key("claude-sonnet-4-6"));
        assert!(PRICES.contains_key("claude-haiku-4-5"));
        assert!(PRICES.contains_key("gemini-2.5-flash"));
        assert!(PRICES.contains_key("gemini-2.5-pro"));
        assert!(PRICES.contains_key("gemini-2.0-flash"));
        assert!(PRICES.contains_key("llama-3.3-70b-versatile"));
        assert!(PRICES.contains_key("llama-4-maverick"));
        assert!(PRICES.contains_key("deepseek-chat"));
        assert!(PRICES.contains_key("deepseek-reasoner"));
        assert!(PRICES.contains_key("qwen-2.5-72b"));
        // 19 models minimum
        assert!(PRICES.len() >= 19);
    }

    #[test]
    fn specific_price_values() {
        let p = PRICES.get("gpt-4o").unwrap();
        assert!((p.input_per_million - 2.50).abs() < f64::EPSILON);
        assert!((p.output_per_million - 10.00).abs() < f64::EPSILON);
    }

    // ── find_price fuzzy matching ────────────────────────────────────

    #[test]
    fn find_price_direct_match() {
        assert!(find_price("gpt-4o").is_some());
    }

    #[test]
    fn find_price_empty_returns_none() {
        assert!(find_price("").is_none());
    }

    #[test]
    fn find_price_unknown_returns_none() {
        assert!(find_price("nonexistent-model").is_none());
    }

    #[test]
    fn find_price_strips_openrouter_prefix() {
        assert!(find_price("openai/gpt-4o").is_some());
        let p = find_price("openai/gpt-4o").unwrap();
        let direct = find_price("gpt-4o").unwrap();
        assert_eq!(p, direct);
    }

    #[test]
    fn find_price_strips_date_suffix() {
        assert!(find_price("gpt-4o-2025-03-01").is_some());
        let p = find_price("gpt-4o-2025-03-01").unwrap();
        let direct = find_price("gpt-4o").unwrap();
        assert_eq!(p, direct);
    }

    #[test]
    fn find_price_strips_date_suffix_partial() {
        // "gpt-4o-2025" → "gpt-4o"
        assert!(find_price("gpt-4o-2025").is_some());
    }

    #[test]
    fn find_price_openrouter_plus_date() {
        // "anthropic/claude-sonnet-4-6-2025-01" → strips prefix → strips dates
        assert!(find_price("anthropic/claude-sonnet-4-6").is_some());
    }

    #[test]
    fn find_price_multi_segment_date_stripping() {
        // "gpt-4-turbo-2024-04-09" → "gpt-4-turbo-2024-04" → "gpt-4-turbo-2024" → "gpt-4-turbo"
        assert!(find_price("gpt-4-turbo-2024-04-09").is_some());
    }

    // ── estimate_cost ────────────────────────────────────────────────

    #[test]
    fn estimate_cost_known_model() {
        // gpt-4o: $2.50/M in, $10.00/M out
        // 1000 in + 500 out = 1000*2.5/1M + 500*10/1M = 0.0025 + 0.005 = 0.0075
        let cost = estimate_cost("gpt-4o", 1000, 500);
        assert!((cost - 0.0075).abs() < 1e-10);
    }

    #[test]
    fn estimate_cost_unknown_model_returns_zero() {
        assert_eq!(estimate_cost("local-llama", 1000, 500), 0.0);
    }

    #[test]
    fn estimate_cost_zero_tokens() {
        assert_eq!(estimate_cost("gpt-4o", 0, 0), 0.0);
    }

    #[test]
    fn estimate_cost_large_tokens() {
        // 1M input + 1M output of gpt-4o = 2.50 + 10.00 = 12.50
        let cost = estimate_cost("gpt-4o", 1_000_000, 1_000_000);
        assert!((cost - 12.50).abs() < 1e-6);
    }

    // ── format_cost ──────────────────────────────────────────────────

    #[test]
    fn format_cost_tiny() {
        assert_eq!(format_cost(0.0001), "< $0.001");
    }

    #[test]
    fn format_cost_small() {
        assert_eq!(format_cost(0.003), "$0.003");
    }

    #[test]
    fn format_cost_medium() {
        assert_eq!(format_cost(0.05), "$0.05");
    }

    #[test]
    fn format_cost_large() {
        assert_eq!(format_cost(12.50), "$12.50");
    }

    #[test]
    fn format_cost_zero() {
        assert_eq!(format_cost(0.0), "< $0.001");
    }

    // ── format_tokens ────────────────────────────────────────────────

    #[test]
    fn format_tokens_small() {
        assert_eq!(format_tokens(500), "500");
    }

    #[test]
    fn format_tokens_thousands() {
        assert_eq!(format_tokens(8200), "8.2K");
    }

    #[test]
    fn format_tokens_exact_thousand() {
        assert_eq!(format_tokens(1000), "1.0K");
    }

    #[test]
    fn format_tokens_millions() {
        assert_eq!(format_tokens(1_200_000), "1.2M");
    }

    #[test]
    fn format_tokens_zero() {
        assert_eq!(format_tokens(0), "0");
    }

    // ── estimate_steps ───────────────────────────────────────────────

    #[test]
    fn estimate_steps_known_model() {
        // gpt-4o: 4000*2.5/1M + 1000*10/1M = 0.01 + 0.01 = 0.02 per step
        // $1.00 / $0.02 = 50 steps
        let steps = estimate_steps("gpt-4o", 1.0);
        assert_eq!(steps, 50);
    }

    #[test]
    fn estimate_steps_unknown_model() {
        assert_eq!(estimate_steps("nonexistent", 1.0), 0);
    }

    #[test]
    fn estimate_steps_zero_budget() {
        assert_eq!(estimate_steps("gpt-4o", 0.0), 0);
    }

    #[test]
    fn estimate_steps_cheap_model() {
        // gpt-4.1-nano: 4000*0.1/1M + 1000*0.4/1M = 0.0004 + 0.0004 = 0.0008 per step
        // $1.00 / $0.0008 = 1250 steps
        let steps = estimate_steps("gpt-4.1-nano", 1.0);
        assert_eq!(steps, 1250);
    }
}
