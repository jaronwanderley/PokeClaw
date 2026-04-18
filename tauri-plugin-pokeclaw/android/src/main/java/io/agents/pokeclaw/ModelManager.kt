package io.agents.pokeclaw

import android.app.Activity
import android.app.ActivityManager
import android.content.Context
import android.os.StatFs
import android.util.Log
import okhttp3.OkHttpClient
import okhttp3.Request
import java.io.File
import java.io.FileOutputStream
import java.util.concurrent.TimeUnit

/**
 * Manages on-device LLM model downloads and storage.
 *
 * Models are downloaded from HuggingFace and stored in the app's
 * external files directory for persistence across app restarts.
 *
 * Ported from legacy LocalModelManager. Uses android.util.Log instead
 * of the legacy logger and does not depend on legacy key-value utils.
 */
object ModelManager {

    private const val TAG = "ModelManager"
    private const val SIZE_TOLERANCE_BYTES = 32L * 1024L * 1024L

    /** Catalog entry for an available on-device model. */
    data class ModelInfo(
        val id: String,
        val displayName: String,
        val url: String,
        val fileName: String,
        val sizeBytes: Long,
        val minRamGb: Int
    )

    /** Models available for download from HuggingFace. */
    val AVAILABLE_MODELS = listOf(
        ModelInfo(
            id = "gemma4-e2b",
            displayName = "Gemma 4 E2B — 2.6GB",
            url = "https://huggingface.co/litert-community/gemma-4-E2B-it-litert-lm/resolve/main/gemma-4-E2B-it.litertlm",
            fileName = "gemma-4-E2B-it.litertlm",
            sizeBytes = 2_580_000_000L,
            minRamGb = 8
        ),
        ModelInfo(
            id = "gemma4-e4b",
            displayName = "Gemma 4 E4B — 3.6GB",
            url = "https://huggingface.co/litert-community/gemma-4-E4B-it-litert-lm/resolve/main/gemma-4-E4B-it.litertlm",
            fileName = "gemma-4-E4B-it.litertlm",
            sizeBytes = 3_650_000_000L,
            minRamGb = 10
        ),
    )

    /**
     * Download progress callback for streaming download events.
     */
    interface DownloadCallback {
        fun onProgress(bytesDownloaded: Long, totalBytes: Long, bytesPerSecond: Long)
        fun onComplete(modelPath: String)
        fun onError(error: String)
    }

    // ------------------------------------------------------------------
    // Model lookup helpers
    // ------------------------------------------------------------------

    /** Find a model in the catalog by ID. */
    fun getModelById(modelId: String): ModelInfo? =
        AVAILABLE_MODELS.firstOrNull { it.id == modelId }

    /**
     * Pick the best model for this device based on available RAM.
     * Devices with 12 GB+ get E4B, everyone else gets E2B.
     */
    fun recommendedModel(context: Context): ModelInfo {
        val totalRamGb = getDeviceRamGb(context)
        return if (totalRamGb >= 12) {
            AVAILABLE_MODELS.first { it.id == "gemma4-e4b" }
        } else {
            AVAILABLE_MODELS.first { it.id == "gemma4-e2b" }
        }
    }

    /** Total device RAM in GB (rounded up). */
    fun getDeviceRamGb(context: Context): Int {
        val activityManager = context.getSystemService(Context.ACTIVITY_SERVICE) as ActivityManager
        val memInfo = ActivityManager.MemoryInfo()
        activityManager.getMemoryInfo(memInfo)
        return (memInfo.totalMem / (1024L * 1024L * 1024L)).toInt() + 1
    }

    /** Highest-spec model supported by this device, or null if none. */
    fun bestSupportedModel(context: Context): ModelInfo? {
        val deviceRamGb = getDeviceRamGb(context)
        return AVAILABLE_MODELS
            .filter { it.minRamGb <= deviceRamGb }
            .maxByOrNull { it.minRamGb }
    }

    /** Check whether a specific model is supported given device RAM. */
    fun isModelSupportedOnDevice(context: Context, model: ModelInfo): Boolean =
        getDeviceRamGb(context) >= model.minRamGb

    // ------------------------------------------------------------------
    // Storage & validation
    // ------------------------------------------------------------------

    /** Directory where model files are stored. */
    fun getModelDir(activity: Activity): File {
        val dir = File(activity.getExternalFilesDir(null), "models")
        if (!dir.exists()) dir.mkdirs()
        return dir
    }

    /** Check if a model file is already downloaded and valid. */
    fun isModelDownloaded(activity: Activity, model: ModelInfo): Boolean {
        val file = File(getModelDir(activity), model.fileName)
        return isValidModelFile(file, model)
    }

    /** Return the absolute path to a downloaded model, or null if absent/invalid. */
    fun getModelPath(activity: Activity, model: ModelInfo): String? {
        val file = File(getModelDir(activity), model.fileName)
        return if (isValidModelFile(file, model)) file.absolutePath else null
    }

    /** Delete a downloaded model (and its temp file) to free space. */
    fun deleteModel(activity: Activity, model: ModelInfo): Boolean {
        val dir = getModelDir(activity)
        val tempFile = File(dir, "${model.fileName}.downloading")
        tempFile.delete()
        val file = File(dir, model.fileName)
        return if (file.exists()) file.delete() else true
    }

    // ------------------------------------------------------------------
    // Download with resume
    // ------------------------------------------------------------------

    /**
     * Download a model from HuggingFace with progress reporting.
     * Supports resume via HTTP Range headers for partial downloads.
     *
     * Must be called from a background thread.
     */
    fun downloadModel(
        activity: Activity,
        model: ModelInfo,
        callback: DownloadCallback
    ) {
        val modelDir = getModelDir(activity)
        val targetFile = File(modelDir, model.fileName)
        val tempFile = File(modelDir, "${model.fileName}.downloading")
        cleanupInvalidFiles(model, targetFile, tempFile)

        // Check free space before starting download
        try {
            val stat = StatFs(modelDir.absolutePath)
            val availableBytes = stat.availableBytes
            val existingTempBytes = if (tempFile.exists()) tempFile.length() else 0L
            val bytesNeeded = model.sizeBytes - existingTempBytes
            if (bytesNeeded > 0 && availableBytes < bytesNeeded) {
                val needGb = String.format("%.1f", bytesNeeded / 1_000_000_000.0)
                val haveGb = String.format("%.1f", availableBytes / 1_000_000_000.0)
                Log.e(TAG, "Not enough storage: need ${needGb}GB, have ${haveGb}GB available")
                callback.onError("Not enough storage: need ${needGb} GB free, only ${haveGb} GB available")
                return
            }
            Log.d(TAG, "Storage check passed: need ${bytesNeeded / 1_000_000}MB, have ${availableBytes / 1_000_000}MB")
        } catch (e: Exception) {
            Log.w(TAG, "Could not check storage, proceeding anyway: ${e.message}")
        }

        try {
            val client = OkHttpClient.Builder()
                .connectTimeout(30, TimeUnit.SECONDS)
                .readTimeout(60, TimeUnit.SECONDS)
                .build()

            // Support resume from partial download
            val existingBytes = if (tempFile.exists()) tempFile.length() else 0L

            val requestBuilder = Request.Builder().url(model.url)
            if (existingBytes > 0) {
                requestBuilder.addHeader("Range", "bytes=$existingBytes-")
                Log.i(TAG, "Resuming download from byte $existingBytes")
            }

            val response = client.newCall(requestBuilder.build()).execute()

            if (!response.isSuccessful && response.code != 206) {
                callback.onError("Download failed: HTTP ${response.code}")
                return
            }

            val isResumedResponse = existingBytes > 0 && response.code == 206
            if (existingBytes > 0 && !isResumedResponse) {
                Log.w(TAG, "Server ignored Range request for ${model.fileName}; restarting download from scratch")
                tempFile.delete()
            }

            val totalBytes = if (isResumedResponse) {
                val contentRange = response.header("Content-Range")
                contentRange?.substringAfterLast("/")?.toLongOrNull() ?: model.sizeBytes
            } else {
                response.body?.contentLength() ?: model.sizeBytes
            }

            val body = response.body ?: run {
                callback.onError("Empty response body")
                return
            }

            val startingBytes = if (isResumedResponse) existingBytes else 0L
            val outputStream = FileOutputStream(tempFile, isResumedResponse)
            val buffer = ByteArray(8192)
            var downloadedBytes = startingBytes
            var lastReportTime = System.currentTimeMillis()
            var lastReportedBytes = startingBytes

            body.byteStream().use { input ->
                outputStream.use { output ->
                    while (true) {
                        val bytesRead = input.read(buffer)
                        if (bytesRead == -1) break
                        output.write(buffer, 0, bytesRead)
                        downloadedBytes += bytesRead

                        val now = System.currentTimeMillis()
                        if (now - lastReportTime >= 200) {
                            val elapsed = (now - lastReportTime) / 1000.0
                            val speed = ((downloadedBytes - lastReportedBytes) / elapsed).toLong()
                            callback.onProgress(downloadedBytes, totalBytes, speed)
                            lastReportTime = now
                            lastReportedBytes = downloadedBytes
                        }
                    }
                }
            }

            if (!isValidModelFile(tempFile, model)) {
                tempFile.delete()
                callback.onError("Downloaded file looks incomplete or corrupted. Please retry.")
                return
            }

            // Rename temp to final
            if (targetFile.exists()) targetFile.delete()
            if (!tempFile.renameTo(targetFile)) {
                callback.onError("Download finished but could not move the model into place")
                return
            }

            Log.i(TAG, "Model downloaded: ${targetFile.absolutePath} (${targetFile.length()} bytes)")
            callback.onComplete(targetFile.absolutePath)

        } catch (e: Exception) {
            Log.e(TAG, "Download failed: ${e.message}", e)
            callback.onError("Download failed: ${e.message}")
        }
    }

    // ------------------------------------------------------------------
    // Download from arbitrary URL
    // ------------------------------------------------------------------

    /**
     * Download a model file from an arbitrary URL to a specified directory.
     * Unlike downloadModel() which uses the static catalog, this accepts any URL.
     *
     * Must be called from a background thread.
     */
    fun downloadFromUrl(
        url: String,
        saveDir: File,
        callback: DownloadCallback
    ) {
        // Extract filename from URL
        val fileName = extractFileNameFromUrl(url)
        Log.i(TAG, "downloadFromUrl: url='$url', saveDir='${saveDir.absolutePath}', fileName='$fileName'")

        // Ensure save directory exists
        if (!saveDir.exists()) saveDir.mkdirs()

        val targetFile = File(saveDir, fileName)
        val tempFile = File(saveDir, "$fileName.downloading")

        try {
            val client = OkHttpClient.Builder()
                .connectTimeout(30, TimeUnit.SECONDS)
                .readTimeout(60, TimeUnit.SECONDS)
                .build()

            // Support resume from partial download
            val existingBytes = if (tempFile.exists()) tempFile.length() else 0L
            val requestBuilder = Request.Builder().url(url)
            if (existingBytes > 0) {
                requestBuilder.addHeader("Range", "bytes=$existingBytes-")
                Log.i(TAG, "downloadFromUrl: resuming from byte $existingBytes")
            }

            val response = client.newCall(requestBuilder.build()).execute()

            if (!response.isSuccessful && response.code != 206) {
                callback.onError("Download failed: HTTP ${response.code}")
                return
            }

            val isResumedResponse = existingBytes > 0 && response.code == 206
            if (existingBytes > 0 && !isResumedResponse) {
                Log.w(TAG, "downloadFromUrl: server ignored Range request; restarting from scratch")
                tempFile.delete()
            }

            val totalBytes = if (isResumedResponse) {
                val contentRange = response.header("Content-Range")
                contentRange?.substringAfterLast("/")?.toLongOrNull() ?: 0L
            } else {
                response.body?.contentLength() ?: 0L
            }

            val body = response.body ?: run {
                callback.onError("Empty response body")
                return
            }

            val startingBytes = if (isResumedResponse) existingBytes else 0L
            val outputStream = FileOutputStream(tempFile, isResumedResponse)
            val buffer = ByteArray(8192)
            var downloadedBytes = startingBytes
            var lastReportTime = System.currentTimeMillis()
            var lastReportedBytes = startingBytes

            body.byteStream().use { input ->
                outputStream.use { output ->
                    while (true) {
                        val bytesRead = input.read(buffer)
                        if (bytesRead == -1) break
                        output.write(buffer, 0, bytesRead)
                        downloadedBytes += bytesRead

                        val now = System.currentTimeMillis()
                        if (now - lastReportTime >= 200) {
                            val elapsed = (now - lastReportTime) / 1000.0
                            val speed = ((downloadedBytes - lastReportedBytes) / elapsed).toLong()
                            callback.onProgress(downloadedBytes, totalBytes, speed)
                            lastReportTime = now
                            lastReportedBytes = downloadedBytes
                        }
                    }
                }
            }

            // Rename temp to final
            if (targetFile.exists()) targetFile.delete()
            if (!tempFile.renameTo(targetFile)) {
                callback.onError("Download finished but could not move the model into place")
                return
            }

            Log.i(TAG, "downloadFromUrl: complete — ${targetFile.absolutePath} (${targetFile.length()} bytes)")
            callback.onComplete(targetFile.absolutePath)

        } catch (e: Exception) {
            Log.e(TAG, "downloadFromUrl: failed — ${e.message}", e)
            callback.onError("Download failed: ${e.message}")
        }
    }

    private fun extractFileNameFromUrl(url: String): String {
        // Try to get the last path segment before query string
        val path = url.split("?").firstOrNull() ?: url
        val segment = path.substringAfterLast("/")
        if (segment.contains(".") && segment.isNotEmpty()) {
            return java.net.URLDecoder.decode(segment, "UTF-8")
        }
        return "model.litertlm"
    }

    // ------------------------------------------------------------------
    // Private helpers
    // ------------------------------------------------------------------

    private fun cleanupInvalidFiles(model: ModelInfo, targetFile: File, tempFile: File) {
        if (targetFile.exists() && !isValidModelFile(targetFile, model)) {
            Log.w(TAG, "Removing invalid completed model file: ${targetFile.absolutePath} (${targetFile.length()} bytes)")
            targetFile.delete()
        }
        if (tempFile.exists() && tempFile.length() > expectedUpperBound(model)) {
            Log.w(TAG, "Removing oversized partial download: ${tempFile.absolutePath} (${tempFile.length()} bytes)")
            tempFile.delete()
        }
    }

    private fun isValidModelFile(file: File, model: ModelInfo): Boolean {
        if (!file.exists()) return false
        val length = file.length()
        if (length <= 0L) return false
        return length in expectedLowerBound(model)..expectedUpperBound(model)
    }

    private fun expectedLowerBound(model: ModelInfo): Long =
        (model.sizeBytes - maxOf(SIZE_TOLERANCE_BYTES, model.sizeBytes / 20)).coerceAtLeast(1L)

    private fun expectedUpperBound(model: ModelInfo): Long =
        model.sizeBytes + maxOf(SIZE_TOLERANCE_BYTES, model.sizeBytes / 20)
}
