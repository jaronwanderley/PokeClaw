// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

use std::sync::atomic::Ordering;
use std::time::Instant;

use log::{error, info, warn};
use serde::Deserialize;
use tauri::ipc::Channel;
use tauri::State;
use std::sync::Arc;

use crate::agent::config::AgentConfig;
use crate::agent::llm_provider::{ChatMessage, LlmProvider, OpenAiProvider};
use crate::agent::loop_runner::{run_agent_loop, EventEmitter};
use crate::agent::task_event::TaskEvent;
use crate::agent::tool_executor::{
    AgentRoundResult, DesktopToolExecutor, TokenUsage, ToolCallResult, ToolExecutor,
};
use crate::agent::tool_registry::ToolRegistry;
use crate::db::chat::ChatMessageRecord;
use crate::db::tasks::{TaskEventRecord, TaskRecord};
use crate::db::Database;
use crate::AgentState;

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

        let executor = DesktopToolExecutor::new();
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
// start_task command — spawns the agent loop in a background tokio task
// ---------------------------------------------------------------------------

/// Start a multi-round agent task. Returns immediately; events stream via
/// the `on_event` Channel parameter. Rejects if a task is already running
/// or if the OpenAI API key has not been set.
#[tauri::command]
pub async fn start_task(
    state: State<'_, AgentState>,
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

    // ── Validate API key ──────────────────────────────────────────
    let api_key = {
        let guard = state.openai_api_key.lock().map_err(|e| e.to_string())?;
        match guard.clone() {
            Some(k) if !k.trim().is_empty() => k,
            _ => {
                warn!("start_task: rejected — OpenAI API key not set");
                return Err("OpenAI API key not set. Use set_openai_api_key first.".to_string());
            }
        }
    };

    // ── Insert task record for persistence ─────────────────────────
    let model_name = "gpt-4o".to_string();
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

    info!("start_task: spawning agent loop task");

    // ── Spawn the agent loop ──────────────────────────────────────
    tokio::spawn(async move {
        let provider = OpenAiProvider::new(api_key, model_name.clone());
        let executor = DesktopToolExecutor::new();
        let registry = ToolRegistry::default();
        let config = AgentConfig::default();
        let emitter = ChannelEventEmitter::new(on_event);

        let result = run_agent_loop(
            task,
            Box::new(provider),
            Box::new(executor),
            registry,
            Box::new(emitter),
            cancel_flag,
            config,
            Some(db_arc),
            task_db_id,
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
