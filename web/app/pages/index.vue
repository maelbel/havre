<script setup lang="ts">
import type { AgendaEvent, Calendar, Response } from '~/composables/useAgenda'
import type { Related, UnreadMessage } from '~/composables/usePlatform'
import type { NoteSummary } from '~/composables/useNotes'
import type { Task } from '~/composables/useTasks'

// Today (the design's "Brief"): the day in one page. A few written lines
// (the AI brief when AI is on), today's meetings with what to read before
// each, invitations waiting for an answer, unread mail, and tomorrow.
useHead({ title: 'Today · Havre' })
const toast = useToasts()
const accounts = useAccounts()
const calendars = useCalendars()
const calendarMap = computed(() => new Map(calendars.data.value.map((c: Calendar) => [c.id, c])))
const accountMap = computed(() => new Map(accounts.data.value.map((a) => [a.id, a])))

const now = ref(new Date())
const today = computed(() => startOfDay(now.value))
const tomorrow = computed(() => addDays(today.value, 1))
let clock: ReturnType<typeof setInterval> | undefined
onMounted(() => (clock = setInterval(() => (now.value = new Date()), 60_000)))
onBeforeUnmount(() => clearInterval(clock))

const greeting = computed(() => {
  const h = now.value.getHours()
  return h < 5 ? 'Good night' : h < 12 ? 'Good morning' : h < 18 ? 'Good afternoon' : 'Good evening'
})
const name = computed(() => accounts.data.value.find((a) => a.sender_name)?.sender_name.split(' ')[0] ?? '')

// ---- Data ------------------------------------------------------------------------------

const events = ref<AgendaEvent[]>([])
const pending = ref<AgendaEvent[]>([])
const unread = ref<{ total: number; messages: UnreadMessage[] }>({ total: 0, messages: [] })
const related = ref<Record<number, Related>>({})
const loaded = ref(false)

async function load() {
  // A week ahead: tomorrow's list, and conflicts for invitations.
  const from = Math.floor(today.value.getTime() / 1000) - 86_400
  const to = Math.floor(addDays(today.value, 8).getTime() / 1000)
  // Each part on its own: the Agenda failing shouldn't hide your mail.
  const [e, p, u] = await Promise.allSettled([fetchEvents(from, to), fetchPendingInvitations(), fetchUnread(12)])
  if (e.status === 'fulfilled') events.value = e.value
  if (p.status === 'fulfilled') pending.value = p.value
  if (u.status === 'fulfilled') unread.value = u.value
  await Promise.all([loadTasks(), loadNotes()])
  loaded.value = true
  loadRelated()
}
onMounted(load)

// ---- Tasks due (when the Tasks app is in the dock) -----------------------------------
const tasksOn = useTasksEnabled()
const { byId: taskLists } = useTaskLists()
const dueTasks = ref<Task[]>([])
async function loadTasks() {
  if (!tasksOn.value) return (dueTasks.value = [])
  try {
    // Late ones and today's, still to do, plus those ticked today.
    const open = await fetchTasks({ to: localDay() + 86_400 })
    dueTasks.value = open
      .filter((t) => t.due != null && (!t.done_at || t.done_at * 1000 >= startOfDay(new Date()).getTime()))
      .sort((a, b) => Number(!!a.done_at) - Number(!!b.done_at) || byDue(a, b))
  } catch {
    dueTasks.value = []
  }
}
watch(tasksOn, loadTasks)
const lateCount = computed(() => dueTasks.value.filter((t) => isLate(t)).length)
const openDue = computed(() => dueTasks.value.filter((t) => !t.done_at).length)
async function toggleTask(task: Task) {
  try {
    const saved = await setTaskDone(task.id, !task.done_at)
    dueTasks.value = dueTasks.value.map((t) => (t.id === saved.id ? saved : t))
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
function lateLabel(t: Task) {
  const days = Math.round((localDay() - (t.due ?? 0)) / 86_400)
  return days === 1 ? '1 day late' : `${days} days late`
}

// ---- Meeting notes (when the Notes app is in the dock) -------------------------------
const notesOn = useNotesEnabled()
const meetingNotes = ref<NoteSummary[]>([])
async function loadNotes() {
  if (!notesOn.value) return (meetingNotes.value = [])
  meetingNotes.value = (await fetchNotes().catch(() => [])).filter((n) => n.event)
}
watch(notesOn, loadNotes)
function noteFor(e: AgendaEvent) {
  const uid = e.uid || `havre-event-${e.id}`
  return meetingNotes.value.find((n) => n.event?.uid === uid && (!e.recurring || n.event.start === e.start))
}
async function takeNotes(e: AgendaEvent) {
  try {
    await openEventNote(e)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

/** "To prepare": the latest mail from each upcoming meeting's people. */
async function loadRelated() {
  const ahead = todays.value.filter((e) => !e.all_day && e.end * 1000 > Date.now() && (e.attendees.length || e.organizer))
  for (const e of ahead.slice(0, 6)) {
    if (related.value[e.id]) continue
    fetchRelated(e.id)
      .then((r) => (related.value = { ...related.value, [e.id]: r }))
      .catch(() => {})
  }
}

let reload: ReturnType<typeof setTimeout> | undefined
useMailEvents((event) => {
  if (event.type === 'status') return
  clearTimeout(reload)
  reload = setTimeout(load, 400)
})

/** Events touching a local day, all-day ones first; declined ones left out. */
function on(day: Date) {
  const from = day.getTime()
  const to = addDays(day, 1).getTime()
  return events.value
    .filter((e) => e.my_response !== 'declined')
    .filter((e) => {
      const { start, end } = eventSpan(e)
      return start.getTime() < to && end.getTime() > from
    })
    .sort((a, b) => Number(b.all_day) - Number(a.all_day) || a.start - b.start)
}
const todays = computed(() => on(today.value))
const tomorrows = computed(() => on(tomorrow.value))
const timed = computed(() => todays.value.filter((e) => !e.all_day))

const color = (e: AgendaEvent) => calendarMap.value.get(e.calendar)?.color ?? 'var(--accent)'
function duration(e: AgendaEvent) {
  const mins = Math.round((e.end - e.start) / 60)
  if (mins < 60) return `${mins} min`
  const h = Math.floor(mins / 60)
  return mins % 60 ? `${h} h ${String(mins % 60).padStart(2, '0')}` : `${h} h`
}
const isPast = (e: AgendaEvent) => e.end * 1000 < now.value.getTime()
function eventLink(e: AgendaEvent) {
  return { path: '/agenda', query: { view: 'day', date: isoDate(eventSpan(e).start), event: String(e.id) } }
}
const prep = (e: AgendaEvent) => related.value[e.id]?.messages[0]

// ---- The brief -------------------------------------------------------------------------
// With AI: a few sentences written by the model (once a day, or ahead at
// the time chosen in Settings). Without: the same shape, counted.
const { status: ai } = useAiStatus()
const brief = ref('')
const briefWarning = ref<string | null>(null)
const briefBusy = ref(false)
let briefAbort: AbortController | null = null
async function writeBrief(refresh = false) {
  briefAbort?.abort()
  briefAbort = new AbortController()
  briefBusy.value = true
  briefWarning.value = null
  if (refresh) brief.value = ''
  try {
    await streamAi(
      '/api/ai/brief',
      { utc_offset: utcOffset(), refresh },
      (t) => {
        const split = splitWarning(t)
        brief.value = split.text.replace(/^[-•*]\s*/gm, '').trim()
        briefWarning.value = split.warning
      },
      briefAbort.signal,
    )
    useAiStatus().refresh()
  } catch (err) {
    if ((err as Error).name !== 'AbortError') briefWarning.value = (err as Error).message
  } finally {
    briefBusy.value = false
  }
}
watch(
  () => ai.value.enabled,
  (on) => {
    if (on && !brief.value && !briefBusy.value) writeBrief()
  },
  { immediate: true },
)
onBeforeUnmount(() => briefAbort?.abort())

const counted = computed(() => {
  if (!loaded.value) return ''
  const n = timed.value.filter((e) => !isPast(e)).length
  const parts = [n === 0 ? 'Nothing left on the calendar today' : `${n} meeting${n > 1 ? 's' : ''} ahead today`]
  if (openDue.value) parts.push(`${openDue.value} task${openDue.value > 1 ? 's' : ''} due${lateCount.value ? ` (${lateCount.value} late)` : ''}`)
  if (pending.value.length) parts.push(`${pending.value.length} invitation${pending.value.length > 1 ? 's' : ''} to answer`)
  if (unread.value.total) parts.push(`${unread.value.total} unread message${unread.value.total > 1 ? 's' : ''}`)
  return parts.length > 1 ? `${parts.slice(0, -1).join(', ')} and ${parts.at(-1)}.` : `${parts[0]}.`
})

// ---- Answering invitations ------------------------------------------------------------

/** "you're free", or what it overlaps. */
function freeLabel(e: AgendaEvent) {
  const clash = events.value.find(
    (o) => o.id !== e.id && !o.all_day && o.my_response !== 'declined' && o.start < e.end && o.end > e.start,
  )
  return clash ? `overlaps ${clash.title}` : 'you’re free'
}
function invitedLabel(e: AgendaEvent) {
  const who = nameIn(e.organizer)
  return who ? `${who} invites you to ${e.title}` : e.title
}
const answering = ref<number | null>(null)
async function answer(e: AgendaEvent, response: Response) {
  answering.value = e.id
  try {
    const updated = await respondToEvent(e.id, response)
    pending.value = pending.value.filter((p) => p.id !== e.id)
    toast.show(`${RESPONSE_LABELS[response]}: ${updated.title}`)
    load()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    answering.value = null
  }
}

const briefTime = computed(() =>
  ai.value.brief_at ? new Date(ai.value.brief_at * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : null,
)
</script>

<template>
  <main class="today">
    <div class="wrap">
      <div class="col">
        <header class="head">
          <div class="mono date">{{ now.toLocaleDateString([], { weekday: 'long', day: 'numeric', month: 'long' }) }}</div>
          <h1 class="serif">{{ greeting }}{{ name ? `, ${name}` : '' }}.</h1>
          <p v-if="ai.enabled && brief" class="serif lead" aria-live="polite">{{ brief }}</p>
          <p v-else-if="ai.enabled && briefBusy" class="serif lead faint">Reading your day…</p>
          <p v-else class="serif lead">{{ counted }}</p>
          <p v-if="briefWarning" class="warn">{{ briefWarning }}</p>
        </header>

        <section aria-labelledby="t-today" class="block">
          <h2 id="t-today" class="eyebrow-h">Today</h2>
          <p v-if="loaded && !todays.length" class="empty">A free day.</p>
          <div v-for="e in todays" :key="e.id" class="slot">
            <span class="mono time">{{ e.all_day ? 'All day' : timeLabel(new Date(e.start * 1000)) }}</span>
            <div class="what">
              <NuxtLink :to="eventLink(e)" class="title-row">
                <span class="dot" :style="{ background: color(e) }" aria-hidden="true" />
                <span class="title">{{ e.title }}</span>
                <span class="faint small">· {{ isPast(e) ? 'done' : e.all_day ? 'all day' : duration(e) }}</span>
                <span v-if="e.my_response === 'needs_action'" class="tag">to answer</span>
              </NuxtLink>
              <div v-if="!isPast(e) && prep(e)" class="prep">
                <span class="strong">To prepare: </span>
                {{ prep(e)!.from }} wrote “{{ prep(e)!.subject }}”<template v-if="prep(e)!.preview"> — {{ prep(e)!.preview!.slice(0, 140) }}{{ prep(e)!.preview!.length > 140 ? '…' : '' }}</template>
                <NuxtLink :to="messageLink(prep(e)!)" class="more">Read it →</NuxtLink>
              </div>
              <div v-if="notesOn && !e.all_day && (noteFor(e) || !isPast(e))" class="note-line">
                <HIcon name="note" :size="13" />
                <NuxtLink v-if="noteFor(e)" :to="{ path: '/notes', query: { note: noteFor(e)!.id } }" class="more">
                  Your note<template v-if="noteFor(e)!.open_todos"> · {{ noteFor(e)!.open_todos }} to-do{{ noteFor(e)!.open_todos === 1 ? '' : 's' }}</template> →
                </NuxtLink>
                <UButton v-else variant="link" size="sm" class="p-0 text-[13px]" :aria-label="`Take notes: ${e.title}`" @click="takeNotes(e)">Take notes</UButton>
              </div>
            </div>
          </div>
        </section>

        <section v-if="tasksOn && dueTasks.length" aria-labelledby="t-tasks" class="block">
          <h2 id="t-tasks" class="eyebrow-h">
            Due today · {{ openDue }}<template v-if="lateCount"> · <span class="late">{{ lateCount }} late</span></template>
          </h2>
          <div class="tasks-due">
            <div v-for="t in dueTasks" :key="t.id" class="due-row" :class="{ late: isLate(t) }">
              <TaskRow :task="t" :list="taskLists.get(t.list)" :hide-list="isLate(t)" compact @toggle="toggleTask(t)" @open="navigateTo({ path: '/tasks', query: { task: t.id } })" />
              <span v-if="isLate(t)" class="late-note">{{ lateLabel(t) }}</span>
            </div>
          </div>
          <NuxtLink :to="{ path: '/tasks', query: { view: 'today' } }" class="more">All tasks →</NuxtLink>
        </section>

        <section v-if="pending.length" aria-labelledby="t-reply" class="block">
          <h2 id="t-reply" class="eyebrow-h">Needs your answer · {{ pending.length }}</h2>
          <div v-for="e in pending" :key="e.id" class="card">
            <span class="avatar" :style="{ '--c': color(e) }" aria-hidden="true">{{ initials(nameIn(e.organizer) || e.title) }}</span>
            <div class="card-main">
              <div class="strong">{{ invitedLabel(e) }}</div>
              <div class="sub">{{ whenLabel(e) }} — {{ freeLabel(e) }}</div>
            </div>
            <div class="actions">
              <UButton size="sm" type="button" :disabled="answering === e.id" @click="answer(e, 'accepted')">
                Accept
              </UButton>
              <UButton
                color="neutral"
                variant="outline"
                size="sm"
                type="button"
                :disabled="answering === e.id"
                @click="answer(e, 'declined')"
              >
                Decline
              </UButton>
              <UButton color="neutral" variant="outline" size="sm" :to="eventLink(e)">Open</UButton>
            </div>
          </div>
        </section>

        <section aria-labelledby="t-unread" class="block">
          <h2 id="t-unread" class="eyebrow-h">Unread<template v-if="unread.total"> · {{ unread.total }}</template></h2>
          <p v-if="loaded && !unread.messages.length" class="empty">
            {{ accounts.data.value.length ? 'Inbox zero.' : 'Add a mail account in Settings to see your mail here.' }}
          </p>
          <NuxtLink v-for="m in unread.messages" :key="`${m.account}-${m.folder}-${m.uid}`" :to="messageLink(m)" class="line">
            <span class="ellipsis"><span class="strong">{{ personLabel(m.from) || 'Unknown sender' }}</span> — {{ m.subject || '(no subject)' }}</span>
            <span class="faint">{{ accountMap.get(m.account)?.name ?? '' }} · {{ shortDate(m.date) }}</span>
          </NuxtLink>
          <NuxtLink v-if="unread.total > unread.messages.length" to="/mail" class="more">All unread mail →</NuxtLink>
        </section>
      </div>

      <aside class="side">
        <div class="box">
          <div class="box-head">
            <span class="strong">Tomorrow</span>
            <span class="mono faint">{{ tomorrow.toLocaleDateString([], { weekday: 'short', day: 'numeric' }) }}</span>
          </div>
          <p v-if="loaded && !tomorrows.length" class="empty">Nothing planned.</p>
          <NuxtLink v-for="e in tomorrows" :key="e.id" :to="eventLink(e)" class="tline">
            <span class="mono time-s">{{ e.all_day ? 'all day' : timeLabel(new Date(e.start * 1000)) }}</span>
            <span :class="{ faint: e.my_response === 'needs_action' }">
              {{ e.title }}<span v-if="e.my_response === 'needs_action'" class="faint"> (if you accept)</span>
            </span>
          </NuxtLink>
          <NuxtLink :to="{ path: '/agenda', query: { view: 'week' } }" class="more">Open the week →</NuxtLink>
        </div>

        <div v-if="ai.enabled" class="box quiet">
          <div class="strong row-i">
            <HIcon :name="ai.local ? 'lock' : 'sparkle'" :size="15" />
            {{ ai.local ? 'Written by the AI app, on your server' : 'Written with ' + (ai.provider === 'claude-code' ? 'Claude' : 'your AI provider') }}
          </div>
          <p>
            <template v-if="briefTime">Prepared at {{ briefTime }}</template><template v-else>Prepared</template>
            by {{ ai.model || 'the default model' }}.
            {{ ai.local ? 'Nothing left your network.' : 'The day’s mail and meetings were sent to it.' }}
          </p>
          <div class="row-links">
            <UButton
              color="primary"
              variant="link"
              class="p-0"
              type="button"
              :disabled="briefBusy"
              @click="writeBrief(true)"
            >
              {{ briefBusy ? 'Writing…' : 'Write again' }}
            </UButton>
            <NuxtLink to="/settings/ai" class="more">AI settings →</NuxtLink>
          </div>
        </div>
      </aside>
    </div>
  </main>
</template>

<style scoped>
.tasks-due {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.due-row {
  position: relative;
  border-radius: 10px;
}
.due-row.late {
  background: color-mix(in oklab, var(--danger) 9%, transparent);
}
.due-row.late :deep(.task) {
  padding-right: 110px;
}
.late-note {
  position: absolute;
  right: 10px;
  top: 8px;
  font-size: 12px;
  color: var(--danger);
}
.late {
  color: var(--danger);
}
.today {
  flex: 1;
  overflow-y: auto;
  background: var(--panel);
}
.wrap {
  max-width: 1080px;
  margin: 0 auto;
  padding: 56px 40px 64px;
  display: flex;
  flex-wrap: wrap;
  gap: 48px;
}
.col {
  flex: 999 1 520px;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 40px;
}
.side {
  flex: 1 1 280px;
  display: flex;
  flex-direction: column;
  gap: 20px;
}
.head {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.date {
  font-size: 12px;
  color: var(--faint);
}
h1 {
  margin: 0;
  font-weight: 400;
  font-size: 46px;
  line-height: 1.05;
  letter-spacing: -0.02em;
}
.lead {
  margin: 0;
  max-width: 560px;
  font-size: 20px;
  line-height: 1.5;
  color: var(--muted);
  white-space: pre-line;
}
.warn {
  margin: 0;
  font-size: 13px;
  color: var(--danger);
}
.block {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.eyebrow-h {
  margin: 0 0 8px;
  font-family: var(--font-mono);
  font-size: 11px;
  font-weight: 400;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--faint);
}
.slot {
  display: flex;
  gap: 16px;
  padding: 12px 0;
  border-top: 1px solid var(--line);
}
.time {
  flex: 0 0 52px;
  font-size: 12.5px;
  color: var(--faint);
  padding-top: 1px;
}
.what {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.title-row {
  display: flex;
  align-items: center;
  gap: 8px;
  color: var(--text);
  text-decoration: none;
}
.title-row:hover .title {
  text-decoration: underline;
}
.dot {
  flex: 0 0 7px;
  height: 7px;
  border-radius: 50%;
}
.title,
.strong {
  font-weight: 500;
  color: var(--text);
}
.small {
  font-size: 13px;
}
.faint {
  color: var(--faint);
}
.tag {
  padding: 1px 8px;
  border-radius: 999px;
  font-size: 11.5px;
  background: color-mix(in oklab, var(--accent) 18%, transparent);
  color: var(--accent);
}
.prep {
  padding: 10px 14px;
  border-radius: 10px;
  background: var(--bg);
  font-size: 13.5px;
  line-height: 1.55;
  color: var(--muted);
}
.more {
  color: var(--accent);
  text-decoration: none;
  font-size: 13px;
}
.note-line {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--faint);
}
.prep .more {
  margin-left: 4px;
}
.more:hover {
  color: var(--text);
}
.card {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 12px 16px;
  padding: 16px;
  margin-bottom: 6px;
  border-radius: 14px;
  border: 1px solid var(--line);
  background: var(--raise);
}
.avatar {
  flex: 0 0 34px;
  height: 34px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  font-weight: 600;
  background: color-mix(in oklab, var(--c) 20%, transparent);
  color: var(--c);
}
.card-main {
  flex: 1 1 260px;
  min-width: 0;
}
.sub {
  font-size: 13px;
  color: var(--muted);
}
.actions {
  display: flex;
  gap: 6px;
}
.line {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  padding: 9px 10px;
  margin: 0 -10px;
  border-radius: 8px;
  color: var(--text);
  text-decoration: none;
  font-size: 13.5px;
}
.line:hover {
  background: var(--hover);
}
.line .faint {
  flex: 0 0 auto;
}
.ellipsis {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.empty {
  margin: 0;
  padding: 12px 0;
  border-top: 1px solid var(--line);
  color: var(--faint);
  font-size: 13.5px;
}
.box {
  padding: 18px 20px;
  border-radius: 16px;
  border: 1px solid var(--line);
  background: var(--bg);
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.box .empty {
  border: 0;
  padding: 0;
}
.box.quiet {
  padding: 16px 20px;
  background: transparent;
  gap: 8px;
  font-size: 13px;
  color: var(--muted);
  line-height: 1.55;
}
.box.quiet p {
  margin: 0;
}
.box-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
}
.box-head .mono {
  font-size: 12px;
}
.tline {
  display: flex;
  gap: 12px;
  font-size: 13.5px;
  color: var(--text);
  text-decoration: none;
}
.tline:hover {
  color: var(--accent);
}
.time-s {
  flex: 0 0 44px;
  font-size: 12px;
  color: var(--faint);
}
.row-i {
  display: flex;
  align-items: center;
  gap: 8px;
}
.row-links {
  display: flex;
  gap: 14px;
  align-items: center;
}
.link {
  border: 0;
  background: none;
  padding: 0;
  color: var(--accent);
  cursor: pointer;
  font: inherit;
  font-size: 13px;
}
@media (max-width: 720px) {
  .wrap {
    padding: 28px 16px 40px;
    gap: 32px;
  }
  h1 {
    font-size: 34px;
  }
  .lead {
    font-size: 18px;
  }
}
</style>
