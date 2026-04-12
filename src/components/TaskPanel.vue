<script setup lang="ts">
import { ref, watch, nextTick } from 'vue'
import { useTask } from '../composables/useTask'

defineProps<{
  visible: boolean
}>()

const emit = defineEmits<{
  close: []
}>()

const {
  taskStatus,
  iterationCount,
  currentTool,
  thinkingText,
  tokenDisplay,
  costDisplay,
  lastAnswer,
  taskError,
  cancelTask,
  resetTask,
} = useTask()

const thinkingRef = ref<HTMLElement | null>(null)

// Auto-scroll thinking text as new content arrives
watch(thinkingText, () => {
  nextTick(() => {
    if (thinkingRef.value) {
      thinkingRef.value.scrollTop = thinkingRef.value.scrollHeight
    }
  })
})

function handleClose() {
  resetTask()
  emit('close')
}

const statusLabel: Record<string, string> = {
  idle: 'Idle',
  running: 'Running',
  completed: 'Completed',
  failed: 'Failed',
  cancelled: 'Cancelled',
}

const statusIcon: Record<string, string> = {
  idle: '⏸',
  running: '⚡',
  completed: '✓',
  failed: '✗',
  cancelled: '⊘',
}
</script>

<template>
  <div v-if="visible" class="task-overlay" @click.self="handleClose">
    <div class="task-panel">
      <!-- Header -->
      <div class="task-header">
        <div class="task-title-row">
          <span class="task-title">Task</span>
          <span v-if="iterationCount > 0" class="iteration-badge">R{{ iterationCount }}</span>
        </div>
        <button class="close-btn" @click="handleClose">
          <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor">
            <path d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z" />
          </svg>
        </button>
      </div>

      <!-- Status indicator -->
      <div class="status-row">
        <span class="status-icon" :class="'status-' + taskStatus">{{ statusIcon[taskStatus] }}</span>
        <span class="status-text" :class="'status-' + taskStatus">{{ statusLabel[taskStatus] }}</span>
        <div v-if="taskStatus === 'running'" class="spinner"></div>
      </div>

      <!-- Current tool -->
      <div v-if="currentTool && taskStatus === 'running'" class="tool-row">
        <div class="spinner-sm"></div>
        <span class="tool-label">Using:</span>
        <span class="tool-name">{{ currentTool }}</span>
      </div>

      <!-- Thinking text (scrollable) -->
      <div v-if="thinkingText" class="thinking-section">
        <div class="thinking-label">Thinking</div>
        <div ref="thinkingRef" class="thinking-text">{{ thinkingText }}</div>
      </div>

      <!-- Token / cost bar -->
      <div v-if="tokenDisplay" class="token-bar">
        <span class="token-info">{{ tokenDisplay }}</span>
        <span v-if="costDisplay" class="token-sep">·</span>
        <span v-if="costDisplay" class="token-cost">{{ costDisplay }}</span>
      </div>

      <!-- Completed answer -->
      <div v-if="taskStatus === 'completed' && lastAnswer" class="answer-section">
        <div class="answer-label">Answer</div>
        <div class="answer-text">{{ lastAnswer }}</div>
      </div>

      <!-- Error display -->
      <div v-if="taskStatus === 'failed' && taskError" class="error-section">
        <div class="error-label">Error</div>
        <div class="error-text">{{ taskError }}</div>
      </div>

      <!-- Cancel button -->
      <button
        v-if="taskStatus === 'running'"
        class="cancel-btn"
        @click="cancelTask"
      >
        Cancel Task
      </button>
    </div>
  </div>
</template>

<style scoped>
.task-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  z-index: 100;
  display: flex;
  align-items: flex-end;
  justify-content: center;
}

.task-panel {
  width: 100%;
  max-width: 390px;
  background: var(--surface);
  border-radius: 16px 16px 0 0;
  padding: 20px 16px 32px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  max-height: 70vh;
  overflow-y: auto;
}

.task-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.task-title-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.task-title {
  font-size: 18px;
  font-weight: 700;
  color: var(--t1);
}

.iteration-badge {
  font-size: 11px;
  font-weight: 700;
  padding: 2px 8px;
  border-radius: 10px;
  background: var(--accent);
  color: #151211;
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

/* Status row */
.status-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.status-icon {
  font-size: 14px;
}

.status-text {
  font-size: 13px;
  font-weight: 600;
}

.status-text.status-running {
  color: #6fcf6f;
}

.status-text.status-completed {
  color: #6fcf6f;
}

.status-text.status-failed {
  color: #e74c3c;
}

.status-text.status-cancelled {
  color: var(--t3);
}

.status-text.status-idle {
  color: var(--t3);
}

/* Spinner */
.spinner {
  width: 14px;
  height: 14px;
  border: 2px solid var(--border);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

.spinner-sm {
  width: 10px;
  height: 10px;
  border: 2px solid var(--border);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
  flex-shrink: 0;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

/* Tool row */
.tool-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border-radius: 8px;
  background: var(--bg);
}

.tool-label {
  font-size: 12px;
  color: var(--t2);
}

.tool-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--accent);
}

/* Thinking section */
.thinking-section {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.thinking-label {
  font-size: 11px;
  font-weight: 700;
  color: var(--t2);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.thinking-text {
  font-size: 12px;
  line-height: 1.5;
  color: var(--t1);
  background: var(--bg);
  border-radius: 8px;
  padding: 10px 12px;
  max-height: 120px;
  overflow-y: auto;
  white-space: pre-wrap;
  word-break: break-word;
}

/* Token bar */
.token-bar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  border-radius: 8px;
  background: var(--bg);
  font-size: 12px;
  color: var(--t2);
}

.token-sep {
  color: var(--t3);
}

.token-cost {
  color: var(--accent);
  font-weight: 600;
}

/* Answer section */
.answer-section {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.answer-label {
  font-size: 11px;
  font-weight: 700;
  color: var(--t2);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.answer-text {
  font-size: 13px;
  line-height: 1.5;
  color: var(--t1);
  background: var(--bg);
  border-radius: 8px;
  padding: 12px;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 150px;
  overflow-y: auto;
}

/* Error section */
.error-section {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.error-label {
  font-size: 11px;
  font-weight: 700;
  color: #e74c3c;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.error-text {
  font-size: 12px;
  color: #e74c3c;
  background: rgba(231, 76, 60, 0.1);
  border-radius: 8px;
  padding: 10px 12px;
  white-space: pre-wrap;
  word-break: break-word;
}

/* Cancel button */
.cancel-btn {
  padding: 12px 16px;
  border-radius: 10px;
  border: 1px solid #e74c3c;
  background: transparent;
  color: #e74c3c;
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s;
}

.cancel-btn:hover {
  background: rgba(231, 76, 60, 0.1);
}

.cancel-btn:active {
  transform: scale(0.98);
}
</style>
