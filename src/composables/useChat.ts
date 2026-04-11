import { ref } from 'vue'

export interface Message {
  id: number
  role: 'user' | 'ai'
  text: string
}

const messages = ref<Message[]>([
  { id: 1, role: 'user', text: 'Hey, what can you do?' },
  { id: 2, role: 'ai', text: 'I can help you control your phone! Send messages, check notifications, read your screen, and automate tasks. Just tell me what you need.' },
  { id: 3, role: 'user', text: 'Can you check my battery level?' },
  { id: 4, role: 'ai', text: 'Sure! Your battery is at 72% and charging. You have about 1 hour and 45 minutes until full.' },
  { id: 5, role: 'user', text: 'Nice! Send "Happy Birthday 🎂" to Mom on WhatsApp' },
  { id: 6, role: 'ai', text: 'Done! I sent "Happy Birthday 🎂" to Mom via WhatsApp. She should receive it shortly.' },
])

let nextId = 7

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
