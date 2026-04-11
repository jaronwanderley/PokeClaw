package io.agents.pokeclaw

import android.app.Activity
import android.util.Log
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import app.tauri.plugin.Invoke
import com.google.ai.edge.litertlm.Backend
import com.google.ai.edge.litertlm.Contents
import com.google.ai.edge.litertlm.ConversationConfig
import com.google.ai.edge.litertlm.SamplerConfig

@TauriPlugin
class PokeclawPlugin(private val activity: Activity) : Plugin(activity) {

    companion object {
        private const val TAG = "PokeclawPlugin"

        /** Default system prompt for on-device conversations. */
        private const val DEFAULT_SYSTEM_PROMPT = "You are a helpful AI assistant running on-device."

        /** Max retries for conversation creation (another client may be closing). */
        private const val MAX_CONVERSATION_RETRIES = 3

        /** Backoff between retries in milliseconds. */
        private const val RETRY_BACKOFF_MS = 1000L

        /** Max send calls before forcing conversation recreation. */
        private const val MAX_SEND_COUNT = 8
    }

    /** Active inference session, null when idle. */
    private var session: InferenceSession? = null

    /** Track whether GPU has failed to prevent retry loops. */
    private var gpuFailed = false

    // -----------------------------------------------------------------------
    // @Command methods — invoked from JS via Tauri plugin IPC
    // -----------------------------------------------------------------------

    @Command
    fun ping(invoke: Invoke) {
        val ret = JSObject()
        ret.put("value", "pong")
        invoke.resolve(ret)
    }

    /**
     * Initialize LiteRT-LM engine and create a conversation session.
     *
     * Args: model_path (String), prefer_gpu (Boolean, default true)
     * Returns: { session_id: String, backend: String }
     */
    @Command
    fun start_session(invoke: Invoke) {
        val args = invoke.getArgs()
        val modelPath = args.getString("model_path")
            ?: return invoke.reject("model_path is required")
        val preferGpu = args.optBoolean("prefer_gpu", true)

        Log.i(TAG, "start_session: model_path=$modelPath, prefer_gpu=$preferGpu")

        // Reject if a session is already active
        if (session != null) {
            Log.w(TAG, "start_session: session already active, reject")
            return invoke.reject("A session is already active. Call stop_session first.")
        }

        val sessionId = "sess-${System.currentTimeMillis().toString(16)}-${(System.nanoTime() and 0xFFFF).toString(16)}"

        try {
            // Acquire engine with GPU/CPU backend selection + fallback
            val cacheDir = activity.cacheDir.absolutePath
            val backend = selectBackend(preferGpu)
            val engine = EngineHolder.getOrCreate(modelPath, cacheDir, backend)
            val backendLabel = EngineHolder.getBackendLabel(modelPath) ?: backendLabel(backend)

            // Create conversation with retry logic
            val conversation = createConversationWithRetry(engine, modelPath)

            session = InferenceSession(
                conversation = conversation,
                modelPath = modelPath,
                backendLabel = backendLabel,
                sessionId = sessionId,
            )

            Log.i(TAG, "start_session: ready — sessionId=$sessionId, backend=$backendLabel")

            val result = JSObject()
            result.put("session_id", sessionId)
            result.put("backend", backendLabel)
            invoke.resolve(result)

        } catch (e: Exception) {
            Log.e(TAG, "start_session: failed — ${e.message}")

            // If GPU failed, record it for next attempt
            if (isGpuBackendFailure(e)) {
                gpuFailed = true
                Log.w(TAG, "start_session: GPU failure detected, will use CPU on next attempt")
            }

            invoke.reject("Failed to start session: ${e.message}", e)
        }
    }

    /**
     * Send a message to the active conversation and return the response.
     *
     * Args: message (String)
     * Returns: { response: String }
     */
    @Command
    fun send_message(invoke: Invoke) {
        val args = invoke.getArgs()
        val message = args.getString("message")
            ?: return invoke.reject("message is required")

        Log.i(TAG, "send_message: '${message.take(80)}...'")

        val currentSession = session
            ?: return invoke.reject("No active session. Call start_session first.")

        // Force conversation recreation if send count exceeds threshold
        if (currentSession.sendCount >= MAX_SEND_COUNT) {
            Log.i(TAG, "send_message: sendCount=${currentSession.sendCount} >= $MAX_SEND_COUNT, recreating conversation")
            try {
                currentSession.close()
                val engine = acquireEngine(currentSession.modelPath)
                val newConversation = createConversationWithRetry(engine, currentSession.modelPath)
                session = InferenceSession(
                    conversation = newConversation,
                    modelPath = currentSession.modelPath,
                    backendLabel = currentSession.backendLabel,
                    sessionId = currentSession.sessionId,
                )
            } catch (e: Exception) {
                Log.e(TAG, "send_message: failed to recreate conversation: ${e.message}")
                return invoke.reject("Failed to recreate conversation: ${e.message}", e)
            }
        }

        try {
            val activeSession = session!!
            val response = activeSession.conversation.sendMessage(message)
            activeSession.recordSend()

            val responseText = response?.toString() ?: ""
            Log.d(TAG, "send_message: response (${responseText.length} chars), sendCount=${activeSession.sendCount}")

            val result = JSObject()
            result.put("response", responseText)
            invoke.resolve(result)

        } catch (e: Exception) {
            Log.e(TAG, "send_message: failed — ${e.message}")

            // GPU failure during inference — fallback to CPU and retry once
            if (!gpuFailed && isGpuBackendFailure(e)) {
                Log.w(TAG, "send_message: GPU inference failed, falling back to CPU")
                try {
                    fallbackToCpu(currentSession.modelPath)
                    val retryResponse = session!!.conversation.sendMessage(message)
                    session!!.recordSend()

                    val result = JSObject()
                    result.put("response", retryResponse?.toString() ?: "")
                    invoke.resolve(result)
                } catch (retryError: Exception) {
                    Log.e(TAG, "send_message: CPU retry also failed: ${retryError.message}")
                    invoke.reject("Inference failed even after CPU fallback: ${retryError.message}", retryError)
                }
            } else {
                invoke.reject("Inference failed: ${e.message}", e)
            }
        }
    }

    /**
     * Stop the active session and release engine resources.
     *
     * Returns: { success: true }
     */
    @Command
    fun stop_session(invoke: Invoke) {
        Log.i(TAG, "stop_session")

        val currentSession = session
        if (currentSession == null) {
            Log.w(TAG, "stop_session: no active session")
            return invoke.reject("No active session to stop.")
        }

        try {
            currentSession.close()
        } catch (e: Exception) {
            Log.w(TAG, "stop_session: error closing session: ${e.message}")
        }

        session = null

        // Release the shared engine
        try {
            EngineHolder.close()
        } catch (e: Exception) {
            Log.w(TAG, "stop_session: error closing engine: ${e.message}")
        }

        Log.i(TAG, "stop_session: done — sessionId=${currentSession.sessionId}")
        val result = JSObject()
        result.put("success", true)
        invoke.resolve(result)
    }

    /**
     * Return the current session status.
     *
     * Returns: { state: "idle"|"loading"|"ready"|"error", model_path?, backend?, session_id?, error? }
     */
    @Command
    fun get_session_status(invoke: Invoke) {
        Log.d(TAG, "get_session_status")

        val result = JSObject()
        val currentSession = session

        if (currentSession == null) {
            result.put("state", "idle")
        } else {
            result.put("state", "ready")
            result.put("session_id", currentSession.sessionId)
            result.put("model_path", currentSession.modelPath)
            result.put("backend", currentSession.backendLabel)
            result.put("send_count", currentSession.sendCount)
        }

        invoke.resolve(result)
    }

    // -----------------------------------------------------------------------
    // Lifecycle
    // -----------------------------------------------------------------------

    override fun onDestroy() {
        Log.i(TAG, "onDestroy: cleaning up session and engine as safety net (D007)")
        try {
            session?.close()
        } catch (e: Exception) {
            Log.w(TAG, "onDestroy: error closing session: ${e.message}")
        }
        session = null
        try {
            EngineHolder.close()
        } catch (e: Exception) {
            Log.w(TAG, "onDestroy: error closing engine: ${e.message}")
        }
        super.onDestroy()
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /**
     * Select GPU or CPU backend based on preference and past failures.
     */
    private fun selectBackend(preferGpu: Boolean): Backend {
        return if (preferGpu && !gpuFailed) {
            Log.d(TAG, "selectBackend: attempting GPU")
            Backend.GPU()
        } else {
            Log.d(TAG, "selectBackend: using CPU (preferGpu=$preferGpu, gpuFailed=$gpuFailed)")
            Backend.CPU()
        }
    }

    /**
     * Get backend label string from a Backend instance.
     */
    private fun backendLabel(backend: Backend): String =
        if (backend is Backend.CPU) "CPU"
        else if (backend is Backend.GPU) "GPU"
        else backend.javaClass.simpleName

    /**
     * Acquire engine for the given model path, handling GPU fallback.
     */
    private fun acquireEngine(modelPath: String): com.google.ai.edge.litertlm.Engine {
        val cacheDir = activity.cacheDir.absolutePath
        val backend = selectBackend(preferGpu = !gpuFailed)

        return try {
            EngineHolder.getOrCreate(modelPath, cacheDir, backend)
        } catch (e: Exception) {
            if (!gpuFailed && isGpuBackendFailure(e)) {
                Log.w(TAG, "acquireEngine: GPU failed, retrying on CPU: ${e.message}")
                gpuFailed = true
                EngineHolder.close()
                EngineHolder.getOrCreate(modelPath, cacheDir, Backend.CPU())
            } else {
                throw e
            }
        }
    }

    /**
     * Create a new conversation with retry logic.
     *
     * Another client may still be closing its session, so we retry with
     * backoff. On 3rd failure, reset the engine and retry.
     */
    private fun createConversationWithRetry(
        engine: com.google.ai.edge.litertlm.Engine,
        modelPath: String,
    ): com.google.ai.edge.litertlm.Conversation {
        val convConfig = ConversationConfig(
            systemInstruction = Contents.of(DEFAULT_SYSTEM_PROMPT),
            samplerConfig = SamplerConfig(
                topK = 64,
                topP = 0.95,
                temperature = 0.8,
            ),
        )

        var lastError: Exception? = null
        for (attempt in 1..MAX_CONVERSATION_RETRIES) {
            try {
                val conversation = engine.createConversation(convConfig)
                Log.i(TAG, "createConversation: success on attempt $attempt")
                return conversation
            } catch (e: Exception) {
                lastError = e
                Log.w(TAG, "createConversation: attempt $attempt failed: ${e.message}")

                // On penultimate failure, reset engine
                if (attempt == MAX_CONVERSATION_RETRIES - 1) {
                    Log.w(TAG, "createConversation: resetting engine before final retry")
                    try {
                        EngineHolder.close()
                        val newEngine = acquireEngine(modelPath)
                        // Retry immediately with new engine — don't sleep
                        try {
                            return newEngine.createConversation(convConfig)
                        } catch (retryErr: Exception) {
                            lastError = retryErr
                            Log.w(TAG, "createConversation: retry with new engine also failed: ${retryErr.message}")
                        }
                    } catch (resetErr: Exception) {
                        Log.e(TAG, "createConversation: engine reset failed: ${resetErr.message}")
                        lastError = resetErr
                    }
                }

                if (attempt < MAX_CONVERSATION_RETRIES) {
                    Thread.sleep(RETRY_BACKOFF_MS)
                }
            }
        }
        throw RuntimeException(
            "Failed to create conversation after $MAX_CONVERSATION_RETRIES retries: ${lastError?.message}",
            lastError
        )
    }

    /**
     * Fallback from GPU to CPU: close existing session, reset engine, recreate.
     */
    private fun fallbackToCpu(modelPath: String) {
        Log.w(TAG, "fallbackToCpu: switching from GPU to CPU for $modelPath")
        gpuFailed = true

        // Close existing session
        try { session?.close() } catch (_: Exception) {}
        session = null

        // Reset and recreate on CPU
        EngineHolder.close()
        val cpuEngine = EngineHolder.getOrCreate(modelPath, activity.cacheDir.absolutePath, Backend.CPU())
        val conversation = createConversationWithRetry(cpuEngine, modelPath)

        session = InferenceSession(
            conversation = conversation,
            modelPath = modelPath,
            backendLabel = "CPU",
            sessionId = "sess-${System.currentTimeMillis().toString(16)}-${(System.nanoTime() and 0xFFFF).toString(16)}-cpu",
        )
        Log.i(TAG, "fallbackToCpu: CPU session ready")
    }

    /**
     * Detect GPU backend failures from exception messages.
     * Ported from legacy LocalModelRuntime.isGpuBackendFailure.
     */
    private fun isGpuBackendFailure(error: Throwable?): Boolean {
        val message = error?.message.orEmpty()
        if (message.isEmpty()) return false
        return message.contains("OpenCL", ignoreCase = true) ||
            message.contains("GPU", ignoreCase = true) ||
            message.contains("nativeSendMessage", ignoreCase = true) ||
            message.contains("Failed to create engine", ignoreCase = true) ||
            message.contains("compiled model", ignoreCase = true)
    }
}
