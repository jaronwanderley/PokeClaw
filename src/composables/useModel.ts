import { ref, computed } from 'vue'
import { invoke, Channel } from '@tauri-apps/api/core'

// ---------------------------------------------------------------------------
// TypeScript interfaces — match Rust serde + Kotlin JSObject shapes
// ---------------------------------------------------------------------------

export interface ModelInfo {
  id: string
  displayName: string
  url: string
  fileName: string
  sizeBytes: number
  minRamGb: number
  isDownloaded: boolean
  localPath: string | null
}

export interface DownloadProgress {
  bytesDownloaded: number
  totalBytes: number
  bytesPerSecond: number
}

type DownloadEvent =
  | { event: 'progress'; data: { bytesDownloaded: number; totalBytes: number; bytesPerSecond: number } }
  | { event: 'complete'; data: { modelPath: string; fileName: string } }
  | { event: 'error'; data: { message: string } }

// ---------------------------------------------------------------------------
// Module-level singleton state (R002/R030: no Pinia)
// ---------------------------------------------------------------------------

const modelList = ref<ModelInfo[]>([])
const isDownloading = ref(false)
const downloadProgress = ref<DownloadProgress>({
  bytesDownloaded: 0,
  totalBytes: 0,
  bytesPerSecond: 0,
})
const selectedModelPath = ref<string | null>(null)
const preferGpu = ref(true)

// Provider auto-switch state
const currentProvider = ref<'openai' | 'anthropic' | 'local'>('openai')

// Shared SAF state
const hasSafPermission = ref(false)
const safFolderName = ref('')
const isSessionLoading = ref(false)

const downloadPercent = computed(() => {
  if (downloadProgress.value.totalBytes === 0) return 0
  return Math.round((downloadProgress.value.bytesDownloaded / downloadProgress.value.totalBytes) * 100)
})

// ---------------------------------------------------------------------------
// Platform detection
// ---------------------------------------------------------------------------

function isAndroid(): boolean {
  return typeof navigator !== 'undefined' && /android/i.test(navigator.userAgent)
}

// ---------------------------------------------------------------------------
// Composable
// ---------------------------------------------------------------------------

export function useModel() {
  async function fetchModels(): Promise<void> {
    try {
      if (isAndroid()) {
        const status = await getSafFolderStatus()
        if (status.hasPermission) {
          await listSafModels()
          return
        }
      }

      const result = await invoke<ModelInfo[] | { models: ModelInfo[] }>('plugin:pokeclaw|listModels')
      if (Array.isArray(result)) {
        modelList.value = result
      } else if (result && 'models' in result) {
        modelList.value = result.models
      } else {
        modelList.value = []
      }
      console.log('[useModel] fetchModels: loaded', modelList.value.length, 'models')
    } catch (err) {
      console.error('[useModel] fetchModels failed:', err)
      modelList.value = []
    }
  }

  async function getSafFolderStatus(): Promise<{ hasPermission: boolean; folderUri?: string; folderName?: string }> {
    try {
      const status = await invoke<{ hasPermission: boolean; folderUri?: string; folderName?: string }>('plugin:pokeclaw|getSafFolderStatus')
      hasSafPermission.value = status.hasPermission
      safFolderName.value = status.folderName || ''
      return status
    } catch (err) {
      console.error('[useModel] getSafFolderStatus failed:', err)
      hasSafPermission.value = false
      return { hasPermission: false }
    }
  }

  async function pickSafFolder(): Promise<{ uri: string } | null> {
    try {
      return await invoke('plugin:pokeclaw|pickSafFolder')
    } catch (err) {
      console.error('[useModel] pickSafFolder failed:', err)
      return null
    }
  }

  async function listSafModels(): Promise<void> {
    try {
      const result = await invoke<{ models: any[] }>('plugin:pokeclaw|listSafModels')
      console.log('[useModel] listSafModels raw result:', JSON.stringify(result))
      const models = Array.isArray(result?.models) ? result.models : []
      modelList.value = models.filter(m => m != null).map(m => {
        const size = m.sizeBytes ?? 0
        const sizeGb = size / (1024 * 1024 * 1024)
        // Estimate minimum RAM as 2x model size, rounded up to nearest GB
        const minRamGb = Math.ceil(sizeGb * 2)
        return {
          id: m.fileName ?? 'unknown',
          displayName: m.fileName ?? 'Unknown Model',
          url: '',
          fileName: m.fileName ?? '',
          sizeBytes: size,
          minRamGb,
          isDownloaded: true,
          localPath: m.safUri ?? null,
        }
      })
      console.log('[useModel] listSafModels: mapped', modelList.value.length, 'models')
    } catch (err) {
      console.error('[useModel] listSafModels failed:', err)
    }
  }

  /**
   * Pick a model file from the filesystem.
   * Android: uses SAF ACTION_OPEN_DOCUMENT.
   * Desktop: uses Tauri dialog open.
   * Returns the local file path or null.
   */
  async function pickModelFile(): Promise<string | null> {
    try {
      if (isAndroid()) {
        const result = await invoke<{ path: string } | null>('plugin:pokeclaw|pickModelFile')
        return result?.path ?? null
      } else {
        const { open } = await import('@tauri-apps/plugin-dialog')
        const selected = await open({
          multiple: false,
          filters: [{ name: 'LiteRT-LM Model', extensions: ['litertlm'] }],
        })
        return selected
      }
    } catch (err) {
      console.error('[useModel] pickModelFile failed:', err)
      return null
    }
  }

  /**
   * Download a model by ID from the static catalog.
   * Streams progress events via a Channel.
   */
  async function downloadModel(modelId: string): Promise<void> {
    if (isDownloading.value) {
      console.warn('[useModel] downloadModel: already downloading')
      return
    }

    isDownloading.value = true
    downloadProgress.value = { bytesDownloaded: 0, totalBytes: 0, bytesPerSecond: 0 }

    const onProgress = new Channel<DownloadEvent>()
    onProgress.onmessage = (event: DownloadEvent) => {
      switch (event.event) {
        case 'progress':
          downloadProgress.value = {
            bytesDownloaded: event.data.bytesDownloaded,
            totalBytes: event.data.totalBytes,
            bytesPerSecond: event.data.bytesPerSecond,
          }
          break
        case 'complete':
          console.log('[useModel] downloadModel complete:', event.data.modelPath)
          selectedModelPath.value = event.data.modelPath
          isDownloading.value = false
          fetchModels()
          break
        case 'error':
          console.error('[useModel] downloadModel error:', event.data.message)
          isDownloading.value = false
          break
      }
    }

    try {
      await invoke('plugin:pokeclaw|downloadModel', { modelId, onProgress })
    } catch (err) {
      console.error('[useModel] downloadModel invoke failed:', err)
      isDownloading.value = false
    }
  }

  /**
   * Download a model from a URL.
   * Android: Step 1 — opens SAF picker to choose save location.
   *            Step 2 — downloads to SAF URI.
   *            Step 3 — resolves with cached local path.
   * Desktop: downloads to user-chosen directory.
   */
  async function downloadFromUrl(url: string, fileName: string, saveDir?: string): Promise<void> {
    if (isDownloading.value) {
      console.warn('[useModel] downloadFromUrl: already downloading')
      return
    }

    isDownloading.value = true
    downloadProgress.value = { bytesDownloaded: 0, totalBytes: 0, bytesPerSecond: 0 }

    const onProgress = new Channel<DownloadEvent>()
    onProgress.onmessage = (event: DownloadEvent) => {
      switch (event.event) {
        case 'progress':
          downloadProgress.value = {
            bytesDownloaded: event.data.bytesDownloaded,
            totalBytes: event.data.totalBytes,
            bytesPerSecond: event.data.bytesPerSecond,
          }
          break
        case 'complete':
          console.log('[useModel] downloadFromUrl complete:', event.data.modelPath)
          selectedModelPath.value = event.data.modelPath
          isDownloading.value = false
          fetchModels()
          break
        case 'error':
          console.error('[useModel] downloadFromUrl error:', event.data.message)
          isDownloading.value = false
          break
      }
    }

    try {
      if (isAndroid()) {
        const { hasPermission } = await getSafFolderStatus()
        if (hasPermission) {
          // If we have a SAF folder, download directly to it
          await invoke('plugin:pokeclaw|downloadToSafFolder', { url, fileName, onProgress })
        } else {
          // Fallback to picking a single file location (legacy or if user prefers)
          const pickResult = await invoke<{ uri: string; displayName: string } | null>('plugin:pokeclaw|pickSaveLocation', { fileName })
          if (!pickResult || !pickResult.uri) {
            console.log('[useModel] SAF picker cancelled')
            isDownloading.value = false
            return
          }
          await invoke('plugin:pokeclaw|downloadToSaf', { url, safUri: pickResult.uri, onProgress })
        }
      } else {
        await invoke('plugin:pokeclaw|downloadModelFromUrl', { url, saveDir: saveDir || '', onProgress })
      }
    } catch (err) {
      console.error('[useModel] downloadFromUrl failed:', err)
      isDownloading.value = false
    }
  }

  async function startSession(): Promise<{ sessionId: string; backend: string } | null> {
    if (!selectedModelPath.value) {
      console.warn('[useModel] startSession: no model selected')
      return null
    }

    isSessionLoading.value = true
    try {
      const result = await invoke<{ session_id?: string; sessionId?: string; backend?: string }>(
        'plugin:pokeclaw|startSession',
        { modelPath: selectedModelPath.value, preferGpu: preferGpu.value },
      )
      const sessionId = result.session_id ?? result.sessionId ?? ''
      const backend = result.backend ?? 'unknown'
      console.log('[useModel] startSession: session active —', sessionId, backend)

      // Auto-switch provider to local when session starts
      try {
        await invoke('setLlmProviderType', { providerType: 'local' })
        currentProvider.value = 'local'
        console.log('[useModel] startSession: provider auto-switched to local')
      } catch (err) {
        console.warn('[useModel] startSession: failed to auto-switch provider to local:', err)
      }

      return { sessionId, backend }
    } catch (err) {
      console.error('[useModel] startSession failed:', err)
      return null
    } finally {
      isSessionLoading.value = false
    }
  }

  async function stopSession(): Promise<void> {
    try {
      await invoke('plugin:pokeclaw|stopSession')
      console.log('[useModel] stopSession: session stopped')
    } catch (err) {
      console.error('[useModel] stopSession failed:', err)
    }
    selectedModelPath.value = null

    // Auto-switch provider back to openai when session stops
    try {
      await invoke('setLlmProviderType', { providerType: 'openai' })
      currentProvider.value = 'openai'
      console.log('[useModel] stopSession: provider auto-switched to openai')
    } catch (err) {
      console.warn('[useModel] stopSession: failed to auto-switch provider to openai:', err)
    }
  }

  /**
   * Explicitly set the LLM provider type. Updates both backend and local ref.
   */
  async function setProvider(type: 'openai' | 'anthropic' | 'local'): Promise<void> {
    try {
      await invoke('setLlmProviderType', { providerType: type })
      currentProvider.value = type
      console.log('[useModel] setProvider: switched to', type)
    } catch (err) {
      console.error('[useModel] setProvider failed:', err)
    }
  }

  async function getSessionStatus(): Promise<{
    state: 'idle' | 'loading' | 'ready' | 'error'
    modelPath?: string
    backend?: string
    sessionId?: string
  }> {
    try {
      const result = await invoke<Record<string, unknown>>('plugin:pokeclaw|getSessionStatus')
      const state = (result.state as string ?? 'idle').toLowerCase() as 'idle' | 'loading' | 'ready' | 'error'
      return {
        state,
        modelPath: (result.model_path ?? result.modelPath) as string | undefined,
        backend: result.backend as string | undefined,
        sessionId: (result.session_id ?? result.sessionId) as string | undefined,
      }
    } catch (err) {
      console.error('[useModel] getSessionStatus failed:', err)
      return { state: 'idle' }
    }
  }

  return {
    modelList,
    isDownloading,
    downloadProgress,
    selectedModelPath,
    preferGpu,
    currentProvider,
    fetchModels,
    pickModelFile,
    downloadModel,
    downloadFromUrl,
    startSession,
    stopSession,
    setProvider,
    getSessionStatus,
    getSafFolderStatus,
    pickSafFolder,
    listSafModels,
    hasSafPermission,
    safFolderName,
    downloadPercent,
    isSessionLoading,
  }
}
