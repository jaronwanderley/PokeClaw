import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

export interface ToolResult {
  success: boolean
  data?: Record<string, unknown>
  error?: string
}

export interface PermissionStatus {
  accessibility_enabled: boolean
  accessibility_running: boolean
  notification_enabled: boolean
  foreground_service: boolean
}

const screenTree = ref('')
const isLoading = ref(false)
const error = ref<string | null>(null)

export function useAccessibility() {
  /**
   * Fetch the accessibility screen tree from the backend.
   * Desktop returns a mock tree; Android returns the real accessibility node tree.
   */
  async function fetchScreenInfo(): Promise<void> {
    isLoading.value = true
    error.value = null
    try {
      const result = await invoke<ToolResult>('get_screen_info')
      if (result.success && result.data) {
        screenTree.value = (result.data as Record<string, unknown>).tree as string
      } else {
        error.value = result.error ?? 'get_screen_info returned failure with no error message'
        screenTree.value = ''
      }
    } catch (err) {
      error.value = String(err)
      screenTree.value = ''
    } finally {
      isLoading.value = false
    }
  }

  /**
   * Check current permission/service status for accessibility, notifications,
   * and foreground service.
   */
  async function checkPermissions(): Promise<PermissionStatus | null> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('check_permissions')
      if (result.success && result.data) {
        return result.data as unknown as PermissionStatus
      } else {
        error.value = result.error ?? 'check_permissions returned failure'
        return null
      }
    } catch (err) {
      error.value = String(err)
      return null
    }
  }

  /**
   * Find UI nodes matching the given text query.
   */
  async function findNodeInfo(text: string): Promise<Record<string, unknown>[] | null> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('find_node_info', { text })
      if (result.success && result.data) {
        return (result.data as Record<string, unknown>).nodes as Record<string, unknown>[]
      } else {
        error.value = result.error ?? 'find_node_info returned failure'
        return null
      }
    } catch (err) {
      error.value = String(err)
      return null
    }
  }

  /**
   * Get device info for a specific category.
   * Supported categories: battery, wifi, storage, bluetooth, screen, device, time
   */
  async function getDeviceInfo(category: string): Promise<string | null> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('get_device_info', { category })
      if (result.success && result.data) {
        return (result.data as Record<string, unknown>).info as string
      } else {
        error.value = result.error ?? 'get_device_info returned failure'
        return null
      }
    } catch (err) {
      error.value = String(err)
      return null
    }
  }

  return { screenTree, isLoading, error, fetchScreenInfo, checkPermissions, findNodeInfo, getDeviceInfo }
}
