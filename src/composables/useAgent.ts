import { ref, readonly } from 'vue'
import { invoke } from '@tauri-apps/api/core'

export interface ToolCallResult {
  name: string
  arguments: Record<string, unknown>
  result: {
    success: boolean
    data?: unknown
    error?: string
  }
}

export interface TokenUsage {
  promptTokens: number
  completionTokens: number
}

export interface AgentRoundResult {
  prompt: string
  model: string
  toolCall: ToolCallResult | null
  responseText: string | null
  latencyMs: number
  tokenUsage: TokenUsage | null
}

const lastRoundResult = ref<AgentRoundResult | null>(null)
const isRunning = ref(false)
const error = ref<string | null>(null)

export function useAgent() {
  /**
   * Execute one agent round: prompt → LLM → (optional) tool → result.
   * Returns the AgentRoundResult from the backend.
   */
  async function testAgentRound(prompt: string): Promise<AgentRoundResult | null> {
    if (!prompt.trim()) {
      error.value = 'Prompt must not be empty'
      return null
    }

    isRunning.value = true
    error.value = null

    try {
      console.log('[useAgent] testAgentRound: prompt=' + prompt)
      const result = await invoke<AgentRoundResult>('testAgentRound', {
        prompt,
        systemPrompt: null,
      })
      lastRoundResult.value = result
      console.log('[useAgent] testAgentRound complete:', result)
      return result
    } catch (err) {
      const msg = String(err)
      error.value = msg
      console.error('[useAgent] testAgentRound failed:', err)
      return null
    } finally {
      isRunning.value = false
    }
  }

  /**
   * Store the OpenAI API key in the backend (in-memory only).
   */
  async function setApiKey(key: string): Promise<boolean> {
    try {
      await invoke('setOpenAiApiKey', { key })
      console.log('[useAgent] API key set successfully')
      return true
    } catch (err) {
      error.value = String(err)
      console.error('[useAgent] setApiKey failed:', err)
      return false
    }
  }

  return {
    lastRoundResult: readonly(lastRoundResult),
    isRunning: readonly(isRunning),
    error: readonly(error),
    testAgentRound,
    setApiKey,
  }
}
