import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

export interface Message {
  id: number
  role: 'user' | 'ai'
  text: string
}

const messages = ref<Message[]>([])

let nextId = 1

export function useChat() {
  async function sendToAgent(text: string) {
    try {
      const response = await invoke<string>('chat', { message: text })
      messages.value.push({
        id: nextId++,
        role: 'ai',
        text: response,
      })
    } catch (err) {
      messages.value.push({
        id: nextId++,
        role: 'ai',
        text: `[IPC Error] ${err}`,
      })
    }
  }

  function sendMessage(text: string) {
    if (!text.trim()) return
    const trimmed = text.trim()
    messages.value.push({
      id: nextId++,
      role: 'user',
      text: trimmed,
    })
    sendToAgent(trimmed)
  }

  return { messages, sendMessage }
}
