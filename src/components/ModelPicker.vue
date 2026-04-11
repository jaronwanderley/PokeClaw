<script setup lang="ts">
import { onMounted, computed } from 'vue'
import { useModel } from '../composables/useModel'

const emit = defineEmits<{
  sessionStarted: []
}>()

const {
  modelList,
  isDownloading,
  downloadProgress,
  selectedModelPath,
  fetchModels,
  pickModelFile,
  downloadModel,
  startSession,
} = useModel()

onMounted(() => {
  fetchModels()
})

const downloadPercent = computed(() => {
  if (downloadProgress.value.totalBytes === 0) return 0
  return Math.round((downloadProgress.value.bytesDownloaded / downloadProgress.value.totalBytes) * 100)
})

function formatSize(bytes: number): string {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(0)} MB`
  return `${(bytes / 1_000).toFixed(0)} KB`
}

async function handleLoad(modelPath: string) {
  selectedModelPath.value = modelPath
  const result = await startSession()
  if (result) {
    emit('sessionStarted')
  }
}

async function handlePickFile() {
  const path = await pickModelFile()
  if (path) {
    selectedModelPath.value = path
    const result = await startSession()
    if (result) {
      emit('sessionStarted')
    }
  }
}

async function handleDownload(modelId: string) {
  await downloadModel(modelId)
}
</script>

<template>
  <div class="picker">
    <div class="picker-header">
      <div class="picker-title">Select a Model</div>
      <div class="picker-subtitle">Choose a model to start chatting</div>
    </div>

    <!-- Pick from device button -->
    <button class="pick-file-btn" @click="handlePickFile">
      <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor">
        <path d="M9 16h6v-6h4l-7-7-7 7h4v6zm-4 2h14v2H5v-2z" />
      </svg>
      Pick from device
    </button>

    <!-- Model list -->
    <div class="model-list">
      <div v-for="model in modelList" :key="model.id" class="model-card">
        <div class="model-info">
          <div class="model-name">{{ model.displayName }}</div>
          <div class="model-meta">
            {{ formatSize(model.sizeBytes) }} · {{ model.minRamGb }} GB RAM
          </div>
        </div>

        <!-- Download progress bar (replaces button during download) -->
        <div v-if="isDownloading" class="download-progress">
          <div class="progress-bar">
            <div class="progress-fill" :style="{ width: downloadPercent + '%' }"></div>
          </div>
          <div class="progress-text">{{ downloadPercent }}%</div>
        </div>

        <!-- Download button -->
        <button
          v-else-if="!model.isDownloaded"
          class="action-btn download-btn"
          :disabled="isDownloading"
          @click="handleDownload(model.id)"
        >
          Download
        </button>

        <!-- Load button (model already downloaded) -->
        <button
          v-else
          class="action-btn load-btn"
          @click="handleLoad(model.localPath ?? model.fileName)"
        >
          Load
        </button>
      </div>
    </div>

    <!-- Empty state -->
    <div v-if="modelList.length === 0 && !isDownloading" class="empty-state">
      No models available. Pick a model file from your device to get started.
    </div>
  </div>
</template>

<style scoped>
.picker {
  display: flex;
  flex-direction: column;
  padding: 16px;
  gap: 12px;
}

.picker-header {
  margin-bottom: 4px;
}

.picker-title {
  font-size: 20px;
  font-weight: 700;
  color: var(--t1);
}

.picker-subtitle {
  font-size: 13px;
  color: var(--t2);
  margin-top: 4px;
}

.pick-file-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 12px 16px;
  border-radius: 12px;
  background: var(--ai);
  color: var(--accent);
  border: 1px dashed var(--aib);
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s;
}

.pick-file-btn:hover {
  background: var(--aib);
  border-color: var(--accent);
}

.pick-file-btn:active {
  transform: scale(0.98);
}

.model-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.model-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 16px;
  border-radius: 12px;
  background: var(--surface);
  border: 1px solid var(--border);
  gap: 12px;
}

.model-info {
  flex: 1;
  min-width: 0;
}

.model-name {
  font-size: 14px;
  font-weight: 600;
  color: var(--t1);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.model-meta {
  font-size: 12px;
  color: var(--t2);
  margin-top: 2px;
}

.action-btn {
  padding: 8px 16px;
  border-radius: 8px;
  border: none;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s;
  flex-shrink: 0;
}

.load-btn {
  background: var(--accent);
  color: #151211;
}

.load-btn:hover {
  opacity: 0.9;
}

.load-btn:active {
  transform: scale(0.96);
}

.download-btn {
  background: var(--ai);
  color: var(--accent);
  border: 1px solid var(--aib);
}

.download-btn:hover {
  background: var(--aib);
}

.download-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.download-progress {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 4px;
  flex-shrink: 0;
  min-width: 80px;
}

.progress-bar {
  width: 80px;
  height: 6px;
  border-radius: 3px;
  background: var(--bg);
  overflow: hidden;
}

.progress-fill {
  height: 100%;
  border-radius: 3px;
  background: var(--accent);
  transition: width 0.2s ease;
}

.progress-text {
  font-size: 11px;
  color: var(--t2);
  font-weight: 600;
}

.empty-state {
  text-align: center;
  padding: 32px 16px;
  color: var(--t2);
  font-size: 14px;
}
</style>
