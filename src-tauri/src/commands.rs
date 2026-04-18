// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

use std::sync::atomic::Ordering;
use std::time::Instant;

use log::{error, info, warn};
use serde::Deserialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use std::sync::Arc;

use tauri_plugin_pokeclaw::InferenceState;

use crate::agent::config::AgentConfig;
use crate::agent::guards::GuardRegistry;
use crate::agent::llm::{ChatMessage, LlmProvider, OpenAiProvider};
use crate::agent::llm::anthropic::AnthropicProvider;
use crate::agent::llm::local::LocalProvider;
use crate::agent::loop_runner::{run_agent_loop, EventEmitter};
use crate::agent::pipeline::{PipelineRouter, Route};
use crate::agent::skill::executor::SkillExecutor;
use crate::agent::skill::registry::SkillRegistry;
use crate::agent::task_event::TaskEvent;
use crate::agent::tool_executor::{
    AgentRoundResult, TokenUsage, ToolCallResult, ToolExecutor,
};
use crate::agent::tool_registry::ToolRegistry;
use crate::db::chat::ChatMessageRecord;
use crate::db::tasks::{TaskEventRecord, TaskRecord};
use crate::db::Database;
use crate::AgentState;
use crate::LlmProviderType;

// ---------------------------------------------------------------------------
// ChannelEventEmitter — adapts tauri::ipc::Channel to EventEmitter trait
// ---------------------------------------------------------------------------

/// Wraps a Tauri IPC Channel to implement the EventEmitter trait used by
/// run_agent_loop. This bridges the agent loop's testable event interface
/// with Tauri's streaming Channel primitive.
struct ChannelEventEmitter {
    channel: Channel<TaskEvent>,
}

impl ChannelEventEmitter {
    fn new(channel: Channel<TaskEvent>) -> Self {
        Self { channel }
    }
}

impl EventEmitter for ChannelEventEmitter {
    fn emit(&self, event: TaskEvent) -> bool {
        self.channel.send(event).is_ok()
    }
}

// ---------------------------------------------------------------------------
// ChannelEventEmitterWrapper — wraps a consumed ChannelEventEmitter
// ---------------------------------------------------------------------------

/// Wraps a ChannelEventEmitter that has already been partially consumed
/// (e.g., for skill progress events) so it can be passed to run_agent_loop
/// after a skill failure fallback. Implements EventEmitter by delegating
/// to the inner ChannelEventEmitter.
struct ChannelEventEmitterWrapper {
    inner: ChannelEventEmitter,
}

impl EventEmitter for ChannelEventEmitterWrapper {
    fn emit(&self, event: TaskEvent) -> bool {
        self.inner.emit(event)
    }
}

// ---------------------------------------------------------------------------
// Date helpers — used by persistence inline in DirectTool/Skill paths
// ---------------------------------------------------------------------------

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd_local(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap_year_local(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap_year_local(year);
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

fn is_leap_year_local(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

// ---------------------------------------------------------------------------
// test_agent_round command
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct TestAgentRoundRequest {
    pub prompt: String,
    pub system_prompt: Option<String>,
}

/// Execute one agent round: prompt → LLM → tool execution → result.
#[tauri::command]
pub async fn test_agent_round(
    app: AppHandle,
    state: State<'_, AgentState>,
    prompt: String,
    system_prompt: Option<String>,
) -> Result<AgentRoundResult, String> {
    let start = Instant::now();
    info!("test_agent_round: prompt='{}'", prompt);

    // Validate prompt
    if prompt.trim().is_empty() {
        return Ok(AgentRoundResult {
            prompt: prompt.clone(),
            model: String::new(),
            tool_call: None,
            response_text: None,
            latency_ms: start.elapsed().as_millis() as u64,
            token_usage: None,
        });
    }

    // Get API key
    let api_key = {
        let key_guard = state.openai_api_key.lock().map_err(|e| e.to_string())?;
        match key_guard.clone() {
            Some(k) if !k.trim().is_empty() => k,
            _ => {
                warn!("test_agent_round: OpenAI API key not set");
                return Ok(AgentRoundResult {
                    prompt: prompt.clone(),
                    model: String::new(),
                    tool_call: None,
                    response_text: Some("OpenAI API key not set. Use set_openai_api_key first.".into()),
                    latency_ms: start.elapsed().as_millis() as u64,
                    token_usage: None,
                });
            }
        }
    };

    // Build tool registry and schemas
    let registry = ToolRegistry::default();
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

    info!("test_agent_round: {} tools loaded from registry", tool_schemas.len());

    // Build messages
    let system_text = system_prompt.unwrap_or_else(|| {
        "You are a phone assistant. Use the available tools to help the user control their phone. \
         When the user asks you to do something, call the appropriate tool."
            .into()
    });

    let messages = vec![
        ChatMessage::System(system_text),
        ChatMessage::User(prompt.clone()),
    ];

    // Create provider and call LLM
    let model = "gpt-4o-mini".to_string();
    let provider = OpenAiProvider::new(api_key, model.clone());

    let llm_response = match provider.chat(messages, tool_schemas).await {
        Ok(resp) => resp,
        Err(e) => {
            error!("test_agent_round: LLM error: {}", e);
            return Ok(AgentRoundResult {
                prompt: prompt.clone(),
                model,
                tool_call: None,
                response_text: Some(format!("LLM error: {}", e)),
                latency_ms: start.elapsed().as_millis() as u64,
                token_usage: None,
            });
        }
    };

    info!(
        "test_agent_round: LLM responded — text={}, tool_calls={}",
        llm_response.text.is_some(),
        llm_response.tool_calls.len()
    );

    // If there are tool calls, execute the first one
    let tool_call_result = if let Some(tc) = llm_response.tool_calls.first() {
        info!("test_agent_round: executing tool '{}' with args: {}", tc.name, tc.arguments);

        let executor = crate::agent::tool_executor::create_executor(&app);
        let args: serde_json::Value = serde_json::from_str(&tc.arguments).unwrap_or_else(|e| {
            warn!("test_agent_round: failed to parse tool arguments as JSON: {} — using empty object", e);
            serde_json::json!({})
        });

        let exec_result = executor.execute(&tc.name, args);

        Some(ToolCallResult {
            name: tc.name.clone(),
            arguments: serde_json::from_str(&tc.arguments).unwrap_or(serde_json::json!({})),
            result: exec_result,
        })
    } else {
        None
    };

    let token_usage = llm_response.usage.map(|u| TokenUsage {
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
    });

    let latency_ms = start.elapsed().as_millis() as u64;
    let result = AgentRoundResult {
        prompt: prompt.clone(),
        model,
        tool_call: tool_call_result,
        response_text: llm_response.text,
        latency_ms,
        token_usage,
    };

    info!(
        "test_agent_round: complete — latency={}ms, tool_call={}, tokens={:?}",
        result.latency_ms,
        result.tool_call.as_ref().map(|tc| tc.name.clone()).unwrap_or_default(),
        result.token_usage,
    );

    Ok(result)
}

// ---------------------------------------------------------------------------
// set_openai_api_key command
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn set_openai_api_key(state: State<'_, AgentState>, key: String) -> Result<(), String> {
    info!("set_openai_api_key: setting API key (length={})", key.len());
    let mut guard = state.openai_api_key.lock().map_err(|e| e.to_string())?;
    *guard = Some(key);
    Ok(())
}

// ---------------------------------------------------------------------------
// set_anthropic_api_key command
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn set_anthropic_api_key(state: State<'_, AgentState>, key: String) -> Result<(), String> {
    info!("set_anthropic_api_key: setting API key (length={})", key.len());
    let mut guard = state.anthropic_api_key.lock().map_err(|e| e.to_string())?;
    *guard = Some(key);
    Ok(())
}

// ---------------------------------------------------------------------------
// set_llm_provider_type command
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn set_llm_provider_type(state: State<'_, AgentState>, provider_type: String) -> Result<(), String> {
    let ptype = match provider_type.to_lowercase().as_str() {
        "openai" => LlmProviderType::OpenAi,
        "anthropic" => LlmProviderType::Anthropic,
        "local" => LlmProviderType::Local,
        _ => return Err(format!("Unknown provider type: '{}'. Use 'openai', 'anthropic', or 'local'.", provider_type)),
    };
    info!("set_llm_provider_type: switching to {:?}", ptype);
    let mut guard = state.llm_provider_type.lock().map_err(|e| e.to_string())?;
    *guard = ptype;
    Ok(())
}

// ---------------------------------------------------------------------------
// start_task command — spawns the agent loop in a background tokio task
// ---------------------------------------------------------------------------

/// Start a multi-round agent task. Returns immediately; events stream via
/// the `on_event` Channel parameter. Rejects if a task is already running
/// or if the OpenAI API key has not been set.
#[tauri::command]
pub async fn start_task(
    app: AppHandle,
    state: State<'_, AgentState>,
    _inference_state: State<'_, InferenceState>,
    task: String,
    on_event: Channel<TaskEvent>,
) -> Result<(), String> {
    info!("start_task: request received — task='{}'", task);

    // ── Guard: already running ────────────────────────────────────
    if state.task_running.load(Ordering::SeqCst) {
        warn!("start_task: rejected — a task is already running");
        return Err("A task is already running. Cancel it first.".to_string());
    }

    // ── Validate task text ────────────────────────────────────────
    if task.trim().is_empty() {
        return Err("Task text must not be empty.".to_string());
    }

    // ── Validate API key & determine provider ─────────────────────
    let provider_type = {
        let guard = state.llm_provider_type.lock().map_err(|e| e.to_string())?;
        guard.clone()
    };

    let (api_key, model_name) = match provider_type {
        LlmProviderType::OpenAi => {
            let key = {
                let guard = state.openai_api_key.lock().map_err(|e| e.to_string())?;
                match guard.clone() {
                    Some(k) if !k.trim().is_empty() => k,
                    _ => {
                        warn!("start_task: rejected — OpenAI API key not set");
                        return Err("OpenAI API key not set. Use set_openai_api_key first.".to_string());
                    }
                }
            };
            (Some(key), "gpt-4o".to_string())
        }
        LlmProviderType::Anthropic => {
            let key = {
                let guard = state.anthropic_api_key.lock().map_err(|e| e.to_string())?;
                match guard.clone() {
                    Some(k) if !k.trim().is_empty() => k,
                    _ => {
                        warn!("start_task: rejected — Anthropic API key not set");
                        return Err("Anthropic API key not set. Use set_anthropic_api_key first.".to_string());
                    }
                }
            };
            (Some(key), "claude-sonnet-4-20250514".to_string())
        }
        LlmProviderType::Local => {
            info!("start_task: using local LLM provider (desktop mock)");
            (None, "local-gemma4".to_string())
        }
    };

    info!("start_task: provider={:?}, model={}", provider_type, model_name);

    // ── Insert task record for persistence ─────────────────────────
    let task_db_id = {
        let db = state.db.lock().map_err(|e| format!("DB lock error: {}", e))?;
        match db.insert_task(&task, &model_name) {
            Ok(id) => {
                info!("start_task: inserted task record id={}", id);
                Some(id)
            }
            Err(e) => {
                error!("start_task: failed to insert task record: {}", e);
                None
            }
        }
    };

    // ── Clone shared state Arcs for the spawned task ──────────────
    let cancel_flag = state.running_task_cancel.clone();
    let task_running = state.task_running.clone();
    let db_arc = state.db.clone();

    // ── Reset cancel flag and mark running ────────────────────────
    cancel_flag.store(false, Ordering::SeqCst);
    task_running.store(true, Ordering::SeqCst);

    // ── Route through 3-tier pipeline ─────────────────────────────
    let skill_registry = SkillRegistry::with_builtins();
    let route = PipelineRouter::route(&task, &skill_registry);
    info!("start_task: routed to {:?}", route);

    match route {
        // ══════════════════════════════════════════════════════════
        // Tier 1: DirectTool — synchronous, no LLM call
        // ══════════════════════════════════════════════════════════
        Route::DirectTool {
            tool_name,
            params,
            description,
        } => {
            info!(
                "start_task: DirectTool executing '{}' — {}",
                tool_name, description
            );

            let executor = crate::agent::tool_executor::create_executor(&app);
            let emitter = ChannelEventEmitter::new(on_event);

            // Emit events for frontend visibility
            emitter.emit(TaskEvent::LoopStart { round: 1 });
            emitter.emit(TaskEvent::ToolAction {
                tool_name: tool_name.clone(),
            });

            let params_value =
                serde_json::Value::Object(params.into_iter().collect());
            let result = executor.execute(&tool_name, params_value);

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
                tool_name: tool_name.clone(),
                success: result.success,
                detail: detail.clone(),
            });

            let answer = if result.success {
                format!("{}: {}", description, detail)
            } else {
                format!("{} failed: {}", description, detail)
            };

            info!("start_task: DirectTool completed — success={}", result.success);
            emitter.emit(TaskEvent::Completed {
                answer: answer.clone(),
                model_name: "direct".to_string(),
            });

            // Persist completion
            if let Some(tid) = task_db_id {
                if let Ok(db_guard) = db_arc.lock() {
                    let completed_at = {
                        let now = std::time::SystemTime::now();
                        let d = now.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                        let secs = d.as_secs();
                        let tod = secs % 86400;
                        let h = tod / 3600;
                        let m = (tod % 3600) / 60;
                        let s = tod % 60;
                        let days = secs / 86400;
                        let (y, mo, dy) = days_to_ymd_local(days);
                        format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", y, mo, dy, h, m, s)
                    };
                    if let Err(e) = db_guard.update_task_status(
                        tid,
                        if result.success { "completed" } else { "failed" },
                        Some(&answer),
                        if result.success { None } else { Some(&detail) },
                        1,
                        0,
                        0.0,
                        1,
                        Some(&completed_at),
                    ) {
                        error!("start_task: failed to persist DirectTool result: {}", e);
                    }
                }
            }

            // Persist tool event
            if let Some(tid) = task_db_id {
                if let Ok(db_guard) = db_arc.lock() {
                    if let Err(e) = db_guard.insert_task_event(
                        tid,
                        "toolResult",
                        Some(&serde_json::json!({
                            "tool_name": tool_name,
                            "success": result.success,
                            "detail": detail,
                            "route": "DirectTool"
                        }).to_string()),
                    ) {
                        error!("start_task: failed to persist tool event: {}", e);
                    }
                }
            }

            // Clear running flag
            task_running.store(false, Ordering::SeqCst);
            Ok(())
        }

        // ══════════════════════════════════════════════════════════
        // Tier 1.5: Skill — sequential step execution, may fall back to agent loop
        // ══════════════════════════════════════════════════════════
        Route::Skill {
            skill_id,
            description,
        } => {
            info!(
                "start_task: Skill '{}' matched — {}",
                skill_id, description
            );

            let skill = skill_registry
                .find_by_id(&skill_id)
                .expect("skill matched by router but not found in registry")
                .clone();

            let skill_task_id = skill_id.clone();
            let _skill_description = description.clone();

            tokio::spawn(async move {
                let executor = crate::agent::tool_executor::create_executor(&app);
                let emitter = ChannelEventEmitter::new(on_event);

                // Emit LoopStart for frontend visibility
                emitter.emit(TaskEvent::LoopStart { round: 1 });

                info!("start_task: executing skill '{}' with {} steps", skill.id, skill.steps.len());

                // Emit progress for each step
                for (i, step) in skill.steps.iter().enumerate() {
                    emitter.emit(TaskEvent::Progress {
                        step: (i + 1) as u32,
                        description: step.description.clone(),
                    });
                }

                let result = SkillExecutor::execute_skill(&skill, &executor, &cancel_flag);

                match result {
                    crate::agent::skill::executor::SkillResult::Completed { answer } => {
                        info!("start_task: skill '{}' completed successfully", skill_task_id);
                        emitter.emit(TaskEvent::Completed {
                            answer: answer.clone(),
                            model_name: "skill".to_string(),
                        });

                        // Persist completion
                        if let (Some(tid), Some(ref db)) = (task_db_id, Some(&db_arc)) {
                            if let Ok(db_guard) = db.lock() {
                                let completed_at = {
                                    let now = std::time::SystemTime::now();
                                    let d = now.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                                    let secs = d.as_secs();
                                    let tod = secs % 86400;
                                    let h = tod / 3600;
                                    let m = (tod % 3600) / 60;
                                    let s = tod % 60;
                                    let days = secs / 86400;
                                    let (y, mo, dy) = days_to_ymd_local(days);
                                    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", y, mo, dy, h, m, s)
                                };
                                if let Err(e) = db_guard.update_task_status(
                                    tid,
                                    "completed",
                                    Some(&answer),
                                    None,
                                    1,
                                    0,
                                    0.0,
                                    skill.steps.len() as i64,
                                    Some(&completed_at),
                                ) {
                                    error!("start_task: failed to persist skill completion: {}", e);
                                }
                            }
                        }

                        task_running.store(false, Ordering::SeqCst);
                    }
                    crate::agent::skill::executor::SkillResult::Failed {
                        error: skill_error,
                        fallback_goal,
                    } => {
                        warn!(
                            "start_task: skill '{}' failed: '{}', falling back to agent loop with task='{}'",
                            skill_task_id, skill_error, fallback_goal
                        );

                        emitter.emit(TaskEvent::Progress {
                            step: 0,
                            description: format!(
                                "Skill '{}' failed, switching to agent loop: {}",
                                skill_task_id, skill_error
                            ),
                        });

                        // Fall through to full agent loop with fallback_goal
                        let app_clone = app.clone();
                        let provider: Box<dyn LlmProvider> = match provider_type {
                            LlmProviderType::OpenAi => {
                                let key = api_key.as_ref().expect("OpenAI key validated above");
                                Box::new(OpenAiProvider::new(key.clone(), model_name.clone()))
                            }
                            LlmProviderType::Anthropic => {
                                let key = api_key.as_ref().expect("Anthropic key validated above");
                                Box::new(AnthropicProvider::new(key.clone(), model_name.clone()))
                            }
                            LlmProviderType::Local => {
                                Box::new(LocalProvider::with_fn(Arc::new(move |prompt| {
                                    let state = app_clone.state::<InferenceState>();
                                    tauri_plugin_pokeclaw::do_send_message(&state, prompt)
                                })))
                            }
                        };
                        let agent_executor = crate::agent::tool_executor::create_executor(&app);
                        let registry = ToolRegistry::default();
                        let config = AgentConfig::default();
                        let agent_emitter = ChannelEventEmitterWrapper { inner: emitter };

                        let agent_result = run_agent_loop(
                            fallback_goal,
                            provider,
                            Box::new(agent_executor),
                            registry,
                            Box::new(agent_emitter),
                            cancel_flag,
                            config,
                            Some(db_arc),
                            task_db_id,
                            None, // guards not active for skill fallback
                        )
                        .await;

                        if let Err(ref e) = agent_result {
                            error!("start_task: agent loop fallback failed — {}", e);
                        } else {
                            info!("start_task: agent loop fallback completed successfully");
                        }

                        task_running.store(false, Ordering::SeqCst);
                    }
                }
            });

            Ok(())
        }

        // ══════════════════════════════════════════════════════════
        // Tier 3: AgentLoop — existing behavior
        // ══════════════════════════════════════════════════════════
        Route::AgentLoop { task: agent_task } => {
            info!("start_task: AgentLoop — spawning full agent loop");

            let app_clone = app.clone();
            let provider: Box<dyn LlmProvider> = match provider_type {
                LlmProviderType::OpenAi => {
                    let key = api_key.as_ref().expect("OpenAI key validated above");
                    Box::new(OpenAiProvider::new(key.clone(), model_name.clone()))
                }
                LlmProviderType::Anthropic => {
                    let key = api_key.as_ref().expect("Anthropic key validated above");
                    Box::new(AnthropicProvider::new(key.clone(), model_name.clone()))
                }
                LlmProviderType::Local => {
                    Box::new(LocalProvider::with_fn(Arc::new(move |prompt| {
                        let state = app_clone.state::<InferenceState>();
                        tauri_plugin_pokeclaw::do_send_message(&state, prompt)
                    })))
                }
            };

            tokio::spawn(async move {
                let executor = crate::agent::tool_executor::create_executor(&app);
                let registry = ToolRegistry::default();
                let config = AgentConfig::default();
                let emitter = ChannelEventEmitter::new(on_event);

                // Create guard registry for this task
                let guards = Some(GuardRegistry::from_task(&agent_task));
                if let Some(ref g) = guards {
                    if g.has_active_guards() {
                        info!("start_task: AgentLoop — guards activated for task");
                    }
                }

                let result = run_agent_loop(
                    agent_task,
                    provider,
                    Box::new(executor),
                    registry,
                    Box::new(emitter),
                    cancel_flag,
                    config,
                    Some(db_arc),
                    task_db_id,
                    guards,
                )
                .await;

                if let Err(ref e) = result {
                    error!("start_task: agent loop failed — {}", e);
                } else {
                    info!("start_task: agent loop completed successfully");
                }

                // ── Clear running flag in finally-equivalent block ────────
                task_running.store(false, Ordering::SeqCst);
            });

            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// cancel_task command — sets the shared cancel flag
// ---------------------------------------------------------------------------

/// Cancel the currently running agent task. Rejects if no task is running.
#[tauri::command]
pub fn cancel_task(state: State<'_, AgentState>) -> Result<(), String> {
    info!("cancel_task: request received");

    if !state.task_running.load(Ordering::SeqCst) {
        warn!("cancel_task: rejected — no task is running");
        return Err("No task is currently running.".to_string());
    }

    state
        .running_task_cancel
        .store(true, Ordering::SeqCst);
    info!("cancel_task: cancel flag set");

    Ok(())
}

// ---------------------------------------------------------------------------
// Chat persistence commands
// ---------------------------------------------------------------------------

/// Persist a chat message to the database. Returns the inserted row ID.
#[tauri::command]
pub fn save_chat_message(
    db: State<'_, Arc<std::sync::Mutex<Database>>>,
    session_id: String,
    role: String,
    content: String,
    metadata: Option<String>,
) -> Result<i64, String> {
    info!(
        "save_chat_message: session_id='{}', role='{}', content_len={}",
        session_id,
        role,
        content.len()
    );
    let db = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    let id = db.insert_chat_message(&session_id, &role, &content, metadata.as_deref())?;
    info!("save_chat_message: inserted row id={}", id);
    Ok(id)
}

/// Load all chat messages for a session, ordered by created_at ascending.
#[tauri::command]
pub fn load_chat_history(
    db: State<'_, Arc<std::sync::Mutex<Database>>>,
    session_id: String,
) -> Result<Vec<ChatMessageRecord>, String> {
    info!("load_chat_history: session_id='{}'", session_id);
    let db = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    let messages = db.list_chat_messages(&session_id)?;
    info!("load_chat_history: returning {} messages", messages.len());
    Ok(messages)
}

// ---------------------------------------------------------------------------
// Task history persistence commands
// ---------------------------------------------------------------------------

/// Load recent task records, ordered by created_at descending.
#[tauri::command]
pub fn load_task_history(
    state: State<'_, AgentState>,
    limit: Option<u32>,
) -> Result<Vec<TaskRecord>, String> {
    let limit = limit.unwrap_or(50);
    info!("load_task_history: limit={}", limit);
    let db = state.db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    let tasks = db.list_tasks(limit as i64)?;
    info!("load_task_history: returning {} tasks", tasks.len());
    Ok(tasks)
}

/// Load all events for a specific task, ordered by created_at ascending.
#[tauri::command]
pub fn load_task_events(
    state: State<'_, AgentState>,
    task_id: i64,
) -> Result<Vec<TaskEventRecord>, String> {
    info!("load_task_events: task_id={}", task_id);
    let db = state.db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    let events = db.list_task_events(task_id)?;
    info!("load_task_events: returning {} events for task_id={}", events.len(), task_id);
    Ok(events)
}
