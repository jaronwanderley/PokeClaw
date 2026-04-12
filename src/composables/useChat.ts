import { ref } from 'vue'
import { invoke, Channel } from '@tauri-apps/api/core'
import { saveChatMessage, loadChatHistory, getSessionId } from './usePersistence'

export interface Message {
  id: number
  role: 'user' | 'ai'
  text: string
}

type StreamEvent =
  | { event: 'token_batch'; data: { tokens: string; batch_index: number } }
  | { event: 'complete'; data: { full_text: string; token_count: number } }
  | { event: 'error'; data: { message: string } }

const messages = ref<Message[]>([])
const streamingText = ref('')
const isStreaming = ref(false)
const sessionStatus = ref<'idle' | 'loading' | 'ready' | 'error'>('idle')

let nextId = 1
let chatInitialized = false

export function useChat() {
  /**
   * Load chat history from the database on first init.
   * Maps ChatMessageRecord[] to Message[] and populates the messages ref.
   * Safe to call multiple times — only loads once.
   */
  async function initChat(): Promise<void> {
    if (chatInitialized) return
    chatInitialized = true
    try {
      const sessionId = getSessionId()
      const records = await loadChatHistory(sessionId)
      if (records.length > 0) {
        messages.value = records.map((r) => ({
          id: nextId++,
          role: r.role === 'user' ? 'user' as const : 'ai' as const,
          text: r.content,
        }))
        console.log(`[useChat] Loaded ${records.length} messages from DB for session ${sessionId}`)
      }
    } catch (err) {
      console.error('[useChat] Failed to load chat history:', err)
    }
  }

  // Auto-initialize on first useChat() call
  initChat()
  /**
   * Fetch the current session status from the backend and update the ref.
   * Rust returns SessionStatus serde enum; Kotlin returns { state, ... }.
   */
  async function updateSessionStatus(): Promise<void> {
    try {
      const result = await invoke<Record<string, unknown>>('get_session_status')
      const state = (result.state as string ?? 'idle').toLowerCase() as 'idle' | 'loading' | 'ready' | 'error'
      sessionStatus.value = state
      console.log('[useChat] updateSessionStatus:', state)
    } catch (err) {
      console.error('[useChat] updateSessionStatus failed:', err)
      sessionStatus.value = 'idle'
    }
  }

  /**
   * Set the session status directly (used by useModel after start/stop).
   */
  function setSessionStatus(status: 'idle' | 'loading' | 'ready' | 'error'): void {
    sessionStatus.value = status
  }

  async function sendStreamingMessage(text: string) {
    const sessionId = getSessionId()
    const onEvent = new Channel<StreamEvent>()

    onEvent.onmessage = (event: StreamEvent) => {
      switch (event.event) {
        case 'token_batch':
          streamingText.value += event.data.tokens
          break
        case 'complete':
          messages.value.push({
            id: nextId++,
            role: 'ai',
            text: event.data.full_text,
          })
          streamingText.value = ''
          isStreaming.value = false
          // Persist AI response to database
          saveChatMessage(sessionId, 'ai', event.data.full_text).catch((err) => {
            console.error('[useChat] Failed to persist AI message:', err)
          })
          break
        case 'error':
          messages.value.push({
            id: nextId++,
            role: 'ai',
            text: `[IPC Error] ${event.data.message}`,
          })
          streamingText.value = ''
          isStreaming.value = false
          break
      }
    }

    try {
      await invoke('send_message', { message: text, onEvent })
    } catch (err) {
      messages.value.push({
        id: nextId++,
        role: 'ai',
        text: `[IPC Error] ${err}`,
      })
      streamingText.value = ''
      isStreaming.value = false
    }
  }

  function sendMessage(text: string) {
    if (!text.trim()) return

    // Guard: only send when session is ready
    if (sessionStatus.value !== 'ready') {
      messages.value.push({
        id: nextId++,
        role: 'ai',
        text: '[Error] No active session. Please load a model first.',
      })
      return
    }

    const trimmed = text.trim()
    messages.value.push({
      id: nextId++,
      role: 'user',
      text: trimmed,
    })
    isStreaming.value = true
    streamingText.value = ''

    // Persist user message to database
    const sessionId = getSessionId()
    saveChatMessage(sessionId, 'user', trimmed).catch((err) => {
      console.error('[useChat] Failed to persist user message:', err)
    })

    sendStreamingMessage(trimmed)
  }

  return { messages, streamingText, isStreaming, sessionStatus, sendMessage, updateSessionStatus, setSessionStatus, initChat }
}
