use std::sync::Mutex;
use tauri::{
    plugin::{Builder, TauriPlugin},
    Runtime, State,
};

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
    pub fn ping() -> String {
        "pong from rust".into()
    }

    #[tauri::command]
    pub fn chat(message: String) -> String {
        log::info!("chat command received: {}", message);
        format!("Echo: {}", message)
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
        ]);

    builder.build()
}
