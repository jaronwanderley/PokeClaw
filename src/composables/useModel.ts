import { ref } from 'vue'
import { invoke, Channel } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'

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

// ---------------------------------------------------------------------------
// Composable
// ---------------------------------------------------------------------------

export function useModel() {
  /**
   * Fetch the model catalog from the backend.
   * Kotlin returns { models: [...] }; Rust desktop returns [...] directly.
   */
  async function fetchModels(): Promise<void> {
    try {
      const result = await invoke<ModelInfo[] | { models: ModelInfo[] }>('list_models')
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

  /**
   * Open a native file picker filtered to .litertlm files.
   * Returns the selected path or null.
   */
  async function pickModelFile(): Promise<string | null> {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: 'LiteRT-LM Model', extensions: ['litertlm'] }],
      })
      if (selected) {
        console.log('[useModel] pickModelFile:', selected)
      }
      return selected
    } catch (err) {
      console.error('[useModel] pickModelFile failed:', err)
      return null
    }
  }

  /**
   * Download a model by ID. Streams progress events via a Channel.
   * On completion, updates selectedModelPath and refreshes the model list.
   */
  async function downloadModel(modelId: string): Promise<void> {
    if (isDownloading.value) {
      console.warn('[useModel] downloadModel: already downloading, ignoring')
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
          // Refresh model list to reflect new download status
          fetchModels()
          break
        case 'error':
          console.error('[useModel] downloadModel error:', event.data.message)
          isDownloading.value = false
          break
      }
    }

    try {
      await invoke('download_model', { modelId, onProgress })
    } catch (err) {
      console.error('[useModel] downloadModel invoke failed:', err)
      isDownloading.value = false
    }
  }

  /**
   * Start an inference session with the selected model.
   * Returns { session_id, backend } on success.
   */
  async function startSession(): Promise<{ sessionId: string; backend: string } | null> {
    if (!selectedModelPath.value) {
      console.warn('[useModel] startSession: no model selected')
      return null
    }

    try {
      // Rust returns { session_id, backend }; Kotlin returns { session_id, backend }
      const result = await invoke<{ session_id?: string; sessionId?: string; backend?: string }>(
        'start_session',
        { modelPath: selectedModelPath.value, preferGpu: preferGpu.value },
      )
      const sessionId = result.session_id ?? result.sessionId ?? ''
      const backend = result.backend ?? 'unknown'
      console.log('[useModel] startSession: session active —', sessionId, backend)
      return { sessionId, backend }
    } catch (err) {
      console.error('[useModel] startSession failed:', err)
      return null
    }
  }

  /**
   * Stop the active inference session.
   */
  async function stopSession(): Promise<void> {
    try {
      await invoke('stop_session')
      console.log('[useModel] stopSession: session stopped')
    } catch (err) {
      console.error('[useModel] stopSession failed:', err)
    }
    selectedModelPath.value = null
  }

  /**
   * Fetch current session status from the backend.
   * Rust returns SessionStatus enum directly; Kotlin returns { state, ... }.
   */
  async function getSessionStatus(): Promise<{
    state: 'idle' | 'loading' | 'ready' | 'error'
    modelPath?: string
    backend?: string
    sessionId?: string
  }> {
    try {
      const result = await invoke<Record<string, unknown>>('get_session_status')
      // Rust serde: { state: "Ready", session_id: "...", backend: "..." }
      // Kotlin: { state: "ready", model_path: "...", backend: "...", session_id: "..." }
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
    fetchModels,
    pickModelFile,
    downloadModel,
    startSession,
    stopSession,
    getSessionStatus,
  }
}
