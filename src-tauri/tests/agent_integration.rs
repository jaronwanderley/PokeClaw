// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use app_lib::agent::config::AgentConfig;
use app_lib::agent::llm::local::LocalProvider;
use app_lib::agent::loop_runner::{run_agent_loop, EventEmitter};
use app_lib::agent::task_event::TaskEvent;
use app_lib::agent::tool_executor::DesktopToolExecutor;
use app_lib::agent::tool_registry::ToolRegistry;

/// Simple emitter that collects events in a Vec for verification.
struct VecEmitter {
    events: Mutex<Vec<TaskEvent>>,
}

impl VecEmitter {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }

    fn get_events(&self) -> Vec<TaskEvent> {
        self.events.lock().unwrap().clone()
    }
}

impl EventEmitter for VecEmitter {
    fn emit(&self, event: TaskEvent) -> bool {
        self.events.lock().unwrap().push(event);
        true
    }
}

/// Wrapper to satisfy trait bounds without violating orphan rules.
struct EmitterWrapper {
    inner: Arc<VecEmitter>,
}

impl EventEmitter for EmitterWrapper {
    fn emit(&self, event: TaskEvent) -> bool {
        self.inner.emit(event)
    }
}

#[tokio::test]
async fn test_e2e_local_agent_integration() {
    // 1. Setup a mock local provider that simulates a conversation.
    // Round 1: Thinking + Tool Call (tap)
    // Round 2: Thinking + Tool Call (finish)
    let call_count = Arc::new(AtomicU32::new(0));
    let provider = LocalProvider::with_fn(Arc::new(move |_prompt| {
        let count = call_count.fetch_add(1, Ordering::SeqCst);
        if count == 0 {
            Ok("I will tap the button at (100, 200). __{ \"name\": \"tap\", \"arguments\": { \"x\": 100, \"y\": 200 } }__".to_string())
        } else if count == 1 {
            Ok("The button was tapped successfully. __{ \"name\": \"finish\", \"arguments\": { \"result\": \"Task completed!\" } }__".to_string())
        } else {
            Ok("Unexpected round".to_string())
        }
    }));

    // 2. Initialize real (desktop) components
    let executor = DesktopToolExecutor::new();
    let registry = ToolRegistry::default();
    let emitter = Arc::new(VecEmitter::new());
    let cancel = Arc::new(AtomicBool::new(false));
    let config = AgentConfig {
        model_name: "local-gemma4".to_string(),
        max_iterations: 5,
        system_prompt: "You are a helpful phone assistant.".to_string(),
        max_tokens: 1000,
        max_cost_usd: 1.0,
        soft_limit_percent: 0.8,
    };

    // 3. Run the agent loop
    let result = run_agent_loop(
        "Tap the button".to_string(),
        Box::new(provider),
        Box::new(executor),
        registry,
        Box::new(EmitterWrapper { inner: emitter.clone() }),
        cancel,
        config,
        None, // No database persistence for this integration test
        None,
        None, // No guards for this simple test
    )
    .await;

    // 4. Verify the loop completed successfully
    assert!(result.is_ok(), "Agent loop should succeed, but got: {:?}", result.err());

    // 5. Verify the sequence of events emitted
    let events = emitter.get_events();

    // Verify LoopStart events
    assert!(events.iter().any(|e| matches!(e, TaskEvent::LoopStart { round: 1 })), "Missing LoopStart round 1");
    assert!(events.iter().any(|e| matches!(e, TaskEvent::LoopStart { round: 2 })), "Missing LoopStart round 2");

    // Verify Thinking events
    assert!(events.iter().any(|e| matches!(e, TaskEvent::Thinking { ref content } if content.contains("tap the button"))), "Missing Thinking for round 1");
    assert!(events.iter().any(|e| matches!(e, TaskEvent::Thinking { ref content } if content.contains("tapped successfully"))), "Missing Thinking for round 2");

    // Verify ToolAction events
    assert!(events.iter().any(|e| matches!(e, TaskEvent::ToolAction { ref tool_name } if tool_name == "tap")), "Missing ToolAction: tap");
    assert!(events.iter().any(|e| matches!(e, TaskEvent::ToolAction { ref tool_name } if tool_name == "finish")), "Missing ToolAction: finish");

    // Verify ToolResult events
    assert!(events.iter().any(|e| matches!(e, TaskEvent::ToolResult { ref tool_name, success: true, .. } if tool_name == "tap")), "Missing ToolResult: tap");
    assert!(events.iter().any(|e| matches!(e, TaskEvent::ToolResult { ref tool_name, success: true, .. } if tool_name == "finish")), "Missing ToolResult: finish");

    // Verify final Completion
    assert!(events.iter().any(|e| matches!(e, TaskEvent::Completed { ref answer, .. } if answer == "Task completed!")), "Missing or incorrect final Completed event");

    println!("E2E Local Agent Integration Test PASSED");
}
