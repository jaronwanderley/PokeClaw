use std::sync::Mutex;
use tauri::{
    plugin::{Builder, TauriPlugin},
    Runtime, State,
};

// ---------------------------------------------------------------------------
// Observation tool shared types (desktop + Android contract)
// ---------------------------------------------------------------------------

/// Structured response matching the Android @Command contract:
/// `{ success: bool, data: Any?, error: String? }`
#[derive(Clone, serde::Serialize)]
pub struct ToolResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Permission/status structure returned by check_permissions.
#[derive(Clone, serde::Serialize)]
pub struct PermissionStatus {
    pub accessibility_enabled: bool,
    pub accessibility_running: bool,
    pub notification_enabled: bool,
    pub foreground_service: bool,
}

// ---------------------------------------------------------------------------
// Session status enum
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "state", content = "detail")]
pub enum SessionStatus {
    Idle,
    Loading(String),
    Ready {
        session_id: String,
        backend: String,
    },
    Error(String),
}

impl Default for SessionStatus {
    fn default() -> Self {
        Self::Idle
    }
}

// ---------------------------------------------------------------------------
// StreamEvent — streaming IPC contract (matches TypeScript discriminated union)
// ---------------------------------------------------------------------------

/// Events streamed from Rust → Vue via `tauri::ipc::Channel`.
/// Serde serialises as `{ event: "token_batch", data: { tokens, batch_index } }` etc.
#[derive(Clone, serde::Serialize)]
#[serde(tag = "event", content = "data")]
pub enum StreamEvent {
    #[serde(rename = "token_batch")]
    TokenBatch { tokens: String, batch_index: u32 },
    #[serde(rename = "complete")]
    Complete { full_text: String, token_count: u32 },
    #[serde(rename = "error")]
    Error { message: String },
}

// ---------------------------------------------------------------------------
// DownloadEvent — download progress IPC contract (matches Kotlin DownloadCallback)
// ---------------------------------------------------------------------------

/// Events streamed from Rust → Vue during model download.
/// Matches the Kotlin DownloadEvent contract from T01.
#[derive(Clone, serde::Serialize)]
#[serde(tag = "event", content = "data")]
pub enum DownloadEvent {
    #[serde(rename = "progress")]
    Progress {
        bytes_downloaded: u64,
        total_bytes: u64,
        bytes_per_second: u64,
    },
    #[serde(rename = "complete")]
    Complete { model_path: String, file_name: String },
    #[serde(rename = "error")]
    Error { message: String },
}

// ---------------------------------------------------------------------------
// ModelInfo — static model catalog entry (matches Kotlin ModelManager.ModelInfo)
// ---------------------------------------------------------------------------

#[derive(Clone, serde::Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub url: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub min_ram_gb: u32,
    pub is_downloaded: bool,
    pub local_path: Option<String>,
}

// ---------------------------------------------------------------------------
// LlmSessionGuard — RAII session handle
// ---------------------------------------------------------------------------

/// RAII guard for an inference session lifecycle.
///
/// Per D007: the primary cleanup path is the explicit `stop_session` command.
/// The Drop impl only logs a warning — it does NOT attempt Kotlin IPC.
pub struct LlmSessionGuard {
    pub model_path: String,
    pub backend: String,   // "gpu" or "cpu"
    pub session_id: String, // UUID-style identifier
}

impl Drop for LlmSessionGuard {
    fn drop(&mut self) {
        log::warn!(
            "LlmSessionGuard dropped without explicit stop_session — session_id={}, backend={}. \
             Explicit stop_session is the primary cleanup path (D007).",
            self.session_id,
            self.backend
        );
    }
}

// ---------------------------------------------------------------------------
// InferenceState — managed via tauri::State
// ---------------------------------------------------------------------------

pub struct InferenceState {
    pub active_session: Mutex<Option<LlmSessionGuard>>,
    pub session_status: Mutex<SessionStatus>,
}

impl Default for InferenceState {
    fn default() -> Self {
        Self {
            active_session: Mutex::new(None),
            session_status: Mutex::new(SessionStatus::Idle),
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: generate a simple session ID (no uuid dependency)
// ---------------------------------------------------------------------------

fn generate_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("sess-{:x}-{:04x}", ts, rand_simple_sixteen())
}

/// Tiny pseudo-random suffix using xorshift seeded from time.
fn rand_simple_sixteen() -> u16 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mut x = seed.wrapping_add(0x9e3779b97f4a7c15);
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    (x as u16) & 0xFFFF
}

/// Convert days since Unix epoch to (year, month, day).
/// Used by desktop mock get_device_info for time category.
fn date_from_days(days_since_epoch: u64) -> (u32, u32, u32) {
    // Algorithm from Howard Hinnant: http://howardhinnant.github.io/date_algorithms.html
    let z = days_since_epoch as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = (y + if m <= 2 { 1 } else { 0 }) as u32;
    (y, m, d)
}

// ---------------------------------------------------------------------------
// Session command implementations (no #[tauri::command] here — just logic)
// ---------------------------------------------------------------------------

mod session_impl {
    use super::*;

    pub fn do_start_session(
        state: &InferenceState,
        model_path: String,
        prefer_gpu: bool,
    ) -> Result<String, String> {
        log::info!(
            "start_session: model_path={}, prefer_gpu={}",
            model_path,
            prefer_gpu
        );

        // Check if a session is already active
        {
            let session = state.active_session.lock().map_err(|e| e.to_string())?;
            if session.is_some() {
                return Err("A session is already active. Call stop_session first.".into());
            }
        }

        let backend = if prefer_gpu { "gpu" } else { "cpu" };
        let session_id = generate_session_id();

        // Set status to Loading
        {
            let mut status = state.session_status.lock().map_err(|e| e.to_string())?;
            *status = SessionStatus::Loading(format!(
                "Initializing {} backend for {}",
                backend, model_path
            ));
        }

        #[cfg(target_os = "android")]
        {
            log::info!(
                "start_session (Android): would invoke Kotlin engine init — session_id={}",
                session_id
            );
            // Kotlin IPC will be wired in the next slice
        }

        #[cfg(not(target_os = "android"))]
        {
            log::info!(
                "start_session (desktop mock): creating mock session — session_id={}",
                session_id
            );
        }

        let guard = LlmSessionGuard {
            model_path: model_path.clone(),
            backend: backend.to_string(),
            session_id: session_id.clone(),
        };
        *state.active_session.lock().map_err(|e| e.to_string())? = Some(guard);

        // Set status to Ready
        {
            let mut status = state.session_status.lock().map_err(|e| e.to_string())?;
            *status = SessionStatus::Ready {
                session_id: session_id.clone(),
                backend: backend.to_string(),
            };
        }

        log::info!(
            "start_session: session active — session_id={}, backend={}",
            session_id,
            backend
        );
        Ok(session_id)
    }

    pub fn do_stop_session(state: &InferenceState) -> Result<(), String> {
        log::info!("stop_session: requesting session stop");

        let session = state
            .active_session
            .lock()
            .map_err(|e| e.to_string())?
            .take();

        if session.is_none() {
            log::warn!("stop_session: no active session to stop");
            return Err("No active session to stop.".into());
        }

        let guard = session.unwrap();
        log::info!(
            "stop_session: session stopped — session_id={}, backend={}",
            guard.session_id,
            guard.backend
        );

        {
            let mut status = state.session_status.lock().map_err(|e| e.to_string())?;
            *status = SessionStatus::Idle;
        }

        Ok(())
    }

    #[allow(dead_code)] // Used by android_commands; desktop uses do_send_message_streaming
    pub fn do_send_message(
        state: &InferenceState,
        message: String,
    ) -> Result<String, String> {
        log::info!("send_message: message='{}'", message);

        {
            let session = state.active_session.lock().map_err(|e| e.to_string())?;
            if session.is_none() {
                return Err("No active session. Call start_session first.".into());
            }
        }

        #[cfg(target_os = "android")]
        {
            log::info!("send_message (Android): would invoke Kotlin inference — message='{}'", message);
            Ok(format!("Android response for: {}", message))
        }

        #[cfg(not(target_os = "android"))]
        {
            log::info!("send_message (desktop mock): echoing message='{}'", message);
            Ok(format!("Echo: {}", message))
        }
    }

    /// Desktop-only streaming send_message. Streams echo response word-by-word
    /// through the Tauri Channel so the frontend sees the same StreamEvent
    /// contract as the Kotlin Android path.
    #[cfg(not(target_os = "android"))]
    pub async fn do_send_message_streaming(
        state: &InferenceState,
        message: String,
        channel: &tauri::ipc::Channel<StreamEvent>,
    ) -> Result<(), String> {
        log::info!("send_message_streaming: message='{}'", message);

        // Validate session is active
        {
            let session = state.active_session.lock().map_err(|e| e.to_string())?;
            if session.is_none() {
                let err_msg = "No active session. Call start_session first.".to_string();
                log::error!("send_message_streaming: {}", err_msg);
                if channel.send(StreamEvent::Error {
                    message: err_msg.clone(),
                }).is_err() {
                    log::warn!("send_message_streaming: failed to send error event — frontend may have disconnected");
                }
                return Err(err_msg);
            }
        }

        let echo_text = format!("Echo: {}", message);
        let words: Vec<&str> = echo_text.split_whitespace().collect();
        let total_words = words.len();

        log::info!(
            "send_message_streaming: starting stream — {} words to send in batches",
            total_words
        );

        // Group words into batches of 1–2 words
        let mut batch_index: u32 = 0;
        let mut word_iter = words.chunks(2);

        while let Some(batch) = word_iter.next() {
            let tokens = batch.join(" ");
            log::debug!(
                "send_message_streaming: sending batch {} — tokens='{}'",
                batch_index,
                tokens
            );

            if let Err(e) = channel.send(StreamEvent::TokenBatch {
                tokens,
                batch_index,
            }) {
                log::warn!(
                    "send_message_streaming: channel.send failed for batch {} — frontend may have disconnected: {}",
                    batch_index,
                    e
                );
                return Ok(()); // Non-fatal: frontend disconnected
            }

            batch_index += 1;

            // ~80ms delay between batches to simulate streaming
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        }

        let full_text = echo_text.clone();
        let token_count = batch_index; // Each batch counts as one "token unit"
        log::info!(
            "send_message_streaming: stream complete — token_count={}, full_text='{}'",
            token_count,
            full_text
        );

        if let Err(e) = channel.send(StreamEvent::Complete {
            full_text,
            token_count,
        }) {
            log::warn!(
                "send_message_streaming: channel.send failed for complete event — frontend may have disconnected: {}",
                e
            );
        }

        Ok(())
    }

    pub fn do_get_session_status(state: &InferenceState) -> Result<SessionStatus, String> {
        let status = state.session_status.lock().map_err(|e| e.to_string())?;
        Ok(status.clone())
    }
}

// ---------------------------------------------------------------------------
// Android command wrappers (registered only on Android)
// ---------------------------------------------------------------------------

#[cfg(target_os = "android")]
mod android_commands {
    use super::*;

    #[tauri::command]
    pub fn start_session(
        state: State<'_, InferenceState>,
        model_path: String,
        prefer_gpu: bool,
    ) -> Result<String, String> {
        session_impl::do_start_session(&state, model_path, prefer_gpu)
    }

    #[tauri::command]
    pub fn stop_session(state: State<'_, InferenceState>) -> Result<(), String> {
        session_impl::do_stop_session(&state)
    }

    #[tauri::command]
    pub fn send_message(
        state: State<'_, InferenceState>,
        message: String,
    ) -> Result<String, String> {
        session_impl::do_send_message(&state, message)
    }

    #[tauri::command]
    pub fn get_session_status(state: State<'_, InferenceState>) -> Result<SessionStatus, String> {
        session_impl::do_get_session_status(&state)
    }

    #[tauri::command]
    pub fn chat(message: String) -> String {
        log::info!("chat command received: {}", message);
        format!("Echo: {}", message)
    }
}

// ---------------------------------------------------------------------------
// Desktop command wrappers (registered only on non-Android)
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "android"))]
mod desktop_commands {
    use super::*;

    #[tauri::command]
    pub fn start_session(
        state: State<'_, InferenceState>,
        model_path: String,
        prefer_gpu: bool,
    ) -> Result<String, String> {
        session_impl::do_start_session(&state, model_path, prefer_gpu)
    }

    #[tauri::command]
    pub fn stop_session(state: State<'_, InferenceState>) -> Result<(), String> {
        session_impl::do_stop_session(&state)
    }

    /// Desktop streaming send_message. Uses Tauri Channel to stream
    /// StreamEvent tokens to the frontend — same contract as Kotlin Android.
    #[tauri::command]
    pub async fn send_message(
        state: State<'_, InferenceState>,
        message: String,
        on_event: tauri::ipc::Channel<StreamEvent>,
    ) -> Result<(), String> {
        session_impl::do_send_message_streaming(&state, message, &on_event).await
    }

    #[tauri::command]
    pub fn get_session_status(state: State<'_, InferenceState>) -> Result<SessionStatus, String> {
        session_impl::do_get_session_status(&state)
    }

    #[tauri::command]
    pub fn ping() -> String {
        "pong from rust".into()
    }

    #[tauri::command]
    pub fn chat(message: String) -> String {
        log::info!("chat command received: {}", message);
        format!("Echo: {}", message)
    }

    /// Desktop mock for list_models. Returns the static model catalog
    /// matching the Kotlin ModelManager.AVAILABLE_MODELS entries.
    /// On desktop, `is_downloaded` is always false and `local_path` is null.
    #[tauri::command]
    pub fn list_models() -> Vec<ModelInfo> {
        log::info!("list_models: returning static model catalog");
        vec![
            ModelInfo {
                id: "gemma4-e2b".into(),
                display_name: "Gemma 4 E2B — 2.6GB".into(),
                url: "https://huggingface.co/litert-community/gemma-4-E2B-it-litert-lm/resolve/main/gemma-4-E2B-it.litertlm".into(),
                file_name: "gemma-4-E2B-it.litertlm".into(),
                size_bytes: 2_580_000_000u64,
                min_ram_gb: 8,
                is_downloaded: false,
                local_path: None,
            },
            ModelInfo {
                id: "gemma4-e4b".into(),
                display_name: "Gemma 4 E4B — 3.6GB".into(),
                url: "https://huggingface.co/litert-community/gemma-4-E4B-it-litert-lm/resolve/main/gemma-4-E4B-it.litertlm".into(),
                file_name: "gemma-4-E4B-it.litertlm".into(),
                size_bytes: 3_650_000_000u64,
                min_ram_gb: 10,
                is_downloaded: false,
                local_path: None,
            },
        ]
    }

    /// Desktop mock for download_model. Simulates a download by sending
    /// 10–20 progress events with increasing bytes, then a complete event.
    /// Uses the same DownloadEvent contract as the Kotlin path.
    #[tauri::command]
    pub async fn download_model(
        model_id: String,
        on_progress: tauri::ipc::Channel<DownloadEvent>,
    ) -> Result<(), String> {
        log::info!("download_model (desktop mock): model_id={}", model_id);

        let model = match model_id.as_str() {
            "gemma4-e2b" => (
                "gemma-4-E2B-it.litertlm".to_string(),
                2_580_000_000u64,
            ),
            "gemma4-e4b" => (
                "gemma-4-E4B-it.litertlm".to_string(),
                3_650_000_000u64,
            ),
            _ => {
                let msg = format!("Unknown model_id: {}", model_id);
                log::error!("download_model: {}", msg);
                let _ = on_progress.send(DownloadEvent::Error {
                    message: msg.clone(),
                });
                return Err(msg);
            }
        };
        let (file_name, total_bytes) = model;

        let steps = 15u64;
        let step_bytes = total_bytes / steps;

        for i in 1..=steps {
            let bytes_downloaded = if i == steps {
                total_bytes // exact total on last step
            } else {
                step_bytes * i
            };
            // Simulate ~500KB/s–5MB/s download speed
            let speed = 500_000u64 + (i * 300_000);

            if let Err(e) = on_progress.send(DownloadEvent::Progress {
                bytes_downloaded,
                total_bytes,
                bytes_per_second: speed,
            }) {
                log::warn!(
                    "download_model: channel send failed at step {}/{} — frontend may have disconnected: {}",
                    i, steps, e
                );
                return Ok(()); // Non-fatal: frontend disconnected
            }

            // Simulate network delay between progress events (~200ms)
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        let model_path = format!("/tmp/models/{}", file_name);
        log::info!(
            "download_model (desktop mock): download complete — model_path={}",
            model_path
        );

        if let Err(e) = on_progress.send(DownloadEvent::Complete {
            model_path: model_path.clone(),
            file_name: file_name.clone(),
        }) {
            log::warn!(
                "download_model: channel send failed for complete event — frontend may have disconnected: {}",
                e
            );
        }

        Ok(())
    }

    // -----------------------------------------------------------------
    // Observation tool desktop mocks
    // -----------------------------------------------------------------

    /// Desktop mock for get_screen_info.
    /// Returns a realistic sample screen tree matching the format
    /// produced by PokeAccessibilityService.getScreenTree().
    #[tauri::command]
    pub fn get_screen_info() -> ToolResult {
        log::info!("get_screen_info (desktop mock): returning sample screen tree");
        let tree = "[n1] \"Messages\" tap (540,80)\n\
                     [n2] \"Search\" tap edit (540,160)\n\
                     [n3] \"John\" tap (270,280)\n\
                     [n4] \"Hey, are you free?\" (270,330)\n\
                     [n5] \"Alice\" tap (270,430)\n\
                     [n6] \"Meeting at 3pm\" (270,480)\n\
                     [n7] \"Send message\" tap (990,2100)";
        ToolResult {
            success: true,
            data: Some(serde_json::json!({ "tree": tree })),
            error: None,
        }
    }

    /// Desktop mock for find_node_info.
    /// Returns a single mock node matching the searched text.
    /// Returns an error response if the text parameter is empty.
    #[tauri::command]
    pub fn find_node_info(text: String) -> ToolResult {
        log::info!("find_node_info (desktop mock): text='{}'", text);
        if text.trim().is_empty() {
            return ToolResult {
                success: false,
                data: None,
                error: Some("text parameter must not be empty".into()),
            };
        }
        let node = serde_json::json!({
            "index": 0,
            "className": "android.widget.TextView",
            "text": text,
            "bounds": "[100,200][400,260]",
            "clickable": true,
        });
        ToolResult {
            success: true,
            data: Some(serde_json::json!({ "nodes": [node] })),
            error: None,
        }
    }

    /// Desktop mock for get_device_info.
    /// Returns category-specific mock info strings matching the format
    /// produced by the Android GetDeviceInfoTool categories.
    /// Returns an error for unknown categories.
    #[tauri::command]
    pub fn get_device_info(category: String) -> ToolResult {
        log::info!("get_device_info (desktop mock): category='{}'", category);
        let info = match category.to_lowercase().as_str() {
            "battery" => "Battery: 85%, charging",
            "wifi" => "WiFi: connected to 'HomeWifi', 2.4GHz, signal -45dBm, 65Mbps",
            "storage" => "Storage: 45.2 GB used of 128.0 GB (35%), 82.8 GB free",
            "bluetooth" => "Bluetooth: enabled, paired devices: [Galaxy Buds, Car Audio]",
            "screen" => "Brightness: 60%, Dark mode: OFF",
            "device" => "Android 14 (API 34), Model: Google Pixel 8",
            "time" => {
                use std::time::{SystemTime, UNIX_EPOCH};
                let secs = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                // Simple UTC time formatting without chrono
                let days_since_epoch = secs / 86400;
                let time_of_day = secs % 86400;
                let hours = (time_of_day / 3600) as u32;
                let minutes = ((time_of_day % 3600) / 60) as u32;
                let seconds = (time_of_day % 60) as u32;
                // Approximate year/month/day (good enough for mock data)
                let (year, month, day) = date_from_days(days_since_epoch);
                let time_str = format!(
                    "Time: {:04}-{:02}-{:02} {:02}:{:02}:{:02} (mock UTC)",
                    year, month, day, hours, minutes, seconds
                );
                return ToolResult {
                    success: true,
                    data: Some(serde_json::json!({ "info": time_str })),
                    error: None,
                };
            }
            _ => {
                return ToolResult {
                    success: false,
                    data: None,
                    error: Some(format!(
                        "Unknown category '{}'. Supported: battery, wifi, storage, bluetooth, screen, device, time",
                        category
                    )),
                };
            }
        };
        ToolResult {
            success: true,
            data: Some(serde_json::json!({ "info": info })),
            error: None,
        }
    }

    /// Desktop mock for check_permissions.
    /// Returns a PermissionStatus with accessibility enabled/running true,
    /// and notification/foreground_service false (simulating a typical
    /// development environment where accessibility is on but notifications
    /// and foreground service are not yet granted).
    #[tauri::command]
    pub fn check_permissions() -> ToolResult {
        log::info!("check_permissions (desktop mock): returning mock permission status");
        let status = PermissionStatus {
            accessibility_enabled: true,
            accessibility_running: true,
            notification_enabled: false,
            foreground_service: false,
        };
        ToolResult {
            success: true,
            data: Some(serde_json::to_value(&status).unwrap_or_else(|_| serde_json::json!({}))),
            error: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin initialization
// ---------------------------------------------------------------------------

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    let builder = Builder::new("pokeclaw");

    #[cfg(target_os = "android")]
    let builder = builder
        .setup(|_app, api| {
            api.register_android_plugin("io.agents.pokeclaw", "PokeclawPlugin")?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            android_commands::start_session,
            android_commands::stop_session,
            android_commands::send_message,
            android_commands::get_session_status,
            android_commands::chat,
        ]);

    #[cfg(not(target_os = "android"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
            desktop_commands::start_session,
            desktop_commands::stop_session,
            desktop_commands::send_message,
            desktop_commands::get_session_status,
            desktop_commands::ping,
            desktop_commands::chat,
            desktop_commands::list_models,
            desktop_commands::download_model,
            desktop_commands::get_screen_info,
            desktop_commands::find_node_info,
            desktop_commands::get_device_info,
            desktop_commands::check_permissions,
        ]);

    builder.build()
}
