<script setup lang="ts">
import type { DueGroup, Parsed, Task, TaskList, TaskSource } from '~/composables/useTasks'

// Tasks: views (Today, Upcoming, From Mail, No date, Done) and your lists on
// the left, the tasks grouped by day in the middle, the selected task on
// the right. `?view=…` or `?list=…` picks what's shown, `?task=…` which
// task is open.
useHead({ title: 'Tasks · Havre' })
const route = useRoute()
const router = useRouter()
const toast = useToasts()

type View = 'today' | 'upcoming' | 'mail' | 'anytime' | 'done'
const VIEWS: { id: View; label: string; icon: 'sun' | 'calendar' | 'mail' | 'circle' | 'check' }[] = [
  { id: 'today', label: 'Today', icon: 'sun' },
  { id: 'upcoming', label: 'Upcoming', icon: 'calendar' },
  { id: 'mail', label: 'From Mail', icon: 'mail' },
  { id: 'anytime', label: 'No date', icon: 'circle' },
  { id: 'done', label: 'Done', icon: 'check' },
]
const listId = computed(() => (route.query.list ? Number(route.query.list) : null))
const view = computed<View | null>(() =>
  listId.value != null ? null : VIEWS.some((v) => v.id === route.query.view) ? (route.query.view as View) : 'upcoming',
)
const selectedId = computed(() => (route.query.task ? Number(route.query.task) : null))
function go(query: Record<string, string | number | undefined>) {
  router.replace({ query: Object.fromEntries(Object.entries(query).filter(([, v]) => v !== undefined)) })
}
function pickView(id: View) {
  go({ view: id })
}
function pickList(id: number) {
  go({ list: id })
}
function open(task: Task) {
  go({ ...route.query, task: task.id } as Record<string, string | number>)
}
function closeTask() {
  const { task: _, ...rest } = route.query
  router.replace({ query: rest })
}

// ---- Data -------------------------------------------------------------------------
const { lists, refresh: refreshLists, byId } = useTaskLists()
const tasks = ref<Task[]>([])
const loading = ref(true)
async function load() {
  try {
    const [all] = await Promise.all([fetchTasks(), refreshLists()])
    tasks.value = all
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    loading.value = false
  }
}
onMounted(load)
let reload: ReturnType<typeof setTimeout> | undefined
useMailEvents((event) => {
  if (event.type === 'tasks' || event.type === 'accounts') {
    clearTimeout(reload)
    reload = setTimeout(load, 300)
  }
})
onBeforeUnmount(() => clearTimeout(reload))

const today = ref(localDay())
// Midnight passes: "Today" moves on.
let tick: ReturnType<typeof setInterval> | undefined
onMounted(() => (tick = setInterval(() => (today.value = localDay()), 60_000)))
onBeforeUnmount(() => clearInterval(tick))

/** Done today (or just ticked): still shown, struck through, until you leave. */
const keepShown = ref(new Set<number>())
const showDone = ref(false)

const open_ = computed(() => tasks.value.filter((t) => !t.done_at))
const counts = computed(() => ({
  today: open_.value.filter((t) => t.due != null && t.due <= today.value).length,
  upcoming: open_.value.length,
  mail: open_.value.filter((t) => t.mail).length,
  anytime: open_.value.filter((t) => t.due == null).length,
  done: 0,
}))

const shown = computed(() => {
  const keep = (t: Task) => !t.done_at || keepShown.value.has(t.id) || showDone.value
  let list = tasks.value
  if (listId.value != null) list = list.filter((t) => t.list === listId.value && keep(t))
  else if (view.value === 'done') list = list.filter((t) => t.done_at).sort((a, b) => (b.done_at ?? 0) - (a.done_at ?? 0))
  else if (view.value === 'today') list = list.filter((t) => t.due != null && t.due <= today.value && keep(t))
  else if (view.value === 'mail') list = list.filter((t) => t.mail && keep(t))
  else if (view.value === 'anytime') list = list.filter((t) => t.due == null && keep(t))
  else list = list.filter(keep)
  return view.value === 'done' ? list : [...list].sort(byDue)
})

const GROUPS: { id: DueGroup; label: string }[] = [
  { id: 'late', label: 'Overdue' },
  { id: 'today', label: 'Today' },
  { id: 'tomorrow', label: 'Tomorrow' },
  { id: 'week', label: 'Next 7 days' },
  { id: 'later', label: 'Later' },
  { id: 'none', label: 'No date' },
]
const groups = computed(() => {
  if (view.value === 'done') return [{ id: 'done', label: 'Done in the last 30 days', sub: '', items: shown.value, late: false }]
  const t = today.value
  const sub = (id: DueGroup) => {
    const fmt = (d: number, opts: Intl.DateTimeFormatOptions) => dayToDate(d).toLocaleDateString('en-GB', opts)
    if (id === 'today') return dayText(dayToDate(t), true)
    if (id === 'tomorrow') return dayText(dayToDate(t + 86_400), true)
    if (id === 'week') return `${fmt(t + 2 * 86_400, { day: 'numeric', month: 'short' })} – ${fmt(t + 7 * 86_400, { day: 'numeric', month: 'short' })}`
    return ''
  }
  return GROUPS.map((g) => {
    const items = shown.value.filter((x) => dueGroup(x, t) === g.id)
    return { id: g.id, label: g.label, sub: g.id === 'late' ? String(items.length) : sub(g.id), items, late: g.id === 'late' }
  }).filter((g) => g.items.length)
})
const doneCount = computed(() => {
  const pool = listId.value != null ? tasks.value.filter((t) => t.list === listId.value) : tasks.value
  return pool.filter((t) => t.done_at && !keepShown.value.has(t.id)).length
})

const heading = computed(() =>
  listId.value != null
    ? byId.value.get(listId.value)?.name ?? 'List'
    : VIEWS.find((v) => v.id === view.value)?.label ?? 'Tasks',
)
const selected = computed(() => tasks.value.find((t) => t.id === selectedId.value) ?? null)

// Lists grouped by who they belong to (an account, a CalDAV server, Havre).
const owners = computed(() => {
  const out: { key: string; owner: string; kind: string; lists: TaskList[] }[] = []
  for (const l of lists.value) {
    const key = `${l.source}:${l.account ?? l.caldav ?? 0}`
    let group = out.find((g) => g.key === key)
    if (!group) {
      const kind = { microsoft: 'To Do', google: 'Google', caldav: 'CalDAV', local: 'this server' }[l.source]
      group = { key, owner: l.owner, kind, lists: [] }
      out.push(group)
    }
    group.lists.push(l)
  }
  return out
})

// ---- Changing ---------------------------------------------------------------------
function replace(task: Task) {
  const i = tasks.value.findIndex((t) => t.id === task.id)
  if (i >= 0) tasks.value.splice(i, 1, task)
  else tasks.value.push(task)
}
async function toggle(task: Task) {
  const done = !task.done_at
  // Show it at once; put it back if the server says no.
  const before = { ...task }
  replace({ ...task, done_at: done ? Math.floor(Date.now() / 1000) : null })
  if (done) keepShown.value.add(task.id)
  try {
    replace(await setTaskDone(task.id, done))
    refreshLists()
    if (done)
      toast.show('Done', {
        action: { label: 'Undo', run: () => toggle(tasks.value.find((t) => t.id === task.id) ?? task) },
      })
  } catch (err) {
    replace(before)
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
function saved(task: Task) {
  // Moved to another list: it has a new id.
  if (selectedId.value != null && task.id !== selectedId.value) {
    tasks.value = tasks.value.filter((t) => t.id !== selectedId.value)
    go({ ...route.query, task: task.id } as Record<string, string | number>)
  }
  replace(task)
  refreshLists()
}
function deleted(id: number) {
  tasks.value = tasks.value.filter((t) => t.id !== id)
  closeTask()
  refreshLists()
  toast.show('Task deleted')
}

// ---- Quick add --------------------------------------------------------------------
const text = ref('')
const preview = ref<Parsed | null>(null)
const adding = ref(false)
let parseTimer: ReturnType<typeof setTimeout> | undefined
watch(text, (value) => {
  clearTimeout(parseTimer)
  if (!value.trim()) {
    preview.value = null
    return
  }
  parseTimer = setTimeout(async () => {
    try {
      preview.value = await parseTask(value)
    } catch {
      preview.value = null
    }
  }, 180)
})
/** Where a new task goes: `#list`, else the list shown, else the default. */
const target = computed(() => {
  const id = preview.value?.list ?? listId.value
  const chosen = id != null ? byId.value.get(id) : undefined
  if (chosen && !chosen.read_only) return chosen
  return lists.value.find((l) => l.is_default) ?? lists.value[0]
})
const previewDue = computed(() => {
  const p = preview.value
  if (p?.due != null) return dueLabel(p)
  // Adding in Today: due today.
  if (view.value === 'today') return 'Today'
  return null
})
async function add() {
  const value = text.value.trim()
  if (!value || adding.value) return
  adding.value = true
  try {
    const parsed = await parseTask(value)
    const due = parsed.due ?? (view.value === 'today' ? today.value : null)
    const task = await createTask({
      list: parsed.list ?? target.value?.id ?? null,
      title: parsed.title,
      due,
      due_at: parsed.due_at,
    })
    replace(task)
    keepShown.value.add(task.id)
    text.value = ''
    preview.value = null
    refreshLists()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    adding.value = false
  }
}

// ---- Lists ------------------------------------------------------------------------
const newList = ref<string | null>(null)
async function addList() {
  const name = newList.value?.trim()
  newList.value = null
  if (!name) return
  try {
    const list = await createTaskList(name)
    await refreshLists()
    pickList(list.id)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

// ---- Sync status ------------------------------------------------------------------
const sources = ref<TaskSource[]>([])
onMounted(async () => {
  sources.value = await fetchTaskSources().catch(() => [])
})
const syncLines = computed(() => {
  const out: { name: string; state: 'ok' | 'bad' | 'off'; note: string }[] = []
  for (const s of sources.value) {
    if (!s.enabled) continue
    const label = { microsoft: 'Microsoft To Do', google: 'Google Tasks', caldav: s.name }[s.kind]
    if (!s.allowed) out.push({ name: label, state: 'bad', note: 'reconnect' })
    else if (s.error) out.push({ name: label, state: 'bad', note: 'error' })
    else {
      const mine = lists.value.filter((l) => l.source === s.kind && `${s.kind === 'caldav' ? 'caldav' : 'account'}:${l.account ?? l.caldav}` === s.key)
      const last = Math.max(0, ...mine.map((l) => l.synced_at ?? 0))
      out.push({ name: label, state: 'ok', note: last ? ago(last) : '' })
    }
  }
  return out
})
function ago(ts: number) {
  const min = Math.round((Date.now() / 1000 - ts) / 60)
  if (min < 1) return 'now'
  if (min < 60) return `${min} min`
  return `${Math.round(min / 60)} h`
}
const syncing = ref(false)
async function syncNow() {
  syncing.value = true
  try {
    await syncTasks()
    await load()
    sources.value = await fetchTaskSources().catch(() => sources.value)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    syncing.value = false
  }
}

// Leaving a view forgets the ticked ones kept on screen.
watch([view, listId], () => keepShown.value.clear())

// `n`: focus quick add.
const input = ref<HTMLInputElement | null>(null)
function onKey(e: KeyboardEvent) {
  const typing = (e.target as HTMLElement)?.closest?.('input, textarea, select, [contenteditable]')
  if (!typing && e.key === 'n' && !e.ctrlKey && !e.metaKey && !e.altKey) {
    e.preventDefault()
    input.value?.focus()
  }
}
onMounted(() => document.addEventListener('keydown', onKey))
onBeforeUnmount(() => document.removeEventListener('keydown', onKey))
</script>

<template>
  <div class="tasks-app" :class="{ 'has-task': !!selected }">
    <SplitGroup id="tasks">
    <SplitPane id="lists" :order="1" :size="240" :min="200" :max="340" collapsible>
    <aside class="side" aria-label="Task lists">
      <h1 class="serif">Tasks</h1>
      <nav class="views" aria-label="Views">
        <button
          v-for="v in VIEWS"
          :key="v.id"
          type="button"
          class="row"
          :aria-current="view === v.id ? 'page' : undefined"
          @click="pickView(v.id)"
        >
          <HIcon :name="v.icon" :size="16" class="ic" />
          <span class="name">{{ v.label }}</span>
          <span v-if="counts[v.id]" class="count mono" :class="{ warn: v.id === 'today' }">{{ counts[v.id] }}</span>
        </button>
      </nav>

      <div v-for="g in owners" :key="g.key" class="owner">
        <div class="owner-head eyebrow">
          <span class="o">{{ g.owner }}</span>
          <span class="k">{{ g.kind }}</span>
        </div>
        <button
          v-for="l in g.lists"
          :key="l.id"
          type="button"
          class="row"
          :aria-current="listId === l.id ? 'page' : undefined"
          :title="l.error ?? undefined"
          @click="pickList(l.id)"
        >
          <span class="dot" :style="{ background: l.color }" aria-hidden="true" />
          <span class="name">{{ l.name }}</span>
          <HIcon v-if="l.error" name="alert" :size="13" class="err" />
          <span v-if="l.open" class="count mono">{{ l.open }}</span>
        </button>
      </div>

      <form v-if="newList !== null" class="new-list" @submit.prevent="addList">
        <input v-model="newList" aria-label="New list name" placeholder="List name" autofocus @blur="addList" />
      </form>
      <button v-else type="button" class="row ghost" @click="newList = ''">
        <HIcon name="plus" :size="14" />
        <span class="name">New list</span>
      </button>

      <span class="grow" />

      <section v-if="syncLines.length" class="sync" aria-label="Sync">
        <div class="eyebrow">Synced with</div>
        <div v-for="s in syncLines" :key="s.name" class="line">
          <span class="led" :class="s.state" aria-hidden="true" />
          <span class="n">{{ s.name }}</span>
          <span class="note" :class="s.state">{{ s.note }}</span>
        </div>
        <div class="sync-actions">
          <NuxtLink to="/settings">Accounts →</NuxtLink>
          <button type="button" class="icon-btn small" aria-label="Sync tasks now" :disabled="syncing" @click="syncNow">
            <HIcon name="refresh" :size="14" :class="{ spin: syncing }" />
          </button>
        </div>
      </section>
    </aside>
    </SplitPane>

    <SplitHandle label="Resize the task lists" fold="lists" show-label="Show lists" />
    <SplitPane id="tasks" :order="2" :min="420">
    <main class="main">
      <header class="bar">
        <h2>{{ heading }}</h2>
        <span class="date mono">{{ dayText(dayToDate(today)) }}</span>
      </header>

      <div class="quick">
        <label class="add">
          <HIcon name="plus" :size="16" class="plus" />
          <span class="sr-only">New task</span>
          <input
            ref="input"
            v-model="text"
            type="text"
            placeholder="Add a task… try “Call Thomas fri 10:00”"
            :disabled="adding"
            @keydown.enter.prevent="add"
          />
          <kbd class="mono">↵</kbd>
        </label>
        <div v-if="text.trim()" class="will" aria-live="polite">
          <span>Will add</span>
          <span v-if="previewDue" class="pill accent">
            <HIcon name="calendar" :size="11" />
            {{ previewDue }}
          </span>
          <span v-if="target" class="pill">
            <span class="dot" :style="{ background: target.color }" aria-hidden="true" />
            {{ target.owner }} · {{ target.name }}
          </span>
          <span class="hint">— words like <span class="mono">fri</span>, <span class="mono">tomorrow</span>, <span class="mono">#list</span> are understood</span>
        </div>
      </div>

      <div class="scroll">
        <div class="col">
          <p v-if="loading" class="empty">Loading…</p>
          <template v-else>
            <section v-for="g in groups" :key="g.id" :aria-label="g.label">
              <div class="group-head">
                <span class="gl" :class="{ late: g.late }">{{ g.label }}</span>
                <span class="gs">{{ g.sub }}</span>
              </div>
              <TaskRow
                v-for="t in g.items"
                :key="t.id"
                :task="t"
                :list="byId.get(t.list)"
                :hide-list="listId != null"
                :selected="t.id === selectedId"
                @toggle="toggle(t)"
                @open="open(t)"
              />
            </section>
            <p v-if="!groups.length" class="empty">
              {{
                view === 'done'
                  ? 'Nothing done in the last 30 days.'
                  : view === 'today'
                    ? 'Nothing due today.'
                    : view === 'mail'
                      ? 'No task from an email yet: open an email and press T.'
                      : 'Nothing to do. Add a task above, or make one from an email (T).'
              }}
            </p>
            <button
              v-if="view !== 'done' && (doneCount || showDone)"
              type="button"
              class="show-done"
              @click="showDone = !showDone"
            >
              {{ showDone ? 'Hide done' : `Show done (${doneCount})` }}
            </button>
          </template>
        </div>
      </div>
    </main>
    </SplitPane>

    <SplitHandle v-if="selected" label="Resize the task details" />
    <SplitPane v-if="selected" id="details" :order="3" :size="380" :min="320" :max="600">
    <TaskDetails
      :key="selected.id"
      :task="selected"
      :lists="lists"
      @saved="saved"
      @deleted="deleted"
      @close="closeTask"
    />
    </SplitPane>
    </SplitGroup>
  </div>
</template>

<style scoped>
.tasks-app {
  flex: 1;
  display: flex;
  min-width: 0;
}
.side {
  flex: 0 0 240px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 20px 12px;
  border-right: 1px solid var(--line);
  overflow-y: auto;
}
h1 {
  margin: 0;
  padding: 0 8px 16px;
  font-size: 28px;
}
.views {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.row {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  height: 34px;
  padding: 0 10px;
  border: 0;
  border-radius: 9px;
  background: transparent;
  color: var(--text);
  text-align: left;
  cursor: pointer;
  font-size: 13.5px;
}
.row:hover {
  background: var(--hover);
}
.row[aria-current='page'] {
  background: var(--sel);
}
.row[aria-current='page'] .ic {
  color: var(--accent);
}
.row.ghost {
  margin-top: 6px;
  height: 32px;
  color: var(--muted);
  font-size: 13px;
}
.ic {
  color: var(--muted);
}
.name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.count {
  font-size: 11.5px;
  color: var(--faint);
}
.count.warn {
  color: var(--danger);
}
.err {
  color: var(--danger);
}
.owner-head {
  display: flex;
  gap: 6px;
  padding: 18px 10px 6px;
}
.owner-head .o {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.owner-head .k {
  text-transform: none;
  letter-spacing: 0;
}
.dot {
  width: 8px;
  height: 8px;
  margin: 0 4px;
  border-radius: 3px;
  flex: none;
}
.new-list input {
  width: 100%;
  height: 32px;
  margin-top: 6px;
  padding: 0 10px;
  border: 1px solid var(--accent);
  border-radius: 9px;
  background: transparent;
  color: var(--text);
  outline: none;
}
.grow {
  flex: 1;
  min-height: 16px;
}
.sync {
  padding: 12px;
  border-radius: 12px;
  border: 1px solid var(--line);
  background: var(--panel);
  display: flex;
  flex-direction: column;
  gap: 7px;
  font-size: 12.5px;
}
.line {
  display: flex;
  align-items: center;
  gap: 8px;
}
.led {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--faint);
}
.led.ok {
  background: var(--ok);
}
.led.bad {
  background: var(--danger);
}
.n {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.note {
  color: var(--faint);
}
.note.bad {
  color: var(--danger);
}
.sync-actions {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.sync-actions a {
  text-decoration: none;
}
.icon-btn.small {
  width: 28px;
  height: 28px;
}
.spin {
  animation: spin 0.9s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
.main {
  flex: 999 1 480px;
  min-width: 0;
  display: flex;
  flex-direction: column;
  background: var(--panel);
}
.bar {
  display: flex;
  align-items: center;
  gap: 12px;
  height: 56px;
  padding: 0 24px;
  border-bottom: 1px solid var(--line);
  flex: none;
}
h2 {
  margin: 0;
  flex: 1;
  font-size: 15px;
  font-weight: 600;
}
.date {
  font-size: 12px;
  color: var(--faint);
}
.quick {
  width: 100%;
  max-width: 760px;
  margin: 0 auto;
  padding: 20px 28px 4px;
  flex: none;
}
.add {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 46px;
  padding: 0 8px 0 14px;
  border: 1px solid var(--line2);
  border-radius: 12px;
  background: var(--bg);
}
.add:focus-within {
  border-color: var(--accent);
}
.plus {
  color: var(--accent);
  flex: none;
}
.add input {
  flex: 1;
  min-width: 0;
  border: 0;
  outline: none;
  background: transparent;
  color: var(--text);
  font-size: 14.5px;
}
kbd {
  font-size: 11px;
  padding: 2px 6px;
  border-radius: 5px;
  border: 1px solid var(--line2);
  color: var(--faint);
}
.will {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  padding: 8px 4px 0;
  font-size: 12px;
  color: var(--faint);
}
.pill {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 8px;
  border-radius: 999px;
  border: 1px solid var(--line2);
  color: var(--text);
}
.pill .dot {
  width: 7px;
  height: 7px;
  margin: 0;
  border-radius: 2px;
}
.pill.accent {
  border-color: transparent;
  background: color-mix(in oklab, var(--accent) 14%, transparent);
  color: var(--accent);
}
.scroll {
  flex: 1;
  overflow-y: auto;
}
.col {
  max-width: 760px;
  margin: 0 auto;
  padding: 6px 28px 28px;
}
.group-head {
  display: flex;
  align-items: baseline;
  gap: 10px;
  padding: 14px 4px 6px;
  border-bottom: 1px solid var(--line);
  margin-bottom: 4px;
}
.gl {
  font-size: 13px;
  font-weight: 600;
}
.gl.late {
  color: var(--danger);
}
.gs {
  font-size: 12px;
  color: var(--faint);
}
.empty {
  padding: 28px 4px;
  color: var(--faint);
}
.show-done {
  margin-top: 14px;
  height: 30px;
  padding: 0 10px;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: var(--faint);
  font-size: 12.5px;
  cursor: pointer;
}
.show-done:hover {
  background: var(--hover);
  color: var(--text);
}

/* Phones: views as tabs on top, quick add at the bottom, a task full screen. */
@media (max-width: 720px) {
  .tasks-app {
    flex-direction: column;
    min-height: 0;
  }
  .side {
    flex: none;
    flex-direction: row;
    flex-wrap: nowrap;
    gap: 6px;
    padding: 14px 16px 10px;
    border-right: 0;
    border-bottom: 1px solid var(--line);
    overflow-x: auto;
    align-items: center;
  }
  h1 {
    padding: 0 8px 0 0;
    font-size: 26px;
  }
  .views {
    flex-direction: row;
    gap: 6px;
  }
  .row {
    width: auto;
    height: 34px;
    padding: 0 12px;
    border: 1px solid var(--line2);
    border-radius: 999px;
    white-space: nowrap;
    flex: none;
  }
  .row .ic,
  .row .dot {
    display: none;
  }
  .row[aria-current='page'] {
    background: var(--text);
    color: var(--bg);
    border-color: var(--text);
  }
  .row[aria-current='page'] .count {
    color: var(--bg);
  }
  .owner {
    display: contents;
  }
  .owner-head,
  .row.ghost,
  .new-list,
  .grow,
  .sync {
    display: none;
  }
  .bar {
    display: none;
  }
  .main {
    flex: 1;
    min-height: 0;
    background: var(--panel);
  }
  .quick {
    order: 3;
    padding: 10px 12px;
    border-top: 1px solid var(--line);
  }
  .add {
    height: 52px;
  }
  kbd,
  .will .hint {
    display: none;
  }
  .col {
    padding: 0 12px 16px;
  }
}
</style>
