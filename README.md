<p align="center">
  <img src="banna.png" width="600" />
</p>
<p align="center">
  <img src="option.png" width="600" />
</p>

<p align="center">
  <a href="https://github.com/agents-io/PitoAgent/stargazers"><img src="https://img.shields.io/github/stars/agents-io/PitoAgent?style=social" alt="Stars" /></a>
  <a href="https://github.com/agents-io/PitoAgent/network/members"><img src="https://img.shields.io/github/forks/agents-io/PitoAgent?style=social" alt="Forks" /></a>
  <img src="https://img.shields.io/badge/Android-9%2B-3DDC84?logo=android&logoColor=white" alt="Android 9+" />
  <img src="https://img.shields.io/badge/license-Apache%202.0-blue" alt="License" />
  <a href="https://github.com/agents-io/PitoAgent/releases/latest"><img src="https://img.shields.io/github/v/release/agents-io/PitoAgent" alt="Latest Release" /></a>
</p>

<p align="center">
  🌐 <a href="https://agents-io.github.io/PitoAgent/">Landing Page</a> — available in English · हिन्दी · 日本語 · Deutsch · 繁中
</p>

# PitoAgent — On-Device AI Phone Agent

**PitoAgent** turns an Android phone into an AI-operated device.

It runs Gemma 4 on-device for private phone control, and it also supports optional cloud models when you want stronger reasoning for harder tasks.

PitoAgent is the first working app built on Gemma 4 that can autonomously control an Android phone.

In Local mode, the entire loop stays inside your device. No account. No API key. No monthly bill.

```
Everyone else:  Phone → Internet → Cloud API → Internet → Phone
                       💳Credit card needed, API key required. Monthly bill attached.

PitoAgent local: Phone → LLM → Phone
                       That's it. No internet. No API key. No bill.
```

**AI controls your phone. And it never leaves your phone.**

PitoAgent is open-source, ships fast, and already handles real chat, task, and automation flows on Android.

Monitor a WhatsApp contact and auto-reply:


https://github.com/user-attachments/assets/4cb4c2bf-90e1-4391-8e08-9d6113634a41

Context-aware WhatsApp auto-reply:

https://github.com/user-attachments/assets/5a43d4d5-458a-4eea-a0a5-58d113255741

https://github.com/user-attachments/assets/5c2966c5-04e6-4b22-8d66-11915ae62096

> **☝️ Auto-reply demo:** PitoAgent monitors messages from Mom, reads what she said, and replies based on context using the on-device LLM. [Watch in higher resolution on YouTube](https://youtube.com/shorts/Vxpf474chm0)

> **☝️ Context demo:** Mom asks "what did I tell you to bring?" — the AI opens the chat, reads the full conversation on screen, sees the earlier message about wine, and replies correctly. This is the difference between context-aware and context-free replies.

https://github.com/user-attachments/assets/89999dd8-a1be-49ad-9419-60c2b38f6374


> **Why is the "hi" demo slow?** That clip was recorded on a CPU-only Android device with no usable GPU or NPU path. Running Gemma 4 E2B on pure CPU takes about 45 seconds to warm up. On stronger phones it is much faster:
> - **Google Tensor G3/G4** (Pixel 8, Pixel 9)
> - **Snapdragon 8 Gen 2/3** (Galaxy S24, OnePlus 12)
> - **Dimensity 9200/9300** (recent MediaTek flagships)
> - **Snapdragon 7+ Gen 2+** (mid-range with GPU)
>
> On these devices, warmup drops to seconds. Same model, better hardware.

## The Story

I'm building this solo. When Gemma 4 landed with native tool calling on LiteRT-LM, I wanted to know whether a phone could become a real on-device agent instead of just another chatbot. PitoAgent is the result.

The interesting part is not just chatting with a local model. The interesting part is getting a local model to read the screen, choose tools, operate apps, keep task state, and finish real phone workflows. That is exactly what this project is built for.

PitoAgent already supports fully on-device automation with Gemma 4 and optional cloud models for stronger task execution. The current focus is broader device support, more generic skills, more local model options, and a cleaner public release path.

**If you hit something interesting, [open an issue](https://github.com/agents-io/PitoAgent/issues).** Real device reports are how this gets better fast.

## See the UI

👉 **[Try the interactive demo on our landing page](https://agents-io.github.io/PitoAgent/)** — click through every screen without installing anything.

## What it does

The model picks the right tool, fills in the parameters, and executes. You don't configure anything per-app. It just reads the screen and acts.

## Architecture

PitoAgent uses a **Tauri v2 + Vue 3 + Rust + Kotlin** architecture with a cross-platform Rust agent loop at its core.

```
┌─────────────────────────────────────────────────────────┐
│                     Vue 3 Frontend                       │
│  Chat UI · TaskPanel · ModelPicker · Settings · Perms   │
│  (Composition API, module-level refs — no Pinia)         │
└────────────────────────┬────────────────────────────────┘
                         │ invoke() / Channel<T>
                         ▼
┌─────────────────────────────────────────────────────────┐
│                    Tauri v2 (Rust)                       │
│                                                          │
│  ┌──────────┐  ┌───────────┐  ┌──────────┐  ┌────────┐ │
│  │ Agent    │  │ Pipeline  │  │ LLM      │  │ SQLite │ │
│  │ Loop     │  │ Router    │  │ Provider │  │ Persist│ │
│  │ (ReAct)  │  │ (3-tier)  │  │ (x4)     │  │        │ │
│  └──────────┘  └───────────┘  └──────────┘  └────────┘ │
│  ┌──────────┐  ┌───────────┐  ┌──────────┐  ┌────────┐ │
│  │ 28-Tool  │  │ Guards    │  │ Budget / │  │ Skill  │ │
│  │ Registry │  │ (Search / │  │ Stuck    │  │ System │ │
│  │          │  │  Email)   │  │ Detect   │  │ (9)    │ │
│  └──────────┘  └───────────┘  └──────────┘  └────────┘ │
└────────────────────────┬────────────────────────────────┘
                         │ Tauri Plugin IPC
                         ▼
┌─────────────────────────────────────────────────────────┐
│              tauri-plugin-pokeclaw (Kotlin)              │
│                                                          │
│  AccessibilityService · NotificationListener             │
│  ForegroundService · LiteRT-LM Engine · ModelManager     │
│  28 @Command tool methods · Streaming Channels           │
└─────────────────────────────────────────────────────────┘
```

### Agent Loop

The Rust agent loop implements a **ReAct** (Reasoning + Acting) pattern:

1. User gives a task in natural language
2. The **3-tier pipeline router** dispatches it:
   - **Tier 1 — DirectTool**: Simple commands (screenshot, go back, open app) execute instantly with zero LLM calls
   - **Tier 1.5 — Skill**: 9 built-in skill workflows run deterministic step sequences
   - **Tier 3 — AgentLoop**: Complex tasks enter the full ReAct reasoning loop
3. The agent loop calls the LLM, parses tool calls, executes them, observes results, and iterates until the task is complete
4. All events stream to the Vue **TaskPanel** in real time via Tauri Channel
5. Everything is persisted in **SQLite** across sessions

### LLM Providers

| Provider | How | Notes |
|----------|-----|-------|
| **OpenAI** | async-openai crate | GPT-4o, GPT-4o-mini, etc. |
| **Anthropic** | reqwest HTTP (Messages API) | Claude Sonnet 4, etc. |
| **Google Gemini** | OpenAI-compatible endpoint | Via `with_base_url()` |
| **Local (Gemma 4)** | LiteRT-LM IPC via Kotlin plugin | Fully on-device, no internet |

### Tools by Platform

Each platform has its own set of tools — not the same tools with different backends, but tools that make sense for that OS. The agent only sees tools available on the current platform. A tool that doesn't exist on the platform is never registered, never shown to the LLM, and never callable.

**Android has taps, sensors, and scheduling. Desktop has clicks, keys, and notifications. iOS has sensors and deep links.**

#### Android (38 tools)

Full phone control via AccessibilityService + NotificationListener + hardware APIs.

| Category | Tools |
|----------|-------|
| **Screen** | `get_screen_info` `find_node_info` `take_screenshot` |
| **Touch** | `tap` `tap_node` `long_press` `swipe` `scroll_to_find` `find_and_tap` |
| **Input** | `input_text` `system_key` |
| **Apps** | `open_app` `get_installed_apps` |
| **Communication** | `send_message` `send_file` `auto_reply` `make_call` |
| **Device** | `get_device_info` `get_notifications` `clipboard` |
| **Hardware** | `toggle_flashlight` `toggle_bluetooth` `scan_bluetooth` `get_sensors` |
| **Alerts** | `show_notification` `vibrate` `play_sound` |
| **Scheduling** | `schedule_task` `set_timer` |
| **Knowledge Base** | `kb_write` `kb_read` `kb_search` `kb_append` `kb_add_todo` |
| **Utility** | `wait` `repeat_actions` `finish` |

- **`toggle_flashlight`** — turn flashlight on/off via CameraManager
- **`toggle_bluetooth`** — enable/disable Bluetooth via BluetoothAdapter
- **`scan_bluetooth`** — scan for nearby BLE devices, returns device name, MAC/address, RSSI signal strength, and estimated distance. `"duration_seconds": 5` → `[{"name": "Mom's Watch", "rssi": -55, "distance_m": 3.2, "type": "wearable"}, ...]`
- **`get_sensors`** — read accelerometer, gyroscope, proximity, light level, battery temp
- **`show_notification`** — post a system notification with title, body, optional action buttons, and sound. `"title": "Mom messaged", "body": "She asked about dinner", "actions": "Reply,Dismiss"`
- **`vibrate`** — vibrate with a pattern: `"short"` (200ms), `"long"` (1s), `"double"`, `"sos"`, or custom millis `"200,100,500"`
- **`play_sound`** — play a notification/alarm/ringtone sound, a beep tone, or **speak text aloud via TTS**. `"type": "tts", "text": "Mom just messaged you"` — the agent can talk to the user without them looking at the screen
- **`schedule_task`** — schedule a future task via AlarmManager/WorkManager that wakes the agent with a prompt (e.g. "remind Mom at 6pm" → agent fires at 18:00 and sends the message)
- **`set_timer`** — simple countdown timer, fires a notification when done

#### Desktop (planned — 34 tools)

Real OS automation via mouse/keyboard/OS APIs + system notifications + scheduling + audio.

| Category | Tools | Backend |
|----------|-------|---------|
| **Screen** | `get_screen_info` `find_node_info` `take_screenshot` | UI Automation / AT-SPI / xcap |
| **Mouse** | `click` `double_click` `right_click` `move_mouse` `drag` `scroll` | enigo / CGEvent / Win32 |
| **Keyboard** | `type_text` `press_key` `hotkey` | enigo / CGEvent / Win32 |
| **Apps** | `open_app` `get_installed_apps` `switch_app` | `std::process::Command` |
| **Communication** | `send_email` | open mailto: link |
| **Device** | `get_device_info` `clipboard` `get_sensors` | sysinfo / arboard |
| **Notifications** | `show_notification` | OS native (toast / NSUserNotification / libnotify) |
| **Bluetooth** | `scan_bluetooth` | btleplug (cross-platform BLE) |
| **Audio** | `play_sound` | rodio / cpal |
| **Scheduling** | `schedule_task` `set_timer` | cron / Task Scheduler / launchd |
| **Knowledge Base** | `kb_write` `kb_read` `kb_search` `kb_append` `kb_add_todo` | file I/O |
| **Utility** | `wait` `repeat_actions` `finish` | pure logic |

- **`show_notification`** — display a system notification (toast on Windows, banner on macOS, libnotify on Linux)
- **`play_sound`** — play audio files, beep tones, or TTS via rodio/cpal. The agent can speak alerts aloud
- **`scan_bluetooth`** — scan for nearby BLE devices with RSSI and distance estimation via btleplug
- **`get_sensors`** — CPU temperature, fan speed, disk health via sysinfo
- **`schedule_task`** — schedule a future agent task (e.g. "check email every morning at 9am") via cron / Task Scheduler / launchd
- **`set_timer`** — countdown timer, shows notification when done

> Desktop tools are planned for M006. Currently the desktop uses mock implementations of Android tools for development and testing.

#### iOS (planned — 19 tools)

Heavily sandboxed, but CoreMotion sensors, local notifications, audio playback, TTS, and flashlight all work in production apps.

| Category | Tools | Scope |
|----------|-------|-------|
| **Device** | `get_device_info` | UIDevice / ProcessInfo |
| **Sensors** | `get_sensors` | CoreMotion (accelerometer, gyroscope, magnetometer, device motion) |
| **Notifications** | `show_notification` | UNUserNotificationCenter (own app, user grants permission) |
| **Audio** | `play_sound` | AVAudioPlayer (sounds) + AVSpeechSynthesizer (TTS) |
| **Bluetooth** | `scan_bluetooth` | CoreBluetooth CBCentralManager (BLE scan, no special entitlement) |
| **Hardware** | `toggle_flashlight` | AVCaptureDevice.setTorchMode (works in production!) |
| **Clipboard** | `clipboard` | UIPasteboard (own app) |
| **Deep Links** | `open_app` `make_call` `send_message` | URL schemes (`tel://`, `message://`) |
| **Timers** | `set_timer` | local countdown, fires notification |
| **Knowledge Base** | `kb_write` `kb_read` `kb_search` `kb_append` `kb_add_todo` | file I/O (sandbox) |
| **Utility** | `wait` `finish` | pure logic |

- **`get_sensors`** — accelerometer, gyroscope, magnetometer via CoreMotion (no special entitlements needed)
- **`show_notification`** — local push notification to the user (own app only, user must grant permission)
- **`play_sound`** — play audio files or **speak text aloud via AVSpeechSynthesizer**. The agent can talk to the user without them looking at the screen
- **`scan_bluetooth`** — scan for nearby BLE devices with name, RSSI, and distance via CoreBluetooth (works in production, no special entitlement)
- **`toggle_flashlight`** — turn flashlight on/off via AVCaptureDevice.setTorchMode — this works in production apps, no special entitlement needed
- **`set_timer`** — local countdown timer, fires a local notification when done

> iOS tools are planned for M005. Apple does not allow production apps to read other apps' UI, simulate touches, or listen to system notifications. But sensors, flashlight, audio, TTS, and local notifications all work within the app's own process.

#### Shared tools (all platforms)

These 7 tools work identically everywhere — pure logic, no OS-specific code:

`kb_write` · `kb_read` · `kb_search` · `kb_append` · `kb_add_todo` · `wait` · `finish`

Full parameter schemas and return types: **[`docs/TOOLS.md`](docs/TOOLS.md)**

### Persistence

SQLite stores everything locally at `app_data_dir/pitoagent.db`:

| Table | What |
|-------|------|
| `chat_messages` | Full chat history with session grouping |
| `tasks` | Task records with status, answer, tokens, cost |
| `task_events` | Every tool call and result for each task |
| `agent_state` | Key-value store for session totals, config |

Session IDs use ISO date format — close and reopen the same day, your conversation continues.

## Skills

Small on-device models get dramatically better when you give them a strong playbook. So we give PitoAgent reusable **Skills** on top of generic tools.

A skill is a predefined workflow: a sequence of tool calls arranged in a specific order. The model follows the recipe step by step instead of reasoning from scratch. Every tool in the chain is generic — `open_app` works with any app, `get_screen_info` works on any screen, `send_message` works with any contact.

9 built-in skills: `dismiss` · `go_home` · `take_screenshot` · `check_notifications` · `open_app` · `send_quick_message` · `search_and_tap` · `scroll_to_bottom` · `volume_control`

Skills save 3–10 LLM rounds per task. When a skill fails, the system automatically falls back to the full agent loop.

This is the same pattern as auto-reply: open chat → read messages → generate reply → send. The tools are generic, the skill is the recipe. As on-device models get smarter, more of this becomes free-form. Right now, skills are how we get reliable automation out of a small local model.

## Proven Quick Tasks

These are tasks we have already run end-to-end during on-device QA.

### Local mode

- Summarize notifications
- Explain clipboard contents
- Analyze storage / apps and suggest cleanup targets
- Check whether the battery needs charging
- Report installed apps
- Report phone temperature
- Report Bluetooth state
- Report battery, storage, and Android version
- Run quick-task cards directly from the UI and return the result in chat
- Route contact-specific send / call tasks correctly and fail cleanly when the contact does not exist on the device

### Cloud mode

- Send a WhatsApp message and auto-return to the same PitoAgent conversation
- Search inside YouTube in the real app
- Check what is trending on Twitter / X and summarize it
- Install or open Telegram from Play Store
- Open Reddit and search for `pitoagent`
- Copy the latest email subject and Google it
- Draft an email saying you will be late
- Preserve task state and session history across cross-app execution and return

## Documentation

| Doc | What |
|-----|------|
| [`docs/TOOLS.md`](docs/TOOLS.md) | All 28 tools with parameter schemas, return types, and platform support |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | Module dependency graph, data flow diagrams, complete file listing |
| [`docs/AGENT_LOOP.md`](docs/AGENT_LOOP.md) | ReAct loop state machine, event types, guard integration, provider switching |

## Tech Stack

| Layer | Tech |
|-------|------|
| Frontend | Vue 3 + Vite (Composition API, TypeScript) |
| App shell | Tauri v2 (Rust) |
| Agent loop | Rust — ReAct cycle with streaming, cancellation, budget/stuck detection |
| LLM providers | OpenAI · Anthropic · Google Gemini · Local (Gemma 4 via LiteRT-LM) |
| Persistence | SQLite (rusqlite) — 4 tables, session continuity |
| Android native | Kotlin — AccessibilityService · NotificationListener · ForegroundService · LiteRT-LM |
| Plugin bridge | `tauri-plugin-pokeclaw` — 28 IPC commands across Kotlin → Rust → Vue |

## Download

[**Download APK**](https://github.com/agents-io/PitoAgent/releases/latest)

> Note: If you are updating from an older public debug build and Android says the package is incompatible, uninstall the old build once and then install the latest APK fresh. We verified that older public builds still receive the in-app update prompt, but the old public debug signing path requires a one-time reinstall for v0.5.0.

### Requirements

| | Minimum | Recommended |
|---|---|---|
| **Android** | 9+ | 12+ |
| **Architecture** | arm64 | arm64 |
| **RAM** | 8 GB | 12 GB+ |
| **Storage** | 3 GB free (model download) | 5 GB+ |
| **GPU** | Not required (CPU works) | Tensor G3/G4, Snapdragon 8 Gen 2+, Dimensity 9200+ |
| **Root** | Not required | Not required |

> ⚠️ 8 GB gets you in the door. 12 GB+ is the sweet spot for the built-in Gemma 4 local models, especially if you want smoother multitasking and faster model bring-up.

## Quick start

1. Install the APK
2. Grant Accessibility permission when prompted
3. If you want background monitor flows, also grant Notification Access
4. In Local mode, the model downloads on first local launch (~2.6 GB)
5. Switch to Chat or Task mode and start using it

Local mode needs no account and no API key. Cloud mode is optional.

## Development Milestones

| Milestone | Status | What |
|-----------|--------|------|
| **M001** — Tauri Foundation & Vue 3 Shell | ✅ Complete | Tauri v2 scaffold, Android plugin architecture, Ember-themed Vue 3 chat UI, IPC bridge proven end-to-end |
| **M002** — Agent Orchestration in Rust | ✅ Complete | 28-tool registry, LlmProvider trait (4 backends), ReAct agent loop, streaming TaskEvents, cancellation, budget/stuck detection, SQLite persistence, 3-tier pipeline routing, 9 skills, execution guards, living docs |
| **M003** — Local LLM Inference Bridge | ✅ Complete | LiteRT-LM SDK bridge, streaming token pipeline (Kotlin→Rust→Vue), model download with progress, GPU/CPU auto-fallback, RAII session lifecycle |
| **M004** — Accessibility & Notification Bridge | ✅ Complete | AccessibilityService (screen reading + gesture dispatch), NotificationListener, ForegroundService, 28 tool commands across Kotlin→Rust→Vue, PermissionPanel |
| **M005** — iOS Target | 📋 Planned | Swift plugins, Core ML inference, Dynamic Island agent tracker |
| **M006** — Desktop Full Stack | 📋 Planned | Real OS automation (Win32/Cocoa/X11), LiteRT-LM via FFI, Windows + macOS + Linux |

462 Rust unit tests across the agent module. All builds pass: `cargo build`, `cargo test`, `vue-tsc`, `vite build`.

## Roadmap

This is the current direction for PitoAgent based on real device testing, open issues, and the most common feature requests.

### Near-term

- **Stabler public releases and upgrades.** The release/signing path is being locked down so future public APKs upgrade cleanly instead of falling back to uninstall/reinstall behavior from the older debug-signed builds.
- **Lower-RAM local model options.** Right now the built-in local model choices are still too heavy for a lot of mid-range phones. Smaller on-device models are high priority.
- **More reliable local model downloads.** Resume/retry behavior, partial download cleanup, and corrupted-model detection are all being hardened so downloads survive weak connections and screen-off/resume cases better.
- **Broader device compatibility.** Samsung, Xiaomi, Dimensity, and low-RAM device issues are being used as real-world test cases for GPU→CPU fallback, model loading, accessibility reconnects, and generic UI control.
- **More generic phone-control skills.** We are continuing to replace brittle, app-specific assumptions with generic tools and reusable skills so tasks survive OEM UI changes better.

### In progress

- **Import your own local `.litertlm` models.** User-accessible local model import is on the roadmap so you can bring your own LiteRT model instead of being locked to the built-in download list.
- **More built-in workflows.** More quick-task / skill coverage is planned beyond the first WhatsApp-centric workflows.
- **Remote control / remote conversation flows.** Controlling a phone from another device is a real request and is on the longer roadmap, but it is not the current top priority compared with local reliability and device coverage.

### Known platform constraints

- **Edge Gallery model detection is not fully under our control.** Android hides other apps' `Android/data/...` sandboxes from normal file-pickers, so PitoAgent cannot generically "see" Edge Gallery's downloaded models unless they are exported into a user-accessible location first.
- **Sideload + accessibility apps may trigger OEM security warnings.** Samsung / Play Protect warnings are being addressed through a cleaner release/signing path, but sideload trust prompts are partly controlled by the platform and OEM policy.

### Where feature requests go

If you want something added, please open an issue. The roadmap above is intentionally built from real requests like:

- smaller local models for lower-end phones
- importing your own local models
- cleaner upgrade/install paths
- remote control from another phone
- broader distribution paths like F-Droid

## Help Wanted

PitoAgent is moving fast, and the roadmap is being shaped directly by real device reports, feature requests, and QA results. If you want to help push local phone agents forward:

- ⭐ **[Star this repo](https://github.com/agents-io/PitoAgent)** if you think local AI phone control matters
- 🐛 **[Open an issue](https://github.com/agents-io/PitoAgent/issues)** when you hit a bug or want a feature
- 🍴 **[Fork it](https://github.com/agents-io/PitoAgent/fork)** and build on it

Every star helps more people find the project. Every issue helps shape the next release.

## Changelog

### v0.5.1 (2026-04-10)
- **Chat/task input is more robust across phones.** The bottom input bar now follows the keyboard and system inset directly instead of relying on outer layout resize, which is safer across Pixel/Samsung navigation modes.
- **Public release signing is now pinned to a stable path.** GitHub releases now refuse to publish unless a stable signing key is configured, and the release workflow builds a signed `release` APK instead of accidentally shipping a debug artifact.
- **Release artifacts now include checksums.** The release pipeline also uploads `SHA256SUMS.txt` alongside the APK for easier verification.

### v0.5.0 (2026-04-10)
- **Previously shipped task flows now actually work.** Fixed stale model config reuse after switching Local/Cloud, fixed task/chat tab drift, fixed accessibility reconnect races, and fixed local task cancellation/session cleanup so tasks return to the right conversation instead of leaving stale state behind.
- **Previously broken task completions now execute the real app flow.** Explicit email-compose tasks now open a real mail composer instead of stopping with draft text in chat, and in-app search tasks can no longer fake-complete before the query is actually typed on screen.
- **Quick-task QA expanded.** Local quick tasks were swept end-to-end on-device, Cloud quick tasks now have cleaner automated coverage, and the QA runner correctly distinguishes real failures from environment blockers like permission dialogs or missing contacts.
- **Task budget default is now unlimited.** Fresh installs no longer inherit a fake default cap during development; users can still set a manual token/cost budget in Settings if they want one.
- **Previously misleading status rows now tell the truth.** Local GPU→CPU fallback now shows the real backend, the Accessibility status row reflects the real system state, and the Task Budget row shows `Unlimited` when no manual budget is set.
- **The updater now covers more real-world installs.** From v0.5.0 onward, accidental debug-build users also get the once-per-day GitHub release check, and the dialog warns when Android may require uninstalling an old debug build before installing the new APK.
- **Verified release-upgrade behavior.** Older public `0.4.0` builds do show the `v0.5.0` in-app update prompt, but upgrading from the old public debug signing path to the new public APK is a one-time uninstall + reinstall instead of an in-place replace.

<details>
<summary>Older versions (v0.1.0 — v0.4.1)</summary>

### v0.4.1 (2026-04-08)
- **Experimental task badge.** Task tab now shows `Experimental — more workflows coming soon`.
- **12 new complex task QA cases.** Added broader Cloud task coverage for YouTube search, contextual messaging, screen reading, settings toggles, app installs, web search, email compose, camera flows, typo tolerance, and ambiguous requests.

### v0.3.2 (2026-04-07)
- **Security fix.** Debug task receivers now disabled in release builds. External apps can no longer trigger tasks via broadcast.
- **Security fix.** The LAN config server was binding to all network interfaces, exposing API keys to anyone on the same WiFi. Now binds to localhost only.

### v0.3.0 (2026-04-07)
- **Cloud LLM support.** Chat and task modes now work with OpenAI, Anthropic, Google, and any OpenAI-compatible API. Switch providers with one tap in the new tabbed LLM Config screen.
- **Real-time token and cost display.** See your token count and running cost in the chat header as you talk. Color shifts from grey to blue to amber to red as usage climbs. No other mobile AI app shows you this.
- **Per-provider API keys.** Store a different API key for each provider. Switching tabs loads the right key automatically.
- **Mid-session model switch.** Start a conversation with GPT-4o, switch to Claude mid-chat, and keep your entire history. The new model picks up where the old one left off.
- **3-tier pipeline router.** Simple commands (call, alarm, open app) now execute instantly with zero LLM calls. Skill-matched tasks run deterministic step sequences. Only complex tasks hit the full agent loop.
- **8 built-in skills.** Search in App, Dismiss Popup, Scroll and Read, Send WhatsApp, Navigate to Tab, and more. Each skill saves 3-10 LLM rounds by running a hardcoded tool sequence instead of reasoning from scratch.
- **Skills UI in Task tab.** Quick Actions section shows all available skills with category icons. Tap to prefill the input bar.
- **Token budget system.** Set soft and hard limits on token usage per task. The floating pill shows live token count and cost, and you can tap to stop a runaway task.
- **Stuck detection.** Five signals detect when the agent is going in circles: repeated actions, unchanged screens, rising token count. Three-level recovery escalates from hints to strategy switches to auto-kill.
- **Enter and Tab key support.** Skills can now press Enter to submit search queries and Tab to move between form fields.

### v0.2.4 (2026-04-06)
- **Task tab redesigned with skill cards.** No more typing free-form commands that Gemma misunderstands. Two skill cards with fill-in-the-blank forms: "Monitor [name] on [WhatsApp]" and "Send [message] to [name] on [WhatsApp]".
- **Java skill routing.** Monitor and send-message tasks bypass the LLM entirely. Instant activation, zero warmup.
- **Progress bar on skill activation.** Card fills up and turns orange when active. You know exactly when monitoring starts.
- **Custom tasks disabled for on-device models.** Gemma is not smart enough to route free-form tasks to the right skill. Switch to a cloud LLM in Settings to unlock the text input.
- **Tasks stay in-app.** Starting a task no longer jumps to the home screen. You see progress in PitoAgent, then it goes to background when ready.

### v0.2.0 (2026-04-06)
- **Auto-reply now reads conversation context.** Before replying, the AI opens the chatroom and reads all visible messages on screen. It no longer forgets what was said 3 messages ago.
- **In-app update checker.** The app checks GitHub Releases once per day and prompts you to download if a newer version exists. No more manually checking.

### v0.1.0 (2026-04-06)
- Initial release. On-device Gemma 4 E2B with tool calling, accessibility-based phone control, auto-reply, task mode.

</details>

## Acknowledgments

PitoAgent exists because of [Gemma 4](https://blog.google/innovation-and-ai/technology/developers-tools/gemma-4/) by [Google DeepMind](https://github.com/google-deepmind). Thank you to [Clément Farabet](https://github.com/clementfarabet), [Olivier Lacombe](https://github.com/olivierlacombe), and the entire Gemma team for shipping an open model with native tool calling under Apache 2.0. You made it possible for a solo developer to build a working phone agent in two nights. The [LiteRT-LM](https://ai.google.dev/edge/litert/llm/overview) runtime is what makes on-device inference practical.

Also inspired by the [OpenClaw](https://github.com/openclaw/openclaw) community 🦞 for proving that AI agents that actually do things are what people want.

And thank you to [Claude Code](https://claude.ai/code) by Anthropic. I'm a CS dropout with zero Android development experience. Claude Code made it possible for me to go from nothing to a working app in two nights. The future is wild.

## Trademark

PitoAgent is a trademark of Nicole / agents.io. The name "PitoAgent" and the PitoAgent logo may not be used to endorse or promote products derived from this software without prior written permission. Forks must be renamed before distribution.

## License

Apache 2.0

Contributors sign our [CLA](CLA.md) before their first PR is merged.
