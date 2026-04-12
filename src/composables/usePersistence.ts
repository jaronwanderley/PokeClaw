import { invoke } from '@tauri-apps/api/core'

/**
 * Persisted chat message record — mirrors the Rust ChatMessageRecord struct.
 */
export interface ChatMessageRecord {
  id: number
  sessionId: string
  role: string
  content: string
  metadata: string | null
  createdAt: string
}

const SESSION_KEY = 'pokeclaw_session_id'

/**
 * Get or create a session ID stored in localStorage.
 * Uses ISO date format (e.g. "2026-04-12"). A new session ID is generated
 * each calendar day — restarting the app on the same day resumes the session.
 */
export function getSessionId(): string {
  let sessionId = localStorage.getItem(SESSION_KEY)
  if (!sessionId) {
    sessionId = new Date().toISOString().split('T')[0] // e.g. "2026-04-12"
    localStorage.setItem(SESSION_KEY, sessionId)
  }
  return sessionId
}

/**
 * Persist a chat message to the SQLite database.
 * Returns the inserted row ID.
 */
export async function saveChatMessage(
  sessionId: string,
  role: string,
  content: string,
  metadata?: string,
): Promise<number> {
  return invoke<number>('save_chat_message', {
    sessionId,
    role,
    content,
    metadata: metadata ?? null,
  })
}

/**
 * Load all chat messages for a session, ordered by created_at ascending.
 */
export async function loadChatHistory(
  sessionId: string,
): Promise<ChatMessageRecord[]> {
  return invoke<ChatMessageRecord[]>('load_chat_history', { sessionId })
}
