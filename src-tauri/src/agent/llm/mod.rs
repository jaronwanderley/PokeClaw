// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

//! LLM provider abstraction layer.
//!
//! Re-exports the core types from `llm_provider` and adds provider-specific
//! implementations (OpenAI, Anthropic, Local) behind the `LlmProvider` trait.

pub mod llm_provider;
pub mod anthropic;

// Re-export the core types for convenience so consumers can `use crate::agent::llm::LlmProvider`.
pub use llm_provider::{
    ChatMessage, LlmProvider, LlmError, LlmResponse, OpenAiProvider, TokenUsage, ToolCall,
};
