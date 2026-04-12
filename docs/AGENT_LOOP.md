# PokeClaw Agent Loop Design

## Overview

The agent loop implements a **ReAct** (Reasoning + Acting) pattern: the LLM reasons about what to do, calls tools to act on the device, observes results, and iterates until the task is complete.

## State Machine

```
                    ┌─────────┐
                    │  START  │
                    └────┬────┘
                         │
                         ▼
                    ┌─────────┐
           ┌───────│  ROUND  │◄──────────────────┐
           │       └────┬────┘                    │
           │            │                         │
           │            ▼                         │
           │    ┌───────────────┐                 │
           │    │  Check Cancel │──── Yes ────► CANCELLED
           │    └───────┬───────┘
           │            │ No
           │            ▼
           │    ┌───────────────┐
           │    │   Call LLM    │──── Error ──► FAILED
           │    └───────┬───────┘
           │            │ OK
           │            ▼
           │    ┌───────────────────┐
           │    │  Token Monitor +  │
           │    │  Budget Check     │──── Hard Limit ──► COMPLETED (budget msg)
           │    └───────┬───────────┘
           │            │ OK / Soft Warning
           │            ▼
           │    ┌───────────────────┐
           │    │  Parse Response   │
           │    └──┬────────────┬───┘
           │       │            │
           │  Text-only      Tool Calls
           │       │            │
           │       ▼            ▼
           │  ┌──────────┐  ┌──────────────────┐
           │  │ Guard    │  │ Execute Tools    │
           │  │ Check    │  │ ┌──────────────┐ │
           │  └──┬───┬───┘  │ │ Record Tool  │ │
           │     │   │      │ │ (guards)     │ │
           │  Block  Allow  │ └──────┬───────┘ │
           │     │   │      │        │         │
           │     │   ▼      │   ┌───────────┐  │
           │     │ COMPLETED│   │ finish    │  │
           │     │          │   │ called?   │  │
           │     │          │   └──┬────┬───┘  │
           │     │          │      │    │      │
           │     │          │   Guard  Allow   │
           │     │          │   Block  │      │
           │     │          │      │   COMPLETED
           │     │          │      │           │
           │     ▼          │      ▼           │
           │  Inject        │  Inject reason   │
           │  Correction    │                  │
           │     │          │                  │
           └─────┼──────────┼──────────────────┘
                 │          │
                 ▼          ▼
           ┌──────────────────┐
           │  Stuck Detection │──── AutoKill ──► COMPLETED (stuck msg)
           └────────┬─────────┘
                    │ Hint / StrategySwitch
                    │ (inject recovery hint)
                    ▼
                 ┌──────┐
                 │ NEXT │ ─── Max iterations? ──► FAILED (exhausted)
                 │ROUND │
                 └──────┘
```

## Event Types

All events emitted by the agent loop via the `EventEmitter` trait:

| Event | When | Data |
|-------|------|------|
| `LoopStart { round }` | Start of each round | Round number (1-based) |
| `TokenUpdate { step, formatted_tokens, formatted_cost }` | After each LLM response | Token count and cost |
| `Thinking { content }` | LLM returns text content | The text the LLM "thought" |
| `ToolAction { tool_name }` | About to execute a tool | Tool name |
| `ToolResult { tool_name, success, detail }` | After tool execution | Tool name, success/failure, result detail |
| `Completed { answer, model_name }` | Task finished normally | Final answer + model used |
| `Cancelled` | Cancel flag detected | — |
| `Failed { error }` | Unrecoverable error | Error message |
| `Progress { step, description }` | Skill step progress | Step number + description |

## Guard Integration Points

Guards are wired into the agent loop at **4 integration points**:

### 1. Task Start — Prompt Injection
```
GuardRegistry::from_task(task_text)
→ Activates matching guards (InAppSearch, EmailCompose)
→ build_prompt_sections() appended to system prompt
```
Logged at info level: `"GuardRegistry: activated guards for task '...' — search=true, email=false"`

### 2. After Tool Execution — Progress Tracking
```
registry.record_successful_tool(name, params)
→ Updates guard internal state (e.g., typed_query=true for InAppSearch)
```
Logged implicitly via guard state transitions.

### 3. Before Text-Only Completion — Blocking Check
```
if should_block_text_only_completion():
    correction = build_completion_correction()
    → Inject correction as User message
    → Continue loop (don't allow completion)
```
Logged at warn level: `"run_agent_loop: guard blocking text-only completion at round N — ..."`

### 4. Before Finish Tool — Block Check
```
if maybe_block_finish(screen_info):
    → Inject block reason as User message
    → Continue loop (don't allow finish)
```
Logged at warn level: `"run_agent_loop: guard blocking finish at round N — ..."`

## Budget & Stuck Detection

### Token Budget
- **Hard limit**: Task terminates with budget message when `total_tokens ≥ max_tokens` or `total_cost ≥ max_cost_usd`
- **Soft limit**: Warning injected as User message at `soft_limit_percent` (default 80%)
- **Config**: `AgentConfig { max_tokens: 250_000, max_cost_usd: 1.00, soft_limit_percent: 0.80 }`

### Stuck Detection
Monitors repeated actions and screen states:

| Level | Behavior |
|-------|----------|
| `Hint` | Inject recovery hint as User message |
| `StrategySwitch` | Inject stronger hint suggesting different approach |
| `AutoKill` | Terminate task with stuck message |

## Provider Switching

The `start_task` command selects an LLM provider based on `LlmProviderType`:

| Provider | Auth | Model | Wire Format |
|----------|------|-------|-------------|
| `OpenAi` | `set_openai_api_key` | `gpt-4o` | async-openai crate |
| `Anthropic` | `set_anthropic_api_key` | `claude-sonnet-4-20250514` | reqwest (custom HTTP) |
| `Local` | None | `local-gemma4` (mock) | Closure-based IPC |

Provider selection is logged at info level in `start_task`:
```
start_task: provider=Anthropic, model=claude-sonnet-4-20250514
```

Tool call parsing failures are logged at warn level with raw text context:
```
run_agent_loop: failed to parse tool args '...': <error>
```

## 3-Tier Pipeline

The `PipelineRouter` determines the execution tier before entering the agent loop:

| Tier | Condition | Behavior |
|------|-----------|----------|
| `DirectTool` | Exact tool pattern match | Execute tool synchronously, no LLM call |
| `Skill` | Skill ID match | Execute predefined steps, fallback to AgentLoop on failure |
| `AgentLoop` | Default | Full ReAct loop with LLM reasoning |

## Message History

The conversation grows each round:
1. `System(system_prompt + guard_sections)` — once at start
2. `User(task)` — once at start
3. `AssistantWithTools(text, tool_calls)` or `Assistant(text)` — each round
4. `ToolResult(tool_call_id, content)` — for each tool call
5. `User(budget_warning | guard_correction | stuck_hint)` — injected as needed

## Testability

- `EventEmitter` trait enables test doubles (`VecEmitter` collects events into a Vec)
- `LlmProvider` trait enables mock providers (`MockProvider` returns canned responses)
- `ToolExecutor` trait enables desktop execution without Android device
- All tests in `loop_runner.rs::tests` run without network or device access
