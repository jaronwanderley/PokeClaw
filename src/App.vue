<script setup lang="ts">
import { ref, watch, nextTick } from 'vue'
import { useChat } from './composables/useChat'
import ChatMessage from './components/ChatMessage.vue'
import MessageInput from './components/MessageInput.vue'

const { messages, sendMessage } = useChat()
const chatRef = ref<HTMLElement | null>(null)

watch(
  () => messages.value.length,
  () => {
    nextTick(() => {
      chatRef.value?.scrollTo({ top: chatRef.value.scrollHeight, behavior: 'smooth' })
    })
  },
)
</script>

<template>
  <div class="app-shell">
    <div class="tb">
      <div class="tb-left">
        <svg viewBox="0 0 24 24" width="20" height="20" fill="#7A6E64" class="tb-icon">
          <path d="M3 18h18v-2H3v2zm0-5h18v-2H3v2zm0-7v2h18V6H3z" />
        </svg>
        <div class="tb-t">Poke<b>Claw</b></div>
      </div>
      <div class="tb-right">
        <div class="tb-b">Local AI</div>
        <svg viewBox="0 0 24 24" width="20" height="20" fill="#7A6E64" class="tb-icon">
          <path d="M19.14 12.94c.04-.3.06-.61.06-.94s-.02-.64-.07-.94l2.03-1.58a.49.49 0 00.12-.61l-1.92-3.32a.49.49 0 00-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94L14.4 2.81a.47.47 0 00-.48-.41h-3.84c-.24 0-.43.17-.47.41L9.25 5.35c-.59.24-1.13.57-1.62.94L5.24 5.33a.49.49 0 00-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58a.49.49 0 00-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.57 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6A3.6 3.6 0 1115.6 12 3.61 3.61 0 0112 15.6z" />
        </svg>
      </div>
    </div>
    <div class="chat" ref="chatRef">
      <ChatMessage v-for="msg in messages" :key="msg.id" :message="msg" />
    </div>
    <div class="ia">
      <MessageInput @send="sendMessage" />
    </div>
  </div>
</template>

<style>
.app-shell {
  display: flex;
  flex-direction: column;
  height: 100%;
  width: 100%;
  max-width: 390px;
  margin: 0 auto;
}

/* Title bar */
.tb {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  background: var(--surface);
  border-bottom: 1px solid var(--div);
}

.tb-left {
  display: flex;
  align-items: center;
  gap: 10px;
}

.tb-t {
  font-size: 18px;
  font-weight: 700;
}

.tb-t b {
  color: var(--accent);
}

.tb-right {
  display: flex;
  align-items: center;
  gap: 10px;
}

.tb-b {
  font-size: 10px;
  padding: 2px 8px;
  border-radius: 10px;
  background: var(--ai);
  color: var(--accent);
  border: 1px solid var(--aib);
}

/* Chat area */
.chat {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow-y: auto;
  padding: 12px 16px;
}

/* Input area */
.ia {
  flex-shrink: 0;
  background: var(--surface);
  border-top: 1px solid var(--div);
}
</style>
