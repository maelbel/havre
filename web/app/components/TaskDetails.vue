<script setup lang="ts">
import type { AgendaEvent } from '~/composables/useAgenda'
import type { Task, TaskInput, TaskList } from '~/composables/useTasks'

// The panel on the right of Tasks: everything about one task, edited in
// place (each change is saved when you leave the field), plus what it's
// linked to and what's on your agenda that day.
const props = defineProps<{ task: Task; lists: TaskList[] }>()
const emit = defineEmits<{ saved: [task: Task]; deleted: [id: number]; close: [] }>()
const toast = useToasts()

const list = computed(() => props.lists.find((l) => l.id === props.task.list))
const writable = computed(() => props.lists.filter((l) => !l.read_only))

// Local copies of the fields, reset when another task is shown.
const title = ref('')
const notes = ref('')
const dueIso = ref('')
const time = ref('')
function reset() {
  title.value = props.task.title
  notes.value = props.task.notes
  dueIso.value = dayToIso(props.task.due)
  time.value = props.task.due_at != null ? hhmmOf(props.task.due_at) : ''
}
watch(() => [props.task.id, props.task.updated_at], reset, { immediate: true })

function hhmmOf(ts: number) {
  const d = new Date(ts * 1000)
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

const saving = ref(false)
async function save(change: Partial<TaskInput>) {
  if (props.task.read_only) return
  saving.value = true
  try {
    const saved = await updateTask(props.task.id, taskInput(props.task, change))
    emit('saved', saved)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
    reset()
  } finally {
    saving.value = false
  }
}

function saveTitle() {
  const t = title.value.trim()
  if (!t) return reset()
  if (t !== props.task.title) save({ title: t })
}
function saveNotes() {
  if (notes.value !== props.task.notes) save({ notes: notes.value })
}
function saveDue() {
  const due = isoToDay(dueIso.value)
  const dueAt = due != null && time.value ? atTime(due, time.value) : null
  if (due === props.task.due && dueAt === props.task.due_at) return
  // A reminder tied to the old time would be wrong now: keep it only if
  // the day is the same.
  const remind = due === props.task.due ? props.task.remind_at : null
  save({ due, due_at: dueAt, remind_at: remind })
}
function clearDue() {
  dueIso.value = ''
  time.value = ''
  save({ due: null, due_at: null, remind_at: null })
}
function moveTo(id: number) {
  if (id !== props.task.list) save({ list: id })
}

// ---- Reminder ---------------------------------------------------------------------
const REMINDERS = [
  { id: 'none', label: 'None' },
  { id: 'at', label: 'At the due time' },
  { id: '15', label: '15 minutes before' },
  { id: '60', label: '1 hour before' },
  { id: 'morning', label: 'That morning (09:00)' },
  { id: 'eve', label: 'The evening before (18:00)' },
] as const
type ReminderId = (typeof REMINDERS)[number]['id'] | 'custom'

function reminderFor(id: ReminderId): number | null {
  const t = props.task
  if (t.due == null) return null
  const base = t.due_at ?? atTime(t.due, '09:00')!
  switch (id) {
    case 'at':
      return base
    case '15':
      return base - 15 * 60
    case '60':
      return base - 3600
    case 'morning':
      return atTime(t.due, '09:00')
    case 'eve':
      return atTime(t.due - 86_400, '18:00')
    default:
      return null
  }
}
const reminder = computed<ReminderId>(() => {
  const at = props.task.remind_at
  if (at == null) return 'none'
  for (const r of REMINDERS) if (r.id !== 'none' && reminderFor(r.id) === at) return r.id
  return 'custom'
})
function setReminder(id: ReminderId) {
  // The first reminder: ask the browser for notifications (Havre Desktop
  // has them already).
  if (id !== 'none' && !isDesktop() && 'Notification' in window && Notification.permission === 'default') {
    Notification.requestPermission().catch(() => {})
  }
  save({ remind_at: reminderFor(id) })
}
const customLabel = computed(() =>
  props.task.remind_at ? `${dayText(new Date(props.task.remind_at * 1000))} · ${taskTime(props.task.remind_at)}` : '',
)

// ---- That day on the agenda ------------------------------------------------------
const day = ref<AgendaEvent[]>([])
const dayLoading = ref(false)
watch(
  () => props.task.due,
  async (due) => {
    day.value = []
    if (due == null) return
    const start = dayToDate(due)
    const from = Math.floor(start.getTime() / 1000)
    dayLoading.value = true
    try {
      const events = await fetchEvents(from, from + 86_400)
      // All-day events are stored on UTC days: keep those of this day only.
      const onDay = (e: AgendaEvent) => (e.all_day ? e.start <= due && e.end > due : true)
      day.value = events.filter(onDay).sort((a, b) => Number(b.all_day) - Number(a.all_day) || a.start - b.start).slice(0, 8)
    } catch {
      /* no Agenda (demo) or offline: just don't show it */
    } finally {
      dayLoading.value = false
    }
  },
  { immediate: true },
)
const calendars = useCalendars()
function eventColor(e: AgendaEvent) {
  return calendars.data.value.find((c) => c.id === e.calendar)?.color ?? 'var(--accent)'
}
function eventTime(e: AgendaEvent) {
  return e.all_day ? 'all day' : new Date(e.start * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}
const dayHeading = computed(() => {
  if (props.task.due == null) return ''
  const label = taskDayLabel(props.task.due)
  return `${label === 'Today' || label === 'Tomorrow' ? label : dayToDate(props.task.due).toLocaleDateString('en-GB', { weekday: 'long' })} on your agenda`
})
const agendaLink = computed(() =>
  props.task.due != null ? { path: '/agenda', query: { view: 'day', date: dayToIso(props.task.due) } } : '/agenda',
)

// ---- Links -----------------------------------------------------------------------
async function openMail() {
  try {
    await openTaskMail(props.task)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
const who = computed(() => props.task.person?.name || props.task.person?.email || '')

// ---- Deleting (two clicks) --------------------------------------------------------
const confirming = ref(false)
watch(() => props.task.id, () => (confirming.value = false))
async function remove() {
  if (!confirming.value) {
    confirming.value = true
    return
  }
  try {
    await deleteTask(props.task.id)
    emit('deleted', props.task.id)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
</script>

<template>
  <aside class="details" aria-label="Task details">
    <div class="head">
      <button type="button" class="icon-btn back" aria-label="Back to the list" @click="emit('close')">
        <HIcon name="back" :size="18" />
      </button>
      <span class="where">
        <span class="dot" :style="{ background: list?.color }" aria-hidden="true" />
        {{ list ? `${list.owner} · ${list.name}` : '' }}
      </span>
      <span class="spacer" />
      <span v-if="saving" class="faint small">Saving…</span>
      <button
        v-if="!task.read_only"
        type="button"
        class="icon-btn"
        :class="{ danger: confirming }"
        :aria-label="confirming ? 'Click again to delete the task' : 'Delete task'"
        :title="confirming ? 'Click again to delete' : 'Delete'"
        @click="remove"
      >
        <HIcon name="trash" :size="16" />
      </button>
    </div>

    <div class="scroll">
      <label class="sr-only" for="task-title">Title</label>
      <textarea
        id="task-title"
        v-model="title"
        class="title serif"
        rows="2"
        :readonly="task.read_only"
        @blur="saveTitle"
        @keydown.enter.prevent="($event.target as HTMLTextAreaElement).blur()"
      />
      <p v-if="task.read_only" class="faint small">This list is shared with you read-only.</p>

      <div class="grid">
        <span class="k">Due</span>
        <span class="due">
          <input
            v-model="dueIso"
            type="date"
            aria-label="Due day"
            :disabled="task.read_only"
            :class="{ late: isLate(task) }"
            @change="saveDue"
          />
          <input
            v-model="time"
            type="time"
            aria-label="Due time"
            :disabled="task.read_only || !dueIso"
            @change="saveDue"
          />
          <button
            v-if="task.due != null && !task.read_only"
            type="button"
            class="icon-btn small"
            aria-label="No due date"
            title="No due date"
            @click="clearDue"
          >
            <HIcon name="close" :size="14" />
          </button>
        </span>

        <label class="k" for="task-list">List</label>
        <select
          id="task-list"
          :value="task.list"
          :disabled="task.read_only"
          @change="moveTo(Number(($event.target as HTMLSelectElement).value))"
        >
          <option v-for="l in writable" :key="l.id" :value="l.id">{{ l.owner }} · {{ l.name }}</option>
        </select>

        <label class="k" for="task-remind">Remind</label>
        <select
          id="task-remind"
          :value="reminder"
          :disabled="task.read_only || task.due == null"
          @change="setReminder(($event.target as HTMLSelectElement).value as ReminderId)"
        >
          <option v-for="r in REMINDERS" :key="r.id" :value="r.id">{{ r.label }}</option>
          <option v-if="reminder === 'custom'" value="custom">{{ customLabel }}</option>
        </select>
      </div>
      <p v-if="task.due == null" class="faint small hint">Give it a due day to set a reminder.</p>

      <label class="block">
        <span class="eyebrow">Notes</span>
        <textarea v-model="notes" rows="4" :readonly="task.read_only" placeholder="Add notes" @blur="saveNotes" />
      </label>

      <section v-if="task.mail" class="block" aria-label="From Mail">
        <span class="eyebrow">From Mail</span>
        <button type="button" class="card link" @click="openMail">
          <HIcon name="mail" :size="15" class="c-work" />
          <span class="txt">
            <span class="t">{{ task.mail.subject || '(no subject)' }}</span>
            <span class="s">{{ task.mail.from }}</span>
          </span>
          <span class="open">Open</span>
        </button>
      </section>

      <section v-if="task.person" class="block" aria-label="With">
        <span class="eyebrow">With</span>
        <NuxtLink :to="`/people/${encodeURIComponent(task.person.email)}`" class="person">
          <span class="avatar" :style="{ background: personColor(task.person.email) }">{{ initials(who) }}</span>
          <span class="txt">
            <span class="t">{{ who }}</span>
            <span class="s">{{ task.person.email }}</span>
          </span>
        </NuxtLink>
      </section>

      <section v-if="task.event" class="block" aria-label="For the meeting">
        <span class="eyebrow">For</span>
        <NuxtLink :to="{ path: '/agenda', query: { view: 'day', date: dayToIso(localDay(new Date(task.event.start * 1000))) } }" class="card link">
          <HIcon name="calendar" :size="15" class="c-lyon" />
          <span class="txt">
            <span class="t">{{ task.event.title }}</span>
            <span class="s">{{ dayText(new Date(task.event.start * 1000)) }} · {{ taskTime(task.event.start) }}</span>
          </span>
        </NuxtLink>
      </section>

      <section v-if="task.due != null" class="block" :aria-label="dayHeading">
        <span class="eyebrow">{{ dayHeading }}</span>
        <div class="day">
          <p v-if="dayLoading" class="faint small pad">Loading…</p>
          <p v-else-if="!day.length" class="faint small pad">Nothing on your agenda.</p>
          <div v-for="e in day" :key="e.id" class="ev">
            <span class="time mono">{{ eventTime(e) }}</span>
            <span class="bar" :style="{ background: eventColor(e) }" aria-hidden="true" />
            <span class="name">{{ e.title }}</span>
          </div>
          <NuxtLink :to="agendaLink" class="more">Open the day in Agenda →</NuxtLink>
        </div>
      </section>
    </div>
  </aside>
</template>

<style scoped>
.details {
  flex: 0 1 380px;
  min-width: 300px;
  display: flex;
  flex-direction: column;
  border-left: 1px solid var(--line);
  background: var(--bg);
}
.head {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 56px;
  padding: 0 12px 0 20px;
  border-bottom: 1px solid var(--line);
  flex: none;
}
.back {
  display: none;
}
.where {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  font-size: 12.5px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 2px;
  flex: none;
}
.spacer {
  flex: 1;
}
.danger {
  color: var(--danger);
  background: color-mix(in oklab, var(--danger) 14%, transparent);
}
.scroll {
  flex: 1;
  overflow-y: auto;
  padding: 18px 22px 28px;
  display: flex;
  flex-direction: column;
  gap: 18px;
}
.title {
  width: 100%;
  margin: 0;
  padding: 4px 6px;
  margin-left: -6px;
  border: 1px solid transparent;
  border-radius: 8px;
  background: transparent;
  color: var(--text);
  font-size: 24px;
  line-height: 1.2;
  resize: none;
  field-sizing: content;
  min-height: 0;
}
.title:hover:not([readonly]),
.title:focus {
  border-color: var(--line2);
  outline: none;
}
.grid {
  display: grid;
  grid-template-columns: 76px 1fr;
  gap: 10px 12px;
  align-items: center;
  font-size: 13.5px;
}
.k {
  color: var(--faint);
  font-size: 12.5px;
}
.due {
  display: flex;
  gap: 6px;
  align-items: center;
  min-width: 0;
}
input[type='date'],
input[type='time'],
select {
  height: 32px;
  padding: 0 8px;
  border: 1px solid var(--line2);
  border-radius: 8px;
  background: transparent;
  color: var(--text);
  font: inherit;
  font-size: 13px;
  min-width: 0;
}
input[type='time'] {
  width: 116px;
}
select {
  background: var(--bg);
  max-width: 100%;
}
input.late {
  color: var(--danger);
}
.icon-btn.small {
  width: 28px;
  height: 28px;
}
.hint {
  margin: -10px 0 0 88px;
}
.small {
  font-size: 12.5px;
}
.faint {
  color: var(--faint);
  margin: 0;
}
.block {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.block textarea {
  padding: 10px 12px;
  border: 1px solid var(--line2);
  border-radius: 10px;
  background: var(--panel);
  color: var(--text);
  font: inherit;
  font-size: 13.5px;
  line-height: 1.55;
  resize: vertical;
}
.card {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border-radius: 10px;
  border: 1px solid var(--line);
  background: var(--panel);
  color: var(--text);
  text-decoration: none;
  text-align: left;
  cursor: pointer;
  width: 100%;
}
.card:hover,
.person:hover {
  background: var(--hover);
}
.c-work {
  color: var(--c-work);
  flex: none;
}
.c-lyon {
  color: var(--c-lyon);
  flex: none;
}
.txt {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.t {
  font-size: 13.5px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.s {
  font-size: 12px;
  color: var(--faint);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.open {
  font-size: 12px;
  color: var(--accent);
}
.person {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 4px;
  margin: -4px;
  border-radius: 10px;
  color: var(--text);
  text-decoration: none;
}
.avatar {
  flex: 0 0 30px;
  height: 30px;
  border-radius: 50%;
  display: grid;
  place-items: center;
  font-size: 11px;
  font-weight: 600;
  color: #1b1b1b;
}
.day {
  border-radius: 10px;
  border: 1px solid var(--line);
  background: var(--panel);
  overflow: hidden;
}
.pad {
  padding: 10px 12px;
}
.ev {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--line);
  font-size: 13px;
}
.time {
  flex: 0 0 48px;
  font-size: 11.5px;
  color: var(--faint);
}
.bar {
  flex: 0 0 3px;
  height: 18px;
  border-radius: 2px;
}
.name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.more {
  display: block;
  padding: 8px 12px;
  font-size: 12.5px;
  text-decoration: none;
}
@media (max-width: 720px) {
  .details {
    position: fixed;
    inset: 0 0 60px 0;
    z-index: 20;
    min-width: 0;
    border-left: 0;
  }
  .head {
    padding-left: 8px;
  }
  .back {
    display: inline-flex;
    width: 44px;
    height: 44px;
  }
}
</style>
