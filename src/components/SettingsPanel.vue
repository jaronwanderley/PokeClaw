<script setup lang="ts">
import { useModel } from '../composables/useModel'

defineProps<{
  visible: boolean
}>()

const emit = defineEmits<{
  close: []
  backendChanged: []
}>()

const { preferGpu, selectedModelPath, stopSession, getSessionStatus } = useModel()

async function handleStopSession() {
  await stopSession()
  emit('close')
}

function handleBackendToggle() {
  preferGpu.value = !preferGpu.value
  emit('backendChanged')
}
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
</style>
