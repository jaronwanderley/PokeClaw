// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

use std::time::Instant;

use log::{info, warn, error};
use serde::Deserialize;
use tauri::State;

use crate::agent::llm_provider::{ChatMessage, LlmProvider, OpenAiProvider};
use crate::agent::tool_executor::{
    AgentRoundResult, DesktopToolExecutor, TokenUsage, ToolCallResult, ToolExecutor,
};
use crate::agent::tool_registry::ToolRegistry;
use crate::AgentState;

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
