<script setup lang="ts">
import type { AgendaEvent, Calendar, EventInput } from '~/composables/useAgenda'

// Create or edit an event (a dialog over the Agenda).
const props = defineProps<{
  /** Editing this event; otherwise a new one from `start`/`end`. */
  event?: AgendaEvent | null
  start?: Date
  end?: Date
  allDay?: boolean
  /** A new event: guests to start with (e.g. "Schedule" on a person). */
  guests?: string[]
  calendars: Calendar[]
}>()
const emit = defineEmits<{ saved: [event: AgendaEvent]; deleted: []; cancel: [] }>()

const writable = computed(() => props.calendars.filter((c) => !c.read_only && (c.enabled || c.id === props.event?.calendar)))
const defaultCalendar = computed(() => writable.value.find((c) => c.is_default) ?? writable.value[0])

const pad = (n: number) => String(n).padStart(2, '0')
const timeOf = (d: Date) => `${pad(d.getHours())}:${pad(d.getMinutes())}`

// The form works in local dates and times; all-day end is the last day
// (inclusive) here, the day after (exclusive) on the server.
const initial = (() => {
  if (props.event) {
    const { start, end } = eventSpan(props.event)
    return {
      start,
      end: props.event.all_day ? addDays(end, -1) : end,
      allDay: props.event.all_day,
    }
  }
  const start = props.start ?? new Date(Math.ceil(Date.now() / 1_800_000) * 1_800_000)
  const end = props.end ?? new Date(start.getTime() + 3_600_000)
  return { start, end: props.allDay ? addDays(end, -1) : end, allDay: !!props.allDay }
})()

const form = reactive({
  title: props.event?.title ?? '',
  calendar: props.event?.calendar ?? defaultCalendar.value?.id ?? 0,
  allDay: initial.allDay,
  startDate: isoDate(initial.start),
  startTime: timeOf(initial.start),
  endDate: isoDate(initial.end),
  endTime: timeOf(initial.end),
  location: props.event?.location ?? '',
  description: props.event?.description ?? '',
  guests: props.event?.attendees.map((a) => a.email).join(', ') ?? props.guests?.join(', ') ?? '',
})

const calendar = computed(() => props.calendars.find((c) => c.id === form.calendar))
/** Microsoft and Google send invitations; CalDAV and Havre's calendar don't. */
const canInvite = computed(() => calendar.value?.source === 'microsoft' || calendar.value?.source === 'google')

function at(date: string, time: string) {
  const d = parseIsoDate(date) ?? new Date()
  const [h, m] = time.split(':').map(Number)
  return new Date(d.getFullYear(), d.getMonth(), d.getDate(), h || 0, m || 0)
}

// Moving the start keeps the duration.
let lastStart = at(form.startDate, form.startTime).getTime()
watch(
  () => [form.startDate, form.startTime],
  () => {
    const start = at(form.startDate, form.startTime).getTime()
    const end = at(form.endDate, form.endTime).getTime() + (start - lastStart)
    lastStart = start
    const e = new Date(end)
    form.endDate = isoDate(e)
    form.endTime = timeOf(e)
  },
)

const busy = ref(false)
const error = ref<string | null>(null)
const confirmDelete = ref(false)
const titleInput = ref<HTMLInputElement | null>(null)
onMounted(() => titleInput.value?.focus())

function input(): EventInput | string {
  if (!form.title.trim()) return 'Give the event a title.'
  let start: number, end: number
  if (form.allDay) {
    const first = parseIsoDate(form.startDate)
    const last = parseIsoDate(form.endDate)
    if (!first || !last) return 'Check the dates.'
    start = localDateToUtc(first)
    end = localDateToUtc(addDays(last, 1))
  } else {
    start = at(form.startDate, form.startTime).getTime() / 1000
    end = at(form.endDate, form.endTime).getTime() / 1000
  }
  if (end <= start) return 'The event must end after it starts.'
  return {
    calendar: form.calendar,
    title: form.title.trim(),
    location: form.location.trim(),
    description: form.description,
    start,
    end,
    all_day: form.allDay,
    attendees: canInvite.value
      ? // Just the addresses, whatever surrounds them ("Anna <anna@x.org>").
        [...new Set(form.guests.match(/[^\s<>,;"]+@[^\s<>,;"]+/g) ?? [])]
      : [],
  }
}

async function save() {
  error.value = null
  const body = input()
  if (typeof body === 'string') return void (error.value = body)
  busy.value = true
  try {
    emit('saved', props.event ? await updateEvent(props.event.id, body) : await createEvent(body))
  } catch (err) {
    error.value = apiErrorMessage(err)
  } finally {
    busy.value = false
  }
}

async function remove() {
  if (!props.event) return
  if (!confirmDelete.value) {
    confirmDelete.value = true
    setTimeout(() => (confirmDelete.value = false), 3000)
    return
  }
  busy.value = true
  try {
    await deleteEvent(props.event.id)
    emit('deleted')
  } catch (err) {
    error.value = apiErrorMessage(err)
  } finally {
    busy.value = false
  }
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    e.stopPropagation()
    emit('cancel')
  } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
    e.preventDefault()
    save()
  }
}
</script>

<template>
  <div class="backdrop" @mousedown.self="emit('cancel')">
    <form class="editor" role="dialog" :aria-label="event ? 'Edit event' : 'New event'" @submit.prevent="save" @keydown="onKeydown">
      <input ref="titleInput" v-model="form.title" class="title" type="text" placeholder="Add a title" aria-label="Title" />

      <div class="row">
        <label class="lbl" for="ev-cal">Calendar</label>
        <div class="cal">
          <span class="dot" :style="{ background: calendar?.color }" />
          <select id="ev-cal" v-model.number="form.calendar">
            <option v-for="c in writable" :key="c.id" :value="c.id">{{ c.name }} · {{ SOURCE_LABELS[c.source] }}</option>
          </select>
        </div>
      </div>

      <div class="row">
        <span class="lbl">When</span>
        <div class="when">
          <div class="line">
            <span class="to">from</span>
            <input v-model="form.startDate" type="date" aria-label="Start date" required />
            <input v-if="!form.allDay" v-model="form.startTime" type="time" step="300" aria-label="Start time" required />
          </div>
          <div class="line">
            <span class="to">to</span>
            <input v-model="form.endDate" type="date" aria-label="End date" required />
            <input v-if="!form.allDay" v-model="form.endTime" type="time" step="300" aria-label="End time" required />
          </div>
          <label class="check">
            <input v-model="form.allDay" type="checkbox" />
            All day
          </label>
        </div>
      </div>

      <div class="row">
        <label class="lbl" for="ev-loc">Where</label>
        <input id="ev-loc" v-model="form.location" type="text" placeholder="Place or meeting link" />
      </div>

      <div v-if="canInvite" class="row">
        <label class="lbl" for="rcpt-guests">Guests</label>
        <div class="stack">
          <RecipientInput v-model="form.guests" label="Guests" bare address-only placeholder="anna@example.com, bob@example.com" />
          <small class="hint">{{ calendar?.source === 'google' ? 'Google' : 'Microsoft' }} emails them an invitation.</small>
        </div>
      </div>

      <div class="row top">
        <label class="lbl" for="ev-notes">Notes</label>
        <textarea id="ev-notes" v-model="form.description" rows="3" placeholder="Agenda, address, dial-in…" />
      </div>

      <div v-if="error" class="error" role="alert">
        <HIcon name="alert" :size="16" />
        <span>{{ error }}</span>
      </div>

      <div class="actions">
        <button
          v-if="event"
          type="button"
          class="btn"
          :class="{ danger: confirmDelete }"
          :disabled="busy"
          @click="remove"
        >
          <HIcon name="trash" :size="15" />
          {{ confirmDelete ? 'Delete for good?' : 'Delete' }}
        </button>
        <span class="spacer" />
        <button type="button" class="btn" :disabled="busy" @click="emit('cancel')">Cancel</button>
        <button type="submit" class="btn btn-primary" :disabled="busy || !writable.length">
          <span v-if="busy" class="spinner" aria-hidden="true" />
          {{ event ? 'Save' : 'Create' }}
        </button>
      </div>
    </form>
  </div>
</template>

<style scoped>
.backdrop {
  position: fixed;
  inset: 0;
  z-index: 40;
  display: grid;
  place-items: center;
  padding: 16px;
  background: rgb(0 0 0 / 0.35);
}
.editor {
  width: min(560px, 100%);
  max-height: calc(100vh - 32px);
  overflow: auto;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 22px 22px 18px;
  border-radius: var(--radius-lg);
  border: 1px solid var(--line2);
  background: var(--panel);
  box-shadow: 0 30px 80px -20px rgb(0 0 0 / 0.55);
}
.title {
  height: 44px;
  padding: 0 2px;
  border: 0;
  border-bottom: 1px solid var(--line2);
  background: transparent;
  color: var(--text);
  font-family: var(--font-serif);
  font-size: 24px;
  outline: none;
}
.title:focus {
  border-bottom-color: var(--accent);
}
.row {
  display: grid;
  grid-template-columns: 80px minmax(0, 1fr);
  align-items: center;
  gap: 12px;
}
.row.top {
  align-items: start;
}
.row.top .lbl {
  padding-top: 10px;
}
.lbl {
  font-size: 12.5px;
  color: var(--muted);
}
input[type='text'],
input[type='date'],
input[type='time'],
select,
textarea {
  height: 38px;
  padding: 0 10px;
  border-radius: 9px;
  border: 1px solid var(--line2);
  background: var(--bg);
  color: var(--text);
  font: inherit;
  font-size: 14px;
  outline: none;
  color-scheme: inherit;
}
textarea {
  height: auto;
  padding: 9px 10px;
  resize: vertical;
  line-height: 1.5;
}
input:focus,
select:focus,
textarea:focus {
  border-color: var(--accent);
}
.cal {
  position: relative;
  display: flex;
  align-items: center;
}
.cal .dot {
  position: absolute;
  left: 11px;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  pointer-events: none;
}
.cal select {
  width: 100%;
  padding-left: 30px;
}
.when {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.line {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
}
.to {
  width: 34px;
  color: var(--faint);
  font-size: 13px;
}
.check {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: var(--muted);
  cursor: pointer;
}
.check input {
  accent-color: var(--accent);
}
.stack {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.hint {
  font-size: 12px;
  color: var(--faint);
}
.error {
  display: flex;
  gap: 10px;
  padding: 10px 12px;
  border-radius: var(--radius);
  background: color-mix(in oklab, var(--danger) 12%, transparent);
  color: var(--danger);
  font-size: 13px;
}
.error span {
  color: var(--text);
}
.actions {
  display: flex;
  align-items: center;
  gap: 8px;
  padding-top: 4px;
}
.spacer {
  flex: 1;
}
.btn.danger {
  border-color: color-mix(in oklab, var(--danger) 50%, transparent);
  background: color-mix(in oklab, var(--danger) 12%, transparent);
  color: var(--danger);
}
.btn:disabled {
  opacity: 0.6;
  cursor: default;
}
.spinner {
  width: 14px;
  height: 14px;
  border-radius: 50%;
  border: 2px solid color-mix(in oklab, var(--on-accent) 30%, transparent);
  border-top-color: var(--on-accent);
  animation: spin 0.7s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
@media (max-width: 560px) {
  .row {
    grid-template-columns: minmax(0, 1fr);
    gap: 4px;
  }
}
</style>
