# PokeClaw Architecture

## Module Dependency Graph

```
┌─────────────────────────────────────────────────────────────────┐
│                         main.rs                                  │
│                     (app entry point)                            │
└──────────┬──────────────────────────────────────────────────────┘
           │
           ▼
┌─────────────────────────────────────────────────────────────────┐
│                          lib.rs                                  │
│  AgentState { openai_api_key, anthropic_api_key,                │
│               llm_provider_type, cancel, running, db }          │
│  LlmProviderType { OpenAi, Anthropic, Local }                   │
│  Tauri plugin setup + invoke_handler registration               │
└──────┬───────────────────────────────────────────┬──────────────┘
       │                                           │
       ▼                                           ▼
┌──────────────────┐                    ┌─────────────────────┐
│   commands.rs    │                    │       db/            │
│  (Tauri cmds)    │◄──────────────────│  mod.rs              │
│  - start_task    │                    │  chat.rs             │
│  - cancel_task   │                    │  tasks.rs            │
│  - set_*_api_key │                    │  state.rs            │
│  - save/load_*   │                    │  migrations.rs       │
└──────┬───────────┘                    └─────────────────────┘
       │
       ▼
┌──────────────────────────────────────────────────────────────┐
│                        agent/                                 │
│  mod.rs — module re-exports                                   │
├──────────┬──────────┬───────────┬──────────┬─────────────────┤
│ config   │ pipeline │  skill/   │ guards/  │    llm/          │
│          │          │           │          │                  │
│ Agent    │ Pipeline  │ Skill     │ Guard    │ LlmProvider      │
│ Config   │ Router    │ Registry  │ Registry │ (trait)          │
│          │ Route     │ Skill     │ InApp    │ OpenAiProvider   │
│          │           │ Executor  │ Search   │ AnthropicProvider│
│          │           │ builtins  │ Email    │ LocalProvider    │
│          │           │           │ Compose  │                  │
├──────────┴──────────┴───────────┴──────────┴─────────────────┤
│  loop_runner.rs  ─── ReAct loop engine                         │
│  tool_registry.rs ── 28 tool definitions                       │
│  tool_executor.rs ── Desktop + Android tool execution          │
│  task_event.rs   ─── Event types (LoopStart, ToolAction, etc.)│
│  budget.rs       ─── Token/cost budget enforcement             │
│  token_monitor.rs ── Token usage tracking & cost estimation    │
│  model_pricing.rs ── Per-model pricing tables                  │
│  stuck_detector.rs ─ Stuck detection & recovery                │
│  context.rs      ─── Screen context management                 │
│  task_parser.rs  ─── Natural language task parsing              │
└──────────────────────────────────────────────────────────────┘
```

## Data Flow

### Task Execution Pipeline

```
User text
    │
    ▼
commands.rs::start_task()
    │
    ├─ Read LlmProviderType (OpenAi / Anthropic / Local)
    ├─ Validate API key for selected provider
    ├─ Insert task record into SQLite
    │
    ▼
pipeline::PipelineRouter::route(task, skill_registry)
    │
    ├─── Route::DirectTool ──────► Execute tool synchronously
    │                             Emit events → Persist result
    │
    ├─── Route::Skill ───────────► SkillExecutor::execute_skill()
    │                             ├── Success → Emit Completed
    │                             └── Failure → Fall back to AgentLoop
    │
    └─── Route::AgentLoop ───────► run_agent_loop()
                                  │
                                  ├── Create LLM provider (by type)
                                  ├── Create GuardRegistry (from task text)
                                  ├── Append guard prompt sections
                                  │
                                  └── ReAct Loop:
                                      ├── Check cancellation
                                      ├── Call LLM (provider.chat)
                                      ├── Token monitor update + budget check
                                      ├── Parse response (text / tool_calls)
                                      │
                                      ├── [Text-only] ── Guard check ──► Block? → inject correction
                                      │                            └─ Allow? → emit Completed
                                      │
                                      ├── [Tool calls] ── Execute each tool
                                      │   ├── record_successful_tool (guard)
                                      │   └── [finish tool] ── Guard check ──► Block? → inject reason
                                      │                                             └─ Allow? → emit Completed
                                      │
                                      └── Stuck detection → RecoveryLevel { Hint, StrategySwitch, AutoKill }
```

### Event Streaming

```
run_agent_loop  ──► EventEmitter trait  ──► ChannelEventEmitter  ──► Tauri IPC Channel  ──► Frontend
                   (abstraction)           (production impl)         (TypeScript)
```

### Persistence

```
Agent Loop Events ──► SQLite (task_events table)
Task Completion  ──► SQLite (tasks table: status, answer, tokens, cost)
Chat Messages    ──► SQLite (chat_messages table: session_id, role, content)
Session Totals   ──► SQLite (agent_state table: session_total_tokens, session_total_cost)
```

## File Listing

| File | Responsibility | Key Types |
|------|---------------|-----------|
| `src-tauri/src/main.rs` | App entry point | — |
| `src-tauri/src/lib.rs` | Tauri setup, state management | `AgentState`, `LlmProviderType` |
| `src-tauri/src/commands.rs` | Tauri IPC commands | `start_task`, `cancel_task`, provider setters |
| `src-tauri/src/agent/mod.rs` | Module re-exports | — |
| `src-tauri/src/agent/config.rs` | Agent configuration | `AgentConfig`, `LOCAL_TASK_PROMPT` |
| `src-tauri/src/agent/loop_runner.rs` | ReAct loop engine | `run_agent_loop()`, `EventEmitter` |
| `src-tauri/src/agent/llm/mod.rs` | LLM module root | Re-exports: `LlmProvider`, `ChatMessage`, etc. |
| `src-tauri/src/agent/llm/llm_provider.rs` | OpenAI provider + shared types | `OpenAiProvider`, `ChatMessage`, `LlmResponse`, `ToolCall`, `TokenUsage`, `LlmError` |
| `src-tauri/src/agent/llm/anthropic.rs` | Anthropic Messages API provider | `AnthropicProvider` |
| `src-tauri/src/agent/llm/local.rs` | Local LLM (Gemma 4 tool call parser) | `LocalProvider`, `parse_gemma4_native_call()` |
| `src-tauri/src/agent/tool_registry.rs` | Tool definitions & schema | `ToolRegistry`, `ToolSpec`, `ToolParam` |
| `src-tauri/src/agent/tool_executor.rs` | Tool execution (desktop/Android) | `DesktopToolExecutor`, `ToolCallResult`, `AgentRoundResult` |
| `src-tauri/src/agent/task_event.rs` | Event types for streaming | `TaskEvent` enum |
| `src-tauri/src/agent/budget.rs` | Token/cost budget enforcement | `TaskBudget`, `Status` |
| `src-tauri/src/agent/token_monitor.rs` | Token tracking & cost estimation | `TokenMonitor`, `TokenStatus` |
| `src-tauri/src/agent/model_pricing.rs` | Per-model pricing tables | `ModelPricing` |
| `src-tauri/src/agent/stuck_detector.rs` | Stuck detection & recovery | `StuckDetector`, `RecoveryLevel` |
| `src-tauri/src/agent/context.rs` | Screen context management | — |
| `src-tauri/src/agent/task_parser.rs` | Natural language task parsing | — |
| `src-tauri/src/agent/pipeline.rs` | 3-tier pipeline router | `PipelineRouter`, `Route` |
| `src-tauri/src/agent/skill/mod.rs` | Skill module root | — |
| `src-tauri/src/agent/skill/registry.rs` | Skill definitions | `SkillRegistry`, `Skill` |
| `src-tauri/src/agent/skill/executor.rs` | Skill step executor | `SkillExecutor`, `SkillResult` |
| `src-tauri/src/agent/skill/builtins.rs` | Built-in skill definitions | — |
| `src-tauri/src/agent/guards/mod.rs` | Guard registry | `GuardRegistry` |
| `src-tauri/src/agent/guards/in_app_search.rs` | In-app search guard | `InAppSearchGuard` |
| `src-tauri/src/agent/guards/email_compose.rs` | Email compose guard | `EmailComposeGuard` |
| `src-tauri/src/db/mod.rs` | Database module root | `Database` |
| `src-tauri/src/db/chat.rs` | Chat message persistence | `ChatMessageRecord` |
| `src-tauri/src/db/tasks.rs` | Task & event persistence | `TaskRecord`, `TaskEventRecord` |
| `src-tauri/src/db/state.rs` | Key-value state store | — |
| `src-tauri/src/db/migrations.rs` | Schema migrations | — |

## Key Design Decisions

1. **Provider abstraction** — `LlmProvider` trait allows swapping between OpenAI, Anthropic, and Local providers without changing the agent loop
2. **EventEmitter trait** — Decouples event delivery from Tauri's `Channel` type, enabling unit testing of the loop
3. **3-tier pipeline** — DirectTool (fast, no LLM) → Skill (multi-step, no LLM) → AgentLoop (full ReAct)
4. **Guard pattern** — Narrow-scope monitors that prevent premature task completion by injecting prompt sections and blocking finish calls
5. **Tool registry** — Declarative tool specs with JSON Schema parameters, supporting both desktop (mock) and mobile (real) execution
6. **Stuck detection** — Monitors repeated tool/action patterns and applies escalating recovery (hint → strategy switch → auto-kill)
