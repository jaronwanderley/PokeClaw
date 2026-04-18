import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

export interface ToolResult {
  success: boolean
  data?: Record<string, unknown>
  error?: string
}

export interface PermissionStatus {
  accessibilityEnabled: boolean
  accessibilityRunning: boolean
  notificationEnabled: boolean
  foregroundService: boolean
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
      const result = await invoke<ToolResult>('plugin:pokeclaw|getScreenInfo')
      if (result.success && result.data) {
        screenTree.value = (result.data as Record<string, unknown>).tree as string
      } else {
        error.value = result.error ?? 'getScreenInfo returned failure'
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
      const result = await invoke<ToolResult>('plugin:pokeclaw|checkAppPermissions')
      if (result.success && result.data) {
        return result.data as unknown as PermissionStatus
      } else {
        error.value = result.error ?? 'checkPermissions returned failure'
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
      const result = await invoke<ToolResult>('plugin:pokeclaw|findNodeInfo', { text })
      if (result.success && result.data) {
        return (result.data as Record<string, unknown>).nodes as Record<string, unknown>[]
      } else {
        error.value = result.error ?? 'findNodeInfo returned failure'
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
      const result = await invoke<ToolResult>('plugin:pokeclaw|getDeviceInfo', { category })
      if (result.success && result.data) {
        return (result.data as Record<string, unknown>).info as string
      } else {
        error.value = result.error ?? 'getDeviceInfo returned failure'
        return null
      }
    } catch (err) {
      error.value = String(err)
      return null
    }
  }

  // -----------------------------------------------------------------
  // Gesture tool methods
  // -----------------------------------------------------------------

  /**
   * Tap at the given screen coordinates.
   */
  async function tap(x: number, y: number): Promise<ToolResult> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('plugin:pokeclaw|tap', { x, y })
      if (!result.success) {
        error.value = result.error ?? 'tap returned failure'
      }
      return result
    } catch (err) {
      error.value = String(err)
      return { success: false, error: String(err) }
    }
  }

  /**
   * Swipe from one point to another.
   */
  async function swipe(
    startX: number,
    startY: number,
    endX: number,
    endY: number,
    durationMs?: number,
  ): Promise<ToolResult> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('plugin:pokeclaw|swipe', {
        startX,
        startY,
        endX,
        endY,
        durationMs,
      })
      if (!result.success) {
        error.value = result.error ?? 'swipe returned failure'
      }
      return result
    } catch (err) {
      error.value = String(err)
      return { success: false, error: String(err) }
    }
  }

  /**
   * Long press at the given coordinates.
   */
  async function longPress(x: number, y: number, durationMs?: number): Promise<ToolResult> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('plugin:pokeclaw|longPress', {
        x,
        y,
        durationMs,
      })
      if (!result.success) {
        error.value = result.error ?? 'longPress returned failure'
      }
      return result
    } catch (err) {
      error.value = String(err)
      return { success: false, error: String(err) }
    }
  }

  /**
   * Tap an accessibility node by its node ID (e.g., "n3").
   */
  async function tapNode(nodeId: string): Promise<ToolResult> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('plugin:pokeclaw|tapNode', { nodeId })
      if (!result.success) {
        error.value = result.error ?? 'tapNode returned failure'
      }
      return result
    } catch (err) {
      error.value = String(err)
      return { success: false, error: String(err) }
    }
  }

  /**
   * Input text into a node (uses clipboard fallback on Android).
   */
  async function inputText(
    text: string,
    nodeId?: string,
    clearFirst?: boolean,
  ): Promise<ToolResult> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('plugin:pokeclaw|inputText', {
        text,
        nodeId,
        clearFirst,
      })
      if (!result.success) {
        error.value = result.error ?? 'inputText returned failure'
      }
      return result
    } catch (err) {
      error.value = String(err)
      return { success: false, error: String(err) }
    }
  }

  /**
   * Scroll to find an element matching the given text.
   */
  async function scrollToFind(
    text: string,
    direction?: string,
    maxScrolls?: number,
  ): Promise<ToolResult> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('plugin:pokeclaw|scrollToFind', {
        text,
        direction,
        maxScrolls,
      })
      if (!result.success) {
        error.value = result.error ?? 'scrollToFind returned failure'
      }
      return result
    } catch (err) {
      error.value = String(err)
      return { success: false, error: String(err) }
    }
  }

  /**
   * Find an element matching the given text and tap it.
   */
  async function findAndTap(
    text: string,
    direction?: string,
    maxScrolls?: number,
  ): Promise<ToolResult> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('plugin:pokeclaw|findAndTap', {
        text,
        direction,
        maxScrolls,
      })
      if (!result.success) {
        error.value = result.error ?? 'findAndTap returned failure'
      }
      return result
    } catch (err) {
      error.value = String(err)
      return { success: false, error: String(err) }
    }
  }

  // -----------------------------------------------------------------
  // Notification / navigation / compound tool methods (S03)
  // -----------------------------------------------------------------

  /**
   * Get active notifications from PokeNotificationListener.
   * Returns array of notification objects with package_name, key, post_time, etc.
   */
  async function getNotifications(): Promise<Record<string, unknown>[] | null> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('plugin:pokeclaw|getNotifications')
      if (result.success && result.data) {
        return (result.data as Record<string, unknown>).notifications as Record<string, unknown>[]
      } else {
        error.value = result.error ?? 'getNotifications returned failure'
        return null
      }
    } catch (err) {
      error.value = String(err)
      return null
    }
  }

  /**
   * Open an app by name or package name.
   * Supports well-known names like "whatsapp", "telegram", etc.
   */
  async function openApp(appName: string): Promise<ToolResult> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('plugin:pokeclaw|openApp', { appName })
      if (!result.success) {
        error.value = result.error ?? 'openApp returned failure'
      }
      return result
    } catch (err) {
      error.value = String(err)
      return { success: false, error: String(err) }
    }
  }

  /**
   * Press a system key action.
   * Supported: back, home, recent_apps, notifications, collapse_notifications, lock_screen, unlock_screen
   */
  async function systemKey(action: string): Promise<ToolResult> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('plugin:pokeclaw|systemKey', { action })
      if (!result.success) {
        error.value = result.error ?? 'systemKey returned failure'
      }
      return result
    } catch (err) {
      error.value = String(err)
      return { success: false, error: String(err) }
    }
  }

  /**
   * Send a chat message to a contact via a messaging app.
   * Compound flow: open app → find contact → type message → send.
   */
  async function sendChatMessage(
    app: string,
    contact: string,
    message: string,
  ): Promise<ToolResult> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('plugin:pokeclaw|sendChatMessage', { app, contact, message })
      if (!result.success) {
        error.value = result.error ?? 'sendChatMessage returned failure'
      }
      return result
    } catch (err) {
      error.value = String(err)
      return { success: false, error: String(err) }
    }
  }

  /**
   * Take a screenshot and save to file.
   * Returns file path, width, and height of the captured image.
   */
  async function takeScreenshot(filePath?: string): Promise<ToolResult> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('plugin:pokeclaw|takeScreenshot', { filePath })
      if (!result.success) {
        error.value = result.error ?? 'takeScreenshot returned failure'
      }
      return result
    } catch (err) {
      error.value = String(err)
      return { success: false, error: String(err) }
    }
  }

  /**
   * Get or set the system clipboard content.
   * @param action "get" or "set"
   * @param text Text to set (required when action is "set")
   */
  async function clipboard(action: string, text?: string): Promise<ToolResult> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('plugin:pokeclaw|clipboard', { action, text })
      if (!result.success) {
        error.value = result.error ?? 'clipboard returned failure'
      }
      return result
    } catch (err) {
      error.value = String(err)
      return { success: false, error: String(err) }
    }
  }

  /**
   * Get list of installed apps. Optionally filter by app/package name.
   */
  async function getInstalledApps(filter?: string): Promise<Record<string, unknown>[] | null> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('plugin:pokeclaw|getInstalledApps', { filter })
      if (result.success && result.data) {
        return (result.data as Record<string, unknown>).apps as Record<string, unknown>[]
      } else {
        error.value = result.error ?? 'getInstalledApps returned failure'
        return null
      }
    } catch (err) {
      error.value = String(err)
      return null
    }
  }

  /**
   * Open a system permission settings page.
   * @param target Which settings page: "accessibility", "notification", or "foreground_service"
   */
  async function openPermissionSettings(target: string): Promise<ToolResult> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('plugin:pokeclaw|openPermissionSettings', { target })
      if (!result.success) {
        error.value = result.error ?? 'openPermissionSettings returned failure'
      }
      return result
    } catch (err) {
      error.value = String(err)
      return { success: false, error: String(err) }
    }
  }

  /**
   * Open the dialer with a phone number or contact name.
   */
  async function makeCall(target: string): Promise<ToolResult> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('plugin:pokeclaw|makeCall', { target })
      if (!result.success) {
        error.value = result.error ?? 'makeCall returned failure'
      }
      return result
    } catch (err) {
      error.value = String(err)
      return { success: false, error: String(err) }
    }
  }

  return {
    screenTree,
    isLoading,
    error,
    fetchScreenInfo,
    checkPermissions,
    findNodeInfo,
    getDeviceInfo,
    tap,
    swipe,
    longPress,
    tapNode,
    inputText,
    scrollToFind,
    findAndTap,
    getNotifications,
    openApp,
    systemKey,
    sendChatMessage,
    takeScreenshot,
    clipboard,
    getInstalledApps,
    makeCall,
    openPermissionSettings,
  }
}
