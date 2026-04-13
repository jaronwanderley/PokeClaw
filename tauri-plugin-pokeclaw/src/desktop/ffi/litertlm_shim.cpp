// litertlm_shim.cpp — C FFI shim implementation for LiteRT-LM C++ APIs.
//
// Wraps LiteRT-LM's Engine, Conversation, and SendMessageAsync C++ APIs
// behind extern "C" functions with opaque pointers.  The C++ types are never
// exposed across the FFI boundary — Rust interacts only through the functions
// declared in litertlm_shim.h.

#include "litertlm_shim.h"

#include <cstring>
#include <memory>
#include <mutex>
#include <sstream>
#include <string>
#include <thread>

// LiteRT-LM C++ headers — available when building against the LiteRT-LM source
// tree or after Bazel build has generated the necessary headers.
#include "runtime/engine/engine.h"

// ---------------------------------------------------------------------------
// Thread-local error storage
// ---------------------------------------------------------------------------

thread_local std::string t_last_error;

static void set_error(const std::string& msg) {
  t_last_error = msg;
}

static void clear_error() {
  t_last_error.clear();
}

// ---------------------------------------------------------------------------
// Backend string → litert::lm::Backend
// ---------------------------------------------------------------------------

static litert::lm::Backend parse_backend(const char* backend_str) {
  if (!backend_str) return litert::lm::Backend::CPU;
  std::string b(backend_str);
  // Case-insensitive comparison
  for (auto& c : b) c = static_cast<char>(tolower(static_cast<unsigned char>(c)));

  if (b == "gpu") return litert::lm::Backend::GPU;
  if (b == "npu") return litert::lm::Backend::NPU;
  return litert::lm::Backend::CPU;
}

// ---------------------------------------------------------------------------
// Engine lifecycle
// ---------------------------------------------------------------------------

LITERTLM_EXPORT
LlmEngine* litertlm_engine_create(const char* model_path, const char* backend) {
  clear_error();

  if (!model_path || model_path[0] == '\0') {
    set_error("litertlm_engine_create: model_path is null or empty");
    return nullptr;
  }

  // 1. Create model assets
  auto model_assets = litert::lm::ModelAssets::Create(model_path);
  if (!model_assets.ok()) {
    set_error("litertlm_engine_create: ModelAssets::Create failed for path='" +
              std::string(model_path) + "': " +
              model_assets.status().ToString());
    return nullptr;
  }

  // 2. Create engine settings
  auto litert_backend = parse_backend(backend);
  auto engine_settings = litert::lm::EngineSettings::CreateDefault(
      *model_assets, litert_backend);
  if (!engine_settings.ok()) {
    set_error("litertlm_engine_create: EngineSettings::CreateDefault failed: " +
              engine_settings.status().ToString());
    return nullptr;
  }

  // 3. Create engine
  auto engine_result = litert::lm::Engine::CreateEngine(*engine_settings);
  if (!engine_result.ok()) {
    set_error("litertlm_engine_create: Engine::CreateEngine failed for path='" +
              std::string(model_path) + "' backend='" +
              std::string(backend ? backend : "cpu") + "': " +
              engine_result.status().ToString());
    return nullptr;
  }

  // Transfer ownership to the caller via the opaque handle.
  // We store the unique_ptr inside a raw pointer wrapper.
  auto* engine_ptr = new std::unique_ptr<litert::lm::Engine>(
      std::move(*engine_result));
  return reinterpret_cast<LlmEngine*>(engine_ptr);
}

LITERTLM_EXPORT
void litertlm_engine_destroy(LlmEngine* engine) {
  if (!engine) return;
  auto* up = reinterpret_cast<std::unique_ptr<litert::lm::Engine>*>(engine);
  delete up;
}

// ---------------------------------------------------------------------------
// Conversation lifecycle
// ---------------------------------------------------------------------------

LITERTLM_EXPORT
LlmConversation* litertlm_conversation_create(LlmEngine* engine) {
  clear_error();

  if (!engine) {
    set_error("litertlm_conversation_create: engine is null");
    return nullptr;
  }

  auto* up = reinterpret_cast<std::unique_ptr<litert::lm::Engine>*>(engine);
  if (!up || !*up) {
    set_error("litertlm_conversation_create: engine handle is invalid");
    return nullptr;
  }

  litert::lm::Engine& eng = **up;

  // Create conversation config from engine defaults
  auto conv_config = litert::lm::ConversationConfig::CreateDefault(eng);
  if (!conv_config.ok()) {
    set_error("litertlm_conversation_create: ConversationConfig::CreateDefault failed: " +
              conv_config.status().ToString());
    return nullptr;
  }

  // Create conversation
  auto conv_result = litert::lm::Conversation::Create(eng, *conv_config);
  if (!conv_result.ok()) {
    set_error("litertlm_conversation_create: Conversation::Create failed: " +
              conv_result.status().ToString());
    return nullptr;
  }

  auto* conv_ptr = new std::unique_ptr<litert::lm::Conversation>(
      std::move(*conv_result));
  return reinterpret_cast<LlmConversation*>(conv_ptr);
}

LITERTLM_EXPORT
void litertlm_conversation_destroy(LlmConversation* conversation) {
  if (!conversation) return;
  auto* up = reinterpret_cast<std::unique_ptr<litert::lm::Conversation>*>(conversation);
  delete up;
}

// ---------------------------------------------------------------------------
// Blocking send_message
// ---------------------------------------------------------------------------

LITERTLM_EXPORT
uint32_t litertlm_send_message(
    LlmConversation* conversation,
    const char* message,
    char* out_buf,
    uint32_t out_buf_len) {
  clear_error();

  if (!conversation) {
    set_error("litertlm_send_message: conversation is null");
    return 0;
  }
  if (!message) {
    set_error("litertlm_send_message: message is null");
    return 0;
  }
  if (!out_buf || out_buf_len == 0) {
    set_error("litertlm_send_message: output buffer is null or zero-length");
    return 0;
  }

  auto* up = reinterpret_cast<std::unique_ptr<litert::lm::Conversation>*>(conversation);
  if (!up || !*up) {
    set_error("litertlm_send_message: conversation handle is invalid");
    return 0;
  }

  // Build JSON message in the format LiteRT-LM expects:
  // {"role": "user", "content": "<user text>"}
  litert::lm::JsonMessage json_msg = {
      {"role", "user"},
      {"content", message}
  };

  auto result = (*up)->SendMessage(json_msg);
  if (!result.ok()) {
    set_error("litertlm_send_message: SendMessage failed: " +
              result.status().ToString());
    return 0;
  }

  // Extract text from the response
  std::string response_text;
  const auto& msg = *result;

  // The response may contain "content" as an array of parts or as a direct text field
  if (msg.contains("content") && msg["content"].is_array()) {
    for (const auto& part : msg["content"]) {
      if (part.contains("type") && part["type"] == "text" && part.contains("text")) {
        response_text += part["text"].get<std::string>();
      }
    }
  } else if (msg.contains("content") && msg["content"].is_object()) {
    if (msg["content"].contains("text")) {
      response_text = msg["content"]["text"].get<std::string>();
    }
  } else if (msg.contains("content") && msg["content"].is_string()) {
    response_text = msg["content"].get<std::string>();
  } else {
    // Fallback: serialize the whole response
    response_text = msg.dump();
  }

  // Copy to output buffer, respecting size limits
  uint32_t copy_len = static_cast<uint32_t>(
      std::min(response_text.size(), static_cast<size_t>(out_buf_len - 1)));
  std::memcpy(out_buf, response_text.c_str(), copy_len);
  out_buf[copy_len] = '\0';

  return copy_len;
}

// ---------------------------------------------------------------------------
// Streaming send_message_async
// ---------------------------------------------------------------------------

LITERTLM_EXPORT
int32_t litertlm_send_message_async(
    LlmConversation* conversation,
    const char* message,
    LitertlmTokenCallback callback,
    void* user_data) {
  clear_error();

  if (!conversation) {
    set_error("litertlm_send_message_async: conversation is null");
    return -1;
  }
  if (!message) {
    set_error("litertlm_send_message_async: message is null");
    return -1;
  }
  if (!callback) {
    set_error("litertlm_send_message_async: callback is null");
    return -1;
  }

  auto* up = reinterpret_cast<std::unique_ptr<litert::lm::Conversation>*>(conversation);
  if (!up || !*up) {
    set_error("litertlm_send_message_async: conversation handle is invalid");
    return -1;
  }

  // Build the streaming callback that adapts LiteRT-LM's
  // absl::AnyInvocable<void(absl::StatusOr<JsonMessage>)> to our C callback.
  LitertlmTokenCallback cb = callback;
  void* ud = user_data;

  auto litert_callback = [cb, ud](absl::StatusOr<litert::lm::JsonMessage> msg) {
    if (!msg.ok()) {
      // Error occurred
      std::string err = "LiteRT-LM inference error: " + msg.status().ToString();
      cb(err.c_str(), static_cast<uint32_t>(err.size()), 0, ud);
      return;
    }

    const auto& json_msg = *msg;

    // Check for end-of-stream: empty/null message signals completion
    if (json_msg.is_null() || json_msg.empty()) {
      cb("", 0, 1, ud);  // is_final = 1
      return;
    }

    // Extract text from the chunk
    std::string chunk_text;
    if (json_msg.contains("content") && json_msg["content"].is_array()) {
      for (const auto& part : json_msg["content"]) {
        if (part.contains("type") && part["type"] == "text" && part.contains("text")) {
          chunk_text += part["text"].get<std::string>();
        }
      }
    } else if (json_msg.contains("content") && json_msg["content"].is_object()) {
      if (json_msg["content"].contains("text")) {
        chunk_text = json_msg["content"]["text"].get<std::string>();
      }
    } else if (json_msg.contains("content") && json_msg["content"].is_string()) {
      chunk_text = json_msg["content"].get<std::string>();
    }

    // Determine if this is the final chunk
    // In LiteRT-LM's streaming, an empty text in a non-null message indicates
    // the model is done generating (the null message case is handled above).
    int32_t is_final = chunk_text.empty() ? 1 : 0;

    cb(chunk_text.c_str(), static_cast<uint32_t>(chunk_text.size()),
       is_final, ud);
  };

  // Build JSON message
  litert::lm::JsonMessage json_msg = {
      {"role", "user"},
      {"content", message}
  };

  // Send asynchronously
  (*up)->SendMessageAsync(json_msg, std::move(litert_callback));

  // Note: SendMessageAsync returns immediately, but the LiteRT-LM engine
  // processes the request on an internal thread. For simplicity in this
  // shim, we rely on the caller (Rust) to wait for the is_final callback.
  //
  // If synchronous blocking is needed, the Rust side can use a
  // synchronization primitive (e.g., a threading::Barrier or oneshot
  // channel) that the final callback signals.

  return 0;
}

// ---------------------------------------------------------------------------
// Error retrieval
// ---------------------------------------------------------------------------

LITERTLM_EXPORT
const char* litertlm_get_last_error() {
  if (t_last_error.empty()) return nullptr;
  return t_last_error.c_str();
}
