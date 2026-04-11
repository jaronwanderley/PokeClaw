// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

package io.agents.pokeclaw.agent.llm

import dev.langchain4j.data.message.ChatMessage
import dev.langchain4j.data.message.SystemMessage
import dev.langchain4j.data.message.UserMessage
import dev.langchain4j.model.anthropic.AnthropicChatModel
import dev.langchain4j.model.chat.request.ChatRequest
import dev.langchain4j.model.openai.OpenAiChatModel
import io.agents.pokeclaw.agent.langchain.http.OkHttpClientBuilderAdapter
import io.agents.pokeclaw.utils.XLog

/**
 * Single source of truth for LLM client creation.
 *
 * Eliminates duplicate client construction in ComposeChatActivity,
 * AutoReplyManager.generateReplyCloud(), and AutoReplyManager.singleLlmCall().
 *
 * Thread-safe. All methods can be called from any thread.
 */
object LlmSessionManager {

    private const val TAG = "LlmSessionManager"

    /**
     * Create a Cloud LLM ChatModel using the user's current config.
     * Returns null if no API key is configured.
     */
    fun createCloudChatModel(temperature: Double = 0.7): dev.langchain4j.model.chat.ChatModel? {
        val config = ModelConfigRepository.snapshot()
        if (config.activeMode == ActiveModelMode.LOCAL) {
            XLog.w(TAG, "createCloudChatModel: local mode is active")
            return null
        }

        val cloud = config.activeCloud
        if (cloud.apiKey.isEmpty()) {
            XLog.w(TAG, "createCloudChatModel: no API key configured")
            return null
        }

        XLog.d(TAG, "createCloudChatModel: provider=${cloud.providerName}, model=${cloud.modelName}, baseUrl=${cloud.resolvedBaseUrl}")
        return when (cloud.agentProvider) {
            io.agents.pokeclaw.agent.LlmProvider.ANTHROPIC -> AnthropicChatModel.builder()
                .httpClientBuilder(OkHttpClientBuilderAdapter())
                .apiKey(cloud.apiKey)
                .modelName(cloud.modelName)
                .baseUrl(cloud.resolvedBaseUrl)
                .temperature(temperature)
                .build()

            else -> OpenAiChatModel.builder()
                .httpClientBuilder(OkHttpClientBuilderAdapter())
                .apiKey(cloud.apiKey)
                .modelName(cloud.modelName.ifEmpty { "gpt-4o-mini" })
                .baseUrl(cloud.resolvedBaseUrl.ifEmpty { "https://api.openai.com/v1" })
                .temperature(temperature)
                .build()
        }
    }

    /**
     * Create a Cloud LlmClient using the resolved active config.
     */
    fun createCloudClient(temperature: Double = 0.7): LlmClient? {
        val config = ModelConfigRepository.snapshot()
        if (config.activeMode == ActiveModelMode.LOCAL) return null
        val cloud = config.activeCloud
        if (cloud.apiKey.isEmpty() || cloud.modelName.isEmpty()) {
            XLog.w(TAG, "createCloudClient: incomplete cloud config")
            return null
        }
        return LlmClientFactory.create(
            config.toAgentConfig(
                temperature = temperature,
                maxIterations = 60
            )
        )
    }

    /**
     * Single-shot LLM call — send one prompt, get one response.
     * Uses the user's selected Cloud or Local model.
     * For quick targeted questions (not a full agent loop).
     *
     * @return LLM response text, or null if failed
     */
    fun singleShot(prompt: String, temperature: Double = 0.3): String? {
        return if (ModelConfigRepository.snapshot().activeMode == ActiveModelMode.CLOUD) {
            singleShotCloud(prompt, temperature)
        } else {
            singleShotLocal(prompt, temperature)
        }
    }

    /**
     * Single-shot Cloud LLM call.
     */
    fun singleShotCloud(prompt: String, temperature: Double = 0.7): String? {
        return try {
            val chatModel = createCloudChatModel(temperature) ?: return null
            val messages = listOf<ChatMessage>(UserMessage.from(prompt))
            val request = ChatRequest.builder().messages(messages).build()
            val response = chatModel.chat(request)
            response.aiMessage().text()
        } catch (e: Exception) {
            XLog.w(TAG, "singleShotCloud failed: ${e.message}")
            null
        }
    }

    /**
     * Single-shot Cloud LLM call with system prompt.
     */
    fun singleShotCloud(systemPrompt: String, userPrompt: String, temperature: Double = 0.7): String? {
        return try {
            val chatModel = createCloudChatModel(temperature) ?: return null
            val messages = listOf<ChatMessage>(
                SystemMessage.from(systemPrompt),
                UserMessage.from(userPrompt)
            )
            val request = ChatRequest.builder().messages(messages).build()
            val response = chatModel.chat(request)
            response.aiMessage().text()
        } catch (e: Exception) {
            XLog.w(TAG, "singleShotCloud failed: ${e.message}")
            null
        }
    }

    /**
     * Single-shot Local LLM call using LiteRT-LM.
     */
    fun singleShotLocal(prompt: String, temperature: Double = 0.3): String? {
        return try {
            val modelPath = ModelConfigRepository.snapshot().local.modelPath
            if (modelPath.isNullOrEmpty()) return null

            val context = io.agents.pokeclaw.ClawApplication.instance
            fun runSingleShot() : String? {
                val engine = LocalModelRuntime.acquireSharedEngine(context, modelPath).engine
                val conv = engine.createConversation(
                    com.google.ai.edge.litertlm.ConversationConfig(
                        com.google.ai.edge.litertlm.Contents.of("You are a helpful assistant. Answer concisely."),
                        emptyList(), emptyList(),
                        com.google.ai.edge.litertlm.SamplerConfig(64, 0.95, temperature, 0)
                    )
                )
                return try {
                    val response = conv.sendMessage(prompt, emptyMap())
                    response.contents?.toString()?.trim()
                } finally {
                    conv.close()
                }
            }

            try {
                runSingleShot()
            } catch (e: Exception) {
                if (!LocalModelRuntime.isGpuBackendFailure(e)) throw e
                XLog.w(TAG, "singleShotLocal: GPU path failed, retrying on CPU: ${e.message}")
                LocalModelRuntime.forceCpuEngine(context, modelPath)
                runSingleShot()
            }
        } catch (e: Exception) {
            XLog.w(TAG, "singleShotLocal failed: ${e.message}")
            null
        }
    }

    /**
     * Check if Cloud LLM is configured (has API key).
     */
    fun isCloudConfigured(): Boolean {
        return ModelConfigRepository.snapshot().defaultCloud.isConfigured
    }
}
