<script setup lang="ts">
import { ref, watch } from 'vue'
import { useAccessibility } from '../composables/useAccessibility'
import type { PermissionStatus } from '../composables/useAccessibility'

const props = defineProps<{
  visible: boolean
}>()

const emit = defineEmits<{
  close: []
}>()

const { checkPermissions, openPermissionSettings } = useAccessibility()

const permissions = ref<PermissionStatus | null>(null)
const loadingTarget = ref<string | null>(null)

const cards = [
  { key: 'accessibilityEnabled' as const, target: 'accessibility', label: 'Accessibility Service', desc: 'Read screen content and perform gestures' },
  { key: 'notificationEnabled' as const, target: 'notification', label: 'Notification Access', desc: 'Read and dismiss notifications' },
  { key: 'foregroundService' as const, target: 'foreground_service', label: 'Foreground Service', desc: 'Keep the agent running in background' },
]

watch(() => props.visible, async (visible) => {
  if (visible) {
    permissions.value = await checkPermissions()
  }
})

async function handleOpenSettings(target: string) {
  loadingTarget.value = target
  await openPermissionSettings(target)
  loadingTarget.value = null
  // Re-check permissions after returning (user may have enabled something)
  permissions.value = await checkPermissions()
}

function isEnabled(key: keyof PermissionStatus): boolean {
  return permissions.value?.[key] ?? false
}
</script>

<template>
  <div v-if="visible" class="perm-overlay" @click.self="emit('close')">
    <div class="perm-panel">
      <div class="perm-header">
        <div class="perm-title">Permissions</div>
        <button class="close-btn" @click="emit('close')">
          <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor">
            <path d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z" />
          </svg>
        </button>
      </div>

      <div class="perm-body">
        <div v-for="card in cards" :key="card.key" class="perm-card">
          <div class="perm-card-top">
            <span class="perm-icon" :class="isEnabled(card.key) ? 'perm-on' : 'perm-off'">
              {{ isEnabled(card.key) ? '✅' : '❌' }}
            </span>
            <div class="perm-card-info">
              <div class="perm-label">{{ card.label }}</div>
              <div class="perm-desc">{{ card.desc }}</div>
            </div>
          </div>
          <button
            class="perm-btn"
            :disabled="loadingTarget === card.target"
            @click="handleOpenSettings(card.target)"
          >
            {{ loadingTarget === card.target ? 'Opening...' : 'Open Settings' }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.perm-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  z-index: 100;
  display: flex;
  align-items: flex-end;
  justify-content: center;
}

.perm-panel {
  width: 100%;
  max-width: 390px;
  max-height: 70vh;
  background: var(--surface);
  border-radius: 16px 16px 0 0;
  overflow-y: auto;
  padding: 20px 16px 32px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.perm-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.perm-title {
  font-size: 18px;
  font-weight: 700;
  color: var(--t1);
}

.close-btn {
  width: 32px;
  height: 32px;
  border-radius: 50%;
  border: none;
  background: var(--bg);
  color: var(--t2);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all 0.15s;
}

.close-btn:hover {
  background: var(--ai);
}

.perm-body {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.perm-card {
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.perm-card-top {
  display: flex;
  align-items: flex-start;
  gap: 10px;
}

.perm-icon {
  font-size: 18px;
  flex-shrink: 0;
  line-height: 1;
}

.perm-on {
  color: #6fcf6f;
}

.perm-off {
  color: #e57373;
}

.perm-card-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.perm-label {
  font-size: 14px;
  font-weight: 600;
  color: var(--t1);
}

.perm-desc {
  font-size: 12px;
  color: var(--t2);
}

.perm-btn {
  align-self: flex-end;
  font-size: 12px;
  padding: 6px 12px;
  border-radius: 6px;
  border: 1px solid var(--border);
  background: var(--ai);
  color: var(--t1);
  cursor: pointer;
  transition: all 0.15s;
}

.perm-btn:hover {
  opacity: 0.9;
}

.perm-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
