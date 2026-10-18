<script setup lang="ts">
import type { TaskSource } from '~/composables/useTasks'

// Settings › Your apps › Tasks: which accounts sync their tasks (Microsoft
// To Do, Google Tasks, CalDAV task lists), and where new tasks go.
const toast = useToasts()
const sources = ref<TaskSource[]>([])
const loaded = ref(false)
const busy = ref<string | null>(null)
const { lists, refresh: refreshLists } = useTaskLists()

async function load() {
  try {
    sources.value = await fetchTaskSources()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    loaded.value = true
  }
}
onMounted(() => {
  load()
  refreshLists()
})
useMailEvents((event) => {
  if (event.type === 'accounts' || event.type === 'tasks') load()
})

async function toggle(s: TaskSource) {
  busy.value = s.key
  try {
    sources.value = await setTaskSource(s.key, !s.enabled)
    await refreshLists()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    busy.value = null
  }
}

const KIND: Record<TaskSource['kind'], string> = {
  microsoft: 'Microsoft To Do',
  google: 'Google Tasks',
  caldav: 'Task lists (VTODO)',
}
function meta(s: TaskSource) {
  if (!s.allowed) return 'Reconnect the account above to allow tasks'
  if (!s.enabled) return s.kind === 'caldav' ? 'Off — calendars only' : 'Off — tasks stay in its app'
  if (s.error) return s.error
  if (busy.value === s.key) return 'Syncing…'
  return `${KIND[s.kind]} · ${s.lists} list${s.lists === 1 ? '' : 's'}`
}
const color = (s: TaskSource) =>
  ({ microsoft: 'var(--c-work)', google: 'var(--c-personal)', caldav: 'var(--c-hol)' })[s.kind]

const writable = computed(() => lists.value.filter((l) => !l.read_only))
const defaultList = computed(() => lists.value.find((l) => l.is_default)?.id ?? null)
async function setDefault(id: number) {
  try {
    await updateTaskList(id, { is_default: true })
    await refreshLists()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
</script>

<template>
  <section aria-label="Task sync" class="task-sync">
    <div>
      <p class="lead">
        Sync each account's to-dos with Havre. Changes go both ways every few minutes; a task made from an email lands
        in that email's account. Havre's own lists stay on this server, and in backups.
      </p>
    </div>
    <p v-if="loaded && !sources.length" class="empty">
      No account can sync tasks yet: Microsoft and Google accounts (signed in with OAuth2) and CalDAV servers (Settings ›
      Calendars) can.
    </p>
    <div v-if="sources.length" class="card list">
      <div v-for="s in sources" :key="s.key" class="row">
        <span class="dot" :style="{ background: color(s) }" aria-hidden="true" />
        <div class="info">
          <strong>{{ s.name }}</strong>
          <span class="meta" :class="{ bad: !s.allowed || (s.enabled && s.error) }">{{ meta(s) }}</span>
        </div>
        <button
          type="button"
          role="switch"
          class="switch"
          :aria-checked="s.enabled"
          :aria-label="`Sync tasks with ${s.name}`"
          :disabled="busy === s.key || (!s.allowed && !s.enabled)"
          @click="toggle(s)"
        >
          <span class="knob" aria-hidden="true" />
        </button>
      </div>
    </div>
    <label v-if="writable.length" class="where">
      <span>New tasks go to</span>
      <select :value="defaultList" @change="setDefault(Number(($event.target as HTMLSelectElement).value))">
        <option v-for="l in writable" :key="l.id" :value="l.id">{{ l.owner }} · {{ l.name }}</option>
      </select>
    </label>
  </section>
</template>

<style scoped>
.task-sync {
  display: flex;
  flex-direction: column;
  gap: 14px;
  margin-top: 8px;
}
h2 {
  margin: 0 0 4px;
  font-size: 16px;
}
.lead {
  margin: 0;
  color: var(--muted);
  font-size: 13.5px;
  max-width: 640px;
}
.empty {
  margin: 0;
  color: var(--faint);
  font-size: 13.5px;
}
.card {
  border-radius: var(--radius);
  border: 1px solid var(--line);
  background: var(--panel);
}
.row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 16px;
  border-bottom: 1px solid var(--line);
}
.row:last-child {
  border-bottom: 0;
}
.dot {
  flex: 0 0 8px;
  height: 8px;
  border-radius: 2px;
}
.info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  font-size: 13.5px;
}
.info strong {
  font-weight: 500;
}
.meta {
  font-size: 12px;
  color: var(--faint);
}
.meta.bad {
  color: var(--danger);
}
.switch {
  flex: 0 0 38px;
  height: 22px;
  padding: 2px;
  border: 0;
  border-radius: 999px;
  background: var(--line2);
  display: flex;
  justify-content: flex-start;
  cursor: pointer;
  position: relative;
}
/* A bigger touch target than the switch itself. */
.switch::before {
  content: '';
  position: absolute;
  inset: -11px -4px;
}
.switch[aria-checked='true'] {
  background: var(--accent);
  justify-content: flex-end;
}
.switch:disabled {
  opacity: 0.5;
  cursor: default;
}
.knob {
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: var(--raise);
}
.where {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
  font-size: 13.5px;
  color: var(--muted);
}
select {
  height: 34px;
  padding: 0 10px;
  border: 1px solid var(--line2);
  border-radius: 8px;
  background: var(--bg);
  color: var(--text);
  font: inherit;
  font-size: 13.5px;
}
</style>
