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

  // -----------------------------------------------------------------
  // Gesture tool methods
  // -----------------------------------------------------------------

  /**
   * Tap at the given screen coordinates.
   */
  async function tap(x: number, y: number): Promise<ToolResult> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('tap', { x, y })
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
      const result = await invoke<ToolResult>('swipe', {
        start_x: startX,
        start_y: startY,
        end_x: endX,
        end_y: endY,
        duration_ms: durationMs,
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
      const result = await invoke<ToolResult>('long_press', {
        x,
        y,
        duration_ms: durationMs,
      })
      if (!result.success) {
        error.value = result.error ?? 'long_press returned failure'
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
      const result = await invoke<ToolResult>('tap_node', { node_id: nodeId })
      if (!result.success) {
        error.value = result.error ?? 'tap_node returned failure'
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
      const result = await invoke<ToolResult>('input_text', {
        text,
        node_id: nodeId,
        clear_first: clearFirst,
      })
      if (!result.success) {
        error.value = result.error ?? 'input_text returned failure'
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
      const result = await invoke<ToolResult>('scroll_to_find', {
        text,
        direction,
        max_scrolls: maxScrolls,
      })
      if (!result.success) {
        error.value = result.error ?? 'scroll_to_find returned failure'
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
      const result = await invoke<ToolResult>('find_and_tap', {
        text,
        direction,
        max_scrolls: maxScrolls,
      })
      if (!result.success) {
        error.value = result.error ?? 'find_and_tap returned failure'
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
      const result = await invoke<ToolResult>('get_notifications')
      if (result.success && result.data) {
        return (result.data as Record<string, unknown>).notifications as Record<string, unknown>[]
      } else {
        error.value = result.error ?? 'get_notifications returned failure'
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
      const result = await invoke<ToolResult>('open_app', { app_name: appName })
      if (!result.success) {
        error.value = result.error ?? 'open_app returned failure'
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
      const result = await invoke<ToolResult>('system_key', { action })
      if (!result.success) {
        error.value = result.error ?? 'system_key returned failure'
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
      const result = await invoke<ToolResult>('send_chat_message', { app, contact, message })
      if (!result.success) {
        error.value = result.error ?? 'send_chat_message returned failure'
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
      const result = await invoke<ToolResult>('take_screenshot', { file_path: filePath })
      if (!result.success) {
        error.value = result.error ?? 'take_screenshot returned failure'
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
      const result = await invoke<ToolResult>('clipboard', { action, text })
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
      const result = await invoke<ToolResult>('get_installed_apps', { filter })
      if (result.success && result.data) {
        return (result.data as Record<string, unknown>).apps as Record<string, unknown>[]
      } else {
        error.value = result.error ?? 'get_installed_apps returned failure'
        return null
      }
    } catch (err) {
      error.value = String(err)
      return null
    }
  }

  /**
   * Open the dialer with a phone number or contact name.
   */
  async function makeCall(target: string): Promise<ToolResult> {
    error.value = null
    try {
      const result = await invoke<ToolResult>('make_call', { target })
      if (!result.success) {
        error.value = result.error ?? 'make_call returned failure'
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
  }
}
