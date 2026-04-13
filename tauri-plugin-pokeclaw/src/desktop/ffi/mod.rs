// desktop/ffi/mod.rs — Safe Rust FFI bindings for LiteRT-LM C++ shim.
//
// Loads `litertlm_bridge.dll` / `.so` / `.dylib` at runtime via `libloading`
// and wraps the raw FFI calls behind `LitertEngine` and `LitertSession`.
// All methods return `Result<T, LitertError>` with descriptive diagnostics.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::Path;
use std::sync::Arc;

use libloading::Library;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during LiteRT-LM FFI operations.
#[derive(Debug)]
pub enum LitertError {
    /// The shared library could not be found or loaded.
    /// Contains the searched path and the OS error.
    LibraryNotFound {
        path: String,
        source: libloading::Error,
    },
    /// Engine creation failed (model load, backend init, etc.).
    EngineCreateFailed {
        model_path: String,
        detail: String,
    },
    /// Conversation creation failed.
    ConversationCreateFailed(String),
    /// Inference failed during send_message.
    InferenceFailed {
        session_id: String,
        detail: String,
    },
    /// The model file path is invalid or empty.
    InvalidModelPath(String),
    /// A session method was called with no active session.
    NoActiveSession,
}

impl std::fmt::Display for LitertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LibraryNotFound { path, source } => {
                write!(
                    f,
                    "LiteRT-LM library not found at '{}': {}. \
                     Build the C++ shim first: cd src/desktop/ffi && mkdir -p build && cd build && cmake .. && cmake --build .",
                    path, source
                )
            }
            Self::EngineCreateFailed { model_path, detail } => {
                write!(
                    f,
                    "Engine creation failed for model_path='{}': {}",
                    model_path, detail
                )
            }
            Self::ConversationCreateFailed(detail) => {
                write!(f, "Conversation creation failed: {}", detail)
            }
            Self::InferenceFailed { session_id, detail } => {
                write!(
                    f,
                    "Inference failed for session_id='{}': {}",
                    session_id, detail
                )
            }
            Self::InvalidModelPath(path) => {
                write!(f, "Invalid model path: '{}'", path)
            }
            Self::NoActiveSession => {
                write!(f, "No active session. Call create_engine first.")
            }
        }
    }
}

impl std::error::Error for LitertError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::LibraryNotFound { source, .. } => Some(source),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// FFI types matching the C shim header (litertlm_shim.h)
// ---------------------------------------------------------------------------

/// Opaque engine handle from the C shim.
type LlmEngine = std::ffi::c_void;

/// Opaque conversation handle from the C shim.
type LlmConversation = std::ffi::c_void;

/// Callback type matching `LitertlmTokenCallback` in the C header.
type LitertlmTokenCallback = unsafe extern "C" fn(
    chunk_text: *const c_char,
    chunk_len: u32,
    is_final: i32,
    user_data: *mut std::ffi::c_void,
);

// ---------------------------------------------------------------------------
// FFI function signatures (loaded dynamically at runtime)
// ---------------------------------------------------------------------------

type FnEngineCreate = unsafe extern "C" fn(*const c_char, *const c_char) -> *mut LlmEngine;
type FnEngineDestroy = unsafe extern "C" fn(*mut LlmEngine);
type FnConversationCreate = unsafe extern "C" fn(*mut LlmEngine) -> *mut LlmConversation;
type FnConversationDestroy = unsafe extern "C" fn(*mut LlmConversation);
type FnSendMessage = unsafe extern "C" fn(*mut LlmConversation, *const c_char, *mut c_char, u32) -> u32;
type FnSendMessageAsync = unsafe extern "C" fn(*mut LlmConversation, *const c_char, LitertlmTokenCallback, *mut std::ffi::c_void) -> i32;
type FnGetLastError = unsafe extern "C" fn() -> *const c_char;

// ---------------------------------------------------------------------------
// Resolved FFI symbols — cached after library load
// ---------------------------------------------------------------------------

struct FfiSymbols {
    engine_create: FnEngineCreate,
    engine_destroy: FnEngineDestroy,
    conversation_create: FnConversationCreate,
    conversation_destroy: FnConversationDestroy,
    send_message: FnSendMessage,
    send_message_async: FnSendMessageAsync,
    get_last_error: FnGetLastError,
}

impl FfiSymbols {
    /// Load all required symbols from the shared library.
    ///
    /// # Safety
    /// The library must export symbols with the exact names and signatures
    /// declared in litertlm_shim.h.
    unsafe fn load(lib: &Library) -> Result<Self, LitertError> {
        let engine_create = *lib
            .get(b"litertlm_engine_create\0")
            .map_err(|e| LitertError::LibraryNotFound {
                path: "symbol: litertlm_engine_create".into(),
                source: e,
            })?;
        let engine_destroy = *lib
            .get(b"litertlm_engine_destroy\0")
            .map_err(|e| LitertError::LibraryNotFound {
                path: "symbol: litertlm_engine_destroy".into(),
                source: e,
            })?;
        let conversation_create = *lib
            .get(b"litertlm_conversation_create\0")
            .map_err(|e| LitertError::LibraryNotFound {
                path: "symbol: litertlm_conversation_create".into(),
                source: e,
            })?;
        let conversation_destroy = *lib
            .get(b"litertlm_conversation_destroy\0")
            .map_err(|e| LitertError::LibraryNotFound {
                path: "symbol: litertlm_conversation_destroy".into(),
                source: e,
            })?;
        let send_message = *lib
            .get(b"litertlm_send_message\0")
            .map_err(|e| LitertError::LibraryNotFound {
                path: "symbol: litertlm_send_message".into(),
                source: e,
            })?;
        let send_message_async = *lib
            .get(b"litertlm_send_message_async\0")
            .map_err(|e| LitertError::LibraryNotFound {
                path: "symbol: litertlm_send_message_async".into(),
                source: e,
            })?;
        let get_last_error = *lib
            .get(b"litertlm_get_last_error\0")
            .map_err(|e| LitertError::LibraryNotFound {
                path: "symbol: litertlm_get_last_error".into(),
                source: e,
            })?;

        Ok(Self {
            engine_create,
            engine_destroy,
            conversation_create,
            conversation_destroy,
            send_message,
            send_message_async,
            get_last_error,
        })
    }
}

// ---------------------------------------------------------------------------
// LitertSession — lightweight conversation wrapper
// ---------------------------------------------------------------------------

/// A conversation session bound to a loaded engine.
///
/// Wraps an opaque `LlmConversation*` handle. Dropping this struct calls
/// `litertlm_conversation_destroy`. Safe to Clone (shared Arc to the engine).
pub struct LitertSession {
    conversation: *mut LlmConversation,
    session_id: String,
    backend: String,
    // Keep engine alive while session exists.
    _engine_ref: Arc<LitertEngineInner>,
}

// The raw pointer is only accessed through FFI calls (unsafe) which require
// a valid engine. The Arc<LitertEngineInner> ensures the engine outlives
// the session.
unsafe impl Send for LitertSession {}
unsafe impl Sync for LitertSession {}

impl LitertSession {
    /// Unique session identifier (generated on creation).
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Backend used for inference ("cpu", "gpu", or "npu").
    pub fn backend(&self) -> &str {
        &self.backend
    }

    /// Send a message synchronously and return the full response text.
    ///
    /// Allocates a 64 KB response buffer. If the model response exceeds this,
    /// it is truncated.
    pub fn send_message(&self, message: &str) -> Result<String, LitertError> {
        log::info!(
            "litertlm_send_message: session_id={}, message_len={}",
            self.session_id,
            message.len()
        );

        let ffi = &self._engine_ref.symbols;

        let c_message = CString::new(message).map_err(|_| {
            LitertError::InferenceFailed {
                session_id: self.session_id.clone(),
                detail: "Message contains NUL byte".into(),
            }
        })?;

        let mut buf = vec![0u8; 65536];

        let written = unsafe {
            (ffi.send_message)(
                self.conversation,
                c_message.as_ptr(),
                buf.as_mut_ptr() as *mut c_char,
                buf.len() as u32 - 1, // leave room for NUL
            )
        };

        if written == 0 {
            let err = self.get_last_error();
            log::error!(
                "litertlm_send_message: failed — session_id={}, error={}",
                self.session_id,
                err
            );
            return Err(LitertError::InferenceFailed {
                session_id: self.session_id.clone(),
                detail: err,
            });
        }

        buf.truncate(written as usize);
        let text = String::from_utf8_lossy(&buf).to_string();

        log::info!(
            "litertlm_send_message: complete — session_id={}, response_len={}",
            self.session_id,
            text.len()
        );

        Ok(text)
    }

    /// Send a message with streaming tokens via a callback.
    ///
    /// The callback is invoked on an internal C++ thread for each token chunk,
    /// and once more with `is_final=true` and empty text to signal completion.
    /// This method **blocks** until streaming is complete.
    ///
    /// `on_token` is called for each non-final chunk with `(text, batch_index)`.
    /// Returns `(full_text, total_chunks)` on success.
    pub fn send_message_streaming<F>(
        &self,
        message: &str,
        on_token: F,
    ) -> Result<(String, u32), LitertError>
    where
        F: FnMut(&str, u32),
    {
        log::info!(
            "litertlm_send_message_async: session_id={}, message_len={}",
            self.session_id,
            message.len()
        );

        let ffi = &self._engine_ref.symbols;

        let c_message = CString::new(message).map_err(|_| {
            LitertError::InferenceFailed {
                session_id: self.session_id.clone(),
                detail: "Message contains NUL byte".into(),
            }
        })?;

        // State shared between the callback closure and this method.
        // Boxed and passed as user_data to the C callback.
        struct StreamState<F: FnMut(&str, u32)> {
            on_token: F,
            batch_index: u32,
            accumulated: String,
            error: Option<String>,
        }

        let state = Box::new(StreamState {
            on_token,
            batch_index: 0,
            accumulated: String::new(),
            error: None,
        });
        let state_ptr = Box::into_raw(state) as *mut std::ffi::c_void;

        // Trampoline callback: adapts the C callback signature to Rust closure.
        unsafe extern "C" fn trampoline<F: FnMut(&str, u32)>(
            chunk_text: *const c_char,
            chunk_len: u32,
            is_final: i32,
            user_data: *mut std::ffi::c_void,
        ) {
            let state = &mut *(user_data as *mut StreamState<F>);

            if is_final != 0 {
                // End-of-stream signal — nothing to do here, the caller
                // will reconstruct state and return the accumulated text.
                return;
            }

            if chunk_len > 0 && !chunk_text.is_null() {
                let slice = std::slice::from_raw_parts(chunk_text as *const u8, chunk_len as usize);
                let text = String::from_utf8_lossy(slice);

                log::debug!(
                    "litertlm_send_message_async: batch {} sent ({} chars)",
                    state.batch_index,
                    text.len()
                );

                (state.on_token)(&text, state.batch_index);
                state.accumulated.push_str(&text);
                state.batch_index += 1;
            } else if chunk_text.is_null() || chunk_len == 0 {
                // Could be an error callback — check if text is non-null for error
                // In the C++ shim, error is sent as non-empty text with is_final=0
                // so this branch handles unexpected null/empty non-final chunks
            }
        }

        let result = unsafe {
            (ffi.send_message_async)(
                self.conversation,
                c_message.as_ptr(),
                trampoline::<F>,
                state_ptr,
            )
        };

        // Recover the state
        let state = unsafe { Box::from_raw(state_ptr as *mut StreamState<F>) };

        if result != 0 {
            let err = self.get_last_error();
            log::error!(
                "litertlm_send_message_async: failed — session_id={}, error={}",
                self.session_id,
                err
            );
            return Err(LitertError::InferenceFailed {
                session_id: self.session_id.clone(),
                detail: err,
            });
        }

        let total = state.batch_index;
        let full_text = state.accumulated.clone();

        log::info!(
            "litertlm_send_message_async: complete — session_id={}, total_chunks={}, total_chars={}",
            self.session_id,
            total,
            full_text.len()
        );

        Ok((full_text, total))
    }

    /// Retrieve the last error from the C shim.
    fn get_last_error(&self) -> String {
        let ffi = &self._engine_ref.symbols;
        unsafe {
            let ptr = (ffi.get_last_error)();
            if ptr.is_null() {
                return "(no error recorded)".into();
            }
            CStr::from_ptr(ptr)
                .to_string_lossy()
                .into_owned()
        }
    }
}

impl Drop for LitertSession {
    fn drop(&mut self) {
        log::info!(
            "LitertSession::drop: destroying conversation — session_id={}",
            self.session_id
        );
        let ffi = &self._engine_ref.symbols;
        unsafe {
            (ffi.conversation_destroy)(self.conversation);
        }
    }
}

// ---------------------------------------------------------------------------
// LitertEngineInner — holds the library + engine handle
// ---------------------------------------------------------------------------

struct LitertEngineInner {
    library: Library,
    symbols: FfiSymbols,
    engine: *mut LlmEngine,
    model_path: String,
    backend: String,
}

unsafe impl Send for LitertEngineInner {}
unsafe impl Sync for LitertEngineInner {}

impl Drop for LitertEngineInner {
    fn drop(&mut self) {
        if !self.engine.is_null() {
            log::info!(
                "LitertEngineInner::drop: destroying engine — model_path={}",
                self.model_path
            );
            unsafe {
                (self.symbols.engine_destroy)(self.engine);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// LitertEngine — top-level entry point
// ---------------------------------------------------------------------------

/// Top-level handle for a loaded LiteRT-LM engine.
///
/// Usage:
/// ```no_run
/// let engine = LitertEngine::new("path/to/litertlm_bridge.dll")?;
/// let session = engine.create_session("model.litertlm", "gpu")?;
/// let (text, chunks) = session.send_message_streaming("Hello", |tok, idx| {
///     println!("[{}] {}", idx, tok);
/// })?;
/// ```
pub struct LitertEngine {
    inner: Arc<LitertEngineInner>,
}

impl LitertEngine {
    /// Load the LiteRT-LM shared library and resolve all FFI symbols.
    ///
    /// Does NOT create an engine yet — call `create_session` to load a model.
    ///
    /// # Arguments
    /// * `library_path` — Path to `litertlm_bridge.dll` / `.so` / `.dylib`.
    ///
    /// # Errors
    /// Returns `LitertError::LibraryNotFound` if the library cannot be loaded
    /// or any required symbol is missing.
    pub fn new<P: AsRef<Path>>(library_path: P) -> Result<Self, LitertError> {
        let path_str = library_path.as_ref().to_string_lossy().to_string();
        log::info!("LitertEngine::new: loading library from '{}'", path_str);

        let library = unsafe {
            Library::new(library_path.as_ref()).map_err(|e| {
                log::error!(
                    "LitertEngine::new: failed to load library from '{}' — {}",
                    path_str, e
                );
                LitertError::LibraryNotFound {
                    path: path_str.clone(),
                    source: e,
                }
            })?
        };

        let symbols = unsafe { FfiSymbols::load(&library)? };

        log::info!(
            "LitertEngine::new: library loaded and symbols resolved — path='{}'",
            path_str
        );

        Ok(Self {
            inner: Arc::new(LitertEngineInner {
                library,
                symbols,
                engine: std::ptr::null_mut(),
                model_path: String::new(),
                backend: String::new(),
            }),
        })
    }

    /// Convenience: load model + create conversation in one step.
    ///
    /// Equivalent to calling `load_model()` then `create_conversation()`.
    /// Use only when you want a single-use engine. For multi-conversation
    /// usage, call `load_model()` once and `create_conversation()` per chat.
    ///
    /// # Arguments
    /// * `model_path` — Path to the `.litertlm` model file.
    /// * `backend` — "cpu", "gpu", or "npu" (case-insensitive, falls back to CPU).
    pub fn create_session<P: AsRef<Path>>(
        &mut self,
        model_path: P,
        backend: &str,
    ) -> Result<LitertSession, LitertError> {
        self.load_model(model_path, backend)?;
        self.create_conversation()
    }

    /// Create a session using the engine already loaded in this LitertEngine.
    ///
    /// Unlike `create_session` which creates a new engine per call, this method
    /// creates a conversation on the shared engine. Use this for the common
    /// case where you load one model and have many conversations.
    pub fn create_conversation(&self) -> Result<LitertSession, LitertError> {
        // Check if we have an engine handle
        if self.inner.engine.is_null() {
            return Err(LitertError::NoActiveSession);
        }

        let ffi = &self.inner.symbols;
        let conv_handle = unsafe { (ffi.conversation_create)(self.inner.engine) };

        if conv_handle.is_null() {
            let err = unsafe {
                let ptr = (ffi.get_last_error)();
                if ptr.is_null() {
                    "unknown error".into()
                } else {
                    CStr::from_ptr(ptr).to_string_lossy().into_owned()
                }
            };
            log::error!(
                "LitertEngine::create_conversation: failed — error={}",
                err
            );
            return Err(LitertError::ConversationCreateFailed(err));
        }

        let session_id = generate_session_id();

        log::info!(
            "LitertEngine::create_conversation: session_id={}, backend={}",
            session_id,
            self.inner.backend
        );

        Ok(LitertSession {
            conversation: conv_handle,
            session_id,
            backend: self.inner.backend.clone(),
            _engine_ref: Arc::clone(&self.inner),
        })
    }

    /// Load a model and initialize the engine (shared, not per-session).
    ///
    /// Call this once after `new()`, then use `create_conversation()` for
    /// each chat session.
    pub fn load_model<P: AsRef<Path>>(
        &mut self,
        model_path: P,
        backend: &str,
    ) -> Result<(), LitertError> {
        let model_path_str = model_path.as_ref().to_string_lossy().to_string();

        log::info!(
            "LitertEngine::load_model: model_path={}, backend={}",
            model_path_str,
            backend
        );

        if model_path_str.is_empty() {
            return Err(LitertError::InvalidModelPath(model_path_str));
        }

        // Destroy existing engine if any
        if !self.inner.engine.is_null() {
            log::info!("LitertEngine::load_model: destroying previous engine");
            unsafe {
                (self.inner.symbols.engine_destroy)(self.inner.engine);
            }
        }

        let ffi = &self.inner.symbols;

        let c_model_path = CString::new(model_path_str.as_str()).map_err(|_| {
            LitertError::InvalidModelPath(format!("contains NUL byte: {}", model_path_str))
        })?;
        let c_backend = CString::new(backend).unwrap_or_else(|_| CString::new("cpu").unwrap());

        let engine_handle = unsafe {
            (ffi.engine_create)(c_model_path.as_ptr(), c_backend.as_ptr())
        };

        if engine_handle.is_null() {
            let err = unsafe {
                let ptr = (ffi.get_last_error)();
                if ptr.is_null() {
                    "unknown error".into()
                } else {
                    CStr::from_ptr(ptr).to_string_lossy().into_owned()
                }
            };
            log::error!(
                "LitertEngine::load_model: engine_create failed — model_path={}, backend={}, error={}",
                model_path_str, backend, err
            );
            return Err(LitertError::EngineCreateFailed {
                model_path: model_path_str,
                detail: err,
            });
        }

        log::info!(
            "litertlm_engine_create: model_path={}, backend={} — SUCCESS",
            model_path_str,
            backend
        );

        // Update inner state (get mut through Arc — we have &mut self so exclusive access)
        let inner = Arc::get_mut(&mut self.inner)
            .expect("LitertEngine::load_model called with outstanding references");
        inner.engine = engine_handle;
        inner.model_path = model_path_str;
        inner.backend = backend.to_string();

        Ok(())
    }

    /// Get the model path of the currently loaded engine, if any.
    pub fn model_path(&self) -> Option<&str> {
        if self.inner.engine.is_null() {
            None
        } else {
            Some(&self.inner.model_path)
        }
    }

    /// Get the backend of the currently loaded engine, if any.
    pub fn backend(&self) -> Option<&str> {
        if self.inner.engine.is_null() {
            None
        } else {
            Some(&self.inner.backend)
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Generate a simple session ID (matches lib.rs pattern).
fn generate_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let suffix = {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let mut x = seed.wrapping_add(0x9e3779b97f4a7c15);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        (x as u16) & 0xFFFF
    };
    format!("litert-{:x}-{:04x}", ts, suffix)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn litert_error_display_library_not_found() {
        let path = "/no/such/library.so";
        let err = LitertError::LibraryNotFound {
            path: path.into(),
            source: libloading::Error::LoadLibraryExWUnknown,
        };
        let msg = format!("{}", err);
        assert!(msg.contains(path), "Error message should contain the path");
        assert!(
            msg.contains("Build the C++ shim first"),
            "Error message should contain build instructions"
        );
    }

    #[test]
    fn litert_error_display_engine_create_failed() {
        let err = LitertError::EngineCreateFailed {
            model_path: "/models/test.litertlm".into(),
            detail: "model file not found".into(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("/models/test.litertlm"));
        assert!(msg.contains("model file not found"));
    }

    #[test]
    fn litert_error_display_inference_failed() {
        let err = LitertError::InferenceFailed {
            session_id: "sess-abc-1234".into(),
            detail: "OOM".into(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("sess-abc-1234"));
        assert!(msg.contains("OOM"));
    }

    #[test]
    fn litert_error_display_invalid_model_path() {
        let err = LitertError::InvalidModelPath("".into());
        let msg = format!("{}", err);
        assert!(msg.contains("Invalid model path"));
    }

    #[test]
    fn litert_error_display_no_active_session() {
        let err = LitertError::NoActiveSession;
        let msg = format!("{}", err);
        assert!(msg.contains("No active session"));
    }

    #[test]
    fn generate_session_id_format() {
        let id = generate_session_id();
        assert!(id.starts_with("litert-"), "Session ID should start with 'litert-'");
        assert!(id.len() > 10, "Session ID should be reasonably long");
    }
}
