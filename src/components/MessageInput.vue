<script setup lang="ts">
import { ref } from 'vue'

defineProps<{
  disabled?: boolean
}>()

const emit = defineEmits<{
  send: [text: string]
}>()

const inputText = ref('')

function handleSend() {
  const text = inputText.value.trim()
  if (!text) return
  emit('send', text)
  inputText.value = ''
}
</script>

<template>
  <div class="ir">
    <input
      v-model="inputText"
      class="inp"
      type="text"
      :disabled="disabled"
      :placeholder="disabled ? 'Waiting for response...' : 'Type a message...'"
      @keyup.enter="handleSend"
    />
    <button class="bs" :class="{ on: inputText.trim().length > 0 && !disabled }" :disabled="disabled" @click="handleSend">
      <svg viewBox="0 0 24 24" width="14" height="14" fill="white">
        <path d="M2.01 21L23 12 2.01 3 2 10l15 2-15 2z" />
      </svg>
    </button>
  </div>
</template>

<style scoped>
.ir {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px 8px;
}

.inp {
  flex: 1;
  padding: 9px 14px;
  border-radius: 20px;
  background: var(--bg);
  border: 1px solid var(--border);
  color: var(--t1);
  font-size: 14px;
  outline: none;
  transition: all 0.2s;
}

.inp:focus {
  border-color: var(--tbrd);
  box-shadow: 0 0 0 2px var(--tglow);
}

.inp::placeholder {
  color: var(--t3);
}

.inp:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.bs {
  width: 34px;
  height: 34px;
  border-radius: 50%;
  border: none;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  flex-shrink: 0;
  transition: all 0.15s;
  background: var(--bg);
  opacity: 0.35;
}

.bs.on {
  background: var(--user);
  opacity: 1;
}

.bs:disabled {
  cursor: not-allowed;
  opacity: 0.25;
}
</style>
