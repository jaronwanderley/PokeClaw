<script setup lang="ts">
import type { Message } from '../composables/useChat'

defineProps<{
  message: Message
}>()
</script>

<template>
  <div class="msg" :class="message.role === 'user' ? 'msg-u' : 'msg-a'">
    <span v-if="message.role === 'ai'" class="msg-avatar">🤖</span>
    <div class="msg-bubble">{{ message.text }}</div>
  </div>
</template>

<style scoped>
.msg {
  display: flex;
  align-items: flex-end;
  gap: 6px;
  max-width: 80%;
  margin-bottom: 4px;
}

.msg-u {
  align-self: flex-end;
  flex-direction: row-reverse;
}

.msg-a {
  align-self: flex-start;
}

.msg-avatar {
  font-size: 16px;
  flex-shrink: 0;
  width: 24px;
  height: 24px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.msg-bubble {
  padding: 10px 14px;
  border-radius: 16px;
  font-size: 13px;
  line-height: 1.5;
  word-wrap: break-word;
}

.msg-u .msg-bubble {
  background: var(--user);
  color: var(--utext);
  border-bottom-right-radius: 4px;
}

.msg-a .msg-bubble {
  background: var(--ai);
  color: var(--ait);
  border: 1px solid var(--aib);
  border-bottom-left-radius: 4px;
}
</style>
