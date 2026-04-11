// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

package io.agents.pokeclaw.agent.llm

import android.content.Context
import com.google.ai.edge.litertlm.Backend
import com.google.ai.edge.litertlm.Engine
import io.agents.pokeclaw.utils.KVUtils
import io.agents.pokeclaw.utils.XLog

data class LocalEngineLease(
    val engine: Engine,
    val backendLabel: String,
)

object LocalModelRuntime {

    private const val TAG = "LocalModelRuntime"

    fun acquireSharedEngine(
        context: Context,
        modelPath: String,
        preferCpu: Boolean = false,
    ): LocalEngineLease {
        val shouldUseCpu = preferCpu || KVUtils.getLocalBackendPreference().equals("CPU", ignoreCase = true)
        if (shouldUseCpu) {
            val engine = EngineHolder.getOrCreate(modelPath, context.cacheDir.path, Backend.CPU())
            return LocalEngineLease(engine = engine, backendLabel = "CPU")
        }

        return try {
            val engine = EngineHolder.getOrCreate(modelPath, context.cacheDir.path, Backend.GPU())
            LocalEngineLease(engine = engine, backendLabel = EngineHolder.getBackendLabel(modelPath) ?: "GPU")
        } catch (e: Exception) {
            if (!isGpuBackendFailure(e)) throw e
            XLog.w(TAG, "GPU runtime failed for $modelPath, retrying on CPU: ${e.message}")
            forceCpuEngine(context, modelPath)
        }
    }

    fun forceCpuEngine(context: Context, modelPath: String): LocalEngineLease {
        KVUtils.setLocalBackendPreference("CPU")
        resetSharedEngine()
        val engine = EngineHolder.getOrCreate(modelPath, context.cacheDir.path, Backend.CPU())
        return LocalEngineLease(engine = engine, backendLabel = "CPU")
    }

    fun resetSharedEngine() {
        try {
            EngineHolder.close()
        } catch (e: Exception) {
            XLog.w(TAG, "resetSharedEngine: close failed", e)
        }
    }

    fun currentBackendLabel(modelPath: String?): String? {
        return EngineHolder.getBackendLabel(modelPath)
    }

    fun isGpuBackendFailure(error: Throwable?): Boolean {
        val message = error?.message.orEmpty()
        if (message.isEmpty()) return false
        return message.contains("OpenCL", ignoreCase = true) ||
            message.contains("GPU", ignoreCase = true) ||
            message.contains("nativeSendMessage", ignoreCase = true) ||
            message.contains("Failed to create engine", ignoreCase = true) ||
            message.contains("compiled model", ignoreCase = true)
    }
}
