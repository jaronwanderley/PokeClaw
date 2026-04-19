<script setup lang="ts">
import { ref, watch, nextTick, onMounted } from 'vue'
import { useChat } from './composables/useChat'
import { useModel } from './composables/useModel'
import { useAccessibility } from './composables/useAccessibility'
import { useTask } from './composables/useTask'
import ChatMessage from './components/ChatMessage.vue'
import MessageInput from './components/MessageInput.vue'
import ModelPicker from './components/ModelPicker.vue'
import SettingsPanel from './components/SettingsPanel.vue'
import PermissionPanel from './components/PermissionPanel.vue'
import TaskPanel from './components/TaskPanel.vue'

const { messages, streamingText, isStreaming, sessionStatus, sendMessage, setSessionStatus } = useChat()
const { preferGpu } = useModel()
const { screenTree, isLoading: isScreenLoading, error: screenError, fetchScreenInfo } = useAccessibility()
const { startTask, taskStatus } = useTask()
const chatRef = ref<HTMLElement | null>(null)
const showSettings = ref(false)
const showDebug = ref(false)
const showPermissions = ref(false)
const showTaskPanel = ref(false)
const isAndroid = /android/i.test(navigator.userAgent)

function scrollToBottom() {
  nextTick(() => {
    chatRef.value?.scrollTo({ top: chatRef.value.scrollHeight, behavior: 'smooth' })
  })
}

// Auto-scroll on new messages
watch(
  () => messages.value.length,
  scrollToBottom,
)

// Auto-scroll as streaming tokens arrive
watch(streamingText, scrollToBottom)

function handleSessionStarted() {
  setSessionStatus('ready')
}

function handleSettingsClose() {
  showSettings.value = false
}

function handleStartTask(text: string) {
  showTaskPanel.value = true
  startTask(text)
}
onMounted(() => {
  console.log('[App] Mounted. sessionStatus:', sessionStatus.value, 'isAndroid:', isAndroid)
})
</script>

<template>
  <div class="app-shell">
    <div class="tb">
      <div class="tb-left">
        <svg viewBox="0 0 24 24" width="20" height="20" fill="#7A6E64" class="tb-icon">
          <path d="M3 18h18v-2H3v2zm0-5h18v-2H3v2zm0-7v2h18V6H3z" />
        </svg>
        <div class="tb-t">Poke<b>Claw</b></div>
      </div>
      <div class="tb-right">
        <div>v1.0.5</div>
        <div v-if="sessionStatus === 'ready'" class="tb-b" :class="{ 'tb-gpu': preferGpu, 'tb-cpu': !preferGpu }">
          {{ preferGpu ? 'GPU' : 'CPU' }}
        </div>
        <div v-else-if="sessionStatus === 'loading'" class="tb-b tb-loading">Loading...</div>
        <div v-else class="tb-b tb-idle">No Model</div>
        <button class="tb-gear" @click="showSettings = !showSettings">
          <svg viewBox="0 0 24 24" width="20" height="20" fill="#7A6E64">
            <path d="M19.14 12.94c.04-.3.06-.61.06-.94s-.02-.64-.07-.94l2.03-1.58a.49.49 0 00.12-.61l-1.92-3.32a.49.49 0 00-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94L14.4 2.81a.47.47 0 00-.48-.41h-3.84c-.24 0-.43.17-.47.41L9.25 5.35c-.59.24-1.13.57-1.62.94L5.24 5.33a.49.49 0 00-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58a.49.49 0 00-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.57 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6A3.6 3.6 0 1115.6 12 3.61 3.61 0 0112 15.6z" />
          </svg>
        </button>
      </div>
    </div>

    <!-- Model picker when idle -->
    <div v-if="sessionStatus === 'idle'" class="model-area">
      <ModelPicker @session-started="handleSessionStarted" />
    </div>

    <!-- Loading indicator -->
    <div v-else-if="sessionStatus === 'loading'" class="loading-area">
      <div class="loading-spinner"></div>
      <div class="loading-text">Loading model...</div>
    </div>

    <!-- Chat UI when session is ready -->
    <template v-else-if="sessionStatus === 'ready'">
      <div class="chat" ref="chatRef">
        <ChatMessage v-for="msg in messages" :key="msg.id" :message="msg" />
        <!-- Streaming message bubble -->
        <div v-if="isStreaming && streamingText" class="msg msg-a streaming-msg">
          <span class="msg-avatar">🤖</span>
          <div class="msg-bubble">
            {{ streamingText }}<span class="cursor"></span>
          </div>
        </div>
        <!-- Typing indicator when streaming starts but no tokens yet -->
        <div v-else-if="isStreaming" class="msg msg-a streaming-msg">
          <span class="msg-avatar">🤖</span>
          <div class="msg-bubble typing-dots">
            <span></span><span></span><span></span>
          </div>
        </div>
      </div>
      <div class="ia">
        <MessageInput :disabled="isStreaming || taskStatus === 'running'" @send="sendMessage" @start-task="handleStartTask" />
      </div>
    </template>

    <!-- Error state area -->
    <div v-else-if="sessionStatus === 'error'" class="error-area">
      <div class="error-icon">⚠️</div>
      <div class="error-title">Erro na Sessão</div>
      <div class="error-message">Ops! Algo deu errado ao carregar ou processar a sessão de IA.</div>
      <button class="error-retry-btn" @click="setSessionStatus('idle')">Voltar e Tentar Novamente</button>
    </div>

    <!-- Catch-all / Not Initialized area -->
    <div v-else class="loading-area">
      <div class="loading-spinner"></div>
      <div class="loading-text">Inicializando...</div>
    </div>

    <!-- Debug: Accessibility Screen Info -->
    <div v-if="sessionStatus === 'ready'" class="debug-section">
      <div class="debug-toggles">
        <button class="debug-toggle" @click="showDebug = !showDebug">
          {{ showDebug ? '▾ Screen Info' : '▸ Screen Info' }}
        </button>
        <button class="debug-toggle" @click="showPermissions = !showPermissions">
          {{ showPermissions ? '▾ Permissions' : '▸ Permissions' }}
        </button>
      </div>
      <div v-if="showDebug" class="debug-content">
        <button class="debug-fetch-btn" :disabled="isScreenLoading" @click="fetchScreenInfo">
          {{ isScreenLoading ? 'Loading...' : 'Fetch Screen Tree' }}
        </button>
        <div v-if="screenError" class="debug-error">{{ screenError }}</div>
        <pre v-if="screenTree" class="debug-tree">{{ screenTree }}</pre>
      </div>
    </div>

    <!-- Settings panel -->
    <SettingsPanel :visible="showSettings" @close="handleSettingsClose" @backend-changed="() => {}" />

    <!-- Permission panel -->
    <PermissionPanel :visible="showPermissions" @close="showPermissions = false" />

    <!-- Task panel -->
    <TaskPanel :visible="showTaskPanel" @close="showTaskPanel = false" />
  </div>
</template>

<style>
.app-shell {
  display: flex;
  flex-direction: column;
  height: 100%;
  width: 100%;
}

/* Title bar */
.tb {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  padding-top: calc(12px + env(safe-area-inset-top, 0px));
  background: var(--surface);
  border-bottom: 1px solid var(--div);
}

.tb-left {
  display: flex;
  align-items: center;
  gap: 10px;
}

.tb-t {
  font-size: 18px;
  font-weight: 700;
}

.tb-t b {
  color: var(--accent);
}

.tb-right {
  display: flex;
  align-items: center;
  gap: 10px;
}

.tb-b {
  font-size: 10px;
  padding: 2px 8px;
  border-radius: 10px;
  background: var(--ai);
  color: var(--accent);
  border: 1px solid var(--aib);
  font-weight: 700;
  letter-spacing: 0.5px;
}

.tb-b.tb-gpu {
  background: #2a4a2a;
  color: #6fcf6f;
  border-color: #3a6a3a;
}

.tb-b.tb-cpu {
  background: #4a3a2a;
  color: #cfaf6f;
  border-color: #6a5a3a;
}

.tb-b.tb-loading {
  animation: pulse-bg 1.5s ease-in-out infinite;
}

.tb-b.tb-idle {
  color: var(--t3);
}

@keyframes pulse-bg {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.5; }
}

.tb-gear {
  background: none;
  border: none;
  cursor: pointer;
  padding: 4px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  transition: background 0.15s;
}

.tb-gear:hover {
  background: var(--ai);
}

/* Model area (fills space when idle) */
.model-area {
  flex: 1;
  overflow-y: auto;
}

/* Loading area */
.loading-area {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 16px;
}

.loading-spinner {
  width: 32px;
  height: 32px;
  border: 3px solid var(--border);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.loading-text {
  font-size: 14px;
  color: var(--t2);
}

/* Chat area */
.chat {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow-y: auto;
  padding: 12px 16px;
}

/* Input area */
.ia {
  flex-shrink: 0;
  background: var(--surface);
  border-top: 1px solid var(--div);
  padding-bottom: env(safe-area-inset-bottom, 0px);
}

/* Streaming message */
.streaming-msg .msg-avatar {
  font-size: 16px;
  flex-shrink: 0;
  width: 24px;
  height: 24px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.streaming-msg .msg-bubble {
  padding: 10px 14px;
  border-radius: 16px;
  font-size: 13px;
  line-height: 1.5;
  word-wrap: break-word;
  background: var(--ai);
  color: var(--ait);
  border: 1px solid var(--aib);
  border-bottom-left-radius: 4px;
}

/* Blinking cursor */
.cursor {
  display: inline-block;
  width: 2px;
  height: 14px;
  background: var(--ait);
  margin-left: 2px;
  vertical-align: text-bottom;
  animation: blink 0.8s step-end infinite;
}

@keyframes blink {
  50% { opacity: 0; }
}

/* Typing dots indicator */
.typing-dots {
  display: flex;
  gap: 4px;
  align-items: center;
  padding: 12px 16px !important;
}

.typing-dots span {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--t3);
  animation: dot-bounce 1.2s ease-in-out infinite;
}

.typing-dots span:nth-child(2) {
  animation-delay: 0.2s;
}

.typing-dots span:nth-child(3) {
  animation-delay: 0.4s;
}

@keyframes dot-bounce {
  0%, 60%, 100% { transform: translateY(0); opacity: 0.4; }
  30% { transform: translateY(-4px); opacity: 1; }
}

/* Debug section */
.debug-section {
  flex-shrink: 0;
  border-top: 1px solid var(--div);
  background: var(--surface);
  padding: 8px 16px;
}

.debug-toggles {
  display: flex;
  gap: 16px;
}

.debug-toggle {
  background: none;
  border: none;
  cursor: pointer;
  font-size: 12px;
  color: var(--t2);
  padding: 4px 0;
  width: 100%;
  text-align: left;
}

.debug-toggle:hover {
  color: var(--t1);
}

.debug-content {
  margin-top: 8px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.debug-fetch-btn {
  font-size: 12px;
  padding: 6px 12px;
  border-radius: 6px;
  border: 1px solid var(--border);
  background: var(--ai);
  color: var(--t1);
  cursor: pointer;
  align-self: flex-start;
}

.debug-fetch-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.debug-error {
  font-size: 12px;
  color: #e57373;
  padding: 4px 8px;
  background: rgba(229, 115, 115, 0.1);
  border-radius: 4px;
}

.debug-tree {
  font-family: 'SF Mono', 'Menlo', 'Consolas', monospace;
  font-size: 11px;
  line-height: 1.5;
  color: var(--t2);
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 8px 12px;
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 200px;
  overflow-y: auto;
  margin: 0;
}

/* Error Area */
.error-area {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 32px;
  text-align: center;
  gap: 12px;
}

.error-icon {
  font-size: 48px;
  margin-bottom: 8px;
}

.error-title {
  font-size: 18px;
  font-weight: 700;
  color: #e57373;
}

.error-message {
  font-size: 14px;
  color: var(--t2);
  margin-bottom: 12px;
}

.error-retry-btn {
  padding: 10px 20px;
  background: var(--ai);
  border: 1px solid var(--aib);
  border-radius: 8px;
  color: var(--t1);
  font-weight: 600;
  cursor: pointer;
}

.error-retry-btn:hover {
  background: var(--surface);
}
</style>
