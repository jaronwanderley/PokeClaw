// litertlm_shim.h — C FFI shim for LiteRT-LM Engine/Conversation C++ APIs.
//
// Exposes opaque-pointer-based extern "C" functions that Rust loads at runtime
// via libloading.  The shim owns the C++ objects and never exposes C++ types
// across the FFI boundary.
//
// Build: see CMakeLists.txt in this directory.
// Usage: Rust loads litertlm_bridge.dll / .so / .dylib at runtime.

#ifndef LITERTLM_SHIM_H_
#define LITERTLM_SHIM_H_

#include <stddef.h>
#include <stdint.h>

#ifdef _WIN32
#  define LITERTLM_EXPORT __declspec(dllexport)
#else
#  define LITERTLM_EXPORT __attribute__((visibility("default")))
#endif

#ifdef __cplusplus
extern "C" {
#endif

// ---------------------------------------------------------------------------
// Opaque handles — pointers are owned by the shim, never dereferenced by Rust
// ---------------------------------------------------------------------------

/// Opaque handle to a LiteRT-LM Engine (heavyweight, holds model weights).
typedef struct LlmEngine LlmEngine;

/// Opaque handle to a LiteRT-LM Conversation (lightweight per-session).
typedef struct LlmConversation LlmConversation;

// ---------------------------------------------------------------------------
// Callback types
// ---------------------------------------------------------------------------

/// Callback invoked by SendMessageAsync for each streaming token chunk.
///
/// @param chunk_text   UTF-8 text chunk (valid only during the callback).
/// @param chunk_len    Byte length of chunk_text (excluding NUL).
/// @param is_final     1 if this is the final chunk (empty text = end signal),
///                     0 otherwise.
/// @param user_data    Opaque pointer passed to litertlm_send_message_async.
typedef void (*LitertlmTokenCallback)(
    const char* chunk_text,
    uint32_t chunk_len,
    int32_t is_final,
    void* user_data);

// ---------------------------------------------------------------------------
// Engine lifecycle
// ---------------------------------------------------------------------------

/// Create a new LiteRT-LM Engine.
///
/// @param model_path  Null-terminated UTF-8 path to the .litertlm model file.
/// @param backend     "cpu", "gpu", or "npu" (case-insensitive). Falls back to
///                    CPU if the requested backend is unavailable.
/// @return            Opaque engine handle, or NULL on failure (call
///                    litertlm_get_last_error for details).
LITERTLM_EXPORT LlmEngine* litertlm_engine_create(
    const char* model_path,
    const char* backend);

/// Destroy a previously created engine and release all associated resources.
///
/// @param engine  Engine handle (may be NULL — no-op).
LITERTLM_EXPORT void litertlm_engine_destroy(LlmEngine* engine);

// ---------------------------------------------------------------------------
// Conversation lifecycle
// ---------------------------------------------------------------------------

/// Create a new Conversation bound to the given engine.
///
/// @param engine  Valid engine handle (must not be NULL).
/// @return        Opaque conversation handle, or NULL on failure.
LITERTLM_EXPORT LlmConversation* litertlm_conversation_create(
    LlmEngine* engine);

/// Destroy a previously created conversation and release resources.
///
/// @param conversation  Conversation handle (may be NULL — no-op).
LITERTLM_EXPORT void litertlm_conversation_destroy(
    LlmConversation* conversation);

// ---------------------------------------------------------------------------
// Inference
// ---------------------------------------------------------------------------

/// Send a message synchronously (blocking).
///
/// @param conversation  Valid conversation handle.
/// @param message       Null-terminated UTF-8 user message.
/// @param out_buf       Output buffer for the model response text.
/// @param out_buf_len   Size of out_buf in bytes.
/// @return              Number of bytes written to out_buf (excluding NUL),
///                      or 0 on failure. If the response exceeds out_buf_len,
///                      it is truncated and the return value equals out_buf_len.
LITERTLM_EXPORT uint32_t litertlm_send_message(
    LlmConversation* conversation,
    const char* message,
    char* out_buf,
    uint32_t out_buf_len);

/// Send a message asynchronously with streaming token callback.
///
/// The callback is invoked on an internal LiteRT-LM thread for each token
/// chunk, and once more with is_final=1 and empty text to signal completion.
///
/// This function blocks until the response is fully streamed.
///
/// @param conversation  Valid conversation handle.
/// @param message       Null-terminated UTF-8 user message.
/// @param callback      Function pointer invoked for each chunk.
/// @param user_data     Opaque pointer forwarded to each callback invocation.
/// @return              0 on success, -1 on failure.
LITERTLM_EXPORT int32_t litertlm_send_message_async(
    LlmConversation* conversation,
    const char* message,
    LitertlmTokenCallback callback,
    void* user_data);

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

/// Retrieve the last error message from the current thread.
///
/// The returned pointer is valid until the next call to any shim function on
/// the same thread.  Returns NULL if no error has been recorded.
///
/// @return  Null-terminated UTF-8 error string, or NULL.
LITERTLM_EXPORT const char* litertlm_get_last_error(void);

#ifdef __cplusplus
}  // extern "C"
#endif

#endif  // LITERTLM_SHIM_H_
