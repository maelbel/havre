<script setup lang="ts">
import type { Task } from '~/composables/useTasks'
import type { AgendaEvent, Calendar, FoundInvitation, Scope } from '~/composables/useAgenda'

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

// Phones: one day at a time under a strip of the week's days. The Week view
// becomes the day view there (seven columns don't fit), but the whole week is
// still loaded so the strip can mark the busy days.
const phone = useIsPhone()
const phoneDay = computed(() => phone.value && view.value !== 'month')

const days = computed(() => {
  if (view.value === 'day' || phoneDay.value) return [anchor.value]
  const first = startOfWeek(anchor.value)
  return Array.from({ length: 7 }, (_, i) => addDays(first, i))
})

/** The period to load (a bit wider than shown for the month grid). */
const range = computed(() => {
  if (view.value === 'month') {
    const first = startOfWeek(startOfMonth(anchor.value))
    return { from: first, to: addDays(first, 42) }
  }
  if (phoneDay.value) {
    const first = startOfWeek(anchor.value)
    return { from: first, to: addDays(first, 7) }
  }
  return { from: days.value[0]!, to: addDays(days.value[days.value.length - 1]!, 1) }
})

/** The week strip (phones): the seven days around the shown one. */
const strip = computed(() => {
  const first = startOfWeek(anchor.value)
  const today = isoDate(now.value)
  return Array.from({ length: 7 }, (_, i) => {
    const d = addDays(first, i)
    const key = isoDate(d)
    return {
      date: d,
      key,
      weekday: d.toLocaleDateString([], { weekday: 'narrow' }),
      day: d.getDate(),
      label: d.toLocaleDateString([], { weekday: 'long', day: 'numeric', month: 'long' }),
      today: key === today,
      selected: key === isoDate(anchor.value),
      busy: busyDays.value.has(key),
    }
  })
})

const title = computed(() => {
  const a = anchor.value
  if (view.value === 'month') return a.toLocaleDateString([], { month: 'long', year: 'numeric' })
  if (view.value === 'day' || phoneDay.value)
    return a.toLocaleDateString([], { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' })
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
/** Phones: the strip's arrows move a week (the day keeps its weekday). */
const stepWeek = (n: number) => go({ date: addDays(anchor.value, n * 7) })

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
/** What the grid draws: on a phone the whole week is loaded, one day shown. */
const gridEvents = computed(() => {
  if (!phoneDay.value) return shown.value
  const from = anchor.value.getTime()
  const to = addDays(anchor.value, 1).getTime()
  return shown.value.filter((e) => {
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
// ---- Moving by dragging (step 21) -----------------------------------------------------
// The week view says where an event was dropped; here we ask what needs
// asking (which occurrences, whether to tell the guests), save, and offer Undo.

interface MoveChange {
  event: AgendaEvent
  start: number
  end: number
}
const moveAsk = ref<{ change: MoveChange; step: 'scope' | 'guests'; scopes: Scope[]; scope: Scope } | null>(null)

/** Microsoft and Google email the guests when an event moves. */
function tellsGuests(e: AgendaEvent) {
  const source = calendarMap.value.get(e.calendar)?.source
  return (source === 'microsoft' || source === 'google') && e.attendees.length > 0
}

function onMove(change: MoveChange) {
  const e = change.event
  if (e.recurring) {
    // Like the editor: "this and the following" isn't offered on the first one, nor for a rule set elsewhere.
    const following: Scope[] = e.first_occurrence || !e.repeat ? [] : ['following']
    moveAsk.value = { change, step: 'scope', scopes: ['this', ...following, 'all'], scope: 'this' }
  } else if (tellsGuests(e)) {
    moveAsk.value = { change, step: 'guests', scopes: [], scope: 'this' }
  } else {
    doMove(change, 'this')
  }
}

function scopeChosen(scope: Scope) {
  const ask = moveAsk.value
  if (!ask) return
  if (tellsGuests(ask.change.event)) moveAsk.value = { ...ask, step: 'guests', scope }
  else {
    moveAsk.value = null
    doMove(ask.change, scope)
  }
}

function guestsAgreed() {
  const ask = moveAsk.value
  moveAsk.value = null
  if (ask) doMove(ask.change, ask.scope)
}

const whenLabel = (t: number) => {
  const d = new Date(t * 1000)
  return `${d.toLocaleDateString([], { weekday: 'short', day: 'numeric', month: 'short' })} ${timeLabel(d)}`
}

async function doMove({ event, start, end }: MoveChange, scope: Scope) {
  // Shown at its new place right away; put back if saving fails.
  const before = events.value
  events.value = events.value.map((x) => (x.id === event.id ? { ...x, start, end } : x))
  try {
    const moved = await updateEvent(event.id, movedInput(event, start, end, scope), scope)
    await load()
    if (selectedId.value === event.id) selectedId.value = moved.id
    // Undo puts one event back; a series change is undone in the editor.
    const undo =
      scope === 'this'
        ? {
            label: 'Undo',
            run: async () => {
              try {
                await updateEvent(moved.id, movedInput(moved, event.start, event.end, 'this'), 'this')
                await load()
                toast.show(`“${event.title}” is back at ${whenLabel(event.start)}`)
              } catch (err) {
                toast.show(apiErrorMessage(err), { tone: 'error' })
              }
            },
          }
        : undefined
    const which = scope === 'all' ? ' (every event)' : scope === 'following' ? ' (and the following ones)' : ''
    // Only the end changed (its bottom edge was dragged): say that.
    const what =
      start === event.start
        ? `“${event.title}” now ends at ${timeLabel(new Date(end * 1000))}`
        : `Moved “${event.title}” to ${whenLabel(start)}`
    toast.show(`${what}${which}`, { action: undo, duration: 8000 })
  } catch (err) {
    events.value = before
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
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
              <UButton
                color="primary"
                variant="solid"
                size="sm"
                type="button"
                :disabled="foundBusy === foundKey(f)"
                @click="acceptFound(f)"
              >
                {{ f.view.can_respond ? 'Accept' : 'Add' }}
              </UButton>
              <UButton
                color="neutral"
                variant="outline"
                size="sm"
                type="button"
                @click="putOff(foundKey(f))"
              >
                {{ f.view.can_respond ? 'Later' : 'Dismiss' }}
              </UButton>
              <UButton color="primary" variant="link" size="sm" class="p-0" :to="messageLink(f)">
                View email
              </UButton>
            </div>
          </div>
          <div v-for="e in shownPending" :key="e.id" class="fcard">
            <div>
              <div class="ft">{{ e.title }}</div>
              <div class="fw">{{ foundWhen(e, e.organizer) }}</div>
            </div>
            <div class="facts">
              <UButton
                color="primary"
                variant="solid"
                size="sm"
                type="button"
                :disabled="foundBusy === `e:${e.id}`"
                @click="acceptPending(e)"
              >
                Accept
              </UButton>
              <UButton
                color="neutral"
                variant="outline"
                size="sm"
                type="button"
                @click="putOff(`e:${e.id}`)"
              >
                Later
              </UButton>
              <UButton
                color="primary"
                variant="link"
                size="sm"
                class="p-0"
                type="button"
                @click="openEvent(e)"
              >
                Open
              </UButton>
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
        <UButton
          icon="i-havre-refresh"
          color="neutral"
          variant="ghost"
          size="sm"
          square
          type="button"
          :ui="{ leadingIcon: syncing ? 'spin' : '' }"
          aria-label="Sync now"
          title="Sync now"
          @click="refresh"
        />
      </div>
    </aside>

    <main class="main">
      <PhoneHeader :title="view === 'month' ? title : weekLabel">
        <template #actions>
          <UButton color="neutral" variant="ghost" size="lg" @click="go({ date: startOfDay(new Date()) })">Today</UButton>
          <UButton
            color="neutral"
            variant="ghost"
            size="lg"
            square
            :icon="view === 'month' ? 'i-havre-clock' : 'i-havre-calendar'"
            :aria-label="view === 'month' ? 'Show days' : 'Show the month'"
            @click="go({ view: view === 'month' ? 'day' : 'month' })"
          />
        </template>
        <!-- The week: tap a day to see it, arrows for the weeks around. -->
        <nav v-if="phoneDay" class="strip" aria-label="Days of the week">
          <UButton color="neutral" variant="ghost" square icon="i-havre-back" aria-label="Previous week" @click="stepWeek(-1)" />
          <button
            v-for="d in strip"
            :key="d.key"
            type="button"
            class="sday"
            :class="{ on: d.selected, today: d.today }"
            :aria-label="d.label + (d.busy ? ', has events' : '')"
            :aria-current="d.selected ? 'date' : undefined"
            @click="go({ date: d.date })"
          >
            <span class="mono wd">{{ d.weekday }}</span>
            <span class="num">{{ d.day }}</span>
            <span class="busy" :class="{ shown: d.busy }" aria-hidden="true" />
          </button>
          <UButton color="neutral" variant="ghost" square icon="i-havre-back" class="flip" aria-label="Next week" @click="stepWeek(1)" />
        </nav>
        <p v-if="phoneDay" class="mono faint strip-day">{{ title }}</p>
      </PhoneHeader>
      <div class="toolbar">
        <h2 class="serif title">{{ title }}</h2>
        <span v-if="weekLabel" class="mono faint week">{{ weekLabel }}</span>
        <span v-if="loading" class="loading" aria-label="Loading" />
        <span class="spacer" />
        <UButton
          color="neutral"
          variant="outline"
          type="button"
          title="Today (T)"
          @click="go({ date: startOfDay(new Date()) })"
        >
          Today
        </UButton>
        <div class="arrows">
          <UButton
            icon="i-havre-back"
            color="neutral"
            variant="ghost"
            square
            type="button"
            :aria-label="`Previous ${view}`"
            title="Previous (←)"
            @click="step(-1)"
          />
          <UButton
            icon="i-havre-back"
            color="neutral"
            variant="ghost"
            square
            class="flip"
            type="button"
            :aria-label="`Next ${view}`"
            title="Next (→)"
            @click="step(1)"
          />
        </div>
        <UTabs
          :model-value="view"
          :items="[
            { label: 'Day', value: 'day' },
            { label: 'Week', value: 'week' },
            { label: 'Month', value: 'month' },
          ]"
          :content="false"
          color="neutral"
          size="sm"
          aria-label="View"
          @update:model-value="(v) => go({ view: v as 'day' | 'week' | 'month' })"
        />
        <UButton
          icon="i-havre-plus"
          color="primary"
          variant="solid"
          type="button"
          title="New event (N)"
          @click="create()"
        >
          New event
        </UButton>
      </div>
      <p v-if="loadError" class="load-error" role="alert">{{ loadError }}</p>
      <UButton v-if="phone" icon="i-havre-plus" size="xl" class="new-fab" aria-label="New event" @click="create()">Event</UButton>

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
          :events="gridEvents"
          :calendars="calendarMap"
          :now="now"
          :selected="selectedId"
          :tasks="dueTasks"
          @select="select"
          @create="create"
          @move="onMove"
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

    <!-- Dragging a repeating event: which ones move. -->
    <RepeatScope
      v-if="moveAsk?.step === 'scope'"
      :event="moveAsk.change.event"
      mode="save"
      :scopes="moveAsk.scopes"
      @choose="scopeChosen"
      @cancel="moveAsk = null"
    />
    <!-- Dragging a meeting: its guests get the new time by email. -->
    <UModal
      v-if="moveAsk?.step === 'guests'"
      :open="true"
      :title="`Send the new time to the ${moveAsk.change.event.attendees.length} guest${moveAsk.change.event.attendees.length > 1 ? 's' : ''}?`"
      :description="`“${moveAsk.change.event.title}” moves to ${whenLabel(moveAsk.change.start)}. ${calendarMap.get(moveAsk.change.event.calendar)?.source === 'google' ? 'Google' : 'Microsoft'} emails them the change.`"
      @update:open="(open) => !open && (moveAsk = null)"
    >
      <template #footer>
        <div class="move-actions">
          <UButton color="neutral" variant="outline" @click="moveAsk = null">Don't move</UButton>
          <UButton autofocus @click="guestsAgreed">Move and send</UButton>
        </div>
      </template>
    </UModal>
  </div>
</template>

<style scoped>
.move-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  width: 100%;
}
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
  border-radius: 4px;
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
  border-radius: 4px;
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
  border-radius: 4px;
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
:deep(.spin) {
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
  border-radius: var(--radius);
  border: 2px solid var(--line2);
  border-top-color: var(--accent);
  animation: spin 0.7s linear infinite;
}
.spacer {
  flex: 1;
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
/* ---- Phones: the header and the week strip replace the toolbar ---- */
.strip {
  display: flex;
  align-items: center;
  gap: 2px;
  margin: -4px -6px 0 -10px;
}
.sday {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  padding: 6px 0;
  border: 0;
  border-radius: 4px;
  background: transparent;
  color: var(--text);
  cursor: pointer;
}
.sday .wd {
  font-size: 10.5px;
  color: var(--faint);
  text-transform: uppercase;
}
.sday .num {
  font-size: 16px;
  font-weight: 500;
}
.sday.today .num {
  color: var(--accent);
}
.sday.on {
  background: var(--text);
  color: var(--bg);
}
.sday.on .wd,
.sday.on.today .num {
  color: var(--bg);
}
.busy {
  width: 4px;
  height: 4px;
  border-radius: 50%;
}
.busy.shown {
  background: var(--accent);
}
.sday.on .busy.shown {
  background: var(--bg);
}
.strip-day {
  margin: 6px 0 8px;
  font-size: 12px;
}
.new-fab {
  position: fixed;
  right: 16px;
  bottom: calc(76px + env(safe-area-inset-bottom));
  z-index: 5;
  border-radius: 4px;
  box-shadow: none;
}
@media (max-width: 720px) {
  .toolbar {
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
  border-radius: 4px;
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
</style>
