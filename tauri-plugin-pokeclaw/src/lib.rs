use std::sync::Mutex;
#[cfg(not(target_os = "android"))]
use std::sync::Arc;
use tauri::{
    plugin::{Builder, TauriPlugin},
    AppHandle, Manager, Runtime, State,
};

// ---------------------------------------------------------------------------
// Test-only re-exports for integration tests
// ---------------------------------------------------------------------------

/// Re-export of desktop FFI types and model manager functions for integration tests.
/// Only compiled for non-Android targets (same as the desktop module).
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod _ffi_test_exports {
    pub use crate::desktop::ffi::{LitertEngine as LitertEngineShim, LitertError};
    pub use crate::desktop::model_manager::{list_models, models_dir};
}

// ---------------------------------------------------------------------------
// Desktop-only modules (FFI bindings for LiteRT-LM native inference)
// ---------------------------------------------------------------------------

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod desktop;

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
    /// Desktop-only: loaded LiteRT-LM engine for real inference.
    /// `None` if library not found (echo mock fallback) or not yet loaded.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub litert_engine: Arc<Mutex<Option<desktop::ffi::LitertEngine>>>,
}

impl Default for InferenceState {
    fn default() -> Self {
        Self {
            active_session: Mutex::new(None),
            session_status: Mutex::new(SessionStatus::Idle),
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            litert_engine: Arc::new(Mutex::new(None)),
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


// ---------------------------------------------------------------------------
// Session command implementations (no #[tauri::command] here — just logic)
// ---------------------------------------------------------------------------

mod session_impl {
    use super::*;

    /// Try to discover the LitertEngine shared library on the system.
    /// Returns the first path that exists, or None.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    fn find_litertlm_library() -> Option<std::path::PathBuf> {
        let candidates = [
            // Next to the executable (most common for Tauri apps)
            std::env::current_exe().ok()?.parent()?.join("litertlm_bridge.dll"),
            std::env::current_exe().ok()?.parent()?.join("litertlm_bridge.so"),
            std::env::current_exe().ok()?.parent()?.join("litertlm_bridge.dylib"),
            // Build output directory
            std::path::PathBuf::from("target/release/litertlm_bridge.dll"),
            std::path::PathBuf::from("target/release/litertlm_bridge.so"),
            std::path::PathBuf::from("target/release/litertlm_bridge.dylib"),
            // CMake build output
            std::path::PathBuf::from("tauri-plugin-pokeclaw/src/desktop/ffi/build/output/litertlm_bridge.dll"),
            std::path::PathBuf::from("tauri-plugin-pokeclaw/src/desktop/ffi/build/output/litertlm_bridge.so"),
            std::path::PathBuf::from("tauri-plugin-pokeclaw/src/desktop/ffi/build/output/litertlm_bridge.dylib"),
        ];
        for candidate in &candidates {
            if candidate.exists() {
                log::info!(
                    "find_litertlm_library: found at '{}'",
                    candidate.display()
                );
                return Some(candidate.clone());
            }
        }
        log::info!(
            "find_litertlm_library: no library found in {} candidate paths",
            candidates.len()
        );
        None
    }

    pub fn do_start_session<R: Runtime>(
        app: &AppHandle<R>,
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
            log::info!("start_session (Android): invoking Kotlin startSession — model_path={}", model_path);
            let handle = app.state::<AndroidPluginHandle<R>>();
            
            let args = serde_json::json!({
                "modelPath": model_path,
                "preferGpu": prefer_gpu,
            });

            let result: serde_json::Value = handle.0.run_mobile_plugin("startSession", args)
                .map_err(|e| format!("Kotlin startSession failed: {}", e))?;
            
            let res_session_id = result.get("sessionId")
                .and_then(|v| v.as_str())
                .unwrap_or(&session_id)
                .to_string();
            
            let res_backend = result.get("backend")
                .and_then(|v| v.as_str())
                .unwrap_or(backend)
                .to_string();

            let mut session = state.active_session.lock().map_err(|e| e.to_string())?;
            *session = Some(LlmSessionGuard {
                model_path: model_path.clone(),
                backend: res_backend.clone(),
                session_id: res_session_id.clone(),
            });

            let mut status = state.session_status.lock().map_err(|e| e.to_string())?;
            *status = SessionStatus::Ready {
                session_id: res_session_id.clone(),
                backend: res_backend,
            };

            return Ok(res_session_id);
        }

        #[cfg(not(target_os = "android"))]
        {
            // Desktop/iOS fallback path
            let session_id = generate_session_id();

            #[cfg(target_os = "ios")]
            {
                log::info!(
                    "start_session (iOS): would invoke Swift engine init — session_id={}",
                    session_id
                );
            }

            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                // Try to load the real LiteRT-LM engine via FFI.
                // If the shared library is not found, fall back to echo mock.
                let engine_result = match find_litertlm_library() {
                    Some(lib_path) => {
                        log::info!(
                            "start_session (desktop): loading LiteRT-LM engine — lib_path={}, model_path={}, backend={}",
                            lib_path.display(), model_path, backend
                        );
                        match desktop::ffi::LitertEngine::new(&lib_path) {
                            Ok(mut engine) => {
                                match engine.load_model(&model_path, backend) {
                                    Ok(()) => {
                                        log::info!(
                                            "start_session (desktop): engine loaded successfully — model_path={}, backend={}",
                                            model_path, backend
                                        );
                                        Ok(Some(engine))
                                    }
                                    Err(e) => {
                                        log::error!(
                                            "start_session (desktop): engine model load failed — {}. Falling back to echo mock.",
                                            e
                                        );
                                        // Set error status briefly, then override to ready with mock
                                        {
                                            let mut status = state.session_status.lock().map_err(|e2| e2.to_string())?;
                                            *status = SessionStatus::Error(format!("Model load failed: {}", e));
                                        }
                                        Ok(None)
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!(
                                    "start_session (desktop): FFI engine init failed — {}. Falling back to echo mock.",
                                    e
                                );
                                Ok(None)
                            }
                        }
                    }
                    None => {
                        log::info!("start_session (desktop): no LiteRT-LM native library found. Using echo mock.");
                        Ok(None)
                    }
                };

                match engine_result {
                    Ok(engine) => {
                        let mut litert = state.litert_engine.lock().map_err(|e| e.to_string())?;
                        *litert = engine;
                    }
                    Err(e) => return Err(e),
                }
            }

            let mut session = state.active_session.lock().map_err(|e| e.to_string())?;
            *session = Some(LlmSessionGuard {
                model_path: model_path.clone(),
                backend: backend.to_string(),
                session_id: session_id.clone(),
            });

            let mut status = state.session_status.lock().map_err(|e| e.to_string())?;
            *status = SessionStatus::Ready {
                session_id: session_id.clone(),
                backend: backend.to_string(),
            };

            return Ok(session_id);
        }
    }

    pub fn do_stop_session<R: Runtime>(
        app: &AppHandle<R>,
        state: &InferenceState,
    ) -> Result<(), String> {
        log::info!("stop_session: requesting session stop");

        let session = state
            .active_session
            .lock()
            .map_err(|e| e.to_string())?
            .take(); // Atomic take: the session is stopped even if subsequent IPC fails

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

        // Drop the LiteRT-LM engine if present (desktop only)
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            let mut litert = state.litert_engine.lock().map_err(|e| e.to_string())?;
            if litert.is_some() {
                log::info!(
                    "stop_session: destroying LiteRT-LM engine — model_path={}",
                    litert.as_ref().map(|e| e.model_path().unwrap_or("unknown")).unwrap_or("none")
                );
                *litert = None; // Drop triggers LitertEngineInner::drop → engine_destroy
            }
        }

        #[cfg(target_os = "android")]
        {
            log::info!("stop_session (Android): invoking Kotlin stopSession");
            let handle = app.state::<AndroidPluginHandle<R>>();
            handle.0.run_mobile_plugin::<serde_json::Value>("stopSession", serde_json::json!({}))
                .map_err(|e| format!("Kotlin stopSession failed: {}", e))?;
        }

        {
            let mut status = state.session_status.lock().map_err(|e| e.to_string())?;
            *status = SessionStatus::Idle;
        }

        Ok(())
    }

    pub fn do_send_message<R: Runtime>(
        app: &AppHandle<R>,
        state: &InferenceState,
        message: String,
    ) -> Result<String, String> {
        log::info!("send_message: message_len={}", message.len());

        {
            let session = state.active_session.lock().map_err(|e| e.to_string())?;
            if session.is_none() {
                return Err("No active session. Call start_session first.".into());
            }
        }

        #[cfg(target_os = "android")]
        {
            log::info!("send_message (Android): invoking Kotlin sendMessage — message_len={}", message.len());
            let handle = app.state::<AndroidPluginHandle<R>>();
            
            let args = serde_json::json!({
                "message": message,
            });

            let result: serde_json::Value = handle.0.run_mobile_plugin("sendMessage", args)
                .map_err(|e| format!("Kotlin sendMessage failed: {}", e))?;
            
            let response = result.get("response")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Kotlin sendMessage returned no response string".to_string())?;
            
            return Ok(response.to_string());
        }

        #[cfg(target_os = "ios")]
        {
            log::info!("send_message (iOS): would invoke Swift inference — message_len={}", message.len());
            Ok(format!("iOS response for message of length {}", message.len()))
        }

        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            // Check if we have a real engine
            let maybe_engine_arc = {
                let litert = state.litert_engine.lock().map_err(|e| e.to_string())?;
                match litert.as_ref() {
                    Some(_) => Some(Arc::clone(&state.litert_engine)),
                    None => None,
                }
            };

            if let Some(engine_arc) = maybe_engine_arc {
                // Real LiteRT-LM inference path
                log::info!("do_send_message (desktop): using real LiteRT-LM engine");
                let litert = engine_arc.lock().map_err(|e| e.to_string())?;
                let engine = litert.as_ref().ok_or("Engine disappeared")?;
                let session = engine.create_conversation().map_err(|e| e.to_string())?;
                session.send_message(&message).map_err(|e| e.to_string())
            } else {
                log::info!("send_message (desktop mock): echoing message_len={}", message.len());
                Ok(format!("Echo: {}", message))
            }
        }
    }

    /// Desktop-only streaming send_message.
    ///
    /// If a real LiteRT-LM engine is loaded, creates a conversation and
    /// streams tokens through the callback. If no engine is loaded (echo mock
    /// fallback), streams a word-by-word echo response.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
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

        // Check if we have a real engine
        let maybe_engine_arc = {
            let litert = state.litert_engine.lock().map_err(|e| e.to_string())?;
            match litert.as_ref() {
                Some(_) => Some(Arc::clone(&state.litert_engine)),
                None => None,
            }
        };

        if let Some(engine_arc) = maybe_engine_arc {
            // ---- Real LiteRT-LM inference path ----
            do_stream_real_inference(engine_arc, message, channel).await
        } else {
            // ---- Echo mock fallback path ----
            do_stream_echo_mock(message, channel).await
        }
    }

    /// Stream real LiteRT-LM inference tokens through the Tauri Channel.
    ///
    /// Creates a new conversation on the shared engine, calls
    /// `send_message_streaming` (which blocks on the C++ callback thread),
    /// and forwards each token batch as a `StreamEvent::TokenBatch`.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    async fn do_stream_real_inference(
        engine_arc: Arc<Mutex<Option<desktop::ffi::LitertEngine>>>,
        message: String,
        channel: &tauri::ipc::Channel<StreamEvent>,
    ) -> Result<(), String> {
        log::info!("do_stream_real_inference: creating conversation for message_len={}", message.len());

        // Create a conversation while holding the engine lock
        let litert_session = {
            let litert = engine_arc.lock().map_err(|e| e.to_string())?;
            match litert.as_ref() {
                Some(engine) => {
                    engine.create_conversation().map_err(|e| {
                        log::error!("do_stream_real_inference: conversation create failed — {}", e);
                        format!("Failed to create conversation: {}", e)
                    })
                }
                None => Err("Engine was dropped between start_session and send_message".into()),
            }
        }?;

        let session_id = litert_session.session_id().to_string();
        log::info!(
            "do_stream_real_inference: conversation created — session_id={}",
            session_id
        );

        // send_message_streaming blocks on the C++ callback thread,
        // so we must run it in a blocking context.
        let channel_clone = channel.clone();
        let msg_for_log = message.clone();

        let result = tokio::task::spawn_blocking(move || {
            litert_session.send_message_streaming(&message, |token_text: &str, batch_index: u32| {
                log::debug!(
                    "send_message_streaming: batch {} sent ({} chars)",
                    batch_index,
                    token_text.len()
                );
                if let Err(e) = channel_clone.send(StreamEvent::TokenBatch {
                    tokens: token_text.to_string(),
                    batch_index,
                }) {
                    log::warn!(
                        "send_message_streaming: channel.send failed for batch {} — frontend may have disconnected: {}",
                        batch_index, e
                    );
                }
            })
        }).await;

        match result {
            Ok(Ok((full_text, total_chunks))) => {
                log::info!(
                    "do_stream_real_inference: stream complete — session_id={}, total_chunks={}, total_chars={}",
                    session_id, total_chunks, full_text.len()
                );
                if let Err(e) = channel.send(StreamEvent::Complete {
                    full_text,
                    token_count: total_chunks,
                }) {
                    log::warn!(
                        "do_stream_real_inference: channel.send failed for complete event — {}",
                        e
                    );
                }
                Ok(())
            }
            Ok(Err(ffi_err)) => {
                let err_msg = format!("Inference failed: {}", ffi_err);
                log::error!(
                    "do_stream_real_inference: inference error — session_id={}, message='{}', error={}",
                    session_id, msg_for_log, ffi_err
                );
                let _ = channel.send(StreamEvent::Error {
                    message: err_msg.clone(),
                });
                Err(err_msg)
            }
            Err(join_err) => {
                let err_msg = format!("Inference task panicked: {}", join_err);
                log::error!(
                    "do_stream_real_inference: spawn_blocking panicked — session_id={}",
                    session_id
                );
                let _ = channel.send(StreamEvent::Error {
                    message: err_msg.clone(),
                });
                Err(err_msg)
            }
        }
    }

    /// Echo mock streaming — word-by-word with 80ms delays.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    async fn do_stream_echo_mock(
        message: String,
        channel: &tauri::ipc::Channel<StreamEvent>,
    ) -> Result<(), String> {
        let echo_text = format!("Echo: {}", message);
        let words: Vec<&str> = echo_text.split_whitespace().collect();
        let total_words = words.len();

        log::info!(
            "send_message_streaming (echo mock): starting stream — {} words to send in batches",
            total_words
        );

        // Group words into batches of 1–2 words
        let mut batch_index: u32 = 0;
        let mut word_iter = words.chunks(2);

        while let Some(batch) = word_iter.next() {
            let tokens = batch.join(" ");
            log::debug!(
                "send_message_streaming (echo mock): sending batch {} — tokens='{}'",
                batch_index,
                tokens
            );

            if let Err(e) = channel.send(StreamEvent::TokenBatch {
                tokens,
                batch_index,
            }) {
                log::warn!(
                    "send_message_streaming (echo mock): channel.send failed for batch {} — frontend may have disconnected: {}",
                    batch_index, e
                );
                return Ok(()); // Non-fatal: frontend disconnected
            }

            batch_index += 1;

            // ~80ms delay between batches to simulate streaming
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        }

        let full_text = echo_text.clone();
        let token_count = batch_index;
        log::info!(
            "send_message_streaming (echo mock): stream complete — token_count={}, full_text='{}'",
            token_count,
            full_text
        );

        if let Err(e) = channel.send(StreamEvent::Complete {
            full_text,
            token_count,
        }) {
            log::warn!(
                "send_message_streaming (echo mock): channel.send failed for complete event — {}",
                e
            );
        }

        Ok(())
    }

    pub fn do_get_session_status(state: &InferenceState) -> Result<SessionStatus, String> {
        let status = state.session_status.lock().map_err(|e| e.to_string())?;
        Ok(status.clone())
    }

    pub fn do_check_app_permissions<R: Runtime>(
        app: &AppHandle<R>,
    ) -> Result<serde_json::Value, String> {
        #[cfg(target_os = "android")]
        {
            let handle = app.state::<AndroidPluginHandle<R>>();
            handle.0.run_mobile_plugin("checkAppPermissions", serde_json::json!({}))
                .map_err(|e| format!("Kotlin checkAppPermissions failed: {}", e))
        }

        #[cfg(not(target_os = "android"))]
        {
            // Desktop fallback: return a mock or actual status if we had one
            Ok(serde_json::json!({
                "success": true,
                "data": {
                    "accessibility_enabled": true,
                    "accessibility_running": true,
                    "notification_enabled": true,
                    "foreground_service": true
                }
            }))
        }
    }
}

pub use session_impl::do_send_message;

// ---------------------------------------------------------------------------
// Android command wrappers (registered only on Android)
// ---------------------------------------------------------------------------

#[cfg(target_os = "android")]
#[allow(non_snake_case)]
mod android_commands {
    use super::*;

    #[tauri::command]
    pub fn startSession<R: Runtime>(
        app: AppHandle<R>,
        state: State<'_, InferenceState>,
        model_path: String,
        prefer_gpu: bool,
    ) -> Result<String, String> {
        session_impl::do_start_session(&app, &state, model_path, prefer_gpu)
    }

    #[tauri::command]
    pub fn stopSession<R: Runtime>(
        app: AppHandle<R>,
        state: State<'_, InferenceState>
    ) -> Result<(), String> {
        session_impl::do_stop_session(&app, &state)
    }

    #[tauri::command]
    pub async fn sendMessage<R: Runtime>(
        app: AppHandle<R>,
        state: State<'_, InferenceState>,
        message: String,
        on_event: tauri::ipc::Channel<StreamEvent>,
    ) -> Result<String, String> {
        let handle = app.state::<AndroidPluginHandle<R>>();
        let plugin = handle.0.clone();

        // Run blocking Kotlin IPC on a dedicated thread
        let result = tokio::task::spawn_blocking(move || {
            log::info!("send_message (Android/spawn_blocking): invoking Kotlin sendMessage — message_len={}", message.len());

            let args = serde_json::json!({
                "message": message,
            });

            let result: serde_json::Value = plugin.run_mobile_plugin("sendMessage", args)
                .map_err(|e| format!("Kotlin sendMessage failed: {}", e))?;

            let response = result.get("response")
                .and_then(|v: &serde_json::Value| v.as_str())
                .ok_or_else(|| "Kotlin sendMessage returned no response string".to_string())?;

            Ok::<String, String>(response.to_string())
        })
        .await
        .map_err(|e| format!("spawn_blocking panicked: {}", e))?;

        // Send result through the streaming channel (must be outside spawn_blocking)
        match &result {
            Ok(text) => {
                let _ = on_event.send(StreamEvent::TokenBatch { tokens: text.clone(), batch_index: 0 });
                let _ = on_event.send(StreamEvent::Complete { full_text: text.clone(), token_count: 0 });
            }
            Err(e) => {
                let _ = on_event.send(StreamEvent::Error { message: e.clone() });
            }
        }

        result
    }

    #[tauri::command]
    pub fn getSessionStatus(state: State<'_, InferenceState>) -> Result<SessionStatus, String> {
        session_impl::do_get_session_status(&state)
    }

    #[tauri::command]
    pub fn checkAppPermissions<R: Runtime>(app: AppHandle<R>) -> Result<serde_json::Value, String> {
        session_impl::do_check_app_permissions(&app)
    }

    #[tauri::command]
    pub fn list_models<R: Runtime>(app: AppHandle<R>) -> Result<serde_json::Value, String> {
        let handle = app.state::<AndroidPluginHandle<R>>();
        handle.0.run_mobile_plugin("listModels", serde_json::json!({}))
            .map_err(|e| format!("Kotlin listModels failed: {}", e))
    }

    #[tauri::command]
    pub async fn download_model<R: Runtime>(
        app: AppHandle<R>,
        model_id: String,
        on_progress: tauri::ipc::Channel<DownloadEvent>,
    ) -> Result<(), String> {
        let handle = app.state::<AndroidPluginHandle<R>>();
        handle.0.run_mobile_plugin::<serde_json::Value>(
            "downloadModel",
            serde_json::json!({ "modelId": model_id, "onProgress": on_progress })
        ).map_err(|e| format!("Kotlin downloadModel failed: {}", e))?;
        Ok(())
    }

    #[tauri::command]
    pub fn pick_model_file<R: Runtime>(app: AppHandle<R>) -> Result<serde_json::Value, String> {
        let handle = app.state::<AndroidPluginHandle<R>>();
        handle.0.run_mobile_plugin("pickModelFile", serde_json::json!({}))
            .map_err(|e| format!("Kotlin pickModelFile failed: {}", e))
    }

    #[tauri::command]
    pub fn get_saf_folder_status<R: Runtime>(app: AppHandle<R>) -> Result<serde_json::Value, String> {
        let handle = app.state::<AndroidPluginHandle<R>>();
        handle.0.run_mobile_plugin("getSafFolderStatus", serde_json::json!({}))
            .map_err(|e| format!("Kotlin getSafFolderStatus failed: {}", e))
    }

    #[tauri::command]
    pub fn pick_saf_folder<R: Runtime>(app: AppHandle<R>) -> Result<serde_json::Value, String> {
        let handle = app.state::<AndroidPluginHandle<R>>();
        handle.0.run_mobile_plugin("pickSafFolder", serde_json::json!({}))
            .map_err(|e| format!("Kotlin pickSafFolder failed: {}", e))
    }

    #[tauri::command]
    pub fn list_saf_models<R: Runtime>(app: AppHandle<R>) -> Result<serde_json::Value, String> {
        let handle = app.state::<AndroidPluginHandle<R>>();
        handle.0.run_mobile_plugin("listSafModels", serde_json::json!({}))
            .map_err(|e| format!("Kotlin listSafModels failed: {}", e))
    }

    #[tauri::command]
    pub fn cache_saf_model<R: Runtime>(app: AppHandle<R>, saf_uri: String) -> Result<serde_json::Value, String> {
        let handle = app.state::<AndroidPluginHandle<R>>();
        handle.0.run_mobile_plugin("cacheSafModel", serde_json::json!({ "safUri": saf_uri }))
            .map_err(|e| format!("Kotlin cacheSafModel failed: {}", e))
    }

    #[tauri::command]
    pub fn get_screen_info<R: Runtime>(app: AppHandle<R>) -> Result<serde_json::Value, String> {
        let handle = app.state::<AndroidPluginHandle<R>>();
        handle.0.run_mobile_plugin("getScreenInfo", serde_json::json!({}))
            .map_err(|e| format!("Kotlin getScreenInfo failed: {}", e))
    }

    #[tauri::command]
    pub fn take_screenshot<R: Runtime>(app: AppHandle<R>) -> Result<serde_json::Value, String> {
        let handle = app.state::<AndroidPluginHandle<R>>();
        handle.0.run_mobile_plugin("takeScreenshot", serde_json::json!({}))
            .map_err(|e| format!("Kotlin takeScreenshot failed: {}", e))
    }

    #[tauri::command]
    pub fn pick_save_location<R: Runtime>(app: AppHandle<R>, file_name: String) -> Result<serde_json::Value, String> {
        let handle = app.state::<AndroidPluginHandle<R>>();
        handle.0.run_mobile_plugin("pickSaveLocation", serde_json::json!({ "fileName": file_name }))
            .map_err(|e| format!("Kotlin pickSaveLocation failed: {}", e))
    }

    #[tauri::command]
    pub fn download_to_saf<R: Runtime>(
        app: AppHandle<R>,
        url: String,
        saf_uri: String,
        on_progress: tauri::ipc::Channel<DownloadEvent>,
    ) -> Result<(), String> {
        let handle = app.state::<AndroidPluginHandle<R>>();
        handle.0.run_mobile_plugin::<serde_json::Value>(
            "downloadToSaf",
            serde_json::json!({ "url": url, "safUri": saf_uri, "onProgress": on_progress })
        ).map_err(|e| format!("Kotlin downloadToSaf failed: {}", e))?;
        Ok(())
    }

    #[tauri::command]
    pub fn download_to_saf_folder<R: Runtime>(
        app: AppHandle<R>,
        url: String,
        file_name: String,
        on_progress: tauri::ipc::Channel<DownloadEvent>,
    ) -> Result<(), String> {
        let handle = app.state::<AndroidPluginHandle<R>>();
        handle.0.run_mobile_plugin::<serde_json::Value>(
            "downloadToSafFolder",
            serde_json::json!({ "url": url, "fileName": file_name, "onProgress": on_progress })
        ).map_err(|e| format!("Kotlin downloadToSafFolder failed: {}", e))?;
        Ok(())
    }

    #[tauri::command]
    pub fn system_key<R: Runtime>(app: AppHandle<R>, action: String) -> Result<serde_json::Value, String> {
        let handle = app.state::<AndroidPluginHandle<R>>();
        handle.0.run_mobile_plugin("systemKey", serde_json::json!({ "action": action }))
            .map_err(|e| format!("Kotlin systemKey failed: {}", e))
    }

    #[tauri::command]
    pub fn clipboard<R: Runtime>(app: AppHandle<R>, action: String, text: Option<String>) -> Result<serde_json::Value, String> {
        let handle = app.state::<AndroidPluginHandle<R>>();
        handle.0.run_mobile_plugin("clipboard", serde_json::json!({ "action": action, "text": text }))
            .map_err(|e| format!("Kotlin clipboard failed: {}", e))
    }

    #[tauri::command]
    pub fn chat(message: String) -> String {
        log::info!("chat command received: {}", message);
        format!("Echo: {}", message)
    }
}

// ---------------------------------------------------------------------------
// iOS command wrappers (registered only on iOS)
// ---------------------------------------------------------------------------

#[cfg(target_os = "ios")]
#[allow(non_snake_case)]
mod ios_commands {
    use super::*;

    #[tauri::command]
    pub fn startSession(
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
    pub async fn sendMessage(
        app: AppHandle<R>,
        state: State<'_, InferenceState>,
        message: String,
        on_event: Option<tauri::ipc::Channel<StreamEvent>>,
    ) -> Result<String, String> {
        let handle: AndroidPluginHandle<R> = app.state::<AndroidPluginHandle<R>>().0.clone();

        let result = tokio::task::spawn_blocking(move || {
            log::info!("send_message (Android): invoking Kotlin sendMessage — message_len={}", message.len());

            let args = serde_json::json!({
                "message": message,
            });

            let result: serde_json::Value = handle.run_mobile_plugin("sendMessage", args)
                .map_err(|e| format!("Kotlin sendMessage failed: {}", e))?;

            let response = result.get("response")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Kotlin sendMessage returned no response string".to_string())?;

            Ok(response.to_string())
        })
        .await
        .map_err(|e| format!("spawn_blocking panicked: {}", e))?;

        // If frontend passed a channel (streaming mode), send events through it
        if let Some(channel) = on_event {
            match &result {
                Ok(text) => {
                    let _ = channel.send(StreamEvent::TokenBatch { tokens: text.clone(), batch_index: 0 });
                    let _ = channel.send(StreamEvent::Complete { full_text: text.clone(), token_count: 0 });
                }
                Err(e) => {
                    let _ = channel.send(StreamEvent::Error { message: e.clone() });
                }
            }
        }

        result
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
// Desktop command wrappers (registered only on desktop)
// ---------------------------------------------------------------------------

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[allow(non_snake_case)]
mod desktop_commands {
    use super::*;

    #[tauri::command]
    pub fn startSession(
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
    pub async fn sendMessage(
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

    /// Desktop list_models. Returns the model catalog enriched with real
    /// filesystem status — scans the models directory for `.litertlm` files
    /// and sets `is_downloaded` and `local_path` accordingly.
    #[tauri::command]
    pub fn list_models() -> Vec<ModelInfo> {
        desktop::model_manager::list_models()
    }

    /// Desktop download_model. Downloads a `.litertlm` file from HuggingFace
    /// with real progress tracking (bytes downloaded, total, speed), saves to
    /// the models directory, and sends `DownloadEvent` progress events via Channel.
    #[tauri::command]
    pub async fn download_model(
        model_id: String,
        on_progress: tauri::ipc::Channel<DownloadEvent>,
    ) -> Result<(), String> {
        desktop::model_manager::download_model(&model_id, &on_progress).await
    }

    /// Desktop download_model_from_url. Downloads a model from an arbitrary URL
    /// to a user-chosen directory with real progress tracking.
    #[tauri::command]
    pub async fn download_model_from_url(
        url: String,
        save_dir: String,
        on_progress: tauri::ipc::Channel<DownloadEvent>,
    ) -> Result<(), String> {
        desktop::model_manager::download_model_from_url(&url, &save_dir, &on_progress).await
    }

    // -----------------------------------------------------------------
    // Observation tool desktop commands
    // -----------------------------------------------------------------

    /// Desktop real get_screen_info.
    /// Enumerates all visible windows and returns a formatted tree string.
    #[tauri::command]
    pub fn get_screen_info() -> ToolResult {
        desktop::screen::do_get_screen_info()
    }

    /// Desktop real find_node_info.
    /// Finds windows matching the given text and returns their metadata as JSON nodes.
    #[tauri::command]
    pub fn find_node_info(text: String) -> ToolResult {
        desktop::screen::do_find_node_info(text)
    }

    /// Desktop real get_device_info.
    /// Delegates to desktop::system for real OS/hardware info (CPU, RAM, disk, OS).
    #[tauri::command]
    pub fn get_device_info(category: String) -> ToolResult {
        log::info!("get_device_info: delegating to desktop::system — category='{}'", category);
        desktop::system::do_get_device_info(&category)
    }

    /// Desktop real check_permissions.
    /// Delegates to desktop::system for real OS permission status.
    #[tauri::command]
    pub fn check_permissions() -> ToolResult {
        log::info!("check_permissions: delegating to desktop::system");
        desktop::system::do_check_permissions()
    }

    // -----------------------------------------------------------------
    // Gesture tool desktop mocks
    // -----------------------------------------------------------------

    /// Desktop real tap. Delegates to desktop::automation for real OS-level click.
    #[tauri::command]
    pub fn tap(x: i32, y: i32) -> ToolResult {
        log::info!("tap: delegating to desktop::automation — x={}, y={}", x, y);
        desktop::automation::do_tap(x, y)
    }

    /// Desktop real swipe. Delegates to desktop::automation for real OS-level mouse drag.
    #[tauri::command]
    pub fn swipe(start_x: i32, start_y: i32, end_x: i32, end_y: i32, duration_ms: Option<i32>) -> ToolResult {
        log::info!(
            "swipe: delegating to desktop::automation — ({},{}) -> ({},{})",
            start_x, start_y, end_x, end_y
        );
        desktop::automation::do_swipe(start_x, start_y, end_x, end_y, duration_ms)
    }

    /// Desktop real long_press. Delegates to desktop::automation for real OS-level click-and-hold.
    #[tauri::command]
    pub fn long_press(x: i32, y: i32, duration_ms: Option<i32>) -> ToolResult {
        log::info!(
            "long_press: delegating to desktop::automation — ({},{})",
            x, y
        );
        desktop::automation::do_long_press(x, y, duration_ms)
    }

    /// Desktop mock for tap_node. Not supported on desktop.
    #[tauri::command]
    pub fn tap_node(_node_id: String) -> ToolResult {
        log::info!("tap_node (desktop): Not supported");
        ToolResult {
            success: false,
            data: None,
            error: Some("tap_node is not supported on desktop. Use coordinate-based tap instead.".into()),
        }
    }

    /// Desktop real input_text. Delegates to desktop::automation for real OS-level keyboard input.
    #[tauri::command]
    pub fn input_text(text: String, node_id: Option<String>, clear_first: Option<bool>) -> ToolResult {
        log::info!(
            "input_text: delegating to desktop::automation — text='{}' ({} chars)",
            text, text.len()
        );
        desktop::automation::do_input_text(&text, node_id.as_deref(), clear_first)
    }

    /// Desktop mock for scroll_to_find. Not supported on desktop.
    #[tauri::command]
    pub fn scroll_to_find(_text: String, _direction: Option<String>, _max_scrolls: Option<i32>) -> ToolResult {
        log::info!("scroll_to_find (desktop): Not supported");
        ToolResult {
            success: false,
            data: None,
            error: Some("scroll_to_find is not supported on desktop. Use manual scrolling and find_node_info instead.".into()),
        }
    }

    /// Desktop mock for find_and_tap. Not supported on desktop.
    #[tauri::command]
    pub fn find_and_tap(_text: String, _direction: Option<String>, _max_scrolls: Option<i32>) -> ToolResult {
        log::info!("find_and_tap (desktop): Not supported");
        ToolResult {
            success: false,
            data: None,
            error: Some("find_and_tap is not supported on desktop. Use find_node_info followed by tap instead.".into()),
        }
    }

    // -----------------------------------------------------------------
    // New tool desktop mocks (T04 — S03)
    // -----------------------------------------------------------------

    /// Desktop mock for get_notifications. Not supported on desktop.
    #[tauri::command]
    pub fn get_notifications() -> ToolResult {
        log::info!("get_notifications (desktop): Not supported");
        ToolResult {
            success: false,
            data: None,
            error: Some("get_notifications is not supported on desktop.".into()),
        }
    }

    /// Desktop real open_app. Launches an app by name or path.
    #[tauri::command]
    pub fn open_app(app_name: String) -> ToolResult {
        log::info!("open_app: delegating to desktop::automation — app_name='{}'", app_name);
        desktop::automation::do_open_app(&app_name)
    }

    /// Desktop real system_key. Delegates to desktop::automation for real OS-level key press.
    #[tauri::command]
    pub fn system_key(action: String) -> ToolResult {
        log::info!("system_key: delegating to desktop::automation — action='{}'", action);
        desktop::automation::do_system_key(&action)
    }

    /// Desktop mock for send_chat_message. Not supported on desktop.
    #[tauri::command]
    pub fn send_chat_message(
        _app: String,
        _contact: String,
        _message: String,
    ) -> ToolResult {
        log::info!("send_chat_message (desktop): Not supported");
        ToolResult {
            success: false,
            data: None,
            error: Some("send_chat_message is not supported on desktop. Use manual app automation if needed.".into()),
        }
    }

    /// Desktop real take_screenshot.
    /// Captures a screenshot of the primary monitor and saves it to the specified path.
    #[tauri::command]
    pub fn take_screenshot(file_path: Option<String>) -> ToolResult {
        desktop::screen::do_take_screenshot(file_path.as_deref())
    }

    /// Desktop real clipboard. Delegates to desktop::system for real OS clipboard access.
    #[tauri::command]
    pub fn clipboard(action: String, text: Option<String>) -> ToolResult {
        log::info!("clipboard: delegating to desktop::system — action='{}'", action);
        let action_lower = action.to_lowercase();
        match action_lower.as_str() {
            "get" => desktop::system::do_clipboard_get(),
            "set" => desktop::system::do_clipboard_set(&text.unwrap_or_default()),
            _ => ToolResult {
                success: false,
                data: None,
                error: Some(format!(
                    "Unknown clipboard action '{}'. Supported: get, set",
                    action
                )),
            },
        }
    }

    /// Desktop real get_installed_apps.
    /// Enumerates currently running apps by checking open windows.
    #[tauri::command]
    pub fn get_installed_apps(filter: Option<String>) -> ToolResult {
        log::info!("get_installed_apps: delegating to desktop::screen — filter={:?}", filter);
        desktop::screen::do_get_installed_apps(filter.as_deref())
    }

    /// Desktop mock for make_call. Not supported on desktop.
    #[tauri::command]
    pub fn make_call(_target: String) -> ToolResult {
        log::info!("make_call (desktop): Not supported");
        ToolResult {
            success: false,
            data: None,
            error: Some("make_call is not supported on desktop.".into()),
        }
    }

    /// Desktop mock for open_permission_settings. Not supported on desktop.
    #[tauri::command]
    pub fn open_permission_settings(_target: String) -> ToolResult {
        log::info!("open_permission_settings (desktop): Not supported");
        ToolResult {
            success: false,
            data: None,
            error: Some("open_permission_settings is not supported on desktop. Permissions are managed by the OS.".into()),
        }
    }

    // -----------------------------------------------------------------
    // SAF command desktop stubs (Android-only, desktop uses file dialog)
    // -----------------------------------------------------------------

    /// Desktop stub for pick_save_location. Uses Tauri dialog save instead.
    #[tauri::command]
    pub async fn pick_save_location(file_name: String) -> Result<String, String> {
        log::info!("pick_save_location (desktop): fileName='{}'", file_name);
        // On desktop, we don't need SAF — return empty to signal frontend to use file dialog
        Err("Not available on desktop. Use the file dialog instead.".into())
    }

    /// Desktop stub for pick_model_file. Uses Tauri dialog open instead.
    #[tauri::command]
    pub fn pickModelFile() -> Result<String, String> {
        log::info!("pick_model_file (desktop): Not available");
        Err("Not available on desktop. Use the file dialog instead.".into())
    }

    /// Desktop stub for download_to_saf. Not needed on desktop.
    #[tauri::command]
    pub fn downloadToSaf(_url: String, _saf_uri: String) -> Result<String, String> {
        Err("Not available on desktop. Use download_model_from_url instead.".into())
    }

    /// Desktop stub for saf_download_model. Not needed on desktop.
    #[tauri::command]
    pub fn safDownloadModel(_url: String, _file_name: String) -> Result<String, String> {
        Err("Not available on desktop. Use download_model_from_url instead.".into())
    }

    // SAF shared-folder desktop stubs (Android-only)
    #[tauri::command]
    pub fn pickSafFolder() -> Result<String, String> {
        Err("Not available on desktop. Use file dialog instead.".into())
    }

    #[tauri::command]
    pub fn getSafFolderStatus() -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({ "hasPermission": false, "folderUri": null, "folderName": null }))
    }

    #[tauri::command]
    pub fn listSafModels() -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({ "models": [] }))
    }

    #[tauri::command]
    pub fn cacheSafModel(_saf_uri: String) -> Result<String, String> {
        Err("Not available on desktop.".into())
    }

    #[tauri::command]
    pub async fn downloadToSafFolder(_url: String, _file_name: String) -> Result<String, String> {
        Err("Not available on desktop. Use download_model_from_url instead.".into())
    }

    // -----------------------------------------------------------------
    // Live Activity desktop mocks
    // -----------------------------------------------------------------

    /// Desktop mock for start_live_activity. Live Activities are iOS-only.
    #[tauri::command]
    pub fn start_live_activity(_title: String) -> ToolResult {
        log::info!("start_live_activity (desktop): Not supported");
        ToolResult {
            success: false,
            data: None,
            error: Some("Live Activities are not available on desktop".into()),
        }
    }

    /// Desktop mock for update_live_activity. Live Activities are iOS-only.
    #[tauri::command]
    pub fn update_live_activity(_step: i32, _total_steps: i32, _description: String, _status: String) -> ToolResult {
        log::info!("update_live_activity (desktop): Not supported");
        ToolResult {
            success: false,
            data: None,
            error: Some("Live Activities are not available on desktop".into()),
        }
    }

    /// Desktop mock for stop_live_activity. Live Activities are iOS-only.
    #[tauri::command]
    pub fn stop_live_activity() -> ToolResult {
        log::info!("stop_live_activity (desktop): Not supported");
        ToolResult {
            success: false,
            data: None,
            error: Some("Live Activities are not available on desktop".into()),
        }
    }

    #[tauri::command]
    pub fn checkAppPermissions<R: Runtime>(app: AppHandle<R>) -> Result<serde_json::Value, String> {
        session_impl::do_check_app_permissions(&app)
    }
}

// ---------------------------------------------------------------------------
// AndroidPluginHandle — wraps the Tauri PluginHandle for managed state
// ---------------------------------------------------------------------------

/// Wrapper around Tauri's `PluginHandle` that can be stored in managed state.
/// This allows `AndroidToolExecutor` to retrieve it at runtime and dispatch
/// tool calls to Kotlin @Command methods via `run_mobile_plugin()`.
#[cfg(target_os = "android")]
pub struct AndroidPluginHandle<R: Runtime>(pub tauri::plugin::PluginHandle<R>);

#[cfg(target_os = "android")]
impl<R: Runtime> Clone for AndroidPluginHandle<R> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

// ---------------------------------------------------------------------------
// Plugin initialization
// ---------------------------------------------------------------------------

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    #[cfg(target_os = "ios")]
    tauri::ios_plugin_binding!(init_plugin_pokeclaw);

    let builder = Builder::new("pokeclaw");

    let builder = builder
        .setup(|app, _api| {
            app.manage(InferenceState::default());
            #[cfg(target_os = "ios")]
            _api.register_ios_plugin(init_plugin_pokeclaw)?;
            #[cfg(target_os = "android")]
            {
                let plugin_handle = _api.register_android_plugin("io.agents.pokeclaw", "PokeclawPlugin")?;
                app.manage(AndroidPluginHandle(plugin_handle));
                log::info!("init: AndroidPluginHandle stored in managed state");
            }
            Ok(())
        });

    #[cfg(target_os = "android")]
    let builder = builder
        .invoke_handler(tauri::generate_handler![
            android_commands::startSession,
            android_commands::stopSession,
            android_commands::sendMessage,
            android_commands::getSessionStatus,
            android_commands::checkAppPermissions,
            android_commands::list_models,
            android_commands::download_model,
            android_commands::pick_model_file,
            android_commands::get_saf_folder_status,
            android_commands::pick_saf_folder,
            android_commands::list_saf_models,
            android_commands::cache_saf_model,
            android_commands::get_screen_info,
            android_commands::take_screenshot,
            android_commands::system_key,
            android_commands::clipboard,
            android_commands::pick_save_location,
            android_commands::download_to_saf,
            android_commands::download_to_saf_folder,
            android_commands::chat,
        ]);

    #[cfg(target_os = "ios")]
    let builder = builder
        .invoke_handler(tauri::generate_handler![
            ios_commands::startSession,
            ios_commands::stopSession,
            ios_commands::sendMessage,
            ios_commands::getSessionStatus,
            ios_commands::chat,
        ]);

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.invoke_handler(tauri::generate_handler![
            desktop_commands::startSession,
            desktop_commands::stopSession,
            desktop_commands::sendMessage,
            desktop_commands::getSessionStatus,
            desktop_commands::ping,
            desktop_commands::chat,
            desktop_commands::list_models,
            desktop_commands::download_model,
            desktop_commands::download_model_from_url,
            desktop_commands::pick_save_location,
            desktop_commands::pick_model_file,
            desktop_commands::download_to_saf,
            desktop_commands::saf_download_model,
            desktop_commands::pick_saf_folder,
            desktop_commands::get_saf_folder_status,
            desktop_commands::list_saf_models,
            desktop_commands::cache_saf_model,
            desktop_commands::download_to_saf_folder,
            desktop_commands::get_screen_info,
            desktop_commands::find_node_info,
            desktop_commands::get_device_info,
            desktop_commands::checkAppPermissions,
            desktop_commands::tap,
            desktop_commands::swipe,
            desktop_commands::long_press,
            desktop_commands::tap_node,
            desktop_commands::input_text,
            desktop_commands::scroll_to_find,
            desktop_commands::find_and_tap,
            desktop_commands::get_notifications,
            desktop_commands::open_app,
            desktop_commands::system_key,
            desktop_commands::send_chat_message,
            desktop_commands::take_screenshot,
            desktop_commands::clipboard,
            desktop_commands::get_installed_apps,
            desktop_commands::make_call,
            desktop_commands::open_permission_settings,
            desktop_commands::start_live_activity,
            desktop_commands::update_live_activity,
            desktop_commands::stop_live_activity,
        ]);

    builder.build()
}
