import { ref } from 'vue'

export interface Message {
  id: number
  role: 'user' | 'ai'
  text: string
}

const messages = ref<Message[]>([])

let nextId = 1

export function useChat() {
  function sendMessage(text: string) {
    if (!text.trim()) return
    messages.value.push({
      id: nextId++,
      role: 'user',
      text: text.trim(),
    })
  }

  return { messages, sendMessage }
}
