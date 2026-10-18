<script setup lang="ts">
import type { Task } from '~/composables/useTasks'
import type { AgendaEvent, Calendar, FoundInvitation } from '~/composables/useAgenda'

// The Agenda: day, week and month views over every calendar (Microsoft
// 365, Google, CalDAV, and Havre's own). The view and date live in the URL
// (?view=week&date=2026-10-14), so reloading keeps your place.
useHead({ title: 'Agenda · Havre' })
const route = useRoute()
const router = useRouter()
const toast = useToasts()

type View = 'day' | 'week' | 'month'
const view = computed<View>(() => (['day', 'week', 'month'].includes(route.query.view as string) ? (route.query.view as View) : 'week'))
const anchor = computed(() => parseIsoDate(route.query.date as string) ?? startOfDay(new Date()))

function go(next: { view?: View; date?: Date }) {
  router.replace({
    query: { ...route.query, view: next.view ?? view.value, date: isoDate(next.date ?? anchor.value) },
  })
}

// "now" ticks every minute (the red line, "today").
const now = ref(new Date())
let clock: ReturnType<typeof setInterval> | undefined
onMounted(() => (clock = setInterval(() => (now.value = new Date()), 60_000)))
onBeforeUnmount(() => clearInterval(clock))

// ---- What's shown ---------------------------------------------------------------

const days = computed(() => {
  if (view.value === 'day') return [anchor.value]
  const first = startOfWeek(anchor.value)
  return Array.from({ length: 7 }, (_, i) => addDays(first, i))
})

/** The period to load (a bit wider than shown for the month grid). */
const range = computed(() => {
  if (view.value === 'month') {
    const first = startOfWeek(startOfMonth(anchor.value))
    return { from: first, to: addDays(first, 42) }
  }
  return { from: days.value[0]!, to: addDays(days.value[days.value.length - 1]!, 1) }
})

const title = computed(() => {
  const a = anchor.value
  if (view.value === 'month') return a.toLocaleDateString([], { month: 'long', year: 'numeric' })
  if (view.value === 'day') return a.toLocaleDateString([], { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' })
  // "12 – 18 October 2026" or "October 12 – 18, 2026", as the language wants.
  const format = new Intl.DateTimeFormat([], { day: 'numeric', month: 'long', year: 'numeric' })
  return format.formatRange(days.value[0]!, days.value[6]!)
})

/** "Week 42" beside the title (week and day views). */
const weekLabel = computed(() => (view.value === 'month' ? '' : `Week ${isoWeek(days.value[0]!)}`))

function step(n: number) {
  const a = anchor.value
  if (view.value === 'month') go({ date: new Date(a.getFullYear(), a.getMonth() + n, 1) })
  else go({ date: addDays(a, n * (view.value === 'week' ? 7 : 1)) })
}

/** The event shown in the side panel. */
const selectedId = ref<number | null>(null)

// ---- Data -------------------------------------------------------------------------

const calendars = useCalendars()
const calendarMap = computed(() => new Map(calendars.data.value.map((c) => [c.id, c])))
const events = ref<AgendaEvent[]>([])
const loading = ref(false)
const loadError = ref<string | null>(null)
let loadSeq = 0

// UTC bounds widened by a day: all-day events are stored at UTC midnight.
async function load() {
  const seq = ++loadSeq
  loading.value = true
  try {
    const from = Math.floor(range.value.from.getTime() / 1000) - 86_400
    const to = Math.floor(range.value.to.getTime() / 1000) + 86_400
    const found = await fetchEvents(from, to)
    if (seq === loadSeq) {
      events.value = found
      loadError.value = null
    }
  } catch (err) {
    if (seq === loadSeq) loadError.value = apiErrorMessage(err)
  } finally {
    if (seq === loadSeq) loading.value = false
  }
}
watch(range, load, { immediate: true })

// Tasks due in the period (when the Tasks app is in the dock).
const tasksOn = useTasksEnabled()
const dueTasks = ref<Task[]>([])
async function loadTasks() {
  if (!tasksOn.value) return (dueTasks.value = [])
  try {
    dueTasks.value = await fetchTasks({ from: localDay(range.value.from), to: localDay(range.value.to) })
  } catch {
    dueTasks.value = []
  }
}
watch([range, tasksOn], loadTasks, { immediate: true })
async function toggleTask(task: Task) {
  try {
    const saved = await setTaskDone(task.id, !task.done_at)
    dueTasks.value = dueTasks.value.map((t) => (t.id === saved.id ? saved : t))
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
function openTask(task: Task) {
  navigateTo({ path: '/tasks', query: { task: task.id } })
}

// Opened from elsewhere (Today, People, Mail) on one event: ?event=id.
watch(
  () => route.query.event,
  (id) => {
    if (id) selectedId.value = Number(id)
  },
  { immediate: true },
)

// ---- Found in Mail: invitations still waiting for you --------------------------------
// From your mail (answer or add them here), plus meetings in Microsoft /
// Google calendars waiting for an answer that came some other way.

const pending = ref<AgendaEvent[]>([])
const found = ref<FoundInvitation[]>([])
async function loadPending() {
  const [p, f] = await Promise.all([
    fetchPendingInvitations().catch(() => pending.value),
    fetchFoundInMail().catch(() => found.value),
  ])
  pending.value = p
  found.value = f
}
onMounted(loadPending)

// "Later" / "Dismiss": hidden in this browser (they stay in Mail).
const later = ref<string[]>([])
onMounted(() => {
  try {
    later.value = JSON.parse(localStorage.getItem('havre-found-later') ?? '[]')
  } catch {
    /* private mode */
  }
})
function putOff(key: string) {
  later.value = [...later.value, key].slice(-200)
  try {
    localStorage.setItem('havre-found-later', JSON.stringify(later.value))
  } catch {
    /* private mode */
  }
}
const foundKey = (f: FoundInvitation) => `m:${f.account}:${f.folder}:${f.uid}`
const shownFound = computed(() => found.value.filter((f) => !later.value.includes(foundKey(f))))
/** Pending meetings not already shown from a message. */
const shownPending = computed(() =>
  pending.value.filter(
    (e) => !later.value.includes(`e:${e.id}`) && !found.value.some((f) => f.view.title === e.title && f.view.start === e.start),
  ),
)
/** "Thu 15 · 12:30 · chez Léon". */
function foundWhen(v: { start: number; all_day: boolean; location?: string }, who?: string | null) {
  const d = new Date(v.start * 1000)
  const parts = [`${d.toLocaleDateString([], { weekday: 'short' })} ${d.getDate()}`]
  if (!v.all_day) parts.push(timeLabel(d))
  const extra = v.location || who
  if (extra) parts.push(extra.replace(/\s*<.*>$/, ''))
  return parts.join(' · ')
}
const foundBusy = ref<string | null>(null)
async function acceptFound(f: FoundInvitation) {
  foundBusy.value = foundKey(f)
  try {
    const ref = { account: f.account, folder: f.folder, uid: f.uid, index: f.index }
    if (f.view.can_respond) await respondToInvitation(ref, 'accepted')
    else await addInvitation(ref)
    toast.show(`${f.view.can_respond ? 'Accepted' : 'Added'} “${f.view.title}”`)
    await Promise.all([loadPending(), load()])
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    foundBusy.value = null
  }
}
async function acceptPending(e: AgendaEvent) {
  foundBusy.value = `e:${e.id}`
  try {
    onResponded(await respondToEvent(e.id, 'accepted'))
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    foundBusy.value = null
  }
}

function openEvent(e: AgendaEvent) {
  go({ date: eventSpan(e).start })
  selectedId.value = e.id
}

function onResponded(e: AgendaEvent) {
  const i = events.value.findIndex((x) => x.id === e.id)
  if (i >= 0) events.value[i] = e
  if (e.my_response === 'declined') {
    events.value = events.value.filter((x) => x.id !== e.id)
    selectedId.value = null
  }
  loadPending()
  toast.show(`${RESPONSE_LABELS[e.my_response ?? '']}: ${e.title}`)
}

/** Events actually in the shown period (the query was a bit wider). */
const shown = computed(() => {
  const from = range.value.from.getTime()
  const to = range.value.to.getTime()
  return events.value.filter((e) => {
    const { start, end } = eventSpan(e)
    return start.getTime() < to && end.getTime() > from
  })
})

const busyDays = computed(() => {
  const set = new Set<string>()
  for (const e of events.value) set.add(isoDate(eventSpan(e).start))
  return set
})

// Live: another device or the background sync changed something.
useMailEvents((event) => {
  if (event.type === 'agenda' || event.type === 'accounts') {
    load()
    loadPending()
    calendars.refresh()
  }
  if (event.type === 'tasks') loadTasks()
})

const syncing = ref(false)
async function refresh() {
  syncing.value = true
  try {
    await syncAgenda()
    await Promise.all([load(), calendars.refresh()])
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    syncing.value = false
  }
}

// ---- Calendars list ------------------------------------------------------------------

const accounts = useAccounts()
const groups = computed(() => {
  const out: { key: string; label: string; calendars: Calendar[] }[] = []
  for (const c of calendars.data.value) {
    const key = c.account ? `a${c.account}` : c.caldav ? `d${c.caldav}` : 'local'
    let group = out.find((g) => g.key === key)
    if (!group) {
      const account = accounts.data.value.find((a) => a.id === c.account)
      const label = account ? account.email : c.source === 'local' ? 'On this Havre' : 'CalDAV'
      group = { key, label, calendars: [] }
      out.push(group)
    }
    group.calendars.push(c)
  }
  return out
})
const needsReconnect = computed(() => accounts.data.value.filter((a) => a.auth !== 'password' && !a.can_calendar))
const errors = computed(() => calendars.data.value.filter((c) => c.enabled && c.error))
const lastSync = computed(() => Math.max(0, ...calendars.data.value.map((c) => c.synced_at ?? 0)))

async function toggle(c: Calendar) {
  try {
    await updateCalendar(c.id, { enabled: !c.enabled })
    await calendars.refresh()
    load()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

// ---- Selecting, creating, editing ------------------------------------------------------

const selected = computed(() => events.value.find((e) => e.id === selectedId.value) ?? null)
const editor = ref<{ event?: AgendaEvent; start?: Date; end?: Date; allDay?: boolean; guests?: string[] } | null>(null)

function select(e: AgendaEvent) {
  selectedId.value = selectedId.value === e.id ? null : e.id
}
function create(range?: { start: Date; end: Date; allDay: boolean }) {
  editor.value = range ?? {}
}
// ?new=1 (from the Ctrl+K palette), &guest=… (Schedule, from Mail or People): open the editor.
watch(
  () => route.query.new,
  (n) => {
    if (!n) return
    const guest = typeof route.query.guest === 'string' ? route.query.guest : null
    editor.value = guest ? { guests: [guest] } : {}
    const { new: _, guest: __, ...rest } = route.query
    router.replace({ query: rest })
  },
  { immediate: true },
)
async function onSaved(e: AgendaEvent) {
  editor.value = null
  selectedId.value = e.id
  await load()
  toast.show('Event saved')
}
async function onDeleted() {
  editor.value = null
  selectedId.value = null
  await load()
  toast.show('Event deleted')
}

// ---- Keyboard -------------------------------------------------------------------------
// ←/→ previous/next · t today · d/w/m day/week/month · n new event · Esc close
function onKey(e: KeyboardEvent) {
  if (e.metaKey || e.ctrlKey || e.altKey || editor.value) return
  const el = e.target instanceof Element ? e.target : null
  if (el?.closest('input, textarea, select, [role="dialog"]')) return
  if (e.key === 'ArrowLeft' || e.key === 'j') step(-1)
  else if (e.key === 'ArrowRight' || e.key === 'k') step(1)
  else if (e.key === 't') go({ date: startOfDay(new Date()) })
  else if (e.key === 'd') go({ view: 'day' })
  else if (e.key === 'w') go({ view: 'week' })
  else if (e.key === 'm') go({ view: 'month' })
  else if (e.key === 'n' || e.key === 'c') {
    e.preventDefault()
    create()
  } else if (e.key === 'Escape') selectedId.value = null
  else return
}
onMounted(() => window.addEventListener('keydown', onKey))
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))

const ago = (seconds: number) => {
  if (!seconds) return 'not yet'
  const diff = Date.now() / 1000 - seconds
  if (diff < 90) return 'just now'
  if (diff < 3600) return `${Math.round(diff / 60)} min ago`
  return new Date(seconds * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}
</script>

<template>
  <div class="agenda">
    <aside class="side" aria-label="Calendars">
      <div class="side-head">
        <h1 class="serif">Agenda</h1>
        <span class="mono faint tiny">{{ calendars.data.value.filter((c) => c.enabled).length }} calendars</span>
      </div>

      <MiniMonth :selected="anchor" :now="now" :busy="busyDays" @pick="(d) => go({ date: d })" />

      <div class="cals">
        <section v-for="g in groups" :key="g.key">
          <p class="eyebrow" :title="g.label">{{ g.label }}</p>
          <label v-for="c in g.calendars" :key="c.id" class="cal" :title="c.error ?? c.name">
            <input type="checkbox" :checked="c.enabled" @change="toggle(c)" />
            <span class="box" :style="{ '--c': c.color }" aria-hidden="true">
              <HIcon v-if="c.enabled" name="check" :size="11" :stroke-width="2.6" />
            </span>
            <span class="name">{{ c.name }}</span>
            <HIcon v-if="c.enabled && c.error" name="alert" :size="13" class="warn" />
          </label>
        </section>
        <NuxtLink v-if="needsReconnect.length" to="/settings" class="notice">
          {{ needsReconnect.map((a) => a.email).join(', ') }}: reconnect in Settings to see
          {{ needsReconnect.length > 1 ? 'their' : 'its' }} calendar.
        </NuxtLink>
        <NuxtLink to="/settings/calendars" class="add">
          <HIcon name="plus" :size="14" />
          Add a CalDAV calendar
        </NuxtLink>

        <section v-if="shownFound.length || shownPending.length" class="found" aria-label="Found in Mail">
          <p class="eyebrow fhead"><HIcon name="mail" :size="13" /> Found in Mail</p>
          <div v-for="f in shownFound" :key="foundKey(f)" class="fcard">
            <div>
              <div class="ft">{{ f.view.title }}</div>
              <div class="fw">{{ foundWhen(f.view, f.from) }}</div>
            </div>
            <div class="facts">
              <button type="button" class="fbtn primary" :disabled="foundBusy === foundKey(f)" @click="acceptFound(f)">
                {{ f.view.can_respond ? 'Accept' : 'Add' }}
              </button>
              <button type="button" class="fbtn" @click="putOff(foundKey(f))">{{ f.view.can_respond ? 'Later' : 'Dismiss' }}</button>
              <NuxtLink :to="messageLink(f)" class="flink">View email</NuxtLink>
            </div>
          </div>
          <div v-for="e in shownPending" :key="e.id" class="fcard">
            <div>
              <div class="ft">{{ e.title }}</div>
              <div class="fw">{{ foundWhen(e, e.organizer) }}</div>
            </div>
            <div class="facts">
              <button type="button" class="fbtn primary" :disabled="foundBusy === `e:${e.id}`" @click="acceptPending(e)">Accept</button>
              <button type="button" class="fbtn" @click="putOff(`e:${e.id}`)">Later</button>
              <button type="button" class="flink" @click="openEvent(e)">Open</button>
            </div>
          </div>
        </section>
      </div>

      <div class="sync">
        <span class="dot" :class="{ bad: errors.length }" aria-hidden="true" />
        <span v-if="errors.length" :title="errors.map((c) => `${c.name}: ${c.error}`).join('\n')">
          {{ errors.length }} calendar{{ errors.length > 1 ? 's' : '' }} can't sync
        </span>
        <span v-else>Synced · {{ ago(lastSync) }}</span>
        <button type="button" class="icon-btn small" :class="{ spin: syncing }" aria-label="Sync now" title="Sync now" @click="refresh">
          <HIcon name="refresh" :size="14" />
        </button>
      </div>
    </aside>

    <main class="main">
      <div class="toolbar">
        <h2 class="serif title">{{ title }}</h2>
        <span v-if="weekLabel" class="mono faint week">{{ weekLabel }}</span>
        <span v-if="loading" class="loading" aria-label="Loading" />
        <span class="spacer" />
        <button type="button" class="tbtn" title="Today (T)" @click="go({ date: startOfDay(new Date()) })">Today</button>
        <div class="arrows">
          <button type="button" class="icon-btn" :aria-label="`Previous ${view}`" title="Previous (←)" @click="step(-1)">
            <HIcon name="back" :size="17" />
          </button>
          <button type="button" class="icon-btn flip" :aria-label="`Next ${view}`" title="Next (→)" @click="step(1)">
            <HIcon name="back" :size="17" />
          </button>
        </div>
        <div class="views" role="tablist" aria-label="View">
          <button
            v-for="v in (['day', 'week', 'month'] as const)"
            :key="v"
            type="button"
            role="tab"
            :aria-selected="view === v"
            :class="{ on: view === v }"
            :title="`${v[0]!.toUpperCase()}${v.slice(1)} (${v[0]!.toUpperCase()})`"
            @click="go({ view: v })"
          >
            {{ v[0]!.toUpperCase() + v.slice(1) }}
          </button>
        </div>
        <button type="button" class="tbtn primary" title="New event (N)" @click="create()">
          <HIcon name="plus" :size="15" :stroke-width="2" />
          New event
        </button>
      </div>
      <p v-if="loadError" class="load-error" role="alert">{{ loadError }}</p>

      <div class="body">
        <AgendaMonth
          v-if="view === 'month'"
          :month="anchor"
          :events="shown"
          :calendars="calendarMap"
          :now="now"
          :selected="selectedId"
          @select="select"
          :tasks="dueTasks"
          @open-day="(d) => go({ view: 'day', date: d })"
          @create="create"
        />
        <AgendaWeek
          v-else
          :key="view"
          :days="days"
          :events="shown"
          :calendars="calendarMap"
          :now="now"
          :selected="selectedId"
          :tasks="dueTasks"
          @select="select"
          @create="create"
          @open-task="openTask"
          @toggle-task="toggleTask"
        />
        <EventDetails
          v-if="selected"
          :event="selected"
          :calendar="calendarMap.get(selected.calendar)"
          @edit="editor = { event: selected }"
          @close="selectedId = null"
          @responded="onResponded"
        />
      </div>
    </main>

    <EventEditor
      v-if="editor"
      :event="editor.event"
      :start="editor.start"
      :end="editor.end"
      :all-day="editor.allDay"
      :guests="editor.guests"
      :calendars="calendars.data.value"
      @saved="onSaved"
      @deleted="onDeleted"
      @cancel="editor = null"
    />
  </div>
</template>

<style scoped>
.agenda {
  flex: 1;
  min-width: 0;
  display: flex;
  background: var(--panel);
}
.side {
  flex: 0 0 248px;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 20px 12px 12px;
  border-right: 1px solid var(--line);
  background: var(--bg);
  min-height: 0;
}
.side-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  padding: 0 8px;
}
.tiny {
  font-size: 11px;
}
.faint {
  color: var(--faint);
}
.side h1 {
  margin: 0;
  font-size: 28px;
}
.cals {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 0 4px;
}
.cals .eyebrow {
  margin: 0 0 4px 4px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.cal {
  position: relative;
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 5px 6px;
  border-radius: 8px;
  font-size: 13.5px;
  cursor: pointer;
}
.cal:hover {
  background: var(--hover);
}
.cal input {
  position: absolute;
  opacity: 0;
}
.cal input:focus-visible + .box {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}
.box {
  flex: 0 0 16px;
  height: 16px;
  display: grid;
  place-items: center;
  border-radius: 5px;
  border: 2px solid var(--c);
  color: var(--bg);
}
.cal input:checked + .box {
  background: var(--c);
}
.name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.warn {
  color: var(--danger);
}
.notice {
  padding: 8px 10px;
  border-radius: 9px;
  background: color-mix(in oklab, var(--accent) 10%, transparent);
  color: var(--muted);
  font-size: 12.5px;
  line-height: 1.5;
  text-decoration: none;
}
.add {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 4px 6px;
  font-size: 12.5px;
  color: var(--faint);
  text-decoration: none;
}
.add:hover {
  color: var(--text);
}
.sync {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 8px 0;
  border-top: 1px solid var(--line);
  font-size: 12px;
  color: var(--faint);
}
.sync span:nth-child(2) {
  flex: 1;
}
.sync .dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--ok);
}
.sync .dot.bad {
  background: var(--danger);
}
.icon-btn.small {
  width: 28px;
  height: 28px;
}
.spin :deep(svg) {
  animation: spin 0.8s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
.main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 12px;
  min-height: 66px;
  padding: 16px 24px;
  border-bottom: 1px solid var(--line);
}
.week {
  font-size: 12px;
}
.tbtn {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  height: 34px;
  padding: 0 12px;
  border: 1px solid var(--line2);
  border-radius: 9px;
  background: transparent;
  color: var(--text);
  font: inherit;
  font-size: 13.5px;
  cursor: pointer;
}
.tbtn:hover {
  background: var(--hover);
}
.tbtn.primary {
  padding: 0 14px;
  border: 0;
  background: var(--accent);
  color: var(--on-accent);
  font-weight: 500;
}
.tbtn.primary:hover {
  filter: brightness(1.08);
}
.arrows {
  display: flex;
}
.flip {
  transform: scaleX(-1);
}
.title {
  margin: 0;
  font-size: 26px;
  letter-spacing: -0.01em;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  text-transform: capitalize;
}
.loading {
  width: 14px;
  height: 14px;
  border-radius: 50%;
  border: 2px solid var(--line2);
  border-top-color: var(--accent);
  animation: spin 0.7s linear infinite;
}
.spacer {
  flex: 1;
}
.views {
  display: inline-flex;
  padding: 2px;
  gap: 2px;
  border-radius: 9px;
  border: 1px solid var(--line);
  background: var(--bg);
}
.views button {
  height: 28px;
  padding: 0 12px;
  border: 0;
  border-radius: 7px;
  background: transparent;
  color: var(--muted);
  font: inherit;
  font-size: 13px;
  cursor: pointer;
}
.views button {
  font-size: 12.5px;
}
.views button.on {
  background: var(--sel);
  color: var(--text);
}
.load-error {
  margin: 0;
  padding: 8px 20px;
  color: var(--danger);
  font-size: 13px;
  border-bottom: 1px solid var(--line);
}
.body {
  flex: 1;
  min-height: 0;
  display: flex;
}
@media (max-width: 1100px) {
  .side {
    display: none;
  }
}
@media (max-width: 720px) {
  .toolbar {
    flex-wrap: wrap;
    padding: 10px 12px;
    gap: 6px;
  }
  .title {
    flex-basis: 100%;
    margin: 0;
    font-size: 21px;
  }
  .week {
    display: none;
  }
}
.found {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 10px;
}
.fhead {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0 0 0 4px;
}
.fcard {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px;
  border-radius: 12px;
  border: 1px solid var(--line2);
  background: var(--raise);
}
.ft {
  font-size: 13.5px;
  font-weight: 500;
  overflow-wrap: anywhere;
}
.fw {
  font-size: 12.5px;
  color: var(--muted);
}
.facts {
  display: flex;
  align-items: center;
  gap: 6px;
}
.fbtn {
  height: 28px;
  padding: 0 10px;
  border: 0;
  border-radius: 7px;
  background: transparent;
  color: var(--muted);
  font: inherit;
  font-size: 12.5px;
  cursor: pointer;
}
.fbtn:hover {
  background: var(--hover);
  color: var(--text);
}
.fbtn.primary {
  background: var(--accent);
  color: var(--on-accent);
  font-weight: 500;
}
.fbtn.primary:hover {
  filter: brightness(1.08);
}
.flink {
  margin-left: auto;
  padding: 0;
  border: 0;
  background: none;
  font: inherit;
  font-size: 12px;
  color: var(--accent);
  text-decoration: none;
  cursor: pointer;
}
</style>
