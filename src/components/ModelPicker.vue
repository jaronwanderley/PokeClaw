<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useModel } from '../composables/useModel'

const emit = defineEmits<{
  sessionStarted: []
}>()

const {
  modelList,
  isDownloading,
  downloadPercent,
  downloadProgress,
  selectedModelPath,
  preferGpu,
  fetchModels,
  pickModelFile,
  downloadModel,
  downloadFromUrl,
  startSession,
  getSafFolderStatus,
  pickSafFolder,
  hasSafPermission,
  safFolderName,
} = useModel()

const isLoadingModels = ref(false)
const modelFetchError = ref<string | null>(null)


async function checkSafStatus() {
  if (isAndroid) {
    isLoadingModels.value = true
    modelFetchError.value = null
    try {
      const status = await getSafFolderStatus()
      if (status.hasPermission) {
        await fetchModels()
      }
    } catch (err) {
      console.error('checkSafStatus failed:', err)
      modelFetchError.value = String(err)
    } finally {
      isLoadingModels.value = false
    }
  } else {
    isLoadingModels.value = true
    try {
      await fetchModels()
    } finally {
      isLoadingModels.value = false
    }
  }
}

onMounted(() => {
  console.log('[ModelPicker] Mounted. isAndroid:', isAndroid)
  checkSafStatus()
})

async function handlePickSafFolder() {
  const result = await pickSafFolder()
  if (result) {
    await checkSafStatus()
  }
}

const isAndroid = /android/i.test(navigator.userAgent)
const urlInput = ref('')
const showUrlPanel = ref(false)
const urlError = ref<string | null>(null)


function formatSize(bytes: number): string {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(0)} MB`
  return `${(bytes / 1_000).toFixed(0)} KB`
}

function formatSpeed(bytesPerSec: number): string {
  if (bytesPerSec >= 1_000_000_000) return `${(bytesPerSec / 1_000_000_000).toFixed(1)} GB/s`
  if (bytesPerSec >= 1_000_000) return `${(bytesPerSec / 1_000_000).toFixed(0)} MB/s`
  if (bytesPerSec >= 1_000) return `${(bytesPerSec / 1_000).toFixed(0)} KB/s`
  return `${bytesPerSec} B/s`
}

async function handleLoad(modelPath: string) {
  selectedModelPath.value = modelPath
  const result = await startSession()
  if (result) emit('sessionStarted')
}

async function handlePickFile() {
  const path = await pickModelFile()
  if (path) {
    selectedModelPath.value = path
    const result = await startSession()
    if (result) emit('sessionStarted')
  }
}

async function handleDownload(modelId: string) {
  await downloadModel(modelId)
  if (selectedModelPath.value) {
    const result = await startSession()
    if (result) emit('sessionStarted')
  }
}

function getGemmaUrl(variant: 'e2b' | 'e4b'): { url: string; fileName: string } {
  if (variant === 'e2b') {
    return {
      url: 'https://huggingface.co/litert-community/gemma-4-E2B-it-litert-lm/resolve/main/gemma-4-E2B-it.litertlm',
      fileName: 'gemma-4-E2B-it.litertlm',
    }
  }
  return {
    url: 'https://huggingface.co/litert-community/gemma-4-E4B-it-litert-lm/resolve/main/gemma-4-E4B-it.litertlm',
    fileName: 'gemma-4-E4B-it.litertlm',
  }
}

async function handleQuickDownload(variant: 'e2b' | 'e4b') {
  urlError.value = null
  const { url, fileName } = getGemmaUrl(variant)
  urlInput.value = url
  await downloadFromUrl(url, fileName)
  if (selectedModelPath.value) {
    const result = await startSession()
    if (result) emit('sessionStarted')
  }
}

async function handleUrlDownload() {
  urlError.value = null
  const url = urlInput.value.trim()
  if (!url) {
    urlError.value = 'Paste a download link first.'
    return
  }
  try { new URL(url) } catch {
    urlError.value = 'Invalid URL. Paste a direct download link for a .litertlm file.'
    return
  }

  // Extract filename from URL
  const urlPath = url.split('?')[0]
  const fileName = urlPath.split('/').pop() || 'model.litertlm'

  await downloadFromUrl(url, fileName)
  if (selectedModelPath.value) {
    const result = await startSession()
    if (result) emit('sessionStarted')
  }
}

function toggleUrlPanel() {
  showUrlPanel.value = !showUrlPanel.value
  urlError.value = null
}
</script>

<template>
  <div class="picker">
    <div class="picker-header">
      <div class="picker-title">Select a Model</div>
      <div class="picker-subtitle">Choose a model to start chatting</div>
    </div>

    <!-- Android SAF Folder Setup -->
    <div v-if="isAndroid && !hasSafPermission" class="saf-setup-card">
      <div class="saf-icon">📂</div>
      <div class="saf-info">
        <div class="saf-title">Pasta de Modelos (SAF)</div>
        <div class="saf-desc">Escolha uma pasta para manter seus modelos centralizados e economizar espaço.</div>
      </div>
      <button class="saf-btn" @click="handlePickSafFolder">
        Selecionar Pasta
      </button>
    </div>

    <!-- Pick from device (only if not Android or has permission) -->
    <button v-if="!isAndroid" class="pick-file-btn" @click="handlePickFile">
      <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor">
        <path d="M9 16h6v-6h4l-7-7-7 7h4v6zm-4 2h14v2H5v-2z" />
      </svg>
      {{ isAndroid ? 'Pick from device' : 'Pick from device' }}
    </button>

    <!-- Content visible after SAF setup -->
    <template v-if="!isAndroid || hasSafPermission">
      <div v-if="isAndroid" class="saf-current-folder">
        <span class="saf-label">Pasta Atual:</span>
        <span class="saf-name">{{ safFolderName || 'Shared Models' }}</span>
        <button class="saf-change-btn" @click="handlePickSafFolder">Alterar</button>
      </div>

      <!-- Download a model -->
      <button class="pick-file-btn url-toggle" @click="toggleUrlPanel">
        <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor">
          <path d="M19 9h-4V3H9v6H5l7 7 7-7zM5 18v2h14v-2H5z" />
        </svg>
        {{ showUrlPanel ? 'Hide download panel' : 'Download a model' }}
      </button>
    </template>

    <!-- Download panel -->
    <div v-if="showUrlPanel" class="url-panel">
      <!-- Quick download buttons -->
      <div class="quick-downloads">
        <div class="section-label">Quick download</div>
        <div class="quick-btns">
          <button
            class="quick-btn"
            :disabled="isDownloading"
            @click="handleQuickDownload('e2b')"
          >
            Gemma 4 E2B
            <span class="quick-btn-size">2.6 GB</span>
          </button>
          <button
            class="quick-btn"
            :disabled="isDownloading"
            @click="handleQuickDownload('e4b')"
          >
            Gemma 4 E4B
            <span class="quick-btn-size">3.6 GB</span>
          </button>
        </div>
        <div class="quick-hint">
          {{ isAndroid ? `Saves to: ${safFolderName || 'Shared Models'}` : 'Saves to your models folder' }}
        </div>
      </div>

      <!-- Custom URL input -->
      <div class="url-input-section">
        <div class="section-label">Or paste a custom link</div>
        <div class="url-row">
          <input
            v-model="urlInput"
            type="url"
            placeholder="https://example.com/model.litertlm"
            class="url-input"
            :disabled="isDownloading"
            @keydown.enter="handleUrlDownload"
          />
          <button
            class="download-url-btn"
            :disabled="isDownloading || !urlInput.trim()"
            @click="handleUrlDownload"
          >
            Download
          </button>
        </div>
        <div v-if="urlError" class="url-error">{{ urlError }}</div>
      </div>

      <!-- Download progress -->
      <div v-if="isDownloading" class="download-progress compact">
        <div class="progress-bar wide">
          <div class="progress-fill" :style="{ width: downloadPercent + '%' }"></div>
        </div>
        <div class="progress-details">
          <span class="progress-text">{{ downloadPercent }}%</span>
          <span v-if="downloadProgress.bytesPerSecond > 0" class="progress-speed">
            {{ formatSpeed(downloadProgress.bytesPerSecond) }}
          </span>
          <span class="progress-bytes">
            {{ formatSize(downloadProgress.bytesDownloaded) }}
            <span v-if="downloadProgress.totalBytes > 0">
              / {{ formatSize(downloadProgress.totalBytes) }}
            </span>
          </span>
        </div>
      </div>
    </div>

    <!-- Model Catalog -->
    <div class="catalog-section">
      <h3 class="section-title">Modelos Disponíveis</h3>
      
      <div v-if="isLoadingModels" class="catalog-loading">
        <div class="loading-spinner small"></div>
        <span>Buscando modelos...</span>
      </div>
      
      <div v-else-if="modelFetchError" class="catalog-error">
        <span>Erro ao carregar modelos: {{ modelFetchError }}</span>
        <button @click="checkSafStatus">Tentar Novamente</button>
      </div>

      <div v-else-if="modelList.length === 0" class="catalog-empty">
        <p v-if="isAndroid && hasSafPermission">
          Nenhum arquivo .litertlm encontrado na pasta selecionada.
        </p>
        <p v-else-if="isAndroid">
          Selecione uma pasta para listar os modelos.
        </p>
        <p v-else>
          Nenhum modelo encontrado no diretório padrão.
        </p>
      </div>

      <div v-else class="model-list">
        <div v-for="model in modelList" :key="model.id" class="model-card">
          <div class="model-info">
            <div class="model-name">{{ model.displayName }}</div>
            <div class="model-meta">
              {{ formatSize(model.sizeBytes) }} · {{ model.minRamGb }} GB RAM
            </div>
          </div>

          <div v-if="isDownloading" class="download-progress">
            <div class="progress-bar">
              <div class="progress-fill" :style="{ width: downloadPercent + '%' }"></div>
            </div>
            <div class="progress-text">{{ downloadPercent }}%</div>
          </div>

          <button
            v-else-if="!model.isDownloaded"
            class="action-btn download-btn"
            :disabled="isDownloading"
            @click="handleDownload(model.id)"
          >
            Download
          </button>

          <button
            v-else
            class="action-btn load-btn"
            @click="handleLoad(model.localPath ?? model.fileName)"
          >
            Load
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.picker {
  display: flex;
  flex-direction: column;
  padding: 16px;
  gap: 12px;
  max-width: 640px;
  margin: 0 auto;
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

.pick-file-btn:hover {
  background: var(--aib);
  border-color: var(--accent);
}

.saf-setup-card {
  display: flex;
  align-items: center;
  padding: 16px;
  background: var(--ai);
  border: 1px solid var(--aib);
  border-radius: 12px;
  gap: 12px;
}

.saf-icon {
  font-size: 24px;
}

.saf-info {
  flex: 1;
}

.saf-title {
  font-size: 14px;
  font-weight: 700;
  color: var(--t1);
}

.saf-desc {
  font-size: 11px;
  color: var(--t2);
  margin-top: 2px;
}

.saf-btn {
  padding: 8px 12px;
  background: var(--accent);
  color: #151211;
  border: none;
  border-radius: 8px;
  font-size: 12px;
  font-weight: 700;
  cursor: pointer;
}

.saf-current-folder {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 8px;
  font-size: 12px;
}

.saf-label {
  color: var(--t2);
  font-weight: 600;
}

.saf-name {
  color: var(--accent);
  font-weight: 700;
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.saf-change-btn {
  background: transparent;
  border: none;
  color: var(--t2);
  font-size: 11px;
  text-decoration: underline;
  cursor: pointer;
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

.pick-file-btn:active {
  transform: scale(0.98);
}

.url-toggle {
  border-style: solid;
  background: var(--surface);
}

.url-toggle:hover {
  background: var(--ai);
}

.url-panel {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 14px;
  border-radius: 12px;
  background: var(--surface);
  border: 1px solid var(--border);
}

.section-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--t2);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  margin-bottom: 6px;
}

.quick-downloads {
  display: flex;
  flex-direction: column;
}

.quick-btns {
  display: flex;
  gap: 8px;
}

.quick-btn {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  padding: 14px 12px;
  border-radius: 10px;
  background: var(--accent);
  color: #151211;
  border: none;
  font-size: 14px;
  font-weight: 700;
  cursor: pointer;
  transition: all 0.15s;
}

.quick-btn:hover:not(:disabled) {
  opacity: 0.9;
}

.quick-btn:active:not(:disabled) {
  transform: scale(0.97);
}

.quick-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.quick-btn-size {
  font-size: 11px;
  font-weight: 500;
  opacity: 0.7;
}

.quick-hint {
  font-size: 11px;
  color: var(--t2);
  text-align: center;
  margin-top: 6px;
}

.url-input-section {
  display: flex;
  flex-direction: column;
}

.url-row {
  display: flex;
  gap: 8px;
}

.url-input {
  flex: 1;
  padding: 10px 12px;
  border-radius: 8px;
  background: var(--bg);
  border: 1px solid var(--border);
  color: var(--t1);
  font-size: 13px;
  outline: none;
  transition: border-color 0.15s;
}

.url-input:focus {
  border-color: var(--accent);
}

.url-input:disabled {
  opacity: 0.5;
}

.url-input::placeholder {
  color: var(--t2);
}

.download-url-btn {
  padding: 10px 16px;
  border-radius: 8px;
  background: var(--accent);
  color: #151211;
  border: none;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  flex-shrink: 0;
  transition: all 0.15s;
}

.download-url-btn:hover:not(:disabled) {
  opacity: 0.9;
}

.download-url-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.url-error {
  margin-top: 6px;
  font-size: 12px;
  color: #e74c3c;
  font-weight: 500;
}

.download-progress.compact {
  padding-top: 4px;
}

.progress-bar.wide {
  width: 100%;
}

.progress-details {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 6px;
}

.progress-speed {
  font-size: 11px;
  color: var(--accent);
  font-weight: 600;
}

.progress-bytes {
  font-size: 11px;
  color: var(--t2);
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

.catalog-empty {
  padding: 40px;
  text-align: center;
  color: var(--t3);
  font-size: 14px;
  background: var(--bg);
  border-radius: 12px;
  border: 1px dashed var(--border);
}

.catalog-loading {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  padding: 40px;
  color: var(--t3);
}

.catalog-error {
  padding: 20px;
  text-align: center;
  color: #e57373;
  background: rgba(229, 115, 115, 0.1);
  border-radius: 8px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.catalog-error button {
  align-self: center;
  padding: 6px 16px;
  background: var(--ai);
  border: 1px solid var(--aib);
  border-radius: 6px;
  color: var(--t1);
  cursor: pointer;
}

.loading-spinner.small {
  width: 24px;
  height: 24px;
  border-width: 2px;
}
</style>
