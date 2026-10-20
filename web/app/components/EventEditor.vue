<script setup lang="ts">
import type { AgendaEvent, Calendar, EventInput, Repeat, Scope } from '~/composables/useAgenda'

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
  repeat: (props.event?.repeat ? structuredClone(toRaw(props.event.repeat)) : null) as Repeat | null,
})

/** An occurrence of a series whose rule was made in another app: Havre keeps that rule. */
const ruleElsewhere = computed(() => !!props.event?.recurring && !props.event.repeat)
/** The first event, which the rule follows (its weekday, its date in the month). */
const firstDay = computed(() => (form.allDay ? (parseIsoDate(form.startDate) ?? new Date()) : at(form.startDate, form.startTime)))

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
    repeat: repeatToSend(form.repeat),
    time_zone: browserTimeZone(),
  }
}

// An occurrence of a repeating event: which ones the change is for.
const asking = ref<{ mode: 'save' | 'delete'; scopes: Scope[]; body?: EventInput } | null>(null)

function saveScopes(body: EventInput): Scope[] {
  const e = props.event!
  const following: Scope[] = e.first_occurrence ? [] : ['following']
  // Another calendar: the series moves as a whole.
  if (body.calendar !== e.calendar) return ['all']
  // A new rule doesn't apply to one event alone.
  if (!ruleElsewhere.value && JSON.stringify(body.repeat) !== JSON.stringify(repeatToSend(e.repeat))) return [...following, 'all']
  return ['this', ...(ruleElsewhere.value ? [] : following), 'all']
}

async function save() {
  error.value = null
  const body = input()
  if (typeof body === 'string') return void (error.value = body)
  if (props.event?.recurring) return void (asking.value = { mode: 'save', scopes: saveScopes(body), body })
  await run(async () => emit('saved', props.event ? await updateEvent(props.event.id, body) : await createEvent(body)))
}

async function remove() {
  if (!props.event) return
  if (props.event.recurring) {
    // The dialog is the confirmation: nothing is picked until you choose.
    const following: Scope[] = props.event.first_occurrence ? [] : ['following']
    return void (asking.value = { mode: 'delete', scopes: ['this', ...following, 'all'] })
  }
  if (!confirmDelete.value) {
    confirmDelete.value = true
    setTimeout(() => (confirmDelete.value = false), 3000)
    return
  }
  await run(async () => {
    await deleteEvent(props.event!.id)
    emit('deleted')
  })
}

/** The scope was chosen: save or delete. */
async function chosen(scope: Scope) {
  const ask = asking.value
  asking.value = null
  if (!ask || !props.event) return
  const id = props.event.id
  await run(async () => {
    if (ask.mode === 'save') emit('saved', await updateEvent(id, ask.body!, scope))
    else {
      await deleteEvent(id, scope)
      emit('deleted')
    }
  })
}

async function run(work: () => Promise<void>) {
  busy.value = true
  try {
    await work()
  } catch (err) {
    error.value = apiErrorMessage(err)
  } finally {
    busy.value = false
  }
}

// Ctrl/⌘+Enter saves (Esc and the backdrop close: UModal does those).
function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
    e.preventDefault()
    save()
  }
}

const calendarItems = computed(() =>
  writable.value.map((c) => ({ label: `${c.name} · ${SOURCE_LABELS[c.source]}`, value: c.id, color: c.color })),
)
</script>

<template>
  <UModal
    :open="true"
    :title="event ? 'Edit event' : 'New event'"
    :close="false"
    :ui="{ header: 'sr-only', body: 'p-0 sm:p-0', content: 'max-w-[560px]' }"
    @update:open="(open) => !open && emit('cancel')"
  >
    <template #body>
      <form class="editor" @submit.prevent="save" @keydown="onKeydown">
        <UInput
          v-model="form.title"
          variant="none"
          placeholder="Add a title"
          aria-label="Title"
          autofocus
          :ui="{ base: 'title' }"
        />

        <UFormField label="Calendar" orientation="horizontal" class="row">
          <USelect v-model="form.calendar" :items="calendarItems" class="w-full">
            <template #leading>
              <span class="dot" :style="{ background: calendar?.color }" aria-hidden="true" />
            </template>
          </USelect>
        </UFormField>

        <UFormField label="When" orientation="horizontal" class="row top">
          <div class="when">
            <div class="line">
              <span class="to">from</span>
              <DateField v-model="form.startDate" label="Start date" required />
              <TimeField v-if="!form.allDay" v-model="form.startTime" label="Start time" required />
            </div>
            <div class="line">
              <span class="to">to</span>
              <DateField v-model="form.endDate" label="End date" required />
              <TimeField v-if="!form.allDay" v-model="form.endTime" label="End time" required />
            </div>
            <UCheckbox v-model="form.allDay" label="All day" />
          </div>
        </UFormField>

        <UFormField label="Repeat" orientation="horizontal" class="row top">
          <p v-if="ruleElsewhere" class="elsewhere">
            <HIcon name="repeat" :size="14" />
            Repeats in a way set in another app. Change the rule there; here you can change this event or all of them.
          </p>
          <RepeatEditor v-else v-model="form.repeat" :start="firstDay" :all-day="form.allDay" />
        </UFormField>

        <UFormField label="Where" orientation="horizontal" class="row">
          <UInput v-model="form.location" placeholder="Place or meeting link" class="w-full" />
        </UFormField>

        <UFormField
          v-if="canInvite"
          label="Guests"
          orientation="horizontal"
          class="row top"
          :help="`${calendar?.source === 'google' ? 'Google' : 'Microsoft'} emails them an invitation.`"
        >
          <RecipientInput
            v-model="form.guests"
            label="Guests"
            bare
            address-only
            placeholder="anna@example.com, bob@example.com"
          />
        </UFormField>

        <UFormField label="Notes" orientation="horizontal" class="row top">
          <UTextarea v-model="form.description" :rows="3" autoresize placeholder="Agenda, address, dial-in…" class="w-full" />
        </UFormField>

        <UAlert v-if="error" color="error" variant="soft" icon="i-havre-alert" :title="error" role="alert" />

        <div class="actions">
          <UButton
            v-if="event"
            icon="i-havre-trash"
            :color="confirmDelete ? 'error' : 'neutral'"
            variant="outline"
            :disabled="busy"
            @click="remove"
          >
            {{ confirmDelete ? 'Delete for good?' : 'Delete' }}
          </UButton>
          <span class="spacer" />
          <UButton color="neutral" variant="outline" :disabled="busy" @click="emit('cancel')">Cancel</UButton>
          <UButton :loading="busy" type="submit" :disabled="busy || !writable.length">
            {{ event ? 'Save' : 'Create' }}
          </UButton>
        </div>
      </form>
      <RepeatScope
        v-if="asking && event"
        :event="event"
        :mode="asking.mode"
        :scopes="asking.scopes"
        @choose="chosen"
        @cancel="asking = null"
      />
    </template>
  </UModal>
</template>

<style scoped>
.editor {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 22px 22px 18px;
}
:deep(.title) {
  height: 44px;
  padding: 0 2px;
  border-bottom: 1px solid var(--line2);
  border-radius: 0;
  font-family: var(--font-serif);
  font-size: 24px;
}
:deep(.title:focus) {
  border-bottom-color: var(--accent);
}
/* Label column 80 px, then the field (one column on phones). */
.row {
  display: grid;
  grid-template-columns: 80px minmax(0, 1fr);
  align-items: center;
  gap: 12px;
}
.row.top {
  align-items: start;
}
.dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
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
.elsewhere {
  display: flex;
  gap: 8px;
  margin: 0;
  padding-top: 7px;
  font-size: 13px;
  line-height: 1.45;
  color: var(--muted);
}
.elsewhere :deep(svg) {
  flex: 0 0 auto;
  margin-top: 2px;
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
@media (max-width: 560px) {
  .row {
    grid-template-columns: minmax(0, 1fr);
    gap: 4px;
  }
}
</style>
