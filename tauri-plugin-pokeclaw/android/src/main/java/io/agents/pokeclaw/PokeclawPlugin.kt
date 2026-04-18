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
import android.content.pm.PackageManager
import android.net.Uri
import android.os.BatteryManager
import android.os.Build
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
        streamingScope.launch(streamingScope.coroutineContext) {
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
        streamingScope.launch(streamingScope.coroutineContext + Dispatchers.IO) {
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
            val notificationEnabled = try {
                // Check if the notification listener is enabled in system settings
                val enabledListeners = Settings.Secure.getString(
                    activity.contentResolver,
                    "enabled_notification_listeners"
                ) ?: ""
                val componentName = "${activity.packageName}/${PokeNotificationListener::class.java.name}"
                enabledListeners.contains(componentName)
            } catch (e: Exception) {
                Log.w(TAG, "check_permissions: failed to check notification listener settings", e)
                false
            }

            val foregroundRunning = PokeForegroundService.isRunning()

            val data = JSObject()
            data.put("accessibility_enabled", PokeAccessibilityService.isEnabledInSettings(activity))
            data.put("accessibility_running", PokeAccessibilityService.isRunning())
            data.put("notification_enabled", notificationEnabled)
            data.put("foreground_service", foregroundRunning)

            Log.i(TAG, "check_permissions: accessibility_enabled=${data.getBoolean("accessibility_enabled")}, accessibility_running=${data.getBoolean("accessibility_running")}, notification_enabled=$notificationEnabled, foreground_service=$foregroundRunning")
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
    // Navigation & Utility @Command methods — S03 new tools
    // -----------------------------------------------------------------------

    /**
     * Well-known app name → package name mapping for convenience.
     */
    private val WELL_KNOWN_APPS = mapOf(
        "whatsapp" to "com.whatsapp",
        "telegram" to "org.telegram.messenger",
        "instagram" to "com.instagram.android",
        "facebook" to "com.facebook.katana",
        "twitter" to "com.twitter.android",
        "x" to "com.twitter.android",
        "youtube" to "com.google.android.youtube",
        "chrome" to "com.android.chrome",
        "gmail" to "com.google.android.gm",
        "google maps" to "com.google.android.apps.maps",
        "maps" to "com.google.android.apps.maps",
        "spotify" to "com.spotify.music",
        "tiktok" to "com.zhiliaoapp.musically",
        "snapchat" to "com.snapchat.android",
        "signal" to "org.thoughtcrime.securesms",
        "discord" to "com.discord",
        "slack" to "com.Slack",
        "line" to "jp.naver.line.android",
        "weChat" to "com.tencent.mm",
        "phone" to "com.google.android.dialer",
        "dialer" to "com.google.android.dialer",
        "settings" to "com.android.settings",
        "camera" to "com.android.camera",
        "photos" to "com.google.android.apps.photos",
        "files" to "com.google.android.apps.nbu.files",
        "messages" to "com.google.android.apps.messaging",
        "sms" to "com.google.android.apps.messaging",
    )

    /**
     * Dispatch a system key action (back, home, recent_apps, etc.) via the accessibility service.
     *
     * Args: action (String, required) — back, home, recent_apps, notifications, collapse_notifications, lock_screen, unlock_screen
     * Returns: { success: Boolean, data: String?, error: String? }
     */
    @Command
    fun system_key(invoke: Invoke) {
        val args = invoke.getArgs()
        val action = args.getString("action")
            ?.lowercase()?.trim()
            ?: return invoke.reject("action is required")

        Log.i(TAG, "system_key: action='$action'")

        try {
            val service = PokeAccessibilityService.getConnectedInstance(3000)
            if (service == null) {
                Log.w(TAG, "system_key: accessibility service not connected after 3000ms")
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "Accessibility service not running. Enable it in Settings > Accessibility.")
                invoke.resolve(result)
                return
            }

            val success: Boolean
            val label: String
            when (action) {
                "back" -> { success = service.pressBack(); label = "Back" }
                "home" -> { success = service.pressHome(); label = "Home" }
                "recent_apps" -> { success = service.openRecentApps(); label = "Recent apps" }
                "notifications" -> { success = service.expandNotifications(); label = "Expand notifications" }
                "collapse_notifications" -> { success = service.collapseNotifications(); label = "Collapse notifications" }
                "lock_screen" -> { success = service.lockScreen(); label = "Lock screen" }
                "unlock_screen" -> { success = service.unlockScreen(); label = "Unlock screen" }
                else -> {
                    Log.w(TAG, "system_key: unknown action '$action'")
                    val result = JSObject()
                    result.put("success", false)
                    result.put("data", null)
                    result.put("error", "Unknown action: '$action'. Supported: back, home, recent_apps, notifications, collapse_notifications, lock_screen, unlock_screen")
                    invoke.resolve(result)
                    return
                }
            }

            Log.i(TAG, "system_key: $action result=$success")
            val result = JSObject()
            result.put("success", success)
            result.put("data", if (success) "$label pressed" else null)
            result.put("error", if (success) null else "System key action '$action' failed — service may have rejected the global action")
            invoke.resolve(result)

        } catch (e: Exception) {
            Log.e(TAG, "system_key: error — ${e.message}", e)
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "system_key failed: ${e.message}")
            invoke.resolve(result)
        }
    }

    /**
     * Open an app by name or package name.
     *
     * Resolves common app names (whatsapp, telegram, etc.) to package names.
     * Falls back to treating the input as a raw package name.
     * After launching, attempts to dismiss any chain-launch dialog by pressing Back after a short delay.
     *
     * Args: app_name (String, required)
     * Returns: { success: Boolean, data: String?, error: String? }
     */
    @Command
    fun open_app(invoke: Invoke) {
        val args = invoke.getArgs()
        val appName = args.getString("app_name")
            ?.trim()
            ?: return invoke.reject("app_name is required")

        Log.i(TAG, "open_app: app_name='$appName'")

        try {
            val service = PokeAccessibilityService.getConnectedInstance(3000)
            if (service == null) {
                Log.w(TAG, "open_app: accessibility service not connected after 3000ms")
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "Accessibility service not running. Enable it in Settings > Accessibility.")
                invoke.resolve(result)
                return
            }

            // Resolve app name to package name
            val packageName = WELL_KNOWN_APPS[appName.lowercase()] ?: run {
                // Check if it's already a package name (contains a dot)
                if (appName.contains(".")) {
                    appName
                } else {
                    // Try to find a matching installed app by label
                    resolveAppNameToPackage(appName)
                }
            }

            if (packageName == null) {
                Log.w(TAG, "open_app: could not resolve app '$appName' to a package name")
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "Could not resolve app '$appName'. Use a well-known name or provide the package name (e.g. com.whatsapp).")
                invoke.resolve(result)
                return
            }

            Log.i(TAG, "open_app: resolved '$appName' → package='$packageName'")
            val launchSuccess = service.openApp(packageName)

            if (launchSuccess) {
                // Dismiss chain-launch dialog after a short delay
                streamingScope.launch(streamingScope.coroutineContext) {
                    Thread.sleep(1500)
                    try {
                        service.pressBack()
                        Log.d(TAG, "open_app: dismissed chain-launch dialog for $packageName")
                    } catch (e: Exception) {
                        Log.d(TAG, "open_app: no chain-launch dialog to dismiss for $packageName")
                    }
                }
            }

            Log.i(TAG, "open_app: result=$launchSuccess for '$appName' ($packageName)")
            val result = JSObject()
            result.put("success", launchSuccess)
            result.put("data", if (launchSuccess) "Opened $appName ($packageName)" else null)
            result.put("error", if (launchSuccess) null else "Failed to open $appName ($packageName). The app may not be installed.")
            invoke.resolve(result)

        } catch (e: Exception) {
            Log.e(TAG, "open_app: error — ${e.message}", e)
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "open_app failed: ${e.message}")
            invoke.resolve(result)
        }
    }

    /**
     * Get active notifications from PokeNotificationListener.
     *
     * No args required.
     * Returns: { success: Boolean, data: [notification objects], error: String? }
     * Each notification: { package_name, key, post_time, ticker_text, is_ongoing, is_clearable }
     */
    @Command
    fun get_notifications(invoke: Invoke) {
        Log.i(TAG, "get_notifications: invoked")

        try {
            val listener = PokeNotificationListener.getInstance()
            if (listener == null) {
                Log.w(TAG, "get_notifications: notification listener not connected")
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "Notification listener not running. Enable it in Settings > Apps > Special access > Notification access.")
                invoke.resolve(result)
                return
            }

            val notifications = listener.getActiveNotificationsList()
            val dataArray = app.tauri.plugin.JSArray()

            for (notif in notifications) {
                val obj = JSObject()
                obj.put("package_name", notif["package_name"] ?: "")
                obj.put("key", notif["key"] ?: "")
                obj.put("post_time", notif["post_time"] ?: 0L)
                obj.put("ticker_text", notif["ticker_text"] ?: "")
                obj.put("is_ongoing", notif["is_ongoing"] ?: false)
                obj.put("is_clearable", notif["is_clearable"] ?: false)
                dataArray.put(obj)
            }

            Log.i(TAG, "get_notifications: returned ${notifications.size} notifications")
            val result = JSObject()
            result.put("success", true)
            result.put("data", dataArray)
            result.put("error", null as String?)
            invoke.resolve(result)

        } catch (e: Exception) {
            Log.e(TAG, "get_notifications: error — ${e.message}", e)
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "get_notifications failed: ${e.message}")
            invoke.resolve(result)
        }
    }

    /**
     * Get or set the system clipboard content.
     *
     * Args: action (String, required — "get" or "set"), text (String, required for "set")
     * Returns: { success: Boolean, data: String?, error: String? }
     */
    @Command
    fun clipboard(invoke: Invoke) {
        val args = invoke.getArgs()
        val action = args.getString("action")
            ?.lowercase()?.trim()
            ?: return invoke.reject("action is required (get or set)")

        Log.i(TAG, "clipboard: action='$action'")

        try {
            val clipboard = activity.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as? ClipboardManager
            if (clipboard == null) {
                Log.e(TAG, "clipboard: ClipboardManager not available")
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "ClipboardManager not available")
                invoke.resolve(result)
                return
            }

            when (action) {
                "get" -> {
                    val clip = clipboard.primaryClip
                    val text = if (clip != null && clip.itemCount > 0) {
                        clip.getItemAt(0)?.text?.toString() ?: ""
                    } else {
                        ""
                    }
                    Log.i(TAG, "clipboard: get returned ${text.length} chars")
                    val result = JSObject()
                    result.put("success", true)
                    result.put("data", text)
                    result.put("error", null as String?)
                    invoke.resolve(result)
                }
                "set" -> {
                    val text = args.getString("text")
                        ?: return invoke.reject("text is required for set action")

                    val latch = java.util.concurrent.CountDownLatch(1)
                    val success = java.util.concurrent.atomic.AtomicBoolean(false)

                    Handler(Looper.getMainLooper()).post {
                        try {
                            val clip = ClipData.newPlainText("text", text)
                            clipboard.setPrimaryClip(clip)
                            success.set(true)
                            Log.d(TAG, "clipboard: set ${text.length} chars")
                        } catch (e: Exception) {
                            Log.e(TAG, "clipboard: set failed — ${e.message}", e)
                        } finally {
                            latch.countDown()
                        }
                    }

                    val completed = latch.await(2, java.util.concurrent.TimeUnit.SECONDS)
                    if (completed && success.get()) {
                        Log.i(TAG, "clipboard: set ${text.length} chars successfully")
                        val result = JSObject()
                        result.put("success", true)
                        result.put("data", "Clipboard set to '${text.take(50)}${if (text.length > 50) "..." else ""}'")
                        result.put("error", null as String?)
                        invoke.resolve(result)
                    } else {
                        Log.e(TAG, "clipboard: set failed or timed out")
                        val result = JSObject()
                        result.put("success", false)
                        result.put("data", null)
                        result.put("error", "Failed to set clipboard${if (!completed) " (timed out)" else ""}")
                        invoke.resolve(result)
                    }
                }
                else -> {
                    Log.w(TAG, "clipboard: unknown action '$action'")
                    val result = JSObject()
                    result.put("success", false)
                    result.put("data", null)
                    result.put("error", "Unknown action: '$action'. Use 'get' or 'set'.")
                    invoke.resolve(result)
                }
            }

        } catch (e: Exception) {
            Log.e(TAG, "clipboard: error — ${e.message}", e)
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "clipboard failed: ${e.message}")
            invoke.resolve(result)
        }
    }

    /**
     * Get the list of installed applications on the device.
     *
     * Args: filter (String, optional — filter by app name, case-insensitive)
     * Returns: { success: Boolean, data: [{ package_name, app_name, is_system }], error: String? }
     */
    @Command
    fun get_installed_apps(invoke: Invoke) {
        val args = invoke.getArgs()
        val filter = args.optString("filter", null)?.lowercase()?.trim()

        Log.i(TAG, "get_installed_apps: filter='$filter'")

        try {
            val pm = activity.packageManager
            val apps = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                pm.getInstalledApplications(PackageManager.ApplicationInfoFlags.of(0))
            } else {
                @Suppress("DEPRECATION")
                pm.getInstalledApplications(0)
            }

            val dataArray = app.tauri.plugin.JSArray()
            var count = 0

            for (appInfo in apps) {
                val label = try {
                    appInfo.loadLabel(pm)?.toString() ?: ""
                } catch (e: Exception) {
                    ""
                }

                // Apply filter if provided
                if (filter != null) {
                    val matchesLabel = label.lowercase().contains(filter)
                    val matchesPackage = appInfo.packageName.lowercase().contains(filter)
                    if (!matchesLabel && !matchesPackage) continue
                }

                val obj = JSObject()
                obj.put("package_name", appInfo.packageName)
                obj.put("app_name", label)
                obj.put("is_system", (appInfo.flags and android.content.pm.ApplicationInfo.FLAG_SYSTEM) != 0)
                dataArray.put(obj)
                count++
            }

            Log.i(TAG, "get_installed_apps: returned $count apps${if (filter != null) " matching '$filter'" else ""}")
            val result = JSObject()
            result.put("success", true)
            result.put("data", dataArray)
            result.put("error", null as String?)
            invoke.resolve(result)

        } catch (e: Exception) {
            Log.e(TAG, "get_installed_apps: error — ${e.message}", e)
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "get_installed_apps failed: ${e.message}")
            invoke.resolve(result)
        }
    }

    /**
     * Initiate a phone call using ACTION_DIAL with optional contact resolution.
     *
     * Args: contact (String, required — phone number or contact name)
     * Returns: { success: Boolean, data: String?, error: String? }
     */
    @Command
    fun make_call(invoke: Invoke) {
        val args = invoke.getArgs()
        val contact = args.getString("contact")
            ?.trim()
            ?: return invoke.reject("contact is required (phone number or contact name)")

        Log.i(TAG, "make_call: contact='$contact'")

        try {
            // If it looks like a phone number (contains digits, possibly with +, -, spaces, parens), dial directly
            val cleanContact = contact.replace("[+\\-()\\s]".toRegex(), "")
            val isPhoneNumber = cleanContact.isNotEmpty() && cleanContact.all { it.isDigit() }

            val phoneNumber = if (isPhoneNumber) {
                contact
            } else {
                // Try to resolve contact name using ContactsContract
                resolveContactToPhone(contact)
            }

            if (phoneNumber == null) {
                Log.w(TAG, "make_call: could not resolve '$contact' to a phone number")
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "Could not resolve '$contact' to a phone number. Provide a phone number directly.")
                invoke.resolve(result)
                return
            }

            Log.i(TAG, "make_call: resolved '$contact' → dialing '$phoneNumber'")
            val dialIntent = Intent(Intent.ACTION_DIAL).apply {
                data = Uri.parse("tel:$phoneNumber")
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            }
            activity.startActivity(dialIntent)

            Log.i(TAG, "make_call: dial screen opened for '$phoneNumber'")
            val result = JSObject()
            result.put("success", true)
            result.put("data", "Dial screen opened for $phoneNumber")
            result.put("error", null as String?)
            invoke.resolve(result)

        } catch (e: Exception) {
            Log.e(TAG, "make_call: error — ${e.message}", e)
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "make_call failed: ${e.message}")
            invoke.resolve(result)
        }
    }

    /**
     * Open an Android Settings page for the given permission target.
     *
     * Args: target (String, required — "accessibility", "notifications", or "foreground")
     * Returns: { success: Boolean, data: { opened: String }, error: String? }
     */
    @Command
    fun open_permission_settings(invoke: Invoke) {
        val args = invoke.getArgs()
        val target = args.getString("target")
            ?.trim()
            ?: return invoke.reject("target is required (accessibility, notifications, or foreground)")

        Log.i(TAG, "open_permission_settings: target='$target'")

        try {
            val intent = when (target.lowercase()) {
                "accessibility" -> Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)
                "notifications" -> Intent("android.settings.ACTION_NOTIFICATION_LISTENER_SETTINGS")
                "foreground" -> Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS).apply {
                    data = Uri.fromParts("package", activity.packageName, null)
                }
                else -> {
                    Log.w(TAG, "open_permission_settings: unknown target '$target'")
                    val result = JSObject()
                    result.put("success", false)
                    result.put("data", null)
                    result.put("error", "Unknown permission target: '$target'. Use 'accessibility', 'notifications', or 'foreground'.")
                    invoke.resolve(result)
                    return
                }
            }
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            activity.startActivity(intent)

            Log.i(TAG, "open_permission_settings: opened '$target' settings")
            val result = JSObject()
            result.put("success", true)
            result.put("data", JSObject().put("opened", "$target settings"))
            result.put("error", null as String?)
            invoke.resolve(result)

        } catch (e: Exception) {
            Log.e(TAG, "open_permission_settings: error — ${e.message}", e)
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "open_permission_settings failed: ${e.message}")
            invoke.resolve(result)
        }
    }

    /**
     * Take a screenshot of the current screen and return the file path.
     *
     * Uses the accessibility service's takeScreenshot API (API 30+).
     * Returns: { success: Boolean, data: String (file path), error: String? }
     */
    @Command
    fun take_screenshot(invoke: Invoke) {
        Log.i(TAG, "take_screenshot: invoked")

        try {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
                Log.w(TAG, "take_screenshot: not supported below API 30 (current: ${Build.VERSION.SDK_INT})")
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "Screenshot requires Android 11 (API 30) or higher. Current: API ${Build.VERSION.SDK_INT}")
                invoke.resolve(result)
                return
            }

            val service = PokeAccessibilityService.getConnectedInstance(3000)
            if (service == null) {
                Log.w(TAG, "take_screenshot: accessibility service not connected after 3000ms")
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "Accessibility service not running. Enable it in Settings > Accessibility.")
                invoke.resolve(result)
                return
            }

            val args = invoke.getArgs()
            val customPath = args.optString("file_path", null)

            val filePath = service.takeScreenshot(customPath)
            if (filePath != null) {
                Log.i(TAG, "take_screenshot: saved to $filePath")
                val result = JSObject()
                result.put("success", true)
                result.put("data", filePath)
                result.put("error", null as String?)
                invoke.resolve(result)
            } else {
                Log.w(TAG, "take_screenshot: service returned null path")
                val result = JSObject()
                result.put("success", false)
                result.put("data", null)
                result.put("error", "Screenshot capture failed — the service may not have capture permission or the screen may be secured")
                invoke.resolve(result)
            }

        } catch (e: Exception) {
            Log.e(TAG, "take_screenshot: error — ${e.message}", e)
            val result = JSObject()
            result.put("success", false)
            result.put("data", null)
            result.put("error", "take_screenshot failed: ${e.message}")
            invoke.resolve(result)
        }
    }

    // -----------------------------------------------------------------------
    // Compound @Command methods — multi-step automation flows
    // -----------------------------------------------------------------------

    /**
     * Send a chat message to a contact through a messaging app.
     *
     * Compound tool that chains: resolve app → open app → wait for active window →
     * find contact in tree → tap contact → find bottom EditText → type message →
     * tap send button (or press Enter).
     *
     * Runs on Dispatchers.IO via streamingScope to avoid ANR.
     *
     * Args: app_name (String, required), contact (String, required), message (String, required)
     * Returns: { success: Boolean, data: String?, error: String? }
     */
    @Command
    fun send_chat_message(invoke: Invoke) {
        val args = invoke.getArgs()
        val appName = args.getString("app_name")
            ?.trim()
            ?: return invoke.reject("app_name is required")
        val contact = args.getString("contact")
            ?.trim()
            ?: return invoke.reject("contact is required")
        val message = args.getString("message")
            ?.trim()
            ?: return invoke.reject("message is required")

        Log.i(TAG, "send_chat_message: app='$appName', contact='$contact', message='${message.take(50)}...'")

        streamingScope.launch(streamingScope.coroutineContext) {
            try {
                val result = performSendChatMessage(appName, contact, message)
                invoke.resolve(result)
            } catch (e: Exception) {
                Log.e(TAG, "send_chat_message: unexpected error — ${e.message}", e)
                invoke.resolve(makeErrorResult("send_chat_message failed: ${e.message}"))
            }
        }
    }

    /**
     * Core logic for send_chat_message: multi-step message sending flow.
     *
     * Steps:
     * 1. Resolve app name to package name (WELL_KNOWN_APPS + PackageManager)
     * 2. Open the app via accessibility service
     * 3. Wait for the app window to become active
     * 4. Dismiss any chain-launch dialog
     * 5. Search the accessibility tree for the contact name
     * 6. Tap the matching contact node
     * 7. Wait for the chat conversation screen to load
     * 8. Find the message input EditText (bottom of screen)
     * 9. Type the message via ACTION_SET_TEXT
     * 10. Find and tap the send button (or press Enter as fallback)
     *
     * Each step logs success/failure for observability.
     */
    private suspend fun performSendChatMessage(appName: String, contact: String, message: String): JSObject {
        return withContext(Dispatchers.IO) {
            // Step 1: Resolve app name to package
            val packageName = resolveAppName(appName)
            if (packageName == null) {
                Log.e(TAG, "send_chat_message: could not resolve app '$appName'")
                return@withContext makeErrorResult("Could not resolve app '$appName'. Use a well-known name (whatsapp, telegram, etc.) or provide the package name.")
            }
            Log.i(TAG, "send_chat_message: resolved '$appName' → $packageName")

            // Step 2: Connect to accessibility service
            val service = PokeAccessibilityService.getConnectedInstance(3000)
            if (service == null) {
                Log.w(TAG, "send_chat_message: accessibility service not connected after 3000ms")
                return@withContext makeErrorResult("Accessibility service not running. Enable it in Settings > Accessibility.")
            }

            // Step 3: Open the app
            Log.i(TAG, "send_chat_message: opening $packageName")
            val launchSuccess = service.openApp(packageName)
            if (!launchSuccess) {
                Log.e(TAG, "send_chat_message: failed to open $packageName")
                return@withContext makeErrorResult("Failed to open $appName ($packageName). The app may not be installed.")
            }

            // Step 4: Wait for app window to load
            val windowReady = waitForWindow(service, packageName, 5000)
            if (!windowReady) {
                Log.w(TAG, "send_chat_message: timed out waiting for $packageName window (proceeding anyway)")
            } else {
                Log.i(TAG, "send_chat_message: $packageName window is active")
            }

            // Step 5: Dismiss chain-launch dialog (manufacturer-specific "Allow" prompts)
            dismissChainLaunchDialog(service)

            // Step 6: Search for contact in the accessibility tree
            Log.i(TAG, "send_chat_message: searching for contact '$contact'")
            val contactNodes = service.findNodesByText(contact)
            val contactNode = contactNodes.firstOrNull {
                it.isVisibleToUser && it.isClickable
            } ?: contactNodes.firstOrNull {
                it.isVisibleToUser
            }

            if (contactNode == null) {
                PokeAccessibilityService.recycleNodes(contactNodes)
                Log.e(TAG, "send_chat_message: contact '$contact' not found in $appName tree")
                return@withContext makeErrorResult("Contact '$contact' not found in $appName. Make sure you are on the contacts/chats list screen.")
            }

            // Tap the contact
            val contactBounds = android.graphics.Rect()
            contactNode.getBoundsInScreen(contactBounds)
            val contactCx = (contactBounds.left + contactBounds.right) / 2
            val contactCy = (contactBounds.top + contactBounds.bottom) / 2
            PokeAccessibilityService.recycleNodes(contactNodes)

            Log.i(TAG, "send_chat_message: tapping contact '$contact' at ($contactCx, $contactCy)")
            val contactTapSuccess = service.performTap(contactCx, contactCy)
            if (!contactTapSuccess) {
                Log.e(TAG, "send_chat_message: failed to tap contact '$contact'")
                return@withContext makeErrorResult("Failed to tap contact '$contact' — gesture dispatch failed.")
            }

            // Step 7: Wait for chat screen to load
            Thread.sleep(1500)

            // Step 8: Find the message input EditText
            Log.i(TAG, "send_chat_message: searching for message input field")
            val root = service.getRootInActiveWindow()
            if (root == null) {
                Log.e(TAG, "send_chat_message: no active window after tapping contact")
                return@withContext makeErrorResult("No active window after tapping contact. The chat may not have opened.")
            }

            val inputNode = findBottomEditableNode(root)
            if (inputNode == null) {
                Log.e(TAG, "send_chat_message: no editable input field found in chat screen")
                return@withContext makeErrorResult("No message input field found in $appName chat. The app layout may not be supported.")
            }

            // Step 9: Type the message
            Log.i(TAG, "send_chat_message: typing message into input field")
            val textArgs = android.os.Bundle()
            textArgs.putCharSequence(
                AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                message
            )
            val typeSuccess = inputNode.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, textArgs)
            if (!typeSuccess) {
                // Fallback: clipboard paste
                Log.w(TAG, "send_chat_message: ACTION_SET_TEXT failed, trying clipboard paste")
                val clipSuccess = setClipboard(message)
                if (clipSuccess) {
                    val selectArgs = android.os.Bundle()
                    selectArgs.putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, 0)
                    selectArgs.putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, Int.MAX_VALUE)
                    inputNode.performAction(AccessibilityNodeInfo.ACTION_SET_SELECTION, selectArgs)
                    val pasteSuccess = inputNode.performAction(AccessibilityNodeInfo.ACTION_PASTE)
                    if (!pasteSuccess) {
                        inputNode.recycle()
                        Log.e(TAG, "send_chat_message: clipboard paste also failed")
                        return@withContext makeErrorResult("Failed to type message — both ACTION_SET_TEXT and clipboard paste failed.")
                    }
                } else {
                    inputNode.recycle()
                    Log.e(TAG, "send_chat_message: failed to set clipboard for paste fallback")
                    return@withContext makeErrorResult("Failed to type message — could not set clipboard for paste fallback.")
                }
            }

            Thread.sleep(300)

            // Step 10: Find and tap send button, or press Enter
            Log.i(TAG, "send_chat_message: searching for send button")
            val sendButton = findSendButton(root)
            if (sendButton != null) {
                val sendBounds = android.graphics.Rect()
                sendButton.getBoundsInScreen(sendBounds)
                val sendCx = (sendBounds.left + sendBounds.right) / 2
                val sendCy = (sendBounds.top + sendBounds.bottom) / 2
                Log.i(TAG, "send_chat_message: tapping send button at ($sendCx, $sendCy)")
                val sendTapSuccess = service.performTap(sendCx, sendCy)
                sendButton.recycle()

                if (sendTapSuccess) {
                    Log.i(TAG, "send_chat_message: message sent successfully to '$contact' via $appName")
                    return@withContext makeSuccessResult("Message sent to '$contact' via $appName")
                } else {
                    Log.e(TAG, "send_chat_message: send button tap failed")
                    return@withContext makeErrorResult("Found send button but tap failed. The message may have been typed but not sent.")
                }
            } else {
                // Fallback: press Enter via accessibility action
                Log.w(TAG, "send_chat_message: no send button found, pressing Enter as fallback")
                val enterArgs = android.os.Bundle()
                enterArgs.putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_MOVEMENT_GRANULARITY_INT, 0)
                val enterSuccess = inputNode.performAction(AccessibilityNodeInfo.ACTION_NEXT_AT_MOVEMENT_GRANULARITY)

                // Alternative: try to dispatch Enter key event via gesture (not reliable)
                // Best effort: report partial success
                inputNode.recycle()
                Log.i(TAG, "send_chat_message: message typed but send button not found (user may need to press Send)")
                return@withContext makeSuccessResult(
                    "Message typed in $appName chat with '$contact'. Send button not found — message may need manual send."
                )
            }
        }
    }

    // -----------------------------------------------------------------------
    // Shared helpers for compound tools
    // -----------------------------------------------------------------------

    /**
     * Resolve an app name to a package name.
     *
     * Checks the WELL_KNOWN_APPS map first, then falls back to PackageManager
     * fuzzy matching. Also accepts raw package names (containing a dot).
     *
     * @param appName App name (e.g. "whatsapp"), display name, or package name.
     * @return Package name if resolved, null otherwise.
     */
    private fun resolveAppName(appName: String): String? {
        // Check well-known map first
        WELL_KNOWN_APPS[appName.lowercase()]?.let { return it }

        // Raw package name (contains a dot)
        if (appName.contains(".")) return appName

        // PackageManager fuzzy match
        return resolveAppNameToPackage(appName)
    }

    /**
     * Dismiss manufacturer-specific chain-launch dialogs ("Always allow", "Open with", etc.)
     * by searching for common button labels and pressing Back as a general dismiss.
     *
     * Common on Xiaomi (MIUI), Huawei (EMUI), Samsung (One UI) when launching apps
     * from an accessibility service or when apps try to open links in other apps.
     *
     * @param service The connected accessibility service.
     */
    private fun dismissChainLaunchDialog(service: PokeAccessibilityService) {
        val dismissLabels = listOf("Allow", "Always allow", "Just once", "Always", "Open", "OK")
        try {
            val root = service.getRootInActiveWindow() ?: return
            for (label in dismissLabels) {
                val nodes = root.findAccessibilityNodeInfosByText(label)
                val button = nodes.firstOrNull {
                    it.isVisibleToUser && it.isClickable
                }
                if (button != null) {
                    val bounds = android.graphics.Rect()
                    button.getBoundsInScreen(bounds)
                    val cx = (bounds.left + bounds.right) / 2
                    val cy = (bounds.top + bounds.bottom) / 2
                    Log.i(TAG, "dismissChainLaunchDialog: found '$label' button at ($cx, $cy), tapping")
                    service.performTap(cx, cy)
                    PokeAccessibilityService.recycleNodes(nodes)
                    Thread.sleep(500)
                    return
                }
                PokeAccessibilityService.recycleNodes(nodes)
            }
        } catch (e: Exception) {
            Log.d(TAG, "dismissChainLaunchDialog: no dialog found or error — ${e.message}")
        }
    }

    /**
     * Wait for a specific package's window to become the active window.
     *
     * @param service The accessibility service.
     * @param packageName The target package name.
     * @param timeoutMs Maximum wait time in milliseconds.
     * @return true if the window became active within the timeout.
     */
    private fun waitForWindow(service: PokeAccessibilityService, packageName: String, timeoutMs: Long): Boolean {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            val root = service.getRootInActiveWindow()
            val windowPkg = root?.packageName?.toString()
            root?.recycle()
            if (windowPkg == packageName) return true
            Thread.sleep(300)
        }
        return false
    }

    /**
     * Find the bottom-most editable node in the tree, which is typically the
     * message input field in chat apps (WhatsApp, Telegram, etc.).
     *
     * Strategy: BFS traversal, track the editable node with the largest Y coordinate
     * (bottom of screen = chat input).
     *
     * @param root The root accessibility node.
     * @return The bottom-most editable node, or null if none found.
     */
    private fun findBottomEditableNode(root: AccessibilityNodeInfo): AccessibilityNodeInfo? {
        var bottomNode: AccessibilityNodeInfo? = null
        var bottomY = -1

        val queue = ArrayDeque<AccessibilityNodeInfo>()
        queue.add(root)

        while (queue.isNotEmpty()) {
            val node = queue.removeFirst()
            if (node.isEditable && node.className?.toString()?.contains("EditText") == true) {
                val bounds = android.graphics.Rect()
                node.getBoundsInScreen(bounds)
                val cy = (bounds.top + bounds.bottom) / 2
                if (cy > bottomY) {
                    bottomY = cy
                    bottomNode?.recycle()
                    bottomNode = node
                    continue // Don't add this node as a child to traverse
                }
            }
            for (i in 0 until node.childCount) {
                val child = node.getChild(i) ?: continue
                queue.add(child)
            }
        }

        return bottomNode
    }

    /**
     * Find the send button in a chat screen.
     *
     * Searches for clickable nodes with common send button labels:
     * "Send", "send", →, ➤, or ImageButton nodes in the bottom-right area.
     *
     * @param root The root accessibility node.
     * @return The send button node, or null if not found.
     */
    private fun findSendButton(root: AccessibilityNodeInfo): AccessibilityNodeInfo? {
        val sendLabels = listOf("Send", "SEND", "send", "→", "➤", "▶", "➡")

        // First try: search by text/contentDescription
        for (label in sendLabels) {
            try {
                val nodes = root.findAccessibilityNodeInfosByText(label)
                val sendBtn = nodes.firstOrNull {
                    it.isVisibleToUser && it.isClickable
                }
                if (sendBtn != null) {
                    PokeAccessibilityService.recycleNodes(nodes.filter { it !== sendBtn })
                    return sendBtn
                }
                PokeAccessibilityService.recycleNodes(nodes)
            } catch (_: Exception) {}
        }

        // Second try: search by content description
        val descLabels = listOf("Send", "send", "Send message", "Submit")
        for (desc in descLabels) {
            try {
                val nodes = root.findAccessibilityNodeInfosByText(desc)
                val btn = nodes.firstOrNull {
                    it.isVisibleToUser && it.isClickable &&
                    (it.contentDescription?.toString()?.contains(desc, ignoreCase = true) == true)
                }
                if (btn != null) {
                    PokeAccessibilityService.recycleNodes(nodes.filter { it !== btn })
                    return btn
                }
                PokeAccessibilityService.recycleNodes(nodes)
            } catch (_: Exception) {}
        }

        // Third try: find ImageButton nodes in the bottom-right quadrant
        val metrics = activity.resources.displayMetrics
        val screenWidth = metrics.widthPixels
        val screenHeight = metrics.heightPixels
        val thresholdX = (screenWidth * 0.6).toInt()
        val thresholdY = (screenHeight * 0.7).toInt()

        return findImageButtonInRegion(root, thresholdX, thresholdY, screenWidth, screenHeight)
    }

    /**
     * Find an ImageButton node in the specified screen region.
     * Used as a last-resort heuristic for finding the send button.
     */
    private fun findImageButtonInRegion(
        root: AccessibilityNodeInfo,
        minX: Int, minY: Int, maxX: Int, maxY: Int
    ): AccessibilityNodeInfo? {
        val queue = ArrayDeque<AccessibilityNodeInfo>()
        queue.add(root)

        while (queue.isNotEmpty()) {
            val node = queue.removeFirst()
            val className = node.className?.toString() ?: ""
            if ((className.contains("ImageButton") || className.contains("Button")) &&
                node.isVisibleToUser && node.isClickable) {
                val bounds = android.graphics.Rect()
                node.getBoundsInScreen(bounds)
                val cx = (bounds.left + bounds.right) / 2
                val cy = (bounds.top + bounds.bottom) / 2
                if (cx >= minX && cy >= minY && cx <= maxX && cy <= maxY) {
                    return node
                }
            }
            for (i in 0 until node.childCount) {
                val child = node.getChild(i) ?: continue
                queue.add(child)
            }
        }
        return null
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

        streamingScope.launch(streamingScope.coroutineContext) {
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

        streamingScope.launch(streamingScope.coroutineContext) {
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
    // App/Contact resolution helpers
    // -----------------------------------------------------------------------

    /**
     * Try to resolve an app name to a package name using PackageManager query.
     * Checks the app label for a case-insensitive match.
     *
     * @return Package name if found, null otherwise.
     */
    private fun resolveAppNameToPackage(appName: String): String? {
        return try {
            val pm = activity.packageManager
            val intent = Intent(Intent.ACTION_MAIN).apply {
                addCategory(Intent.CATEGORY_LAUNCHER)
            }
            val resolveInfos = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                pm.queryIntentActivities(intent, PackageManager.ResolveInfoFlags.of(0))
            } else {
                @Suppress("DEPRECATION")
                pm.queryIntentActivities(intent, 0)
            }

            val lowerName = appName.lowercase()
            for (ri in resolveInfos) {
                val label = ri.loadLabel(pm)?.toString()?.lowercase() ?: ""
                if (label.contains(lowerName)) {
                    val packageName = ri.activityInfo.packageName
                    Log.d(TAG, "resolveAppNameToPackage: '$appName' → $packageName (matched label '$label')")
                    return packageName
                }
            }

            // Fallback: check if appName is part of any package name
            for (ri in resolveInfos) {
                if (ri.activityInfo.packageName.lowercase().contains(lowerName)) {
                    Log.d(TAG, "resolveAppNameToPackage: '$appName' → ${ri.activityInfo.packageName} (matched package)")
                    return ri.activityInfo.packageName
                }
            }

            null
        } catch (e: Exception) {
            Log.w(TAG, "resolveAppNameToPackage: failed — ${e.message}")
            null
        }
    }

    /**
     * Try to resolve a contact name to a phone number using ContactsContract.
     *
     * @return Phone number string if found, null otherwise.
     */
    private fun resolveContactToPhone(contactName: String): String? {
        return try {
            val cursor = activity.contentResolver.query(
                android.provider.ContactsContract.CommonDataKinds.Phone.CONTENT_URI,
                arrayOf(
                    android.provider.ContactsContract.CommonDataKinds.Phone.NUMBER,
                    android.provider.ContactsContract.CommonDataKinds.Phone.DISPLAY_NAME
                ),
                "${android.provider.ContactsContract.CommonDataKinds.Phone.DISPLAY_NAME} LIKE ?",
                arrayOf("%$contactName%"),
                null
            )

            cursor?.use {
                if (it.moveToFirst()) {
                    val number = it.getString(
                        it.getColumnIndexOrThrow(android.provider.ContactsContract.CommonDataKinds.Phone.NUMBER)
                    )
                    val name = it.getString(
                        it.getColumnIndexOrThrow(android.provider.ContactsContract.CommonDataKinds.Phone.DISPLAY_NAME)
                    )
                    Log.d(TAG, "resolveContactToPhone: '$contactName' → $number ($name)")
                    number
                } else {
                    Log.d(TAG, "resolveContactToPhone: no match for '$contactName'")
                    null
                }
            }
        } catch (e: SecurityException) {
            Log.w(TAG, "resolveContactToPhone: permission denied — ${e.message}")
            null
        } catch (e: Exception) {
            Log.w(TAG, "resolveContactToPhone: failed — ${e.message}")
            null
        }
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
