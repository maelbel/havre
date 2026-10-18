<script setup lang="ts">
import type { AiActivity } from '~/composables/useAi'

// The activity log: what the assistant did (generations, tool calls) and
// what you confirmed. Kept 90 days on the server.
const emit = defineEmits<{ close: [] }>()
const items = ref<AiActivity[]>([])
const error = ref<string | null>(null)
const loading = ref(true)

onMounted(async () => {
  try {
    items.value = await fetchAiActivity(200)
  } catch (err) {
    error.value = apiErrorMessage(err)
  } finally {
    loading.value = false
  }
})

function when(at: number) {
  return new Date(at * 1000).toLocaleString(undefined, {
    weekday: 'short',
    day: 'numeric',
    month: 'short',
    hour: '2-digit',
    minute: '2-digit',
  })
}

/** "mcp:Claude Code" → "Claude Code (MCP)". */
function who(source: string) {
  if (source.startsWith('mcp:')) return `${source.slice(4)} · MCP`
  return source === 'havre' ? 'Havre' : source
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') emit('close')
}
</script>

<template>
  <div class="backdrop" @click.self="emit('close')" @keydown="onKeydown">
    <div class="sheet" role="dialog" aria-label="Assistant activity" tabindex="-1">
      <header>
        <div>
          <div class="eyebrow">Last 90 days</div>
          <h3>Assistant activity</h3>
        </div>
        <button type="button" class="btn small" aria-label="Close" @click="emit('close')"><HIcon name="close" :size="14" /></button>
      </header>
      <p v-if="loading" class="faint">Loading…</p>
      <p v-else-if="error" class="error"><HIcon name="alert" :size="14" /> {{ error }}</p>
      <p v-else-if="!items.length" class="faint">Nothing yet. Summaries, briefs, questions and what you confirm show up here.</p>
      <ol v-else class="list">
        <li v-for="a in items" :key="a.id">
          <span class="mono faint at">{{ when(a.at) }}</span>
          <span class="what">
            <span class="action">{{ a.action }}</span>
            <span v-if="a.detail" class="muted detail">{{ a.detail }}</span>
          </span>
          <span class="pill mono">{{ who(a.source) }}</span>
        </li>
      </ol>
    </div>
  </div>
</template>

<style scoped>
.backdrop {
  position: fixed;
  inset: 0;
  z-index: 60;
  display: grid;
  place-items: center;
  padding: 16px;
  background: color-mix(in srgb, #000 45%, transparent);
}
.sheet {
  width: min(640px, 100%);
  max-height: min(80vh, 720px);
  overflow: auto;
  padding: 24px;
  border: 1px solid var(--line);
  border-radius: 16px;
  background: var(--raise);
  box-shadow: 0 24px 60px rgb(0 0 0 / 0.35);
}
header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  margin-bottom: 16px;
}
h3 {
  margin: 4px 0 0;
  font-family: var(--font-serif);
  font-weight: 400;
  font-size: 24px;
}
.eyebrow {
  font-family: var(--font-mono);
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--faint);
}
.list {
  list-style: none;
  margin: 0;
  padding: 0;
}
.list li {
  display: grid;
  grid-template-columns: 150px 1fr auto;
  gap: 12px;
  align-items: baseline;
  padding: 10px 0;
  border-top: 1px solid var(--line);
  font-size: 13px;
}
.at {
  font-size: 11.5px;
}
.what {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.detail {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.pill {
  font-size: 10.5px;
  padding: 2px 8px;
  border: 1px solid var(--line);
  border-radius: 999px;
  color: var(--muted);
}
@media (max-width: 560px) {
  .list li {
    grid-template-columns: 1fr auto;
  }
  .at {
    grid-column: 1 / -1;
  }
}
</style>
