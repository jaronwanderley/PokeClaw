<script setup lang="ts">
import { ref, watch } from 'vue'
import { useModel } from '../composables/useModel'
import { useAgent } from '../composables/useAgent'

const props = defineProps<{
  visible: boolean
}>()

const emit = defineEmits<{
  close: []
  backendChanged: []
}>()

const { preferGpu, selectedModelPath, stopSession, getSessionStatus, currentProvider, setProvider } = useModel()
const { testAgentRound, setApiKey, lastRoundResult, isRunning, error: agentError } = useAgent()

const apiKeyInput = ref('')
const agentPrompt = ref('Tap the Messages app')
const apiKeySaved = ref(false)
const agentResultExpanded = ref(false)
const sessionActive = ref(false)

async function handleStopSession() {
  await stopSession()
  emit('close')
}

function handleBackendToggle() {
  preferGpu.value = !preferGpu.value
  emit('backendChanged')
}

async function handleSaveApiKey() {
  if (!apiKeyInput.value.trim()) return
  const ok = await setApiKey(apiKeyInput.value.trim())
  if (ok) {
    apiKeySaved.value = true
    apiKeyInput.value = ''
  }
}

async function handleTestAgentRound() {
  if (!agentPrompt.value.trim()) return
  await testAgentRound(agentPrompt.value.trim())
  agentResultExpanded.value = true
}

async function handleProviderChange(type: 'openai' | 'anthropic' | 'local') {
  await setProvider(type)
}

async function checkSessionActive() {
  try {
    const status = await getSessionStatus()
    sessionActive.value = status.state === 'ready'
  } catch {
    sessionActive.value = false
  }
}

// Check session status when panel becomes visible
watch(() => props.visible, (newVal) => {
  if (newVal) checkSessionActive()
})
</script>

<template>
  <div v-if="visible" class="settings-overlay" @click.self="emit('close')">
    <div class="settings-panel">
      <div class="settings-header">
        <div class="settings-title">Settings</div>
        <button class="close-btn" @click="emit('close')">
          <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor">
            <path d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z" />
          </svg>
        </button>
      </div>

      <!-- GPU/CPU toggle -->
      <div class="setting-row">
        <div class="setting-label">
          <span class="setting-name">Backend</span>
          <span class="setting-desc">Use GPU acceleration when available</span>
        </div>
        <button
          class="toggle-btn"
          :class="{ active: preferGpu }"
          @click="handleBackendToggle"
        >
          <span class="toggle-label">{{ preferGpu ? 'GPU' : 'CPU' }}</span>
          <span class="toggle-indicator"></span>
        </button>
      </div>

      <!-- LLM Provider selector -->
      <div class="setting-row">
        <div class="setting-label">
          <span class="setting-name">LLM Provider</span>
          <span class="setting-desc">Choose provider for agent tasks</span>
        </div>
        <div class="provider-select">
          <button
            v-for="p in (['openai', 'anthropic', 'local'] as const)"
            :key="p"
            class="provider-btn"
            :class="{ active: currentProvider === p }"
            @click="handleProviderChange(p)"
          >
            {{ p === 'openai' ? 'OpenAI' : p === 'anthropic' ? 'Anthropic' : 'Local' }}
            <span v-if="p === 'local' && sessionActive" class="provider-check">✓</span>
          </button>
        </div>
      </div>

      <!-- Session info -->
      <div v-if="selectedModelPath" class="session-info">
        <div class="session-label">Active Session</div>
        <div class="session-detail">
          <span class="session-key">Model:</span>
          <span class="session-value">{{ selectedModelPath?.split('/').pop() ?? selectedModelPath }}</span>
        </div>
      </div>

      <!-- Change model button -->
      <button v-if="selectedModelPath" class="change-model-btn" @click="handleStopSession">
        Change Model
      </button>

      <!-- OpenAI API Key -->
      <div class="setting-section">
        <div class="setting-name">OpenAI API Key</div>
        <div class="setting-desc" style="margin-bottom: 8px">
          Required for agent round testing. Stored in-memory only.
        </div>
        <div class="api-key-row">
          <input
            v-model="apiKeyInput"
            type="password"
            class="api-key-input"
            placeholder="sk-..."
            @keyup.enter="handleSaveApiKey"
          />
          <button class="save-btn" @click="handleSaveApiKey">
            {{ apiKeySaved ? '✓' : 'Save' }}
          </button>
        </div>
      </div>

      <!-- Agent Test -->
      <div class="setting-section">
        <div class="setting-name">Agent Test</div>
        <div class="setting-desc" style="margin-bottom: 8px">
          Run one LLM → tool execution round.
        </div>
        <div class="agent-test-row">
          <input
            v-model="agentPrompt"
            class="agent-prompt-input"
            placeholder="Enter a prompt..."
            @keyup.enter="handleTestAgentRound"
          />
          <button
            class="test-btn"
            :disabled="isRunning"
            @click="handleTestAgentRound"
          >
            {{ isRunning ? '...' : 'Run' }}
          </button>
        </div>
        <div v-if="agentError" class="agent-error">{{ agentError }}</div>
        <div v-if="lastRoundResult && agentResultExpanded" class="agent-result">
          <div class="result-row">
            <span class="result-key">Model:</span>
            <span class="result-value">{{ lastRoundResult.model }}</span>
          </div>
          <div class="result-row">
            <span class="result-key">Latency:</span>
            <span class="result-value">{{ lastRoundResult.latencyMs }}ms</span>
          </div>
          <div v-if="lastRoundResult.toolCall" class="result-row">
            <span class="result-key">Tool:</span>
            <span class="result-value">{{ lastRoundResult.toolCall.name }}</span>
          </div>
          <div v-if="lastRoundResult.toolCall" class="result-row">
            <span class="result-key">Tool Success:</span>
            <span class="result-value">{{ lastRoundResult.toolCall.result.success ? '✓' : '✗' }}</span>
          </div>
          <div v-if="lastRoundResult.responseText" class="result-row">
            <span class="result-key">Response:</span>
            <span class="result-value">{{ lastRoundResult.responseText }}</span>
          </div>
          <div v-if="lastRoundResult.tokenUsage" class="result-row">
            <span class="result-key">Tokens:</span>
            <span class="result-value">
              {{ lastRoundResult.tokenUsage.promptTokens }}+{{ lastRoundResult.tokenUsage.completionTokens }}
            </span>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  z-index: 100;
  display: flex;
  align-items: flex-end;
  justify-content: center;
}

.settings-panel {
  width: 100%;
  max-width: 390px;
  background: var(--surface);
  border-radius: 16px 16px 0 0;
  padding: 20px 16px 32px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.settings-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.settings-title {
  font-size: 18px;
  font-weight: 700;
  color: var(--t1);
}

.close-btn {
  width: 32px;
  height: 32px;
  border-radius: 50%;
  border: none;
  background: var(--bg);
  color: var(--t2);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all 0.15s;
}

.close-btn:hover {
  background: var(--ai);
}

.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 0;
  border-bottom: 1px solid var(--div);
}

.setting-label {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.setting-name {
  font-size: 14px;
  font-weight: 600;
  color: var(--t1);
}

.setting-desc {
  font-size: 12px;
  color: var(--t2);
}

.toggle-btn {
  position: relative;
  width: 64px;
  height: 32px;
  border-radius: 16px;
  border: none;
  background: var(--ai);
  cursor: pointer;
  transition: background 0.2s;
  display: flex;
  align-items: center;
  padding: 0 8px;
  flex-shrink: 0;
}

.toggle-btn.active {
  background: var(--accent);
}

.toggle-label {
  font-size: 11px;
  font-weight: 700;
  color: var(--bg);
  min-width: 24px;
}

.toggle-btn.active .toggle-label {
  color: #151211;
}

.toggle-indicator {
  position: absolute;
  right: 4px;
  top: 4px;
  width: 24px;
  height: 24px;
  border-radius: 50%;
  background: var(--t1);
  transition: transform 0.2s;
}

.toggle-btn.active .toggle-indicator {
  transform: translateX(0);
}

.toggle-btn:not(.active) .toggle-indicator {
  transform: translateX(-32px);
}

/* Provider selector */
.provider-select {
  display: flex;
  gap: 6px;
  flex-shrink: 0;
}

.provider-btn {
  padding: 4px 10px;
  border-radius: 6px;
  border: 1px solid var(--border);
  background: var(--bg);
  color: var(--t2);
  font-size: 11px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s;
  display: flex;
  align-items: center;
  gap: 3px;
}

.provider-btn:hover {
  border-color: var(--accent);
  color: var(--t1);
}

.provider-btn.active {
  background: var(--accent);
  color: #151211;
  border-color: var(--accent);
}

.provider-check {
  color: #2a7a2a;
  font-weight: 700;
  font-size: 12px;
}

.provider-btn.active .provider-check {
  color: #151211;
}

.session-info {
  padding: 12px;
  border-radius: 8px;
  background: var(--bg);
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.session-label {
  font-size: 11px;
  font-weight: 700;
  color: var(--t2);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.session-detail {
  font-size: 13px;
  display: flex;
  gap: 6px;
}

.session-key {
  color: var(--t2);
}

.session-value {
  color: var(--t1);
  word-break: break-all;
}

.change-model-btn {
  padding: 12px 16px;
  border-radius: 10px;
  border: 1px solid var(--border);
  background: transparent;
  color: var(--t1);
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s;
}

.change-model-btn:hover {
  background: var(--ai);
}

.change-model-btn:active {
  transform: scale(0.98);
}

.setting-section {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 12px 0;
  border-top: 1px solid var(--div);
}

.api-key-row,
.agent-test-row {
  display: flex;
  gap: 8px;
}

.api-key-input,
.agent-prompt-input {
  flex: 1;
  padding: 8px 12px;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--bg);
  color: var(--t1);
  font-size: 13px;
  font-family: inherit;
  outline: none;
}

.api-key-input:focus,
.agent-prompt-input:focus {
  border-color: var(--accent);
}

.save-btn,
.test-btn {
  padding: 8px 16px;
  border-radius: 8px;
  border: none;
  background: var(--accent);
  color: #151211;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s;
  white-space: nowrap;
}

.save-btn:hover,
.test-btn:hover {
  opacity: 0.9;
}

.test-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.agent-error {
  font-size: 12px;
  color: #e74c3c;
  margin-top: 4px;
}

.agent-result {
  margin-top: 8px;
  padding: 10px;
  border-radius: 8px;
  background: var(--bg);
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.result-row {
  font-size: 12px;
  display: flex;
  gap: 6px;
}

.result-key {
  color: var(--t2);
  min-width: 70px;
}

.result-value {
  color: var(--t1);
  word-break: break-all;
}
</style>
