<script setup lang="ts">
import type { SyncStatus } from '~/composables/useMail'

// One line at the bottom of the folder list: is my mail up to date?
const props = defineProps<{ status: SyncStatus | null }>()

// Re-render "2 min ago" every 30 s.
const now = ref(Date.now())
let timer: ReturnType<typeof setInterval> | undefined
onMounted(() => (timer = setInterval(() => (now.value = Date.now()), 30_000)))
onBeforeUnmount(() => clearInterval(timer))

function ago(seconds: number) {
  const diff = Math.max(0, Math.round(now.value / 1000 - seconds))
  if (diff < 45) return 'just now'
  if (diff < 3600) return `${Math.round(diff / 60)} min ago`
  return `${Math.round(diff / 3600)} h ago`
}

const view = computed(() => {
  const s = props.status
  if (!s || s.state === 'starting') return { tone: 'busy', text: 'Connecting to your mailbox…' }
  if (s.state === 'error') return { tone: 'error', text: 'Can’t reach the mail server — retrying', title: s.error ?? '' }
  if (s.state === 'syncing') return { tone: 'busy', text: s.backfilling ? 'Downloading older messages…' : 'Syncing…' }
  if (s.backfilling) return { tone: 'busy', text: 'Downloading older messages…' }
  const when = s.last_success ? ago(s.last_success) : ''
  return { tone: 'ok', text: when ? `Synced ${when}` : 'Up to date', title: s.push ? 'New mail arrives instantly' : 'Checked every 5 minutes' }
})
</script>

<template>
  <div class="sync" :class="view.tone" :title="view.title" role="status">
    <span v-if="view.tone === 'busy'" class="spin" aria-hidden="true" />
    <span v-else class="dot" aria-hidden="true" />
    <span class="text">{{ view.text }}</span>
  </div>
</template>

<style scoped>
.sync {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 8px 0;
  border-top: 1px solid var(--line);
  font-size: 12px;
  color: var(--faint);
}
.text {
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.dot {
  flex: 0 0 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--ok);
}
.error .dot {
  background: var(--danger);
}
.error {
  color: var(--danger);
}
.spin {
  flex: 0 0 10px;
  height: 10px;
  border-radius: 50%;
  border: 1.5px solid color-mix(in oklab, var(--accent) 25%, transparent);
  border-top-color: var(--accent);
  animation: spin 0.8s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
