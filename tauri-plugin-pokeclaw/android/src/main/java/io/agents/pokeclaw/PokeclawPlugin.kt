package io.agents.pokeclaw

import android.app.Activity
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.content.Intent
import android.content.IntentFilter
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.net.wifi.WifiInfo
import android.net.wifi.WifiManager
import android.content.ClipData
import android.content.ClipboardManager
import android.os.BatteryManager
import android.os.Environment
import android.os.Handler
import android.os.Looper
import android.os.StatFs
import android.provider.Settings
import android.util.Log
import android.view.accessibility.AccessibilityNodeInfo
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
import kotlinx.coroutines.withContext

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
    // Observation @Command methods — bridge accessibility service to IPC
    // -----------------------------------------------------------------------

    /**
     * Return the current screen accessibility tree as a structured string.
     *
     * No args required.
     * Returns: { success: Boolean, data: String?, error: String? }
     */
    @Command
    fun get_screen_info(invoke: Invoke) {
        Log.i(TAG, "get_screen_info: invoked")

        try {
            val service = PokeAccessibilityService.getConnectedInstance(3000)
            if (service == null) {
                Log.w(TAG, "get_screen_info: accessibility service not connected after 3000ms")
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "Accessibility service not running. Enable it in Settings > Accessibility.")
                invoke.resolve(result)
                return
            }

            val tree = service.getScreenTree()
            if (tree == null) {
                Log.w(TAG, "get_screen_info: screen tree is null (no active window)")
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "System dialog is blocking screen read")
                invoke.resolve(result)
                return
            }

            Log.i(TAG, "get_screen_info: tree returned (${tree.length} chars)")
            val result = JSObject()
            result.put("success", true)
            result.put("data", tree)
            result.put("error", null)
            invoke.resolve(result)

        } catch (e: Exception) {
            Log.e(TAG, "get_screen_info: error — ${e.message}", e)
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "Failed to read screen: ${e.message}")
            invoke.resolve(result)
        }
    }

    /**
     * Find accessibility nodes matching the given text.
     *
     * Args: text (String, required)
     * Returns: { success: Boolean, data: [node details], error: String? }
     * Each node: { index, className, text, contentDescription, clickable, enabled, visible, bounds }
     */
    @Command
    fun find_node_info(invoke: Invoke) {
        val args = invoke.getArgs()
        val text = args.getString("text")
            ?: return invoke.reject("text is required")

        Log.i(TAG, "find_node_info: text='$text'")

        try {
            val service = PokeAccessibilityService.getConnectedInstance(3000)
            if (service == null) {
                Log.w(TAG, "find_node_info: accessibility service not connected")
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "Accessibility service not running. Enable it in Settings > Accessibility.")
                invoke.resolve(result)
                return
            }

            val nodes = service.findNodesByText(text)
            val nodesArray = app.tauri.plugin.JSArray()

            for ((index, node) in nodes.withIndex()) {
                try {
                    val nodeObj = JSObject()
                    nodeObj.put("index", index)
                    nodeObj.put("className", node.className?.toString() ?: "")
                    nodeObj.put("text", node.text?.toString() ?: "")
                    nodeObj.put("contentDescription", node.contentDescription?.toString() ?: "")
                    nodeObj.put("clickable", node.isClickable)
                    nodeObj.put("enabled", node.isEnabled)
                    nodeObj.put("visible", node.isVisibleToUser)

                    val bounds = android.graphics.Rect()
                    node.getBoundsInScreen(bounds)
                    nodeObj.put("bounds", bounds.toShortString())

                    nodesArray.put(nodeObj)
                } catch (e: Exception) {
                    Log.w(TAG, "find_node_info: error reading node $index: ${e.message}")
                }
            }

            PokeAccessibilityService.recycleNodes(nodes)

            Log.i(TAG, "find_node_info: found ${nodes.size} nodes for '$text'")
            val result = JSObject()
            result.put("success", true)
            result.put("data", nodesArray)
            result.put("error", null)
            invoke.resolve(result)

        } catch (e: Exception) {
            Log.e(TAG, "find_node_info: error — ${e.message}", e)
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "Failed to find nodes: ${e.message}")
            invoke.resolve(result)
        }
    }

    /**
     * Get device system info for a given category.
     *
     * Args: category (String, required) — battery, wifi, storage, bluetooth, screen, device, time
     * Returns: { success: Boolean, data: String, error: String? }
     */
    @Command
    fun get_device_info(invoke: Invoke) {
        val args = invoke.getArgs()
        val category = args.getString("category")
            ?: return invoke.reject("category is required")

        Log.i(TAG, "get_device_info: category='$category'")

        try {
            val info: String = when (category.lowercase().trim()) {
                "battery" -> getBatteryInfo()
                "wifi" -> getWifiInfo()
                "storage" -> getStorageInfo()
                "bluetooth" -> getBluetoothInfo()
                "screen" -> getScreenDeviceInfo()
                "device" -> getDeviceDetails()
                "time" -> getCurrentTime()
                else -> {
                    val result = JSObject()
                    result.put("success", false)
                    result.put("data", "")
                    result.put("error", "Unknown category: $category. Use: battery, wifi, storage, bluetooth, screen, device, time")
                    invoke.resolve(result)
                    return
                }
            }

            Log.i(TAG, "get_device_info: $category -> ${info.take(80)}")
            val result = JSObject()
            result.put("success", true)
            result.put("data", info)
            result.put("error", null)
            invoke.resolve(result)

        } catch (e: Exception) {
            Log.e(TAG, "get_device_info: error for $category — ${e.message}", e)
            val result = JSObject()
            result.put("success", false)
            result.put("data", "")
            result.put("error", "Failed to get $category info: ${e.message}")
            invoke.resolve(result)
        }
    }

    /**
     * Check permission and service status for accessibility, notifications, foreground.
     *
     * No args required.
     * Returns: { success: Boolean, data: { accessibility_enabled, accessibility_running, notification_enabled, foreground_service }, error: String? }
     */
    @Command
    fun check_permissions(invoke: Invoke) {
        Log.i(TAG, "check_permissions: invoked")

        try {
            val data = JSObject()
            data.put("accessibility_enabled", PokeAccessibilityService.isEnabledInSettings(activity))
            data.put("accessibility_running", PokeAccessibilityService.isRunning())
            // NotificationListener is S03 scope — placeholder false
            data.put("notification_enabled", false)
            // Foreground service is S03 scope — placeholder false
            data.put("foreground_service", false)

            Log.i(TAG, "check_permissions: enabled=${data.getBoolean("accessibility_enabled")}, running=${data.getBoolean("accessibility_running")}")
            val result = JSObject()
            result.put("success", true)
            result.put("data", data)
            result.put("error", null)
            invoke.resolve(result)

        } catch (e: Exception) {
            Log.e(TAG, "check_permissions: error — ${e.message}", e)
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "Failed to check permissions: ${e.message}")
            invoke.resolve(result)
        }
    }

    // -----------------------------------------------------------------------
    // Gesture @Command methods — bridge gesture primitives to IPC
    // -----------------------------------------------------------------------

    /**
     * Perform a tap gesture at the specified screen coordinates.
     *
     * Args: x (int, required), y (int, required)
     * Returns: { success: Boolean, data: String?, error: String? }
     */
    @Command
    fun tap(invoke: Invoke) {
        val args = invoke.getArgs()
        val x: Int
        val y: Int
        try {
            x = args.getInt("x")
            y = args.getInt("y")
        } catch (e: Exception) {
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "x and y are required integers")
            invoke.resolve(result)
            return
        }

        Log.i(TAG, "tap: x=$x, y=$y")

        if (x < 0 || y < 0) {
            Log.w(TAG, "tap: negative coordinates rejected ($x, $y)")
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "Coordinates must be non-negative. Got x=$x, y=$y")
            invoke.resolve(result)
            return
        }

        try {
            val service = PokeAccessibilityService.getConnectedInstance(3000)
            if (service == null) {
                Log.w(TAG, "tap: accessibility service not connected after 3000ms")
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "Accessibility service not running. Enable it in Settings > Accessibility.")
                invoke.resolve(result)
                return
            }

            val success = service.performTap(x, y)
            Log.i(TAG, "tap: result=$success at ($x, $y)")
            val result = JSObject()
            result.put("success", success)
            result.put("data", if (success) "Tapped at ($x, $y)" else null)
            result.put("error", if (success) null else "Gesture dispatch failed — system may have rejected or cancelled the gesture")
            invoke.resolve(result)

        } catch (e: Exception) {
            Log.e(TAG, "tap: error — ${e.message}", e)
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "Tap failed: ${e.message}")
            invoke.resolve(result)
        }
    }

    /**
     * Perform a swipe gesture from one point to another.
     *
     * Args: start_x (int, required), start_y (int, required),
     *       end_x (int, required), end_y (int, required),
     *       duration_ms (int, optional, default 500)
     * Returns: { success: Boolean, data: String?, error: String? }
     */
    @Command
    fun swipe(invoke: Invoke) {
        val args = invoke.getArgs()
        val startX: Int
        val startY: Int
        val endX: Int
        val endY: Int
        try {
            startX = args.getInt("start_x")
            startY = args.getInt("start_y")
            endX = args.getInt("end_x")
            endY = args.getInt("end_y")
        } catch (e: Exception) {
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "start_x, start_y, end_x, end_y are required integers")
            invoke.resolve(result)
            return
        }
        val durationMs = args.optInt("duration_ms", 500)

        Log.i(TAG, "swipe: ($startX,$startY) → ($endX,$endY), duration_ms=$durationMs")

        if (startX < 0 || startY < 0 || endX < 0 || endY < 0) {
            Log.w(TAG, "swipe: negative coordinates rejected")
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "All coordinates must be non-negative. Got start=($startX,$startY) end=($endX,$endY)")
            invoke.resolve(result)
            return
        }

        if (durationMs < 0) {
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "duration_ms must be non-negative. Got $durationMs")
            invoke.resolve(result)
            return
        }

        try {
            val service = PokeAccessibilityService.getConnectedInstance(3000)
            if (service == null) {
                Log.w(TAG, "swipe: accessibility service not connected after 3000ms")
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "Accessibility service not running. Enable it in Settings > Accessibility.")
                invoke.resolve(result)
                return
            }

            val success = service.performSwipe(startX, startY, endX, endY, durationMs.toLong())
            Log.i(TAG, "swipe: result=$success ($startX,$startY) → ($endX,$endY)")
            val result = JSObject()
            result.put("success", success)
            result.put("data", if (success) "Swiped from ($startX,$startY) to ($endX,$endY) in ${durationMs}ms" else null)
            result.put("error", if (success) null else "Gesture dispatch failed — system may have rejected or cancelled the gesture")
            invoke.resolve(result)

        } catch (e: Exception) {
            Log.e(TAG, "swipe: error — ${e.message}", e)
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "Swipe failed: ${e.message}")
            invoke.resolve(result)
        }
    }

    /**
     * Perform a long-press gesture at the specified coordinates.
     *
     * Args: x (int, required), y (int, required),
     *       duration_ms (int, optional, default 1000)
     * Returns: { success: Boolean, data: String?, error: String? }
     */
    @Command
    fun long_press(invoke: Invoke) {
        val args = invoke.getArgs()
        val x: Int
        val y: Int
        try {
            x = args.getInt("x")
            y = args.getInt("y")
        } catch (e: Exception) {
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "x and y are required integers")
            invoke.resolve(result)
            return
        }
        val durationMs = args.optInt("duration_ms", 1000)

        Log.i(TAG, "long_press: x=$x, y=$y, duration_ms=$durationMs")

        if (x < 0 || y < 0) {
            Log.w(TAG, "long_press: negative coordinates rejected ($x, $y)")
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "Coordinates must be non-negative. Got x=$x, y=$y")
            invoke.resolve(result)
            return
        }

        if (durationMs < 0) {
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "duration_ms must be non-negative. Got $durationMs")
            invoke.resolve(result)
            return
        }

        try {
            val service = PokeAccessibilityService.getConnectedInstance(3000)
            if (service == null) {
                Log.w(TAG, "long_press: accessibility service not connected after 3000ms")
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "Accessibility service not running. Enable it in Settings > Accessibility.")
                invoke.resolve(result)
                return
            }

            val success = service.performLongPress(x, y, durationMs.toLong())
            Log.i(TAG, "long_press: result=$success at ($x, $y) for ${durationMs}ms")
            val result = JSObject()
            result.put("success", success)
            result.put("data", if (success) "Long-pressed at ($x, $y) for ${durationMs}ms" else null)
            result.put("error", if (success) null else "Gesture dispatch failed — system may have rejected or cancelled the gesture")
            invoke.resolve(result)

        } catch (e: Exception) {
            Log.e(TAG, "long_press: error — ${e.message}", e)
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "Long press failed: ${e.message}")
            invoke.resolve(result)
        }
    }

    /**
     * Tap an accessibility node by its ID (e.g. "n3").
     *
     * Resolves the node's center coordinates from the nodeIdMap populated by
     * the most recent get_screen_info call, then performs a tap at that location.
     *
     * Args: node_id (String, required)
     * Returns: { success: Boolean, data: String?, error: String? }
     */
    @Command
    fun tap_node(invoke: Invoke) {
        val args = invoke.getArgs()
        val rawNodeId = args.getString("node_id")
        if (rawNodeId.isNullOrBlank()) {
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "node_id is required")
            invoke.resolve(result)
            return
        }

        // Normalize: strip brackets if user passes "[n3]"
        val nodeId = rawNodeId.trim().removeSurrounding("[", "]")
        Log.i(TAG, "tap_node: rawNodeId='$rawNodeId', normalized='$nodeId'")

        try {
            val service = PokeAccessibilityService.getConnectedInstance(3000)
            if (service == null) {
                Log.w(TAG, "tap_node: accessibility service not connected after 3000ms")
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "Accessibility service not running. Enable it in Settings > Accessibility.")
                invoke.resolve(result)
                return
            }

            val coords = service.getNodeCoordinates(nodeId)
            if (coords == null) {
                Log.w(TAG, "tap_node: node '$nodeId' not found in nodeIdMap")
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "Node '$nodeId' not found. Call get_screen_info first to refresh node IDs.")
                invoke.resolve(result)
                return
            }

            val x = coords[0]
            val y = coords[1]
            Log.i(TAG, "tap_node: resolved '$nodeId' → ($x, $y), performing tap")

            val success = service.performTap(x, y)
            Log.i(TAG, "tap_node: result=$success for node '$nodeId' at ($x, $y)")
            val result = JSObject()
            result.put("success", success)
            result.put("data", if (success) "Tapped node '$nodeId' at ($x, $y)" else null)
            result.put("error", if (success) null else "Gesture dispatch failed — system may have rejected or cancelled the gesture")
            invoke.resolve(result)

        } catch (e: Exception) {
            Log.e(TAG, "tap_node: error — ${e.message}", e)
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "tap_node failed: ${e.message}")
            invoke.resolve(result)
        }
    }

    /**
     * Input text into a focused or specified editable field.
     *
     * Two-strategy approach: ACTION_SET_TEXT first, clipboard paste as fallback.
     * If node_id is provided, taps that node first to focus it.
     *
     * Args: text (String, required), node_id (String, optional),
     *       clear_first (Boolean, optional, default true)
     * Returns: { success: Boolean, data: String?, error: String? }
     */
    @Command
    fun input_text(invoke: Invoke) {
        val args = invoke.getArgs()
        val text = args.getString("text")
        if (text.isNullOrBlank()) {
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "text is required")
            invoke.resolve(result)
            return
        }

        val rawNodeId = args.optString("node_id", null)
        val clearFirst = args.optBoolean("clear_first", true)

        Log.i(TAG, "input_text: text='${text.take(40)}...', node_id=$rawNodeId, clear_first=$clearFirst")

        try {
            val service = PokeAccessibilityService.getConnectedInstance(3000)
            if (service == null) {
                Log.w(TAG, "input_text: accessibility service not connected after 3000ms")
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "Accessibility service not running. Enable it in Settings > Accessibility.")
                invoke.resolve(result)
                return
            }

            // If node_id provided, tap it to focus first
            if (rawNodeId != null) {
                val nodeId = rawNodeId.trim().removeSurrounding("[", "]")
                val coords = service.getNodeCoordinates(nodeId)
                if (coords == null) {
                    Log.w(TAG, "input_text: node '$nodeId' not found in nodeIdMap")
                    val result = JSObject()
                    result.put("success", false)
                    result.put("data", null)
                    result.put("error", "Node '$nodeId' not found. Call get_screen_info first to refresh node IDs.")
                    invoke.resolve(result)
                    return
                }
                Log.d(TAG, "input_text: tapping node '$nodeId' at (${coords[0]}, ${coords[1]}) to focus")
                service.performTap(coords[0], coords[1])
                Thread.sleep(300)
            }

            // Find focused editable node
            val root = service.getRootInActiveWindow()
            if (root == null) {
                Log.w(TAG, "input_text: no active window")
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "No active window. Make sure an app is in the foreground.")
                invoke.resolve(result)
                return
            }

            var targetNode: AccessibilityNodeInfo? = null
            try {
                // Try focused input node first
                targetNode = root.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
                if (targetNode != null && !targetNode.isEditable) {
                    targetNode.recycle()
                    targetNode = null
                }

                // Fallback: traverse for any editable node
                if (targetNode == null) {
                    targetNode = findEditableNode(root)
                }

                if (targetNode == null) {
                    Log.w(TAG, "input_text: no focused editable node found")
                    val result = JSObject()
                    result.put("success", false)
                    result.put("data", null)
                    result.put("error", "No editable text field is focused. Tap a text field first or provide node_id.")
                    invoke.resolve(result)
                    return
                }

                // Strategy 1: ACTION_SET_TEXT
                val args1 = android.os.Bundle()
                if (clearFirst) {
                    // Select all existing text, then set to empty
                    val selectArgs = android.os.Bundle()
                    selectArgs.putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, 0)
                    selectArgs.putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, Int.MAX_VALUE)
                    targetNode.performAction(AccessibilityNodeInfo.ACTION_SET_SELECTION, selectArgs)

                    args1.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text)
                    val setSuccess = targetNode.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, args1)
                    Log.i(TAG, "input_text: ACTION_SET_TEXT (clear+set) result=$setSuccess, strategy=ACTION_SET_TEXT")

                    if (setSuccess) {
                        val result = JSObject()
                        result.put("success", true)
                        result.put("data", "Text input via ACTION_SET_TEXT: '${text.take(30)}...'")
                        result.put("error", null)
                        invoke.resolve(result)
                        return
                    }
                } else {
                    // Append mode: get existing text, append new text
                    val existingText = targetNode.text?.toString() ?: ""
                    val newText = existingText + text
                    args1.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, newText)
                    val setSuccess = targetNode.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, args1)
                    Log.i(TAG, "input_text: ACTION_SET_TEXT (append) result=$setSuccess, strategy=ACTION_SET_TEXT")

                    if (setSuccess) {
                        val result = JSObject()
                        result.put("success", true)
                        result.put("data", "Text appended via ACTION_SET_TEXT: '${text.take(30)}...'")
                        result.put("error", null)
                        invoke.resolve(result)
                        return
                    }
                }

                // Strategy 2: Clipboard paste fallback
                Log.i(TAG, "input_text: ACTION_SET_TEXT failed, trying clipboard paste fallback")
                val clipSuccess = setClipboard(text)
                if (!clipSuccess) {
                    Log.e(TAG, "input_text: failed to set clipboard")
                    val result = JSObject()
                    result.put("success", false)
                    result.put("data", null)
                    result.put("error", "Failed to set clipboard for paste fallback")
                    invoke.resolve(result)
                    return
                }

                if (clearFirst) {
                    // Clear existing text first
                    val selectArgs = android.os.Bundle()
                    selectArgs.putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, 0)
                    selectArgs.putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, Int.MAX_VALUE)
                    targetNode.performAction(AccessibilityNodeInfo.ACTION_SET_SELECTION, selectArgs)
                }

                val pasteSuccess = targetNode.performAction(AccessibilityNodeInfo.ACTION_PASTE)
                Log.i(TAG, "input_text: ACTION_PASTE result=$pasteSuccess, strategy=clipboard_paste")

                if (pasteSuccess) {
                    val result = JSObject()
                    result.put("success", true)
                    result.put("data", "Text input via clipboard paste: '${text.take(30)}...'")
                    result.put("error", null)
                    invoke.resolve(result)
                } else {
                    val result = JSObject()
                    result.put("success", false)
                    result.put("data", null)
                    result.put("error", "Both ACTION_SET_TEXT and clipboard paste failed. The text field may not support text input.")
                    invoke.resolve(result)
                }

            } finally {
                targetNode?.recycle()
            }

        } catch (e: Exception) {
            Log.e(TAG, "input_text: error — ${e.message}", e)
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "Input text failed: ${e.message}")
            invoke.resolve(result)
        }
    }

    /**
     * Scroll through screens to find a node matching the given text.
     *
     * Uses coroutines to avoid ANR since this loops with scroll gestures.
     * Detects scroll end by comparing the screen tree before and after each scroll.
     *
     * Args: text (String, required), direction (String, optional, default "down"),
     *       max_scrolls (Int, optional, default 10, clamped 1-20)
     * Returns: { success: Boolean, data: String?, error: String? }
     */
    @Command
    fun scroll_to_find(invoke: Invoke) {
        val args = invoke.getArgs()
        val text = args.getString("text")
        if (text.isNullOrBlank()) {
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "text is required")
            invoke.resolve(result)
            return
        }

        val direction = args.optString("direction", "down").lowercase().trim()
        val maxScrolls = args.optInt("max_scrolls", 10).coerceIn(1, 20)

        Log.i(TAG, "scroll_to_find: text='$text', direction=$direction, max_scrolls=$maxScrolls")

        kotlinx.coroutines.launch(streamingScope.coroutineContext) {
            try {
                val result = performScrollToFind(text, direction, maxScrolls)
                invoke.resolve(result)
            } catch (e: Exception) {
                Log.e(TAG, "scroll_to_find: error — ${e.message}", e)
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "scroll_to_find failed: ${e.message}")
                invoke.resolve(result)
            }
        }
    }

    /**
     * Scroll through screens to find a node matching the given text, then tap it.
     *
     * Uses coroutines to avoid ANR since this loops with scroll gestures.
     * Same scroll logic as scroll_to_find, but taps the node when found.
     *
     * Args: text (String, required), direction (String, optional, default "down"),
     *       max_scrolls (Int, optional, default 10, clamped 1-20)
     * Returns: { success: Boolean, data: String?, error: String? }
     */
    @Command
    fun find_and_tap(invoke: Invoke) {
        val args = invoke.getArgs()
        val text = args.getString("text")
        if (text.isNullOrBlank()) {
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "text is required")
            invoke.resolve(result)
            return
        }

        val direction = args.optString("direction", "down").lowercase().trim()
        val maxScrolls = args.optInt("max_scrolls", 10).coerceIn(1, 20)

        Log.i(TAG, "find_and_tap: text='$text', direction=$direction, max_scrolls=$maxScrolls")

        kotlinx.coroutines.launch(streamingScope.coroutineContext) {
            try {
                val result = performFindAndTap(text, direction, maxScrolls)
                invoke.resolve(result)
            } catch (e: Exception) {
                Log.e(TAG, "find_and_tap: error — ${e.message}", e)
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "find_and_tap failed: ${e.message}")
                invoke.resolve(result)
            }
        }
    }

    // -----------------------------------------------------------------------
    // Complex gesture helpers
    // -----------------------------------------------------------------------

    /**
     * Sets the system clipboard to the given text via ClipboardManager.
     * Must run on the main thread — uses Handler(Looper.getMainLooper()) + CountDownLatch.
     *
     * @return true if clipboard was set successfully, false otherwise.
     */
    private fun setClipboard(text: String): Boolean {
        val latch = java.util.concurrent.CountDownLatch(1)
        val success = java.util.concurrent.atomic.AtomicBoolean(false)

        Handler(Looper.getMainLooper()).post {
            try {
                val clipboard = activity.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as? ClipboardManager
                if (clipboard == null) {
                    Log.e(TAG, "setClipboard: ClipboardManager not available")
                    latch.countDown()
                    return@post
                }
                val clip = ClipData.newPlainText("text", text)
                clipboard.setPrimaryClip(clip)
                success.set(true)
                Log.d(TAG, "setClipboard: clipboard set successfully")
            } catch (e: Exception) {
                Log.e(TAG, "setClipboard: failed — ${e.message}", e)
            } finally {
                latch.countDown()
            }
        }

        return try {
            latch.await(2, java.util.concurrent.TimeUnit.SECONDS)
            if (!success.get()) {
                Log.e(TAG, "setClipboard: timed out or failed")
            }
            success.get()
        } catch (e: InterruptedException) {
            Thread.currentThread().interrupt()
            Log.e(TAG, "setClipboard: interrupted", e)
            false
        }
    }

    /**
     * Find the first editable node in the tree via BFS.
     */
    private fun findEditableNode(root: AccessibilityNodeInfo): AccessibilityNodeInfo? {
        val queue = ArrayDeque<AccessibilityNodeInfo>()
        queue.add(root)
        while (queue.isNotEmpty()) {
            val node = queue.removeFirst()
            if (node.isEditable) return node
            for (i in 0 until node.childCount) {
                val child = node.getChild(i) ?: continue
                queue.add(child)
            }
        }
        return null
    }

    /**
     * Core scroll-to-find logic shared between scroll_to_find and find_and_tap.
     *
     * @param text Text to search for.
     * @param direction "down" or "up".
     * @param maxScrolls Maximum scroll iterations.
     * @return JSObject with the search result.
     */
    private suspend fun performScrollToFind(text: String, direction: String, maxScrolls: Int): JSObject {
        return withContext(Dispatchers.IO) {
            val service = PokeAccessibilityService.getConnectedInstance(3000)
            if (service == null) {
                Log.w(TAG, "scroll_to_find: accessibility service not connected after 3000ms")
                return@withContext makeErrorResult("Accessibility service not running. Enable it in Settings > Accessibility.")
            }

            // First check current screen
            val initialNodes = service.findNodesByText(text)
            val initialMatch = initialNodes.firstOrNull { it.isVisibleToUser }
            if (initialMatch != null) {
                val bounds = android.graphics.Rect()
                initialMatch.getBoundsInScreen(bounds)
                PokeAccessibilityService.recycleNodes(initialNodes)
                Log.i(TAG, "scroll_to_find: found '$text' on current screen at ${bounds.toShortString()}")
                return@withContext makeSuccessResult("Found '$text' on current screen at ${bounds.toShortString()}")
            }
            PokeAccessibilityService.recycleNodes(initialNodes)

            // Get screen dimensions for scroll coordinates
            val screenSize = service.getScreenSize()
            val screenWidth = screenSize[0]
            val screenHeight = screenSize[1]
            val centerX = screenWidth / 2
            val startY: Int
            val endY: Int
            when (direction) {
                "up" -> {
                    startY = screenHeight / 4
                    endY = (screenHeight * 3) / 4
                }
                else -> { // "down"
                    startY = (screenHeight * 3) / 4
                    endY = screenHeight / 4
                }
            }

            // Scroll loop
            for (i in 1..maxScrolls) {
                // Capture tree before scroll for end-detection
                val treeBefore = service.getScreenTree() ?: ""

                val swipeSuccess = service.performSwipe(centerX, startY, centerX, endY, 300)
                if (!swipeSuccess) {
                    Log.w(TAG, "scroll_to_find: scroll #$i swipe failed")
                }
                Thread.sleep(500)

                // Check if tree changed (scroll-end detection)
                val treeAfter = service.getScreenTree() ?: ""
                if (treeBefore == treeAfter && swipeSuccess) {
                    Log.i(TAG, "scroll_to_find: reached ${if (direction == "up") "top" else "bottom"} after $i scrolls (tree unchanged)")
                    return@withContext makeErrorResult("Reached ${if (direction == "up") "top" else "bottom"} of scrollable content after $i scrolls. '$text' not found.")
                }

                // Search for the text after scrolling
                val nodes = service.findNodesByText(text)
                val match = nodes.firstOrNull { it.isVisibleToUser }
                if (match != null) {
                    val bounds = android.graphics.Rect()
                    match.getBoundsInScreen(bounds)
                    val cx = (bounds.left + bounds.right) / 2
                    val cy = (bounds.top + bounds.bottom) / 2
                    PokeAccessibilityService.recycleNodes(nodes)
                    Log.i(TAG, "scroll_to_find: found '$text' after $i scrolls at ${bounds.toShortString()}")
                    return@withContext makeSuccessResult("Found '$text' after $i scrolls at ${bounds.toShortString()}, center ($cx, $cy)")
                }
                PokeAccessibilityService.recycleNodes(nodes)

                Log.d(TAG, "scroll_to_find: scroll #$i — '$text' not found, continuing")
            }

            Log.i(TAG, "scroll_to_find: '$text' not found after $maxScrolls scrolls")
            makeErrorResult("'$text' not found after $maxScrolls scrolls in direction '$direction'")
        }
    }

    /**
     * Core find-and-tap logic: scroll to find text, then tap the node.
     *
     * @param text Text to search for.
     * @param direction "down" or "up".
     * @param maxScrolls Maximum scroll iterations.
     * @return JSObject with the result.
     */
    private suspend fun performFindAndTap(text: String, direction: String, maxScrolls: Int): JSObject {
        return withContext(Dispatchers.IO) {
            val service = PokeAccessibilityService.getConnectedInstance(3000)
            if (service == null) {
                Log.w(TAG, "find_and_tap: accessibility service not connected after 3000ms")
                return@withContext makeErrorResult("Accessibility service not running. Enable it in Settings > Accessibility.")
            }

            // First check current screen
            val initialNodes = service.findNodesByText(text)
            val initialMatch = initialNodes.firstOrNull { it.isVisibleToUser }
            if (initialMatch != null) {
                val bounds = android.graphics.Rect()
                initialMatch.getBoundsInScreen(bounds)
                val cx = (bounds.left + bounds.right) / 2
                val cy = (bounds.top + bounds.bottom) / 2
                PokeAccessibilityService.recycleNodes(initialNodes)
                Log.i(TAG, "find_and_tap: found '$text' on current screen, tapping at ($cx, $cy)")
                val tapSuccess = service.performTap(cx, cy)
                return@withContext if (tapSuccess) {
                    makeSuccessResult("Found '$text' on current screen and tapped at ($cx, $cy)")
                } else {
                    makeErrorResult("Found '$text' at ($cx, $cy) but tap gesture failed")
                }
            }
            PokeAccessibilityService.recycleNodes(initialNodes)

            // Get screen dimensions for scroll coordinates
            val screenSize = service.getScreenSize()
            val screenWidth = screenSize[0]
            val screenHeight = screenSize[1]
            val centerX = screenWidth / 2
            val startY: Int
            val endY: Int
            when (direction) {
                "up" -> {
                    startY = screenHeight / 4
                    endY = (screenHeight * 3) / 4
                }
                else -> { // "down"
                    startY = (screenHeight * 3) / 4
                    endY = screenHeight / 4
                }
            }

            // Scroll loop
            for (i in 1..maxScrolls) {
                val treeBefore = service.getScreenTree() ?: ""

                val swipeSuccess = service.performSwipe(centerX, startY, centerX, endY, 300)
                if (!swipeSuccess) {
                    Log.w(TAG, "find_and_tap: scroll #$i swipe failed")
                }
                Thread.sleep(500)

                // Scroll-end detection
                val treeAfter = service.getScreenTree() ?: ""
                if (treeBefore == treeAfter && swipeSuccess) {
                    Log.i(TAG, "find_and_tap: reached ${if (direction == "up") "top" else "bottom"} after $i scrolls (tree unchanged)")
                    return@withContext makeErrorResult("Reached ${if (direction == "up") "top" else "bottom"} of scrollable content after $i scrolls. '$text' not found.")
                }

                // Search for the text after scrolling
                val nodes = service.findNodesByText(text)
                val match = nodes.firstOrNull { it.isVisibleToUser }
                if (match != null) {
                    val bounds = android.graphics.Rect()
                    match.getBoundsInScreen(bounds)
                    val cx = (bounds.left + bounds.right) / 2
                    val cy = (bounds.top + bounds.bottom) / 2
                    PokeAccessibilityService.recycleNodes(nodes)
                    Log.i(TAG, "find_and_tap: found '$text' after $i scrolls, tapping at ($cx, $cy)")
                    val tapSuccess = service.performTap(cx, cy)
                    return@withContext if (tapSuccess) {
                        makeSuccessResult("Found '$text' after $i scrolls and tapped at ($cx, $cy)")
                    } else {
                        makeErrorResult("Found '$text' at ($cx, $cy) after $i scrolls but tap gesture failed")
                    }
                }
                PokeAccessibilityService.recycleNodes(nodes)

                Log.d(TAG, "find_and_tap: scroll #$i — '$text' not found, continuing")
            }

            Log.i(TAG, "find_and_tap: '$text' not found after $maxScrolls scrolls")
            makeErrorResult("'$text' not found after $maxScrolls scrolls in direction '$direction'")
        }
    }

    /** Helper to build a success ToolResult JSObject. */
    private fun makeSuccessResult(data: String): JSObject {
        val result = JSObject()
        result.put("success", true)
        result.put("data", data)
        result.put("error", null as String?)
        return result
    }

    /** Helper to build an error ToolResult JSObject. */
    private fun makeErrorResult(error: String): JSObject {
        val result = JSObject()
        result.put("success", false)
        result.put("data", null as String?)
        result.put("error", error)
        return result
    }

    // -----------------------------------------------------------------------
    // Device info helpers (ported from legacy GetDeviceInfoTool)
    // -----------------------------------------------------------------------

    private fun getBatteryInfo(): String {
        val bm = activity.getSystemService(android.content.Context.BATTERY_SERVICE) as? BatteryManager
            ?: return "Battery: unable to query"
        val level = bm.getIntProperty(BatteryManager.BATTERY_PROPERTY_CAPACITY)
        val status = bm.getIntProperty(BatteryManager.BATTERY_PROPERTY_STATUS)
        val charging = status == BatteryManager.BATTERY_STATUS_CHARGING ||
                status == BatteryManager.BATTERY_STATUS_FULL

        val filter = IntentFilter(Intent.ACTION_BATTERY_CHANGED)
        val batteryIntent = activity.registerReceiver(null, filter)
        val tempRaw = batteryIntent?.getIntExtra(BatteryManager.EXTRA_TEMPERATURE, 0) ?: 0
        val tempC = tempRaw / 10.0f

        val sb = StringBuilder()
        sb.append("Battery: ").append(level).append("%")
        sb.append(if (charging) ", charging" else ", not charging")
        if (tempC > 0) sb.append(String.format(", %.1f°C", tempC))
        return sb.toString()
    }

    private fun getWifiInfo(): String {
        val wm = activity.applicationContext.getSystemService(android.content.Context.WIFI_SERVICE) as? WifiManager
            ?: return "WiFi: unable to query"
        if (!wm.isWifiEnabled) return "WiFi: disabled"

        val cm = activity.getSystemService(android.content.Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
        val info = wm.connectionInfo
        if (info == null || info.networkId == -1) return "WiFi: enabled but not connected"

        val ssid = info.ssid?.replace("\"", "") ?: "unknown"
        val rssi = info.rssi
        val freq = info.frequency
        val speed = info.linkSpeed
        val band = if (freq > 4900) "5GHz" else "2.4GHz"

        return "WiFi: connected to '$ssid', $band, signal ${rssi}dBm, ${speed}Mbps"
    }

    private fun getStorageInfo(): String {
        val stat = StatFs(Environment.getDataDirectory().absolutePath)
        val totalBytes = stat.totalBytes
        val freeBytes = stat.availableBytes
        val usedBytes = totalBytes - freeBytes
        val pct = (usedBytes * 100 / totalBytes).toInt()
        return "Storage: ${formatBytes(usedBytes)} used of ${formatBytes(totalBytes)} ($pct%), ${formatBytes(freeBytes)} free"
    }

    private fun getBluetoothInfo(): String {
        val adapter = BluetoothAdapter.getDefaultAdapter()
            ?: return "Bluetooth: not available on this device"
        if (!adapter.isEnabled) return "Bluetooth: disabled"

        val sb = StringBuilder("Bluetooth: enabled")
        try {
            val bonded = adapter.bondedDevices
            if (!bonded.isNullOrEmpty()) {
                sb.append(", paired devices: ")
                val names = bonded.take(5).map {
                    it.name ?: it.address
                }
                sb.append(names.joinToString(", "))
                if (bonded.size > 5) sb.append("...")
            }
        } catch (e: SecurityException) {
            sb.append(" (cannot list devices — permission denied)")
        }
        return sb.toString()
    }

    private fun getScreenDeviceInfo(): String {
        val sb = StringBuilder()
        try {
            val brightness = Settings.System.getInt(activity.contentResolver, Settings.System.SCREEN_BRIGHTNESS)
            val pct = brightness * 100 / 255
            sb.append("Brightness: ").append(pct).append("%")
        } catch (e: Settings.SettingNotFoundException) {
            sb.append("Brightness: unknown")
        }

        val nightMode = activity.resources.configuration.uiMode and
                android.content.res.Configuration.UI_MODE_NIGHT_MASK
        val isDark = nightMode == android.content.res.Configuration.UI_MODE_NIGHT_YES
        sb.append(", Dark mode: ").append(if (isDark) "ON" else "OFF")

        try {
            val autoBrightness = Settings.System.getInt(activity.contentResolver,
                Settings.System.SCREEN_BRIGHTNESS_MODE)
            sb.append(", Auto-brightness: ").append(if (autoBrightness == 1) "ON" else "OFF")
        } catch (_: Settings.SettingNotFoundException) {}

        return sb.toString()
    }

    private fun getDeviceDetails(): String {
        val sb = StringBuilder()
        sb.append("Android ").append(android.os.Build.VERSION.RELEASE)
        sb.append(" (API ").append(android.os.Build.VERSION.SDK_INT).append(")")
        sb.append(", Model: ").append(android.os.Build.MANUFACTURER).append(" ").append(android.os.Build.MODEL)
        sb.append(", Build: ").append(android.os.Build.DISPLAY)
        val security = android.os.Build.VERSION.SECURITY_PATCH
        if (!security.isNullOrEmpty()) {
            sb.append(", Security patch: ").append(security)
        }
        return sb.toString()
    }

    private fun getCurrentTime(): String {
        val sdf = java.text.SimpleDateFormat("yyyy-MM-dd HH:mm:ss z", java.util.Locale.getDefault())
        val localTime = sdf.format(java.util.Date())
        val tz = java.util.TimeZone.getDefault()
        return "Current time: $localTime (timezone: ${tz.id}, UTC offset: ${tz.rawOffset / 3600000}h)"
    }

    private fun formatBytes(bytes: Long): String {
        return if (bytes >= 1_000_000_000L) {
            String.format("%.1f GB", bytes / 1_000_000_000.0)
        } else {
            String.format("%.0f MB", bytes / 1_000_000.0)
        }
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
