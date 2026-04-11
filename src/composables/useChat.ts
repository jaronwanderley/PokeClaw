import { ref } from 'vue'

export interface Message {
  id: number
  role: 'user' | 'ai'
  text: string
}

const messages = ref<Message[]>([])

let nextId = 1

export function useChat() {
  function simulateReply(text: string) {
    setTimeout(() => {
      messages.value.push({
        id: nextId++,
        role: 'ai',
        text: `You said: ${text}`,
      })
    }, 800)
  }

  function sendMessage(text: string) {
    if (!text.trim()) return
    const trimmed = text.trim()
    messages.value.push({
      id: nextId++,
      role: 'user',
      text: trimmed,
    })
    simulateReply(trimmed)
  }

  return { messages, sendMessage }
}
