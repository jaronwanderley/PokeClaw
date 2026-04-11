package io.agents.pokeclaw

import android.app.Activity
import android.util.Log
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import app.tauri.plugin.Invoke
import com.google.ai.edge.litertlm.Backend
import com.google.ai.edge.litertlm.Contents
import com.google.ai.edge.litertlm.ConversationConfig
import com.google.ai.edge.litertlm.MessageCallback
import com.google.ai.edge.litertlm.SamplerConfig
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

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

        /** Approximate chars per token for batch-size calculations. */
        private const val CHARS_PER_TOKEN = 4
    }

    /** Args class for send_message with streaming Channel support. */
    @InvokeArg
    inner class SendMessageArgs {
        lateinit var message: String
        var batchSize: Int = 5
        lateinit var onEvent: Channel
    }

    /** Args class for download_model with streaming Channel for progress. */
    @InvokeArg
    inner class DownloadModelArgs {
        lateinit var modelId: String
        lateinit var onProgress: Channel
    }

    /** Active inference session, null when idle. */
    private var session: InferenceSession? = null

    /** Track whether GPU has failed to prevent retry loops. */
    private var gpuFailed = false

    /** Coroutine scope for streaming inference, cancelled in onDestroy(). */
    private val streamingScope = CoroutineScope(Dispatchers.Default + Job())

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
     * Send a message to the active conversation and stream the response back
     * via a Tauri Channel.
     *
     * Args (via SendMessageArgs): message (String), batchSize (Int, default 5),
     *   onEvent (Channel — receives StreamEvent objects)
     *
     * Stream events sent via channel:
     *   { event: "token_batch", data: { tokens: String, batch_index: Int } }
     *   { event: "complete", data: { full_text: String, token_count: Int } }
     *   { event: "error", data: { message: String } }
     */
    @Command
    fun send_message(invoke: Invoke) {
        val args: SendMessageArgs
        try {
            args = invoke.parseArgs(SendMessageArgs::class.java)
        } catch (e: Exception) {
            return invoke.reject("Invalid arguments: ${e.message}", e)
        }

        val message = args.message
        val batchSize = args.batchSize.coerceAtLeast(1)
        val channel = args.onEvent

        Log.i(TAG, "send_message: '${message.take(80)}...', batchSize=$batchSize, channelId=${channel.id}")

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

        val activeSession = session!!

        Log.i(TAG, "send_message: starting async streaming inference")

        // Launch streaming on a coroutine — this method returns immediately
        kotlinx.coroutines.launch(streamingScope.coroutineContext) {
            val fullText = StringBuilder()
            val batchBuffer = StringBuilder()
            var batchIndex = 0
            var accumulatedLength = 0
            val batchThreshold = batchSize * CHARS_PER_TOKEN

            try {
                activeSession.conversation.sendMessageAsync(
                    message,
                    object : MessageCallback {
                        override fun onMessage(msg: com.google.ai.edge.litertlm.Message) {
                            val newText = msg?.toString() ?: ""
                            if (newText.isEmpty()) return

                            // Detect token granularity: accumulated vs incremental
                            val delta: String
                            if (newText.length > accumulatedLength) {
                                // Accumulated mode — extract just the new part
                                delta = newText.substring(accumulatedLength)
                                accumulatedLength = newText.length
                            } else {
                                // Incremental mode — use the full text as the delta
                                delta = newText
                                accumulatedLength += newText.length
                            }

                            fullText.append(delta)
                            batchBuffer.append(delta)

                            // Send batch when buffer exceeds threshold
                            if (batchBuffer.length >= batchThreshold) {
                                sendTokenBatch(channel, batchBuffer.toString(), batchIndex)
                                Log.d(TAG, "send_message: batch $batchIndex sent (${batchBuffer.length} chars)")
                                batchIndex++
                                batchBuffer.clear()
                            }
                        }

                        override fun onDone() {
                            // Flush remaining buffered text
                            if (batchBuffer.isNotEmpty()) {
                                sendTokenBatch(channel, batchBuffer.toString(), batchIndex)
                                Log.d(TAG, "send_message: final batch $batchIndex sent (${batchBuffer.length} chars)")
                                batchIndex++
                                batchBuffer.clear()
                            }

                            val finalText = fullText.toString()
                            val tokenCount = finalText.length / CHARS_PER_TOKEN
                            activeSession.recordSend()

                            // Send complete event
                            try {
                                val completeData = JSObject()
                                completeData.put("full_text", finalText)
                                completeData.put("token_count", tokenCount)
                                val completeEvent = JSObject()
                                completeEvent.put("event", "complete")
                                completeEvent.put("data", completeData)
                                channel.send(completeEvent)
                            } catch (e: Exception) {
                                Log.w(TAG, "send_message: failed to send complete event: ${e.message}")
                            }

                            Log.i(TAG, "send_message: complete — ${finalText.length} chars, $batchIndex batches, ~$tokenCount tokens")
                            invoke.resolve()
                        }

                        override fun onError(throwable: Throwable) {
                            Log.e(TAG, "send_message: streaming error — ${throwable.message}")

                            // GPU failure during streaming — attempt CPU fallback and retry
                            if (!gpuFailed && isGpuBackendFailure(throwable)) {
                                Log.w(TAG, "send_message: GPU failure during streaming, attempting CPU fallback")
                                try {
                                    fallbackToCpu(activeSession.modelPath)
                                    // Retry streaming on the new CPU conversation
                                    val retrySession = session!!
                                    val retryFullText = StringBuilder()
                                    val retryBuffer = StringBuilder()
                                    var retryBatchIndex = 0
                                    var retryAccumulatedLength = 0

                                    retrySession.conversation.sendMessageAsync(
                                        message,
                                        object : MessageCallback {
                                            override fun onMessage(msg: com.google.ai.edge.litertlm.Message) {
                                                val newText = msg?.toString() ?: ""
                                                if (newText.isEmpty()) return

                                                val delta: String
                                                if (newText.length > retryAccumulatedLength) {
                                                    delta = newText.substring(retryAccumulatedLength)
                                                    retryAccumulatedLength = newText.length
                                                } else {
                                                    delta = newText
                                                    retryAccumulatedLength += newText.length
                                                }

                                                retryFullText.append(delta)
                                                retryBuffer.append(delta)

                                                if (retryBuffer.length >= batchThreshold) {
                                                    sendTokenBatch(channel, retryBuffer.toString(), retryBatchIndex)
                                                    Log.d(TAG, "send_message: CPU retry batch $retryBatchIndex sent (${retryBuffer.length} chars)")
                                                    retryBatchIndex++
                                                    retryBuffer.clear()
                                                }
                                            }

                                            override fun onDone() {
                                                if (retryBuffer.isNotEmpty()) {
                                                    sendTokenBatch(channel, retryBuffer.toString(), retryBatchIndex)
                                                    retryBatchIndex++
                                                    retryBuffer.clear()
                                                }

                                                val finalText = retryFullText.toString()
                                                val tokenCount = finalText.length / CHARS_PER_TOKEN
                                                retrySession.recordSend()

                                                try {
                                                    val completeData = JSObject()
                                                    completeData.put("full_text", finalText)
                                                    completeData.put("token_count", tokenCount)
                                                    val completeEvent = JSObject()
                                                    completeEvent.put("event", "complete")
                                                    completeEvent.put("data", completeData)
                                                    channel.send(completeEvent)
                                                } catch (e: Exception) {
                                                    Log.w(TAG, "send_message: failed to send CPU retry complete event: ${e.message}")
                                                }

                                                Log.i(TAG, "send_message: CPU retry complete — ${finalText.length} chars, $retryBatchIndex batches")
                                                invoke.resolve()
                                            }

                                            override fun onError(retryError: Throwable) {
                                                Log.e(TAG, "send_message: CPU retry also failed: ${retryError.message}")
                                                sendErrorEvent(channel, "Inference failed even after CPU fallback: ${retryError.message}")
                                                invoke.reject("Inference failed even after CPU fallback: ${retryError.message}", retryError)
                                            }
                                        },
                                        null as? java.util.Map<String, Any>
                                    )
                                } catch (fallbackErr: Exception) {
                                    Log.e(TAG, "send_message: CPU fallback failed: ${fallbackErr.message}")
                                    sendErrorEvent(channel, "CPU fallback failed: ${fallbackErr.message}")
                                    invoke.reject("CPU fallback failed: ${fallbackErr.message}", fallbackErr)
                                }
                            } else {
                                sendErrorEvent(channel, "Streaming inference error: ${throwable.message}")
                                invoke.reject("Streaming inference error: ${throwable.message}", throwable as? Exception)
                            }
                        }
                    },
                    null as? java.util.Map<String, Any>
                )
            } catch (e: Exception) {
                Log.e(TAG, "send_message: failed to start streaming — ${e.message}")
                sendErrorEvent(channel, "Failed to start streaming: ${e.message}")
                invoke.reject("Failed to start streaming: ${e.message}", e)
            }
        }
    }

    /**
     * Send a token_batch event via the Channel.
     */
    private fun sendTokenBatch(channel: Channel, tokens: String, batchIndex: Int) {
        try {
            val data = JSObject()
            data.put("tokens", tokens)
            data.put("batch_index", batchIndex)
            val event = JSObject()
            event.put("event", "token_batch")
            event.put("data", data)
            channel.send(event)
        } catch (e: Exception) {
            Log.w(TAG, "sendTokenBatch: channel.send failed (frontend disconnected?): ${e.message}")
        }
    }

    /**
     * Send an error event via the Channel. Non-fatal if channel.send fails.
     */
    private fun sendErrorEvent(channel: Channel, errorMessage: String) {
        try {
            val data = JSObject()
            data.put("message", errorMessage)
            val event = JSObject()
            event.put("event", "error")
            event.put("data", data)
            channel.send(event)
        } catch (e: Exception) {
            Log.w(TAG, "sendErrorEvent: channel.send failed: ${e.message}")
        }
    }

    /**
     * Return the catalog of available models with their download status.
     *
     * Returns: JSON array of { id, displayName, url, fileName, sizeBytes, minRamGb, isDownloaded, localPath }
     */
    @Command
    fun list_models(invoke: Invoke) {
        Log.i(TAG, "list_models")

        try {
            val array = app.tauri.plugin.JSArray()
            for (model in ModelManager.AVAILABLE_MODELS) {
                val isDownloaded = ModelManager.isModelDownloaded(activity, model)
                val localPath = ModelManager.getModelPath(activity, model)

                val obj = JSObject()
                obj.put("id", model.id)
                obj.put("displayName", model.displayName)
                obj.put("url", model.url)
                obj.put("fileName", model.fileName)
                obj.put("sizeBytes", model.sizeBytes)
                obj.put("minRamGb", model.minRamGb)
                obj.put("isDownloaded", isDownloaded)
                obj.put("localPath", localPath)
                array.put(obj)

                Log.d(TAG, "list_models: ${model.id} — isDownloaded=$isDownloaded, localPath=$localPath")
            }

            val result = JSObject()
            result.put("models", array)
            invoke.resolve(result)
        } catch (e: Exception) {
            Log.e(TAG, "list_models: failed — ${e.message}", e)
            invoke.reject("Failed to list models: ${e.message}", e)
        }
    }

    /**
     * Download a model by ID, streaming progress events via a Channel.
     *
     * Args (via DownloadModelArgs): modelId (String), onProgress (Channel)
     *
     * Channel events:
     *   { event: "progress", data: { bytesDownloaded, totalBytes, bytesPerSecond } }
     *   { event: "complete", data: { modelPath, fileName } }
     *   { event: "error",    data: { message } }
     */
    @Command
    fun download_model(invoke: Invoke) {
        val args: DownloadModelArgs
        try {
            args = invoke.parseArgs(DownloadModelArgs::class.java)
        } catch (e: Exception) {
            return invoke.reject("Invalid arguments: ${e.message}", e)
        }

        val modelId = args.modelId
        val channel = args.onProgress

        Log.i(TAG, "download_model: modelId=$modelId, channelId=${channel.id}")

        val model = ModelManager.getModelById(modelId)
        if (model == null) {
            Log.e(TAG, "download_model: unknown modelId=$modelId")
            return invoke.reject("Unknown model ID: $modelId")
        }

        // Already downloaded? Resolve immediately.
        val existingPath = ModelManager.getModelPath(activity, model)
        if (existingPath != null) {
            Log.i(TAG, "download_model: $modelId already downloaded at $existingPath")
            val completeData = JSObject()
            completeData.put("modelPath", existingPath)
            completeData.put("fileName", model.fileName)
            val completeEvent = JSObject()
            completeEvent.put("event", "complete")
            completeEvent.put("data", completeData)
            try { channel.send(completeEvent) } catch (_: Exception) {}
            invoke.resolve()
            return
        }

        // Launch download on IO coroutine
        kotlinx.coroutines.launch(streamingScope.coroutineContext + Dispatchers.IO) {
            Log.i(TAG, "download_model: starting download for ${model.fileName}")

            try {
                ModelManager.downloadModel(activity, model, object : ModelManager.DownloadCallback {
                    override fun onProgress(bytesDownloaded: Long, totalBytes: Long, bytesPerSecond: Long) {
                        try {
                            val data = JSObject()
                            data.put("bytesDownloaded", bytesDownloaded)
                            data.put("totalBytes", totalBytes)
                            data.put("bytesPerSecond", bytesPerSecond)
                            val event = JSObject()
                            event.put("event", "progress")
                            event.put("data", data)
                            channel.send(event)
                        } catch (e: Exception) {
                            Log.w(TAG, "download_model: progress channel send failed: ${e.message}")
                        }
                    }

                    override fun onComplete(modelPath: String) {
                        Log.i(TAG, "download_model: complete — $modelPath")
                        try {
                            val data = JSObject()
                            data.put("modelPath", modelPath)
                            data.put("fileName", model.fileName)
                            val event = JSObject()
                            event.put("event", "complete")
                            event.put("data", data)
                            channel.send(event)
                        } catch (e: Exception) {
                            Log.w(TAG, "download_model: complete channel send failed: ${e.message}")
                        }
                        invoke.resolve()
                    }

                    override fun onError(error: String) {
                        Log.e(TAG, "download_model: error — $error")
                        try {
                            val data = JSObject()
                            data.put("message", error)
                            val event = JSObject()
                            event.put("event", "error")
                            event.put("data", data)
                            channel.send(event)
                        } catch (e: Exception) {
                            Log.w(TAG, "download_model: error channel send failed: ${e.message}")
                        }
                        invoke.reject("Download failed: $error")
                    }
                })
            } catch (e: Exception) {
                Log.e(TAG, "download_model: unexpected error — ${e.message}", e)
                try {
                    val data = JSObject()
                    data.put("message", e.message ?: "Unknown error")
                    val event = JSObject()
                    event.put("event", "error")
                    event.put("data", data)
                    channel.send(event)
                } catch (_: Exception) {}
                invoke.reject("Download failed: ${e.message}", e)
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
        streamingScope.cancel()
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
