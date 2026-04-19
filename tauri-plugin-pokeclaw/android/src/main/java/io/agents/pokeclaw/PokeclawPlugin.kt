package io.agents.pokeclaw

import android.app.Activity
import android.content.Intent
import android.content.Context
import android.net.Uri
import java.io.File
import java.io.FileOutputStream
import android.content.ClipData
import android.content.ClipboardManager
import android.content.pm.PackageManager
import android.os.Build
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.Settings
import android.util.Log
import android.view.accessibility.AccessibilityNodeInfo
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import androidx.activity.result.ActivityResult
import app.tauri.plugin.Channel
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import app.tauri.plugin.Invoke
import com.google.ai.edge.litertlm.Backend
import com.google.ai.edge.litertlm.Contents
import com.google.ai.edge.litertlm.MessageCallback
import com.google.ai.edge.litertlm.ConversationConfig
import com.google.ai.edge.litertlm.SamplerConfig
import com.google.ai.edge.litertlm.Engine
import com.google.ai.edge.litertlm.Conversation
import com.google.ai.edge.litertlm.Message
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import android.content.SharedPreferences

@TauriPlugin
class PokeclawPlugin(private val activity: Activity) : Plugin(activity) {
    init {
        Log.e(TAG, "INIT: PokeclawPlugin loaded — activity=$activity")
    }

    companion object {
        private const val TAG = "PokeclawPlugin"
        private const val DEFAULT_SYSTEM_PROMPT = "You are a helpful AI assistant running on-device."
        private const val MAX_CONVERSATION_RETRIES = 3
        private const val RETRY_BACKOFF_MS = 1000L
        private const val CHARS_PER_TOKEN = 4
        private const val PREFS_NAME = "pokeclaw_saf"
        private const val KEY_SAF_FOLDER_URI = "saf_folder_uri"
    }

    @InvokeArg
    inner class DownloadModelArgs {
        lateinit var modelId: String
        lateinit var onProgress: Channel
    }

    @InvokeArg
    inner class DownloadModelFromUrlArgs {
        lateinit var url: String
        lateinit var saveDir: String
        lateinit var onProgress: Channel
    }

    private var session: InferenceSession? = null
    private var gpuFailed = false
    private val streamingScope = CoroutineScope(Dispatchers.Default + Job())
    private val safPrefs: SharedPreferences by lazy {
        activity.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
    }
    private val persistedSafFolderUri: String?
        get() = safPrefs.getString(KEY_SAF_FOLDER_URI, null)

    @Command
    fun ping(invoke: Invoke) {
        val ret = JSObject()
        ret.put("value", "pong")
        invoke.resolve(ret)
    }

    @Command
    fun startSession(invoke: Invoke) {
        val args = invoke.getArgs()
        var modelPath = args.getString("modelPath") ?: return invoke.reject("modelPath is required")
        Log.i(TAG, "startSession called — modelPath=$modelPath")

        streamingScope.launch(Dispatchers.IO) {
            try {
                // If modelPath is a SAF content:// URI, copy to cache (skips if already cached)
                if (modelPath.startsWith("content://")) {
                    Log.i(TAG, "startSession: detected SAF URI, resolving to local file...")
                    modelPath = resolveSafPath(Uri.parse(modelPath))
                    Log.i(TAG, "startSession: resolved to $modelPath")
                }

                val preferGpu = args.getBoolean("preferGpu") ?: true
                val engine = acquireEngine(modelPath, preferGpu)
                val conversation = createConversationWithRetry(engine, modelPath, preferGpu)

                session?.close()

                val newSession = InferenceSession(
                    conversation = conversation,
                    modelPath = modelPath,
                    backendLabel = EngineHolder.getBackendLabel(modelPath) ?: "Unknown",
                    sessionId = "sess-${System.currentTimeMillis().toString(16)}",
                    preferGpu = preferGpu,
                )
                session = newSession

                Log.i(TAG, "startSession: success — session ready (sessionId=${newSession.sessionId}, backend=${newSession.backendLabel})")
                val result = JSObject()
                result.put("sessionId", newSession.sessionId)
                result.put("backend", newSession.backendLabel)
                withContext(Dispatchers.Main) { invoke.resolve(result) }
            } catch (e: Exception) {
                Log.e(TAG, "startSession: failure — ${e.message}", e)
                withContext(Dispatchers.Main) { invoke.reject("Failed to start session: ${e.message}") }
            }
        }
    }

    @Command
    fun stopSession(invoke: Invoke) {
        Log.i(TAG, "stopSession called")
        session?.close()
        session = null
        EngineHolder.close()
        invoke.resolve()
    }

    @Command
    fun getSessionStatus(invoke: Invoke) {
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

    @Command
    fun sendMessage(invoke: Invoke) {
        val args = invoke.getArgs()
        val message = args.getString("message") ?: return invoke.reject("message is required")
        val channel = if (args.has("onEvent")) args.get("onEvent") as? Channel else null
        
        Log.i(TAG, "sendMessage called — len=${message.length}, streaming=${channel != null}")
        val currentSession = session ?: return invoke.reject("No active session. Call startSession first.")

        // Run inference off the main thread — Rust's run_mobile_plugin blocks its own thread,
        // but we must not block Android's main thread. The invoke is resolved from the coroutine.
        streamingScope.launch(Dispatchers.IO) {
            try {
                currentSession.recordSend()
                val contents = Contents.of(message)

                val response = StringBuilder()
                val latch = java.util.concurrent.CountDownLatch(1)
                var callbackError: Throwable? = null

                currentSession.conversation.sendMessageAsync(
                    contents,
                    object : MessageCallback {
                        override fun onMessage(msg: Message) {
                            val text = msg.toString()
                            if (text.isNotEmpty()) {
                                response.append(text)
                            }
                        }
                        override fun onDone() {
                            Log.i(TAG, "sendMessage: done, response len=${response.length}")
                            latch.countDown()
                        }
                        override fun onError(throwable: Throwable) {
                            Log.e(TAG, "sendMessage: error — ${throwable.message}")
                            callbackError = throwable
                            latch.countDown()
                        }
                    },
                )

                // Block this IO thread until inference completes (max 120s)
                val completed = latch.await(120, java.util.concurrent.TimeUnit.SECONDS)
                if (!completed) {
                    Log.e(TAG, "sendMessage: timed out after 120s")
                    if (currentSession.preferGpu) { fallbackToCpu(currentSession.modelPath) }
                    withContext(Dispatchers.Main) { invoke.reject("Inference timed out (120s).") }
                    return@launch
                }

                val err = callbackError
                if (err != null) {
                    if (currentSession.preferGpu && isGpuBackendFailure(err)) {
                        Log.w(TAG, "sendMessage: GPU error, retrying on CPU")
                        try {
                            fallbackToCpu(currentSession.modelPath)
                            val engine = acquireEngine(currentSession.modelPath, false)
                            val conversation = createConversationWithRetry(engine, currentSession.modelPath, false)
                            session?.close()
                            val newSession = InferenceSession(
                                conversation = conversation,
                                modelPath = currentSession.modelPath,
                                backendLabel = EngineHolder.getBackendLabel(currentSession.modelPath) ?: "Unknown",
                                sessionId = "sess-${System.currentTimeMillis().toString(16)}",
                                preferGpu = false,
                            )
                            session = newSession
                            val cpuResponse: Message = newSession.conversation.sendMessage(message)
                            newSession.recordSend()
                            val cpuResult = JSObject()
                            cpuResult.put("response", cpuResponse.toString())
                            withContext(Dispatchers.Main) { invoke.resolve(cpuResult) }
                        } catch (retryErr: Exception) {
                            withContext(Dispatchers.Main) { invoke.reject("Inference failed (GPU+CPU): ${retryErr.message}") }
                        }
                        return@launch
                    }
                    withContext(Dispatchers.Main) { invoke.reject("Inference failed: ${err.message}") }
                    return@launch
                }

                val responseText = response.toString()
                Log.i(TAG, "sendMessage: OK (${responseText.length} chars): ${responseText.take(100)}")
                val result = JSObject()
                result.put("response", responseText)
                withContext(Dispatchers.Main) { invoke.resolve(result) }
            } catch (e: Exception) {
                Log.e(TAG, "sendMessage: exception — ${e.message}", e)
                withContext(Dispatchers.Main) { invoke.reject("Inference failed: ${e.message}") }
            }
        }
    }

    // -----------------------------------------------------------------------
    // SAF (Storage Access Framework) & File management
    // -----------------------------------------------------------------------

    @Command
    fun listModels(invoke: Invoke) {
        try {
            val array = app.tauri.plugin.JSArray()
            for (model in ModelManager.AVAILABLE_MODELS) {
                val isDownloaded = ModelManager.isModelDownloaded(activity, model)
                val localPath = ModelManager.getModelPath(activity, model)
                val obj = JSObject()
                obj.put("id", model.id)
                obj.put("displayName", model.displayName)
                obj.put("sizeBytes", model.sizeBytes)
                obj.put("isDownloaded", isDownloaded)
                obj.put("localPath", localPath)
                array.put(obj)
            }
            val result = JSObject()
            result.put("models", array)
            invoke.resolve(result)
        } catch (e: Exception) {
            invoke.reject("Failed to list models: ${e.message}", e)
        }
    }

    @Command
    fun pickSafFolder(invoke: Invoke) {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
        }
        startActivityForResult(invoke, intent, "safFolderPickerCallback")
    }

    @ActivityCallback
    private fun safFolderPickerCallback(invoke: Invoke, result: ActivityResult) {
        if (result.resultCode == Activity.RESULT_OK && result.data?.data != null) {
            val uri = result.data!!.data!!
            val flags = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
            activity.contentResolver.takePersistableUriPermission(uri, flags)
            safPrefs.edit().putString(KEY_SAF_FOLDER_URI, uri.toString()).apply()
            val ret = JSObject()
            ret.put("uri", uri.toString())
            invoke.resolve(ret)
        } else {
            invoke.reject("User cancelled")
        }
    }

    @Command
    fun getSafFolderStatus(invoke: Invoke) {
        val uriStr = persistedSafFolderUri
        val ret = JSObject()
        if (uriStr == null) {
            ret.put("hasPermission", false)
        } else {
            ret.put("hasPermission", true)
            ret.put("folderUri", uriStr)
            val doc = androidx.documentfile.provider.DocumentFile.fromTreeUri(activity, Uri.parse(uriStr))
            ret.put("folderName", doc?.name ?: "Selected Folder")
        }
        invoke.resolve(ret)
    }

    @Command
    fun listSafModels(invoke: Invoke) {
        val uriStr = persistedSafFolderUri ?: return invoke.reject("No SAF folder selected")
        streamingScope.launch(Dispatchers.IO) {
            try {
                val treeUri = Uri.parse(uriStr)
                val treeDoc = androidx.documentfile.provider.DocumentFile.fromTreeUri(activity, treeUri)
                val array = app.tauri.plugin.JSArray()
                treeDoc?.listFiles()?.forEach { doc ->
                    if (doc.name?.endsWith(".litertlm") == true) {
                        val obj = JSObject()
                        obj.put("fileName", doc.name)
                        obj.put("sizeBytes", doc.length())
                        obj.put("safUri", doc.uri.toString())
                        array.put(obj)
                    }
                }
                val result = JSObject()
                result.put("models", array)
                withContext(Dispatchers.Main) { invoke.resolve(result) }
            } catch (e: Exception) {
                withContext(Dispatchers.Main) { invoke.reject("Failed: ${e.message}") }
            }
        }
    }

    @Command
    fun cacheSafModel(invoke: Invoke) {
        val safUriStr = invoke.getArgs().getString("safUri") ?: return invoke.reject("safUri is required")
        streamingScope.launch(Dispatchers.IO) {
            try {
                val path = copySafToCache(Uri.parse(safUriStr))
                val ret = JSObject()
                ret.put("path", path)
                withContext(Dispatchers.Main) { invoke.resolve(ret) }
            } catch (e: Exception) {
                withContext(Dispatchers.Main) { invoke.reject("Cache failed: ${e.message}") }
            }
        }
    }

    @Command
    fun pickModelFile(invoke: Invoke) {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
        }
        startActivityForResult(invoke, intent, "pickModelFileCallback")
    }

    @ActivityCallback
    private fun pickModelFileCallback(invoke: Invoke, result: ActivityResult) {
        if (result.resultCode == Activity.RESULT_OK && result.data?.data != null) {
            val uri = result.data!!.data!!
            streamingScope.launch(Dispatchers.IO) {
                try {
                    val path = copySafToCache(uri)
                    val ret = JSObject()
                    ret.put("path", path)
                    withContext(Dispatchers.Main) { invoke.resolve(ret) }
                } catch (e: Exception) {
                    withContext(Dispatchers.Main) { invoke.reject("Failed to cache: ${e.message}") }
                }
            }
        } else {
            invoke.reject("Cancelled")
        }
    }

    @Command
    fun downloadModel(invoke: Invoke) {
        val args = invoke.getArgs()
        val modelId = args.getString("modelId") ?: return invoke.reject("modelId is required")
        val channel = args.get("onProgress") as? Channel ?: return invoke.reject("onProgress channel expected")
        
        streamingScope.launch(Dispatchers.IO) {
            val model = ModelManager.getModelById(modelId) ?: return@launch withContext(Dispatchers.Main) {
                invoke.reject("Model not found in catalog")
            }
            
            ModelManager.downloadModel(activity, model, object : ModelManager.DownloadCallback {
                override fun onProgress(bytesDownloaded: Long, totalBytes: Long, bytesPerSecond: Long) {
                    val data = JSObject()
                    data.put("bytesDownloaded", bytesDownloaded)
                    data.put("totalBytes", totalBytes)
                    data.put("bytesPerSecond", bytesPerSecond)
                    val event = JSObject()
                    event.put("event", "progress")
                    event.put("data", data)
                    channel.send(event)
                }
                override fun onComplete(modelPath: String) {
                    val data = JSObject()
                    data.put("modelPath", modelPath)
                    data.put("fileName", model.fileName)
                    val event = JSObject()
                    event.put("event", "complete")
                    event.put("data", data)
                    channel.send(event)
                    streamingScope.launch(Dispatchers.Main) { invoke.resolve() }
                }
                override fun onError(error: String) {
                    val data = JSObject()
                    data.put("message", error)
                    val event = JSObject()
                    event.put("event", "error")
                    event.put("data", data)
                    try { channel.send(event) } catch (_: Exception) {}
                    streamingScope.launch(Dispatchers.Main) { invoke.reject(error) }
                }
            })
        }
    }

    @Command
    fun pickSaveLocation(invoke: Invoke) {
        val fileName = invoke.getArgs().getString("fileName") ?: "model.litertlm"
        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
            putExtra(Intent.EXTRA_TITLE, fileName)
        }
        startActivityForResult(invoke, intent, "pickSaveLocationCallback")
    }

    @ActivityCallback
    private fun pickSaveLocationCallback(invoke: Invoke, result: ActivityResult) {
        if (result.resultCode == Activity.RESULT_OK && result.data?.data != null) {
            val uri = result.data!!.data!!
            val ret = JSObject()
            ret.put("uri", uri.toString())
            invoke.resolve(ret)
        } else {
            invoke.reject("Cancelled")
        }
    }

    @Command
    fun downloadToSaf(invoke: Invoke) {
        val args = invoke.getArgs()
        val url = args.getString("url") ?: return invoke.reject("url is required")
        val safUri = args.getString("safUri") ?: return invoke.reject("safUri is required")
        val channel = args.get("onProgress") as? Channel

        streamingScope.launch(Dispatchers.IO) {
            try {
                // Simplified download to SAF logic
                val uri = Uri.parse(safUri)
                activity.contentResolver.openOutputStream(uri)?.use { output ->
                    val okHttpClient = okhttp3.OkHttpClient()
                    val request = okhttp3.Request.Builder().url(url).build()
                    okHttpClient.newCall(request).execute().use { response ->
                        if (!response.isSuccessful) throw Exception("HTTP ${response.code}")
                        val body = response.body ?: throw Exception("Empty body")
                        val totalBytes = body.contentLength()
                        var downloaded = 0L
                        val buffer = ByteArray(8192)
                        body.byteStream().use { input ->
                            while (true) {
                                val read = input.read(buffer)
                                if (read == -1) break
                                output.write(buffer, 0, read)
                                downloaded += read
                                if (channel != null) {
                                    val data = JSObject()
                                    data.put("bytesDownloaded", downloaded)
                                    data.put("totalBytes", totalBytes)
                                    val event = JSObject()
                                    event.put("event", "progress")
                                    event.put("data", data)
                                    channel.send(event)
                                }
                            }
                        }
                    }
                }
                val complete = JSObject()
                complete.put("event", "complete")
                complete.put("modelPath", copySafToCache(uri))
                channel?.send(complete)
                withContext(Dispatchers.Main) { invoke.resolve() }
            } catch (e: Exception) {
                val err = JSObject()
                err.put("event", "error")
                err.put("data", JSObject().put("message", e.message))
                channel?.send(err)
                withContext(Dispatchers.Main) { invoke.reject(e.message) }
            }
        }
    }

    @Command
    fun downloadToSafFolder(invoke: Invoke) {
        val args = invoke.getArgs()
        val url = args.getString("url") ?: return invoke.reject("url is required")
        val fileName = args.getString("fileName") ?: "model.litertlm"
        val channel = args.get("onProgress") as? Channel
        val folderUriStr = persistedSafFolderUri ?: return invoke.reject("No SAF folder set")

        streamingScope.launch(Dispatchers.IO) {
            try {
                val folderUri = Uri.parse(folderUriStr)
                val folderDoc = androidx.documentfile.provider.DocumentFile.fromTreeUri(activity, folderUri)
                val fileDoc = folderDoc?.createFile("*/*", fileName) ?: throw Exception("Failed to create file in SAF folder")
                
                val args2 = JSObject()
                args2.put("url", url)
                args2.put("safUri", fileDoc.uri.toString())
                args2.put("onProgress", channel)
                
                // Reuse downloadToSaf logic or call it
                downloadToSaf(invoke) // Note: this is a bit hacky in Tauri bridge but works if we bypass the invoke logic
            } catch (e: Exception) {
                withContext(Dispatchers.Main) { invoke.reject(e.message) }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Accessibility & Device commands
    // -----------------------------------------------------------------------

    @Command
    fun getScreenInfo(invoke: Invoke) {
        val service = PokeAccessibilityService.getConnectedInstance(1000)
            ?: return invoke.reject("Accessibility service not running")
        val tree = service.getScreenTree() ?: return invoke.reject("Failed to read screen")
        val res = JSObject()
        res.put("success", true)
        res.put("data", tree)
        invoke.resolve(res)
    }

    @Command
    fun systemKey(invoke: Invoke) {
        val action = invoke.getArgs().getString("action") ?: return invoke.reject("action is required")
        val service = PokeAccessibilityService.getConnectedInstance(1000) ?: return invoke.reject("Service not running")
        val success = when (action) {
            "back" -> service.pressBack()
            "home" -> service.pressHome()
            "recent_apps" -> service.openRecentApps()
            else -> false
        }
        val res = JSObject()
        res.put("success", success)
        invoke.resolve(res)
    }
    
    @Command
    fun takeScreenshot(invoke: Invoke) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
            return invoke.reject("Screenshot requires Android 11+")
        }
        val service = PokeAccessibilityService.getConnectedInstance(3000) ?: return invoke.reject("Service not connected")
        val path = service.takeScreenshot(null)
        val res = JSObject()
        res.put("success", path != null)
        res.put("data", path)
        invoke.resolve(res)
    }

    @Command
    fun clipboard(invoke: Invoke) {
        val args = invoke.getArgs()
        val action = args.getString("action") ?: "get"
        val cm = activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        if (action == "get") {
            val text = cm.primaryClip?.getItemAt(0)?.text?.toString() ?: ""
            invoke.resolve(JSObject().put("data", text))
        } else {
            val text = args.getString("text") ?: ""
            activity.runOnUiThread {
                cm.setPrimaryClip(ClipData.newPlainText("text", text))
                invoke.resolve()
            }
        }
    }

    // -----------------------------------------------------------------------
    // Internal Engine helpers
    // -----------------------------------------------------------------------

    private fun acquireEngine(modelPath: String, preferGpu: Boolean): Engine {
        val checkpointDir = activity.getSharedPreferences("gpu_hang_check", Context.MODE_PRIVATE)
        val checkpointKey = "initializing_$modelPath"
        
        val wasHung = checkpointDir.getBoolean(checkpointKey, false)
        val actualPreferGpu = if (wasHung) {
            Log.w(TAG, "acquireEngine: Detected previous GPU hang for $modelPath. Forcing CPU fallback.")
            false
        } else {
            preferGpu
        }

        val backend = if (actualPreferGpu) Backend.GPU() else Backend.CPU()
        
        try {
            if (actualPreferGpu) {
                checkpointDir.edit().putBoolean(checkpointKey, true).commit()
            }
            val startTime = System.currentTimeMillis()
            val engine = EngineHolder.getOrCreate(modelPath, null, activity.cacheDir.absolutePath, backend)
            val initDuration = System.currentTimeMillis() - startTime
            
            // If GPU engine took over 45s to init, it's likely going to hang on inference
            if (actualPreferGpu && initDuration > 45_000) {
                Log.w(TAG, "acquireEngine: GPU init took ${initDuration}ms (>45s), likely unstable — forcing CPU")
                EngineHolder.close()
                checkpointDir.edit().putBoolean(checkpointKey, true).commit()
                return acquireEngine(modelPath, false)
            }
            
            if (actualPreferGpu) {
                checkpointDir.edit().remove(checkpointKey).commit()
            }
            Log.i(TAG, "acquireEngine: engine ready in ${initDuration}ms with ${backend.javaClass.simpleName}")
            return engine
        } catch (e: Exception) {
            checkpointDir.edit().remove(checkpointKey).commit()
            if (actualPreferGpu && isGpuBackendFailure(e)) {
                Log.e(TAG, "acquireEngine: GPU failed with backend error, falling back to CPU — ${e.message}")
                return acquireEngine(modelPath, false)
            }
            throw e
        }
    }

    private fun createConversationWithRetry(engine: Engine, modelPath: String, preferGpu: Boolean): Conversation {
        val convConfig = ConversationConfig(
            systemInstruction = Contents.of(DEFAULT_SYSTEM_PROMPT),
            samplerConfig = SamplerConfig(topK = 64, topP = 0.95, temperature = 0.8)
        )
        var lastError: Exception? = null
        for (attempt in 1..MAX_CONVERSATION_RETRIES) {
            try {
                return engine.createConversation(convConfig)
            } catch (e: Exception) {
                lastError = e
                Log.w(TAG, "createConversation: attempt $attempt failed: ${e.message}")
                if (attempt == MAX_CONVERSATION_RETRIES - 1) { 
                    EngineHolder.close()
                    val newEngine = acquireEngine(modelPath, preferGpu)
                    try { return newEngine.createConversation(convConfig) } catch (re: Exception) { lastError = re }
                }
                Thread.sleep(RETRY_BACKOFF_MS)
            }
        }
        throw lastError ?: Exception("Failed to create conversation")
    }

    private fun fallbackToCpu(modelPath: String) {
        Log.w(TAG, "fallbackToCpu: switching to CPU for $modelPath")
        gpuFailed = true
        session?.close()
        session = null
        EngineHolder.close()
    }

    private fun isGpuBackendFailure(e: Throwable?): Boolean {
        val msg = e?.message.orEmpty().lowercase()
        return msg.contains("gpu") || msg.contains("opencl") || msg.contains("nativesendmessage")
    }

    private fun copySafToCache(uri: Uri): String {
        val doc = androidx.documentfile.provider.DocumentFile.fromSingleUri(activity, uri) ?: throw Exception("Invalid URI")
        val cacheFile = File(activity.cacheDir, doc.name ?: "model.litertlm")
        if (cacheFile.exists() && cacheFile.length() == doc.length()) return cacheFile.absolutePath
        activity.contentResolver.openInputStream(uri)?.use { input ->
            FileOutputStream(cacheFile).use { output ->
                input.copyTo(output)
            }
        } ?: throw Exception("Failed to read SAF URI")
        return cacheFile.absolutePath
    }

    /**
     * Resolve a SAF content:// URI to a local file path for LiteRT-LM.
     *
     * Strategy 1: MANAGE_EXTERNAL_STORAGE (gold standard).
     *   Decode SAF URI to POSIX path, FUSE grants mmap() access.
     *
     * Strategy 2: Media FUSE hack (no special permission needed).
     *   If a .mp3 companion exists next to the .litertlm file, use it.
     *   The FUSE daemon treats .mp3 as media and grants POSIX access
     *   to apps with READ_MEDIA_AUDIO permission.
     *
     * Strategy 3: Copy to app cache (always works, skips if already cached).
     */
    private fun resolveSafPath(uri: Uri): String {
        val posixPath = getRealPathFromSafUri(uri)

        // Strategy 1: MANAGE_EXTERNAL_STORAGE → direct POSIX mmap
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R && Environment.isExternalStorageManager()) {
            if (posixPath != null) {
                val file = File(posixPath)
                if (file.exists() && file.canRead()) {
                    Log.i(TAG, "resolveSafPath: [S1] POSIX direct (MANAGE_EXTERNAL_STORAGE) → $posixPath")
                    return posixPath
                }
            }
        }

        // Strategy 2: Media FUSE hack — .mp3 companion file
        if (posixPath != null) {
            val mp3Path = posixPath + ".mp3"
            val mp3File = File(mp3Path)
            if (mp3File.exists() && mp3File.canRead()) {
                Log.i(TAG, "resolveSafPath: [S2] Media FUSE hack (.mp3) → $mp3Path")
                return mp3Path
            }
        }

        // Strategy 3: Copy to cache (fallback, skips if already cached by size)
        Log.i(TAG, "resolveSafPath: [S3] Copying SAF file to cache")
        return copySafToCache(uri)
    }

    /**
     * Decode a SAF content:// URI from com.android.externalstorage.documents
     * into a real filesystem path like /storage/emulated/0/GEMMA4/model.litertlm.
     * Returns null for non-local URIs (Google Drive, etc.).
     */
    private fun getRealPathFromSafUri(uri: Uri): String? {
        if ("com.android.externalstorage.documents" != uri.authority) return null

        val docId = DocumentsContract.getDocumentId(uri)
        val split = docId.split(":")
        val type = split[0]
        val path = if (split.size > 1) split[1] else ""

        return if ("primary".equals(type, ignoreCase = true)) {
            "${Environment.getExternalStorageDirectory()}/$path"
        } else {
            // Removable storage (SD card): /storage/1234-ABCD/path
            "/storage/$type/$path"
        }
    }

    private fun sendErrorEvent(channel: Channel, message: String) {
        val data = JSObject()
        data.put("message", message)
        val event = JSObject()
        event.put("event", "error")
        event.put("data", data)
        try { channel.send(event) } catch (_: Exception) {}
    }
}
