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

</script>

<template>
  <UModal
    :open="true"
    title="Assistant activity"
    description="Last 90 days"
    :ui="{ content: 'max-w-[640px]' }"
    @update:open="(open) => !open && emit('close')"
  >
    <template #body>
      <p v-if="loading" class="faint">Loading…</p>
      <UAlert v-else-if="error" color="error" variant="soft" icon="i-havre-alert" :title="error" role="alert" />
      <p v-else-if="!items.length" class="faint">Nothing yet. Summaries, briefs, questions and what you confirm show up here.</p>
      <ol v-else class="list">
        <li v-for="a in items" :key="a.id">
          <span class="mono faint at">{{ when(a.at) }}</span>
          <span class="what">
            <span class="action">{{ a.action }}</span>
            <span v-if="a.detail" class="muted detail">{{ a.detail }}</span>
          </span>
          <UBadge color="neutral" variant="outline" size="sm" class="font-mono">{{ who(a.source) }}</UBadge>
        </li>
      </ol>
    </template>
  </UModal>
</template>

<style scoped>
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
@media (max-width: 560px) {
  .list li {
    grid-template-columns: 1fr auto;
  }
  .at {
    grid-column: 1 / -1;
  }
}
</style>
