import { ref, readonly } from 'vue'
import { invoke, Channel } from '@tauri-apps/api/core'

/**
 * Discriminated union matching the Rust TaskEvent serde output.
 * Rust uses #[serde(tag = "event", content = "data", rename_all = "camelCase")].
 */
export type TaskEventPayload =
  | { event: 'loopStart'; data: { round: number } }
  | { event: 'toolAction'; data: { tool_name: string } }
  | { event: 'toolResult'; data: { tool_name: string; success: boolean; detail: string } }
  | { event: 'thinking'; data: { content: string } }
  | { event: 'tokenUpdate'; data: { step: number; formatted_tokens: string; formatted_cost: string } }
  | { event: 'completed'; data: { answer: string; model_name: string } }
  | { event: 'failed'; data: { error: string } }
  | { event: 'cancelled' }
  | { event: 'progress'; data: { step: number; description: string } }

export type TaskStatus = 'idle' | 'running' | 'completed' | 'failed' | 'cancelled'

// ── Module-level reactive state (singleton across all consumers) ──────

const taskStatus = ref<TaskStatus>('idle')
const iterationCount = ref(0)
const currentTool = ref<string | null>(null)
const thinkingText = ref('')
const tokenDisplay = ref('')
const costDisplay = ref('')
const lastAnswer = ref<string | null>(null)
const taskError = ref<string | null>(null)
const events = ref<TaskEventPayload[]>([])
const modelName = ref<string | null>(null)

// ── Composable ────────────────────────────────────────────────────────

export function useTask() {
  /**
   * Start a multi-round agent task. Creates a Channel<TaskEvent> and
   * streams progress events from the Rust backend.
   */
  async function startTask(task: string): Promise<void> {
    if (!task.trim()) return
    if (taskStatus.value === 'running') return

    // Reset state for new task
    iterationCount.value = 0
    currentTool.value = null
    thinkingText.value = ''
    tokenDisplay.value = ''
    costDisplay.value = ''
    lastAnswer.value = null
    taskError.value = null
    events.value = []
    modelName.value = null

    const onEvent = new Channel<TaskEventPayload>()

    onEvent.onmessage = (event: TaskEventPayload) => {
      events.value = [...events.value, event]

      switch (event.event) {
        case 'loopStart':
          iterationCount.value = event.data.round
          break

        case 'toolAction':
          currentTool.value = event.data.tool_name
          break

        case 'toolResult':
          currentTool.value = null
          break

        case 'thinking':
          thinkingText.value += event.data.content
          break

        case 'tokenUpdate':
          tokenDisplay.value = event.data.formatted_tokens
          costDisplay.value = event.data.formatted_cost
          break

        case 'completed':
          lastAnswer.value = event.data.answer
          modelName.value = event.data.model_name
          taskStatus.value = 'completed'
          currentTool.value = null
          break

        case 'failed':
          taskError.value = event.data.error
          taskStatus.value = 'failed'
          currentTool.value = null
          break

        case 'cancelled':
          taskStatus.value = 'cancelled'
          currentTool.value = null
          break

        case 'progress':
          // Progress events update thinking text as supplementary info
          break
      }
    }

    taskStatus.value = 'running'

    try {
      await invoke('start_task', { task, onEvent })
    } catch (err) {
      // invoke itself rejected (e.g. already running, no API key)
      taskError.value = String(err)
      taskStatus.value = 'failed'
    }
  }

  /**
   * Cancel the currently running task.
   */
  async function cancelTask(): Promise<void> {
    if (taskStatus.value !== 'running') return

    try {
      await invoke('cancel_task')
      // Status will be updated by the cancelled event from the backend,
      // but set optimistically in case the event doesn't arrive
      taskStatus.value = 'cancelled'
    } catch (err) {
      console.error('[useTask] cancel_task failed:', err)
    }
  }

  /**
   * Reset all task state to idle.
   */
  function resetTask(): void {
    taskStatus.value = 'idle'
    iterationCount.value = 0
    currentTool.value = null
    thinkingText.value = ''
    tokenDisplay.value = ''
    costDisplay.value = ''
    lastAnswer.value = null
    taskError.value = null
    events.value = []
    modelName.value = null
  }

  return {
    taskStatus: readonly(taskStatus),
    iterationCount: readonly(iterationCount),
    currentTool: readonly(currentTool),
    thinkingText: readonly(thinkingText),
    tokenDisplay: readonly(tokenDisplay),
    costDisplay: readonly(costDisplay),
    lastAnswer: readonly(lastAnswer),
    taskError: readonly(taskError),
    events: readonly(events),
    modelName: readonly(modelName),
    startTask,
    cancelTask,
    resetTask,
  }
}
