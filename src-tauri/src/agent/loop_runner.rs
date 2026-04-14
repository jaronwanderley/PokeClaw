// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

//! ReAct-style agent loop runner with TaskEvent streaming.
//!
//! The core `run_agent_loop` function implements a multi-round cycle:
//! system prompt → LLM → parse tool calls → execute → feed result back → repeat.
//!
//! Event emission is behind an `EventEmitter` trait so the loop is testable
//! without Tauri's `Channel` type. The Tauri IPC integration will wrap this
//! with a `ChannelTaskEventEmitter`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use log::{info, warn, error};

use crate::agent::budget::{self, TaskBudget};
use crate::agent::config::AgentConfig;
use crate::agent::guards::GuardRegistry;
use crate::agent::llm::{ChatMessage, LlmProvider};
use crate::agent::stuck_detector::{RecoveryLevel, StuckDetector};
use crate::agent::task_event::TaskEvent;
use crate::agent::token_monitor::TokenMonitor;
use crate::agent::tool_executor::ToolExecutor;
use crate::agent::tool_registry::ToolRegistry;
use crate::db::Database;

// ---------------------------------------------------------------------------
// EventEmitter trait — abstracts event delivery for testability
// ---------------------------------------------------------------------------

/// Trait for emitting TaskEvent values. Implemented by both Tauri Channel
/// wrappers (production) and Vec collectors (tests).
pub trait EventEmitter: Send + Sync {
    /// Emit an event. Returns false if the receiver is gone (e.g. channel closed).
    fn emit(&self, event: TaskEvent) -> bool;
}

// ---------------------------------------------------------------------------
// Persistence helpers — non-fatal DB operations for the agent loop
// ---------------------------------------------------------------------------

/// Persist a task event. Logs errors but never fails the agent loop.
fn persist_task_event(db_arc: &Arc<std::sync::Mutex<Database>>, task_id: i64, event_type: &str, event_data: &str) {
    match db_arc.lock() {
        Ok(db) => {
            if let Err(e) = db.insert_task_event(task_id, event_type, Some(event_data)) {
                error!("persist_task_event: task_id={}, type={}: {}", task_id, event_type, e);
            } else {
                info!("persist_task_event: task_id={}, type={}", task_id, event_type);
            }
        }
        Err(e) => {
            error!("persist_task_event: DB lock error for task_id={}: {}", task_id, e);
        }
    }
}

/// Persist a completed task with answer, tokens, cost, and tool count.
/// Also updates cumulative session totals in agent_state.
fn persist_task_completed(
    db_arc: &Arc<std::sync::Mutex<Database>>,
    task_id: i64,
    round: u32,
    answer: &str,
    total_tokens: i64,
    total_cost_usd: f64,
    tool_count: i64,
) {
    match db_arc.lock() {
        Ok(db) => {
            let completed_at = chrono_now_rfc3339();
            if let Err(e) = db.update_task_status(
                task_id,
                "completed",
                Some(answer),
                None,
                round as i64,
                total_tokens,
                total_cost_usd,
                tool_count,
                Some(&completed_at),
            ) {
                error!("persist_task_completed: task_id={}: {}", task_id, e);
            } else {
                info!(
                    "persist_task_completed: task_id={}, tokens={}, cost={:.4}, tools={}",
                    task_id, total_tokens, total_cost_usd, tool_count
                );
            }

            // Update cumulative session totals
            update_session_totals(&db, total_tokens, total_cost_usd);
        }
        Err(e) => {
            error!("persist_task_completed: DB lock error for task_id={}: {}", task_id, e);
        }
    }
}

/// Persist a failed task with error info.
fn persist_task_failed(
    db_arc: &Arc<std::sync::Mutex<Database>>,
    task_id: i64,
    round: u32,
    error_msg: &str,
    token_monitor: &TokenMonitor,
) {
    match db_arc.lock() {
        Ok(db) => {
            let status = token_monitor.get_status();
            let completed_at = chrono_now_rfc3339();
            if let Err(e) = db.update_task_status(
                task_id,
                "failed",
                None,
                Some(error_msg),
                round as i64,
                status.total_tokens as i64,
                status.estimated_cost_usd,
                0, // tool_count not tracked accurately on failure path
                Some(&completed_at),
            ) {
                error!("persist_task_failed: task_id={}: {}", task_id, e);
            } else {
                info!("persist_task_failed: task_id={}, error='{}'", task_id, error_msg);
            }

            // Update cumulative session totals even on failure
            update_session_totals(&db, status.total_tokens as i64, status.estimated_cost_usd);
        }
        Err(e) => {
            error!("persist_task_failed: DB lock error for task_id={}: {}", task_id, e);
        }
    }
}

/// Persist a cancelled task.
fn persist_task_cancelled(db_arc: &Arc<std::sync::Mutex<Database>>, task_id: i64) {
    match db_arc.lock() {
        Ok(db) => {
            let completed_at = chrono_now_rfc3339();
            if let Err(e) = db.update_task_status(
                task_id,
                "cancelled",
                None,
                None,
                0,
                0,
                0.0,
                0,
                Some(&completed_at),
            ) {
                error!("persist_task_cancelled: task_id={}: {}", task_id, e);
            } else {
                info!("persist_task_cancelled: task_id={}", task_id);
            }
        }
        Err(e) => {
            error!("persist_task_cancelled: DB lock error for task_id={}: {}", task_id, e);
        }
    }
}

/// Update cumulative session token/cost totals in agent_state.
fn update_session_totals(db: &Database, new_tokens: i64, new_cost: f64) {
    // Accumulate tokens
    let current_tokens: i64 = db
        .get_state("session_total_tokens")
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if let Err(e) = db.set_state("session_total_tokens", &(current_tokens + new_tokens).to_string()) {
        error!("update_session_totals: failed to update session_total_tokens: {}", e);
    }

    // Accumulate cost
    let current_cost: f64 = db
        .get_state("session_total_cost")
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    if let Err(e) = db.set_state("session_total_cost", &(current_cost + new_cost).to_string()) {
        error!("update_session_totals: failed to update session_total_cost: {}", e);
    }
}

/// Returns current datetime in RFC 3339 / ISO 8601 format.
fn chrono_now_rfc3339() -> String {
    // Use std::time for simplicity — format as ISO 8601 without chrono dependency
    // SQLite's datetime('now') format: YYYY-MM-DD HH:MM:SS
    let now = std::time::SystemTime::now();
    let duration = now.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs();
    // Calculate date components
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
    // Calculate year/month/day from days since epoch (simplified algorithm)
    let (year, month, day) = days_to_ymd(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        year, month, day, hours, minutes, seconds
    )
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap_year(year);
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 0u64;
    for &md in &month_days {
        month += 1;
        if days < md {
            break;
        }
        days -= md;
    }
    (year, month, days + 1)
}

fn is_leap_year(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

// ---------------------------------------------------------------------------
// run_agent_loop — the core ReAct cycle
// ---------------------------------------------------------------------------

/// Run the ReAct-style agent loop until completion, failure, or cancellation.
///
/// This is the primary entry point for agent task execution. It:
/// 1. Builds the system prompt and initial message history
/// 2. Iterates up to `config.max_iterations` rounds
/// 3. Calls the LLM, parses tool calls, executes tools, feeds results back
/// 4. Streams TaskEvent progress via the emitter
/// 5. Enforces token budgets, stuck detection, and cancellation
///
/// Returns `Ok(())` on normal completion (Completed/Cancelled) or `Err(msg)`
/// if the loop cannot proceed (e.g. initial LLM call fails).
pub async fn run_agent_loop(
    task: String,
    provider: Box<dyn LlmProvider>,
    executor: Box<dyn ToolExecutor>,
    registry: ToolRegistry,
    emitter: Box<dyn EventEmitter>,
    cancel: Arc<AtomicBool>,
    config: AgentConfig,
    db: Option<Arc<std::sync::Mutex<Database>>>,
    task_db_id: Option<i64>,
    guards: Option<GuardRegistry>,
) -> Result<(), String> {
    info!(
        "run_agent_loop: starting task '{}' with model '{}', max_iterations={}",
        task, config.model_name, config.max_iterations
    );

    // Build tool schemas from registry
    let tool_schemas: Vec<serde_json::Value> = registry
        .all_specs()
        .iter()
        .map(|spec| {
            serde_json::json!({
                "name": spec.name,
                "description": spec.description_en,
                "parameters": spec.parameters_json_schema()
            })
        })
        .collect();

    info!("run_agent_loop: {} tools available", tool_schemas.len());

    // Build guard prompt sections (if guards are active)
    let mut guard_prompt_suffix = String::new();
    let mut guards = guards; // make mutable for recording
    if let Some(ref g) = guards {
        guard_prompt_suffix = g.build_prompt_sections();
        if !guard_prompt_suffix.is_empty() {
            info!("run_agent_loop: guard prompt sections added ({} chars)", guard_prompt_suffix.len());
        }
    }

    // Initialize message history
    let mut system_text = config.system_prompt.clone();
    if !guard_prompt_suffix.is_empty() {
        system_text.push_str("\n\n");
        system_text.push_str(&guard_prompt_suffix);
    }
    let mut messages: Vec<ChatMessage> = vec![
        ChatMessage::System(system_text),
        ChatMessage::User(task.clone()),
    ];

    // Initialize monitors
    let mut token_monitor = TokenMonitor::new(&config.model_name);
    let mut stuck_detector = StuckDetector::with_defaults();
    let budget = TaskBudget::new(
        config.max_tokens,
        config.max_cost_usd,
        config.soft_limit_percent,
    );
    let mut total_tool_count: i64 = 0;

    // Main ReAct loop
    for round in 1..=config.max_iterations {
        // ── a. Check cancellation ──────────────────────────────────
        if cancel.load(Ordering::Relaxed) {
            info!("run_agent_loop: cancelled at round {}", round);
            emitter.emit(TaskEvent::Cancelled);
            if let (Some(ref db), Some(tid)) = (&db, task_db_id) {
                persist_task_cancelled(db, tid);
            }
            return Ok(());
        }

        // ── b. Emit loop start ────────────────────────────────────
        emitter.emit(TaskEvent::LoopStart { round });

        // ── c. Call LLM ───────────────────────────────────────────
        info!(
            "run_agent_loop: round {}, calling LLM with {} messages",
            round,
            messages.len()
        );

        let response = match provider.chat(messages.clone(), tool_schemas.clone()).await {
            Ok(resp) => resp,
            Err(e) => {
                let error_msg = e.to_string();
                error!("run_agent_loop: LLM error at round {}: {}", round, error_msg);
                emitter.emit(TaskEvent::Failed {
                    error: error_msg.clone(),
                });
                if let (Some(ref db), Some(tid)) = (&db, task_db_id) {
                    persist_task_failed(db, tid, round, &error_msg, &token_monitor);
                }
                return Err(error_msg);
            }
        };

        // ── e. Update token monitor, emit TokenUpdate ─────────────
        if let Some(ref usage) = response.usage {
            token_monitor.record(
                round,
                Some(usage.prompt_tokens),
                Some(usage.completion_tokens),
                None,
            );
        }
        let token_status = token_monitor.get_status();
        emitter.emit(TaskEvent::TokenUpdate {
            step: token_status.step,
            formatted_tokens: token_status.formatted_tokens.clone(),
            formatted_cost: token_status.formatted_cost.clone(),
        });

        // ── f. Budget enforcement ──────────────────────────────────
        let budget_status = budget.check(
            token_status.total_tokens,
            token_status.estimated_cost_usd,
        );

        match budget_status {
            budget::Status::HardLimit => {
                let msg = format!(
                    "Token budget exceeded: {} tokens (${:.2}). Task terminated.",
                    token_status.formatted_tokens, token_status.estimated_cost_usd
                );
                warn!("run_agent_loop: {}", msg);
                emitter.emit(TaskEvent::Completed {
                    answer: msg.clone(),
                    model_name: config.model_name.clone(),
                });
                if let (Some(ref db_arc), Some(tid)) = (&db, task_db_id) {
                    persist_task_completed(
                        db_arc,
                        tid,
                        round,
                        &msg,
                        token_status.total_tokens as i64,
                        token_status.estimated_cost_usd,
                        total_tool_count,
                    );
                }
                return Ok(());
            }
            budget::Status::SoftLimit => {
                let warning = format!(
                    "[Budget Warning] {:.0}% of token budget used ({} tokens, ${:.2}). Please wrap up soon.",
                    (token_status.total_tokens as f64 / config.max_tokens as f64) * 100.0,
                    token_status.formatted_tokens,
                    token_status.estimated_cost_usd,
                );
                warn!("run_agent_loop: {}", warning);
                messages.push(ChatMessage::User(warning));
            }
            budget::Status::Ok => {}
        }

        // ── g. Add assistant response to history ──────────────────
        if response.tool_calls.is_empty() {
            // Text-only response
            if let Some(ref text) = response.text {
                messages.push(ChatMessage::Assistant(text.clone()));
            }
        } else {
            // Response with tool calls — use AssistantWithTools
            messages.push(ChatMessage::AssistantWithTools {
                text: response.text.clone(),
                tool_calls: response.tool_calls.clone(),
            });
        }

        // ── h. Emit thinking text ─────────────────────────────────
        if let Some(ref text) = response.text {
            if !text.trim().is_empty() {
                emitter.emit(TaskEvent::Thinking {
                    content: text.clone(),
                });
            }
        }

        // ── i. No tool calls → check guards, then task complete ────
        if response.tool_calls.is_empty() {
            // Guard: check if text-only completion should be blocked
            let blocked = if let Some(ref g) = guards {
                if g.should_block_text_only_completion() {
                    let correction = g.build_completion_correction();
                    warn!(
                        "run_agent_loop: guard blocking text-only completion at round {} — {}",
                        round, correction
                    );
                    messages.push(ChatMessage::User(correction));
                    true
                } else {
                    false
                }
            } else {
                false
            };

            if blocked {
                // Continue loop — the correction message will prompt the LLM to use tools
                continue;
            }

            let answer = response.text.unwrap_or_else(|| "Task completed.".to_string());
            info!("run_agent_loop: completed at round {} (no tool calls)", round);
            emitter.emit(TaskEvent::Completed {
                answer: answer.clone(),
                model_name: config.model_name.clone(),
            });
            if let (Some(ref db_arc), Some(tid)) = (&db, task_db_id) {
                let status = token_monitor.get_status();
                persist_task_completed(
                    db_arc,
                    tid,
                    round,
                    &answer,
                    status.total_tokens as i64,
                    status.estimated_cost_usd,
                    total_tool_count,
                );
            }
            return Ok(());
        }

        // ── j. Execute tool calls ─────────────────────────────────
        let mut guard_blocked_finish = false;
        for tool_call in &response.tool_calls {
            emitter.emit(TaskEvent::ToolAction {
                tool_name: tool_call.name.clone(),
            });

            info!(
                "run_agent_loop: executing tool '{}' (id={})",
                tool_call.name, tool_call.id
            );

            // Parse arguments
            let params: serde_json::Value = if tool_call.arguments.is_empty() {
                serde_json::Value::Object(serde_json::Map::new())
            } else {
                serde_json::from_str(&tool_call.arguments).unwrap_or_else(|e| {
                    warn!(
                        "run_agent_loop: failed to parse tool args '{}': {}",
                        tool_call.arguments, e
                    );
                    serde_json::Value::Object(serde_json::Map::new())
                })
            };

            let result = executor.execute(&tool_call.name, params);

            let detail = if result.success {
                result
                    .data
                    .as_ref()
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| "Success".to_string())
            } else {
                result
                    .error
                    .clone()
                    .unwrap_or_else(|| "Unknown error".to_string())
            };

            emitter.emit(TaskEvent::ToolResult {
                tool_name: tool_call.name.clone(),
                success: result.success,
                detail: detail.clone(),
            });

            // Persist tool result event to DB
            total_tool_count += 1;
            if let (Some(ref db_arc), Some(tid)) = (&db, task_db_id) {
                persist_task_event(db_arc, tid, "toolResult", &serde_json::json!({
                    "tool_name": tool_call.name,
                    "success": result.success,
                    "detail": detail,
                }).to_string());
            }

            // Guard: record successful tool execution
            if result.success {
                let parsed_params: serde_json::Value = if tool_call.arguments.is_empty() {
                    serde_json::Value::Object(serde_json::Map::new())
                } else {
                    serde_json::from_str(&tool_call.arguments).unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
                };
                if let Some(ref mut g) = guards {
                    g.record_successful_tool(&tool_call.name, &parsed_params);
                }
            }

            // Extract finish answer before consuming result.data
            let finish_answer = if tool_call.name == "finish" {
                result.data.as_ref().and_then(|d| {
                    d.get("result")
                        .and_then(|v| v.as_str().map(|s| s.to_string()))
                })
            } else {
                None
            };

            // Add tool result to message history
            let tool_result_content = if result.success {
                result
                    .data
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| "Success".to_string())
            } else {
                result
                    .error
                    .unwrap_or_else(|| "Unknown error".to_string())
            };
            messages.push(ChatMessage::ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: tool_result_content,
            });

            // ── k. Finish tool detection with guard check ────────
            if tool_call.name == "finish" {
                // Guard: check if finish should be blocked
                if let Some(ref g) = guards {
                    if let Some(block_reason) = g.maybe_block_finish(None) {
                        warn!("run_agent_loop: guard blocking finish at round {} — {}", round, block_reason);
                        // Replace the finish tool result with a user message telling the agent to continue
                        messages.push(ChatMessage::User(format!(
                            "Do not call finish yet. {}",
                            block_reason
                        )));
                        guard_blocked_finish = true;
                    }
                }

                if guard_blocked_finish {
                    break; // Break out of tool call loop, continue outer round loop
                }

                let answer = finish_answer.unwrap_or_else(|| "Task completed.".to_string());
                info!("run_agent_loop: finish tool called at round {}", round);
                emitter.emit(TaskEvent::Completed {
                    answer: answer.clone(),
                    model_name: config.model_name.clone(),
                });
                if let (Some(ref db_arc), Some(tid)) = (&db, task_db_id) {
                    let status = token_monitor.get_status();
                    persist_task_completed(
                        db_arc,
                        tid,
                        round,
                        &answer,
                        status.total_tokens as i64,
                        status.estimated_cost_usd,
                        total_tool_count,
                    );
                }
                return Ok(());
            }
        }

        // ── l. Stuck detection ────────────────────────────────────
        let action_name = response
            .tool_calls
            .first()
            .map(|tc| tc.name.clone())
            .unwrap_or_default();

        let detection = stuck_detector.record(
            &action_name,
            0, // screen_hash — placeholder until real screen data is integrated
            0, // screen_diff_count — placeholder
            None,
        );

        if let Some(det) = detection {
            match det.level {
                RecoveryLevel::AutoKill => {
                    let msg = format!(
                        "Agent appears stuck ({}). Terminating task.",
                        det.signal.description()
                    );
                    warn!("run_agent_loop: {}", msg);
                    emitter.emit(TaskEvent::Completed {
                        answer: msg.clone(),
                        model_name: config.model_name.clone(),
                    });
                    if let (Some(ref db_arc), Some(tid)) = (&db, task_db_id) {
                        let status = token_monitor.get_status();
                        persist_task_completed(
                            db_arc,
                            tid,
                            round,
                            &msg,
                            status.total_tokens as i64,
                            status.estimated_cost_usd,
                            total_tool_count,
                        );
                    }
                    return Ok(());
                }
                RecoveryLevel::Hint | RecoveryLevel::StrategySwitch => {
                    if !det.recovery_hint.is_empty() {
                        warn!(
                            "run_agent_loop: stuck detected (level={}): {}",
                            det.level,
                            det.signal.description()
                        );
                        messages.push(ChatMessage::User(det.recovery_hint));
                    }
                }
            }
        }
    }

    // ── Exhausted max iterations ───────────────────────────────────
    let msg = format!(
        "Agent loop exhausted maximum iterations ({})",
        config.max_iterations
    );
    warn!("run_agent_loop: {}", msg);
    emitter.emit(TaskEvent::Failed {
        error: msg.clone(),
    });
    if let (Some(ref db_arc), Some(tid)) = (&db, task_db_id) {
        persist_task_failed(db_arc, tid, config.max_iterations, &msg, &token_monitor);
    }
    Err(msg)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::llm::{LlmError, LlmResponse, TokenUsage, ToolCall};
    use crate::agent::task_event::TaskEvent;
    use async_trait::async_trait;
    use std::sync::Mutex;

    // ── Collecting emitter for tests ───────────────────────────────

    struct VecEmitter {
        events: Mutex<Vec<TaskEvent>>,
    }

    impl VecEmitter {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
            }
        }

        fn events(&self) -> Vec<TaskEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    impl EventEmitter for VecEmitter {
        fn emit(&self, event: TaskEvent) -> bool {
            self.events.lock().unwrap().push(event);
            true
        }
    }

    /// Blanket impl so tests can use `Arc<VecEmitter>` → `Box<dyn EventEmitter>`.
    impl<T: EventEmitter + ?Sized> EventEmitter for std::sync::Arc<T> {
        fn emit(&self, event: TaskEvent) -> bool {
            (**self).emit(event)
        }
    }

    // ── Mock LLM provider ──────────────────────────────────────────

    /// Mock provider that returns pre-configured responses in sequence.
    struct MockProvider {
        responses: Mutex<Vec<LlmResponse>>,
    }

    impl MockProvider {
        fn new(responses: Vec<LlmResponse>) -> Self {
            Self {
                responses: Mutex::new(responses),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        async fn chat(
            &self,
            _messages: Vec<ChatMessage>,
            _tools: Vec<serde_json::Value>,
        ) -> Result<LlmResponse, LlmError> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Ok(LlmResponse {
                    text: Some("No more mock responses".to_string()),
                    tool_calls: vec![],
                    usage: Some(TokenUsage {
                        prompt_tokens: 10,
                        completion_tokens: 5,
                    }),
                });
            }
            Ok(responses.remove(0))
        }
    }

    /// Mock provider that always errors.
    struct ErrorProvider {
        error: LlmError,
    }

    #[async_trait]
    impl LlmProvider for ErrorProvider {
        async fn chat(
            &self,
            _messages: Vec<ChatMessage>,
            _tools: Vec<serde_json::Value>,
        ) -> Result<LlmResponse, LlmError> {
            Err(self.error.clone())
        }
    }

    // ── Helper to create test config ───────────────────────────────

    fn test_config() -> AgentConfig {
        AgentConfig {
            model_name: "test-model".to_string(),
            max_iterations: 5,
            system_prompt: "You are a test agent.".to_string(),
            max_tokens: 250_000,
            max_cost_usd: 1.00,
            soft_limit_percent: 0.80,
        }
    }

    fn make_registry() -> ToolRegistry {
        ToolRegistry::default()
    }

    // ── Test: simple text completion ───────────────────────────────

    #[tokio::test]
    async fn test_loop_text_completion() {
        let provider = MockProvider::new(vec![LlmResponse {
            text: Some("I'm done!".to_string()),
            tool_calls: vec![],
            usage: Some(TokenUsage {
                prompt_tokens: 100,
                completion_tokens: 10,
            }),
        }]);

        let executor = crate::agent::tool_executor::ToolExecutorHandle::new();
        let emitter = VecEmitter::new();
        let cancel = Arc::new(AtomicBool::new(false));

        let result = run_agent_loop(
            "Say hello".to_string(),
            Box::new(provider),
            Box::new(executor),
            make_registry(),
            Box::new(emitter),
            cancel,
            test_config(),
            None,
            None,
            None,
        )
        .await;

        assert!(result.is_ok());

        // Can't check emitter events here since it was moved — we need a different approach
    }

    // ── Test: events collected via Arc ─────────────────────────────

    #[tokio::test]
    async fn test_events_emitted_in_order() {
        use std::sync::Arc;

        let provider = MockProvider::new(vec![
            // Round 1: LLM returns a tool call
            LlmResponse {
                text: Some("I'll tap the button.".to_string()),
                tool_calls: vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "tap".to_string(),
                    arguments: r#"{"x":100,"y":200}"#.to_string(),
                }],
                usage: Some(TokenUsage {
                    prompt_tokens: 50,
                    completion_tokens: 20,
                }),
            },
            // Round 2: LLM returns text only (done)
            LlmResponse {
                text: Some("Done tapping!".to_string()),
                tool_calls: vec![],
                usage: Some(TokenUsage {
                    prompt_tokens: 80,
                    completion_tokens: 10,
                }),
            },
        ]);

        let executor = crate::agent::tool_executor::ToolExecutorHandle::new();
        let emitter = Arc::new(VecEmitter::new());
        let emitter_clone = emitter.clone();
        let cancel = Arc::new(AtomicBool::new(false));

        let result = run_agent_loop(
            "Tap the button".to_string(),
            Box::new(provider),
            Box::new(executor),
            make_registry(),
            Box::new(emitter_clone),
            cancel,
            test_config(),
            None,
            None,
            None,
        )
        .await;

        assert!(result.is_ok());

        let events = emitter.events();

        // Verify event sequence
        assert!(matches!(events[0], TaskEvent::LoopStart { round: 1 }));
        assert!(matches!(&events[1], TaskEvent::TokenUpdate { step: 1, .. }));
        assert!(matches!(&events[2], TaskEvent::Thinking { .. }));
        assert!(matches!(&events[3], TaskEvent::ToolAction { ref tool_name } if tool_name == "tap"));
        assert!(matches!(&events[4], TaskEvent::ToolResult { ref tool_name, success: true, .. } if tool_name == "tap"));
        assert!(matches!(events[5], TaskEvent::LoopStart { round: 2 }));
        assert!(matches!(&events[6], TaskEvent::TokenUpdate { step: 2, .. }));
        assert!(matches!(&events[7], TaskEvent::Thinking { .. }));
        assert!(matches!(&events[8], TaskEvent::Completed { .. }));
    }

    // ── Test: finish tool ──────────────────────────────────────────

    #[tokio::test]
    async fn test_finish_tool_completes() {
        use std::sync::Arc;

        let provider = MockProvider::new(vec![LlmResponse {
            text: None,
            tool_calls: vec![ToolCall {
                id: "call_finish".to_string(),
                name: "finish".to_string(),
                arguments: r#"{"result":"Task complete!","success":true}"#.to_string(),
            }],
            usage: Some(TokenUsage {
                prompt_tokens: 50,
                completion_tokens: 10,
            }),
        }]);

        let executor = crate::agent::tool_executor::ToolExecutorHandle::new();
        let emitter = Arc::new(VecEmitter::new());
        let emitter_clone = emitter.clone();
        let cancel = Arc::new(AtomicBool::new(false));

        let result = run_agent_loop(
            "Do something".to_string(),
            Box::new(provider),
            Box::new(executor),
            make_registry(),
            Box::new(emitter_clone),
            cancel,
            test_config(),
            None,
            None,
            None,
        )
        .await;

        assert!(result.is_ok());

        let events = emitter.events();
        // Should have: LoopStart, TokenUpdate, ToolAction, ToolResult, Completed
        let completed_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, TaskEvent::Completed { .. }))
            .collect();
        assert_eq!(completed_events.len(), 1);

        if let TaskEvent::Completed { answer, .. } = completed_events[0] {
            assert_eq!(answer, "Task complete!");
        }
    }

    // ── Test: cancellation ─────────────────────────────────────────

    #[tokio::test]
    async fn test_cancellation() {
        use std::sync::Arc;

        let cancel = Arc::new(AtomicBool::new(true)); // Pre-cancelled

        let provider = MockProvider::new(vec![LlmResponse {
            text: Some("Should not see this".to_string()),
            tool_calls: vec![],
            usage: None,
        }]);

        let executor = crate::agent::tool_executor::ToolExecutorHandle::new();
        let emitter = Arc::new(VecEmitter::new());
        let emitter_clone = emitter.clone();

        let result = run_agent_loop(
            "Cancelled task".to_string(),
            Box::new(provider),
            Box::new(executor),
            make_registry(),
            Box::new(emitter_clone),
            cancel,
            test_config(),
            None,
            None,
            None,
        )
        .await;

        assert!(result.is_ok());

        let events = emitter.events();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], TaskEvent::Cancelled));
    }

    // ── Test: LLM error ───────────────────────────────────────────

    #[tokio::test]
    async fn test_llm_error_returns_failed() {
        use std::sync::Arc;

        let provider = ErrorProvider {
            error: LlmError::AuthFailed,
        };

        let executor = crate::agent::tool_executor::ToolExecutorHandle::new();
        let emitter = Arc::new(VecEmitter::new());
        let emitter_clone = emitter.clone();
        let cancel = Arc::new(AtomicBool::new(false));

        let result = run_agent_loop(
            "Will fail".to_string(),
            Box::new(provider),
            Box::new(executor),
            make_registry(),
            Box::new(emitter_clone),
            cancel,
            test_config(),
            None,
            None,
            None,
        )
        .await;

        assert!(result.is_err());

        let events = emitter.events();
        assert!(events.len() >= 2, "Expected at least 2 events (LoopStart + Failed), got {}", events.len());
        assert!(matches!(&events[0], TaskEvent::LoopStart { .. }));
        assert!(matches!(&events[1], TaskEvent::Failed { ref error } if error.contains("Authentication failed")));
    }

    // ── Test: max iterations exhausted ─────────────────────────────

    #[tokio::test]
    async fn test_max_iterations_exhausted() {
        use std::sync::Arc;

        // Provider that always returns tool calls (never finishes)
        let responses: Vec<LlmResponse> = (0..10)
            .map(|_| LlmResponse {
                text: None,
                tool_calls: vec![ToolCall {
                    id: format!("call_{}", "x"),
                    name: "get_screen_info".to_string(),
                    arguments: "{}".to_string(),
                }],
                usage: Some(TokenUsage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                }),
            })
            .collect();

        let provider = MockProvider::new(responses);
        let executor = crate::agent::tool_executor::ToolExecutorHandle::new();
        let emitter = Arc::new(VecEmitter::new());
        let emitter_clone = emitter.clone();
        let cancel = Arc::new(AtomicBool::new(false));

        let mut config = test_config();
        config.max_iterations = 3;

        let result = run_agent_loop(
            "Never ending task".to_string(),
            Box::new(provider),
            Box::new(executor),
            make_registry(),
            Box::new(emitter_clone),
            cancel,
            config,
            None,
            None,
            None,
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("exhausted maximum iterations"));

        let events = emitter.events();
        let failed_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, TaskEvent::Failed { .. }))
            .collect();
        assert_eq!(failed_events.len(), 1);
    }

    // ── Test: multi-round with tool calls ──────────────────────────

    #[tokio::test]
    async fn test_multi_round_tool_execution() {
        use std::sync::Arc;

        let provider = MockProvider::new(vec![
            // Round 1: get screen info
            LlmResponse {
                text: Some("Let me check the screen.".to_string()),
                tool_calls: vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "get_screen_info".to_string(),
                    arguments: "{}".to_string(),
                }],
                usage: Some(TokenUsage {
                    prompt_tokens: 100,
                    completion_tokens: 20,
                }),
            },
            // Round 2: tap a button
            LlmResponse {
                text: Some("I see a button. Let me tap it.".to_string()),
                tool_calls: vec![ToolCall {
                    id: "call_2".to_string(),
                    name: "tap".to_string(),
                    arguments: r#"{"x":540,"y":960}"#.to_string(),
                }],
                usage: Some(TokenUsage {
                    prompt_tokens: 200,
                    completion_tokens: 15,
                }),
            },
            // Round 3: finish
            LlmResponse {
                text: None,
                tool_calls: vec![ToolCall {
                    id: "call_3".to_string(),
                    name: "finish".to_string(),
                    arguments: r#"{"result":"Button tapped successfully!"}"#.to_string(),
                }],
                usage: Some(TokenUsage {
                    prompt_tokens: 250,
                    completion_tokens: 10,
                }),
            },
        ]);

        let executor = crate::agent::tool_executor::ToolExecutorHandle::new();
        let emitter = Arc::new(VecEmitter::new());
        let emitter_clone = emitter.clone();
        let cancel = Arc::new(AtomicBool::new(false));

        let result = run_agent_loop(
            "Tap the button".to_string(),
            Box::new(provider),
            Box::new(executor),
            make_registry(),
            Box::new(emitter_clone),
            cancel,
            test_config(),
            None,
            None,
            None,
        )
        .await;

        assert!(result.is_ok());

        let events = emitter.events();

        // 3 rounds of: LoopStart + TokenUpdate + Thinking + ToolAction + ToolResult
        // Last round: LoopStart + TokenUpdate + ToolAction + ToolResult + Completed (no Thinking since text is None)
        let loop_starts: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, TaskEvent::LoopStart { .. }))
            .collect();
        assert_eq!(loop_starts.len(), 3);

        let completed: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, TaskEvent::Completed { .. }))
            .collect();
        assert_eq!(completed.len(), 1);

        if let TaskEvent::Completed { answer, .. } = completed[0] {
            assert_eq!(answer, "Button tapped successfully!");
        }
    }

    // ── Test: message history grows correctly ──────────────────────

    #[tokio::test]
    async fn test_message_history_grows() {
        // Use a custom mock that verifies message history length
        use std::sync::Arc;

        struct HistoryInspectingProvider {
            call_count: Mutex<u32>,
        }

        #[async_trait]
        impl LlmProvider for HistoryInspectingProvider {
            async fn chat(
                &self,
                messages: Vec<ChatMessage>,
                _tools: Vec<serde_json::Value>,
            ) -> Result<LlmResponse, LlmError> {
                let mut count = self.call_count.lock().unwrap();
                *count += 1;

                match *count {
                    1 => {
                        // First call: system + user = 2 messages
                        assert_eq!(messages.len(), 2);
                        assert!(matches!(&messages[0], ChatMessage::System(_)));
                        assert!(matches!(&messages[1], ChatMessage::User(_)));

                        Ok(LlmResponse {
                            text: Some("Checking screen.".to_string()),
                            tool_calls: vec![ToolCall {
                                id: "c1".to_string(),
                                name: "get_screen_info".to_string(),
                                arguments: "{}".to_string(),
                            }],
                            usage: Some(TokenUsage {
                                prompt_tokens: 50,
                                completion_tokens: 10,
                            }),
                        })
                    }
                    2 => {
                        // Second call: system + user + assistantWithTools + toolResult = 4
                        assert_eq!(messages.len(), 4);
                        assert!(matches!(&messages[2], ChatMessage::AssistantWithTools { .. }));
                        assert!(matches!(&messages[3], ChatMessage::ToolResult { .. }));

                        Ok(LlmResponse {
                            text: Some("Done!".to_string()),
                            tool_calls: vec![],
                            usage: Some(TokenUsage {
                                prompt_tokens: 100,
                                completion_tokens: 5,
                            }),
                        })
                    }
                    _ => Ok(LlmResponse {
                        text: Some("Done.".to_string()),
                        tool_calls: vec![],
                        usage: None,
                    }),
                }
            }
        }

        let provider = HistoryInspectingProvider {
            call_count: Mutex::new(0),
        };
        let executor = crate::agent::tool_executor::ToolExecutorHandle::new();
        let emitter = Arc::new(VecEmitter::new());
        let cancel = Arc::new(AtomicBool::new(false));

        let result = run_agent_loop(
            "Test message history".to_string(),
            Box::new(provider),
            Box::new(executor),
            make_registry(),
            Box::new(emitter.clone()),
            cancel,
            test_config(),
            None,
            None,
            None,
        )
        .await;

        assert!(result.is_ok());
    }

    // ── Test: EventEmitter trait object safety ─────────────────────

    #[tokio::test]
    async fn test_event_emitter_trait_dispatch() {
        // Verify that Box<dyn EventEmitter> works correctly
        let emitter: Box<dyn EventEmitter> = Box::new(VecEmitter::new());
        assert!(emitter.emit(TaskEvent::Cancelled));
    }

    // ── Test: cancel mid-loop ──────────────────────────────────────

    #[tokio::test]
    async fn test_cancel_between_rounds() {
        use std::sync::Arc;

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();

        struct CancelAfterFirstProvider {
            cancel: Arc<AtomicBool>,
        }

        #[async_trait]
        impl LlmProvider for CancelAfterFirstProvider {
            async fn chat(
                &self,
                _messages: Vec<ChatMessage>,
                _tools: Vec<serde_json::Value>,
            ) -> Result<LlmResponse, LlmError> {
                // Set cancel flag after first call
                self.cancel.store(true, Ordering::Relaxed);
                Ok(LlmResponse {
                    text: None,
                    tool_calls: vec![ToolCall {
                        id: "c1".to_string(),
                        name: "get_screen_info".to_string(),
                        arguments: "{}".to_string(),
                    }],
                    usage: Some(TokenUsage {
                        prompt_tokens: 50,
                        completion_tokens: 10,
                    }),
                })
            }
        }

        let provider = CancelAfterFirstProvider {
            cancel: cancel_clone,
        };
        let executor = crate::agent::tool_executor::ToolExecutorHandle::new();
        let emitter = Arc::new(VecEmitter::new());

        let result = run_agent_loop(
            "Will be cancelled".to_string(),
            Box::new(provider),
            Box::new(executor),
            make_registry(),
            Box::new(emitter.clone()),
            cancel,
            test_config(),
            None,
            None,
            None,
        )
        .await;

        assert!(result.is_ok());

        let events = emitter.events();
        // Round 1 completes normally, round 2 gets cancelled
        assert!(events
            .iter()
            .any(|e| matches!(e, TaskEvent::Cancelled)));
    }

    // ── Test: empty tool call arguments handled ────────────────────

    #[tokio::test]
    async fn test_empty_tool_call_arguments() {
        use std::sync::Arc;

        let provider = MockProvider::new(vec![LlmResponse {
            text: None,
            tool_calls: vec![ToolCall {
                id: "call_empty".to_string(),
                name: "get_screen_info".to_string(),
                arguments: "".to_string(), // empty arguments
            }],
            usage: Some(TokenUsage {
                prompt_tokens: 50,
                completion_tokens: 10,
            }),
        }, LlmResponse {
            text: Some("Done".to_string()),
            tool_calls: vec![],
            usage: Some(TokenUsage {
                prompt_tokens: 80,
                completion_tokens: 5,
            }),
        }]);

        let executor = crate::agent::tool_executor::ToolExecutorHandle::new();
        let emitter = Arc::new(VecEmitter::new());
        let cancel = Arc::new(AtomicBool::new(false));

        let result = run_agent_loop(
            "Empty args test".to_string(),
            Box::new(provider),
            Box::new(executor),
            make_registry(),
            Box::new(emitter.clone()),
            cancel,
            test_config(),
            None,
            None,
            None,
        )
        .await;

        assert!(result.is_ok());
    }

    // ── Test: invalid JSON in tool arguments ───────────────────────

    #[tokio::test]
    async fn test_invalid_tool_arguments_handled() {
        use std::sync::Arc;

        let provider = MockProvider::new(vec![LlmResponse {
            text: None,
            tool_calls: vec![ToolCall {
                id: "call_bad".to_string(),
                name: "get_screen_info".to_string(),
                arguments: "not valid json{".to_string(),
            }],
            usage: Some(TokenUsage {
                prompt_tokens: 50,
                completion_tokens: 10,
            }),
        }, LlmResponse {
            text: Some("Done".to_string()),
            tool_calls: vec![],
            usage: Some(TokenUsage {
                prompt_tokens: 80,
                completion_tokens: 5,
            }),
        }]);

        let executor = crate::agent::tool_executor::ToolExecutorHandle::new();
        let emitter = Arc::new(VecEmitter::new());
        let cancel = Arc::new(AtomicBool::new(false));

        // Should not panic on invalid JSON
        let result = run_agent_loop(
            "Invalid args test".to_string(),
            Box::new(provider),
            Box::new(executor),
            make_registry(),
            Box::new(emitter.clone()),
            cancel,
            test_config(),
            None,
            None,
            None,
        )
        .await;

        assert!(result.is_ok());
    }
}
