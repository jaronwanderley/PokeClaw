package io.agents.pokeclaw

import android.util.Log
import com.google.ai.edge.litertlm.Backend
import com.google.ai.edge.litertlm.Engine
import com.google.ai.edge.litertlm.EngineConfig

/**
 * Process-wide singleton that keeps a single LiteRT-LM Engine alive.
 *
 * Engine initialisation takes 2-3 s on CPU backend. This holder avoids
 * re-creating the engine when switching between activities or tasks.
 *
 * Thread safety: all mutations are @Synchronized so multiple callers
 * can invoke getOrCreate() safely.
 */
object EngineHolder {

    private const val TAG = "EngineHolder"

    private var engine: Engine? = null
    private var currentModelPath: String? = null
    private var currentBackendLabel: String? = null
    private var currentPfd: android.os.ParcelFileDescriptor? = null

    private fun backendLabel(backend: Backend): String =
        if (backend is Backend.CPU) "CPU"
        else if (backend is Backend.GPU) "GPU"
        else backend.javaClass.simpleName

    /**
     * Return the existing Engine if the model path matches, otherwise close the
     * old one and create a fresh Engine for the new model.
     *
     * @param modelPath  absolute path to the .task model file or /proc/self/fd/..
     * @param pfd        optional ParcelFileDescriptor to keep alive for the engine
     * @param cacheDir   app's cacheDir.path
     * @param backend    CPU or GPU backend
     */
    @Synchronized
    fun getOrCreate(modelPath: String, pfd: android.os.ParcelFileDescriptor?, cacheDir: String, backend: Backend): Engine {
        val existing = engine
        if (existing != null && currentModelPath == modelPath) {
            Log.d(TAG, "getOrCreate: reusing engine for $modelPath (${currentBackendLabel ?: "unknown"})")
            // If the caller provided a new PFD for the same existing model path, close it to avoid leaks.
            // (Typically won't happen because they reuse the same path string)
            if (pfd != null && pfd != currentPfd) {
                try { pfd.close() } catch (e: Exception) {}
            }
            return existing
        }

        // Different model or first call — close old engine first
        if (existing != null) {
            Log.i(TAG, "getOrCreate: model changed ($currentModelPath -> $modelPath), closing old engine")
            try { existing.close() } catch (e: Exception) {
                Log.w(TAG, "getOrCreate: error closing old engine: ${e.message}")
            }
            try { currentPfd?.close() } catch (e: Exception) {}
            engine = null
            currentModelPath = null
            currentPfd = null
        }

        Log.i(TAG, "getOrCreate: creating new engine for $modelPath with ${backend.javaClass.simpleName}")
        return try {
            val engineConfig = EngineConfig(
                modelPath = modelPath,
                backend = backend,
                maxNumTokens = 8192,
                cacheDir = cacheDir
            )
            val newEngine = Engine(engineConfig).also { it.initialize() }
            engine = newEngine
            currentModelPath = modelPath
            currentBackendLabel = backendLabel(backend)
            currentPfd = pfd
            Log.i(TAG, "getOrCreate: engine ready for $modelPath ($currentBackendLabel)")
            newEngine
        } catch (e: Exception) {
            Log.e(TAG, "getOrCreate: failed to create engine for $modelPath: ${e.message}")
            // Close the new PFD if engine creation failed so it doesn't leak
            try { pfd?.close() } catch (closeEx: Exception) {}
            throw e
        }
    }

    /**
     * Explicitly close and release the engine. Call when the model is being
     * unloaded entirely. Normal session transitions should just close their
     * Conversation objects.
     */
    @Synchronized
    fun close() {
        Log.i(TAG, "close: releasing engine for $currentModelPath")
        try { engine?.close() } catch (e: Exception) {
            Log.w(TAG, "close: error closing engine: ${e.message}")
        }
        try { currentPfd?.close() } catch (e: Exception) {}
        engine = null
        currentModelPath = null
        currentBackendLabel = null
        currentPfd = null
        Log.i(TAG, "close: done")
    }

    /** Returns true if an engine is live for the given model path. */
    @Synchronized
    fun isReady(modelPath: String): Boolean =
        engine != null && currentModelPath == modelPath

    /** Returns the actual backend label of the current engine, if any. */
    @Synchronized
    fun getBackendLabel(modelPath: String? = null): String? {
        return if (modelPath == null || currentModelPath == modelPath) currentBackendLabel else null
    }
}
