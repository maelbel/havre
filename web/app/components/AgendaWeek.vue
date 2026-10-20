<script setup lang="ts">
import type { AgendaEvent, Calendar } from '~/composables/useAgenda'
import type { Task } from '~/composables/useTasks'

// The day and week views: one column per day, hours down the side.
// - Events lasting a whole day (or more) sit in the band at the top.
// - Overlapping events share their column's width.
// - Click or drag on an empty slot to create an event there.
// - Tasks due on a day (when the Tasks app is in the dock) sit under the
//   all-day band; those with a time also show in the day as an outline.
const props = defineProps<{
  days: Date[]
  events: AgendaEvent[]
  calendars: Map<number, Calendar>
  now: Date
  selected: number | null
  tasks?: Task[]
}>()
const emit = defineEmits<{
  select: [event: AgendaEvent]
  create: [range: { start: Date; end: Date; allDay: boolean }]
  openTask: [task: Task]
  toggleTask: [task: Task]
}>()

// ---- Tasks ----------------------------------------------------------------------
const tasksByDay = computed(() =>
  props.days.map((d) => (props.tasks ?? []).filter((t) => t.due === localDay(d)).sort(byDue)),
)
const hasTasks = computed(() => tasksByDay.value.some((list) => list.length))
/** Timed tasks, placed in the hour grid like a 30-minute event. */
function timedTasks(day: Date) {
  return (props.tasks ?? [])
    .filter((t) => t.due === localDay(day) && t.due_at != null)
    .map((t) => {
      const at = new Date(t.due_at! * 1000)
      return { task: t, top: (at.getHours() + at.getMinutes() / 60) * HOUR }
    })
}

const HOUR = 56 // px per hour (as in the design)
const SNAP = 15 // minutes

const color = (e: AgendaEvent) => props.calendars.get(e.calendar)?.color ?? 'var(--accent)'

// ---- All-day band -------------------------------------------------------------

const isAllDayLike = (e: AgendaEvent) => e.all_day || e.end - e.start >= 86_400

interface Bar {
  event: AgendaEvent
  from: number // first column (0-based)
  to: number // last column
  lane: number
  continuesBefore: boolean
  continuesAfter: boolean
}

const bars = computed<Bar[]>(() => {
  const first = props.days[0]!
  const last = props.days[props.days.length - 1]!
  const out: Bar[] = []
  const lanes: number[] = [] // last used column per lane
  const items = props.events
    .filter(isAllDayLike)
    .map((event) => {
      const { start, end } = eventSpan(event)
      // An all-day event's end is exclusive; a timed one ending at 10:00
      // still covers that day.
      const lastDay = event.all_day ? addDays(end, -1) : startOfDay(new Date(end.getTime() - 1))
      return { event, start: startOfDay(start), lastDay }
    })
    .filter((i) => i.lastDay >= first && i.start <= last)
    .sort((a, b) => a.start.getTime() - b.start.getTime() || b.lastDay.getTime() - a.lastDay.getTime())
  for (const item of items) {
    const from = Math.max(0, Math.round((item.start.getTime() - first.getTime()) / DAY_MS))
    const to = Math.min(props.days.length - 1, Math.round((item.lastDay.getTime() - first.getTime()) / DAY_MS))
    let lane = lanes.findIndex((end) => end < from)
    if (lane === -1) lane = lanes.length
    lanes[lane] = to
    out.push({ event: item.event, from, to, lane, continuesBefore: item.start < first, continuesAfter: item.lastDay > last })
  }
  return out
})
const allDayLanes = computed(() => Math.max(1, ...bars.value.map((b) => b.lane + 1)))

// ---- Timed events ---------------------------------------------------------------

interface Block {
  event: AgendaEvent
  top: number
  height: number
  left: number // fraction of the column
  width: number
  startsBefore: boolean
}

function layoutDay(day: Date): Block[] {
  const dayStart = day.getTime()
  const dayEnd = addDays(day, 1).getTime()
  const segments = props.events
    .filter((e) => !isAllDayLike(e))
    .map((event) => ({ event, start: event.start * 1000, end: Math.max(event.end * 1000, event.start * 1000 + 15 * 60_000) }))
    .filter((s) => s.start < dayEnd && s.end > dayStart)
    .map((s) => ({ ...s, from: Math.max(s.start, dayStart), to: Math.min(s.end, dayEnd) }))
    .sort((a, b) => a.from - b.from || b.to - a.to)

  // Clusters of events that overlap each other share the width.
  const blocks: Block[] = []
  let cluster: typeof segments = []
  let clusterEnd = 0
  const flush = () => {
    const columns: number[] = []
    const placed = cluster.map((s) => {
      let col = columns.findIndex((end) => end <= s.from)
      if (col === -1) col = columns.length
      columns[col] = s.to
      return { s, col }
    })
    for (const { s, col } of placed) {
      blocks.push({
        event: s.event,
        top: ((s.from - dayStart) / 3_600_000) * HOUR,
        height: Math.max(((s.to - s.from) / 3_600_000) * HOUR - 2, 18),
        left: col / columns.length,
        width: 1 / columns.length,
        startsBefore: s.start < dayStart,
      })
    }
    cluster = []
  }
  for (const s of segments) {
    if (cluster.length && s.from >= clusterEnd) flush()
    cluster.push(s)
    clusterEnd = Math.max(clusterEnd, s.to)
  }
  if (cluster.length) flush()
  return blocks
}

const columns = computed(() => props.days.map((day) => ({ day, blocks: layoutDay(day) })))

const nowTop = computed(() => (props.now.getHours() * 60 + props.now.getMinutes()) * (HOUR / 60))
const isToday = (d: Date) => sameDay(d, props.now)

// ---- Creating by clicking / dragging ---------------------------------------------------

const draft = ref<{ col: number; from: number; to: number } | null>(null)
let dragOrigin: { col: number; minute: number; moved: boolean } | null = null

function minuteAt(e: PointerEvent, el: HTMLElement) {
  const y = e.clientY - el.getBoundingClientRect().top
  return Math.min(24 * 60, Math.max(0, Math.floor((y / HOUR) * 60 / SNAP) * SNAP))
}

function onPointerDown(e: PointerEvent, col: number) {
  if (e.button !== 0 || (e.target as HTMLElement).closest('.event')) return
  const el = e.currentTarget as HTMLElement
  el.setPointerCapture(e.pointerId)
  const minute = minuteAt(e, el)
  dragOrigin = { col, minute, moved: false }
  draft.value = { col, from: minute, to: minute + 60 }
}

function onPointerMove(e: PointerEvent) {
  if (!dragOrigin || !draft.value) return
  const minute = minuteAt(e, e.currentTarget as HTMLElement)
  if (Math.abs(minute - dragOrigin.minute) >= SNAP) dragOrigin.moved = true
  if (!dragOrigin.moved) return
  const a = Math.min(minute, dragOrigin.minute)
  const b = Math.max(minute, dragOrigin.minute)
  draft.value = { col: dragOrigin.col, from: a, to: Math.max(b, a + SNAP) }
}

function onPointerUp() {
  if (!dragOrigin || !draft.value) return
  const { col, from, to } = draft.value
  const day = props.days[col]!
  dragOrigin = null
  const at = (m: number) => new Date(day.getFullYear(), day.getMonth(), day.getDate(), 0, m)
  emit('create', { start: at(from), end: at(Math.min(to, 24 * 60)), allDay: false })
  // Keep the ghost until the editor opens over it.
  setTimeout(() => (draft.value = null), 150)
}

function createAllDay(col: number) {
  const day = props.days[col]!
  emit('create', { start: day, end: addDays(day, 1), allDay: true })
}

// Start scrolled to the morning (or to now, if later).
const scroller = ref<HTMLElement | null>(null)
onMounted(() => {
  const hour = props.days.some(isToday) ? Math.max(7, props.now.getHours() - 2) : 7.5
  scroller.value?.scrollTo({ top: hour * HOUR })
})

// A selected event out of sight (opened from Today, People…): bring it in.
watch(
  () => [props.selected, props.events.length] as const,
  async ([id]) => {
    const e = props.events.find((x) => x.id === id)
    const box = scroller.value
    if (!e || e.all_day || !box) return
    await nextTick()
    const start = new Date(e.start * 1000)
    const top = (start.getHours() + start.getMinutes() / 60) * HOUR
    if (top < box.scrollTop || top > box.scrollTop + box.clientHeight - HOUR) {
      box.scrollTo({ top: Math.max(0, top - HOUR), behavior: 'smooth' })
    }
  },
  { immediate: true },
)

const dayLabel = (d: Date) => d.toLocaleDateString([], { weekday: 'short' })
</script>

<template>
  <div class="week" :style="{ '--cols': days.length, '--hour': `${HOUR}px` }">
    <!-- Day names -->
    <div class="head">
      <div class="gutter" />
      <div v-for="d in days" :key="d.getTime()" class="day-name" :class="{ today: isToday(d) }">
        <span class="dow">{{ dayLabel(d) }}</span>
        <span class="num">{{ d.getDate() }}</span>
      </div>
    </div>

    <!-- All-day band -->
    <div class="allday" :style="{ '--lanes': allDayLanes }">
      <div class="gutter label">all day</div>
      <div class="band">
        <button
          v-for="(d, i) in days"
          :key="d.getTime()"
          type="button"
          class="band-cell"
          :style="{ gridColumn: i + 1 }"
          :aria-label="`New all-day event on ${d.toDateString()}`"
          @click="createAllDay(i)"
        />
        <button
          v-for="b in bars"
          :key="`${b.event.id}`"
          type="button"
          class="bar"
          :class="{ selected: selected === b.event.id, before: b.continuesBefore, after: b.continuesAfter }"
          :style="{ gridColumn: `${b.from + 1} / ${b.to + 2}`, gridRow: b.lane + 1, '--c': color(b.event) }"
          @click="emit('select', b.event)"
        >
          {{ b.event.title }}<template v-if="b.event.recurring"><HIcon name="repeat" :size="11" class="rep" /><span class="sr-only"> · repeats</span></template>
        </button>
      </div>
    </div>

    <!-- Tasks due -->
    <div v-if="hasTasks" class="taskband">
      <div class="gutter label">tasks</div>
      <div class="tcols">
        <div v-for="(list, i) in tasksByDay" :key="i" class="tcol">
          <div v-if="list.length && days.length === 1" class="tcount">{{ list.length }} task{{ list.length > 1 ? 's' : '' }} due</div>
          <div v-for="t in list" :key="t.id" class="titem" :class="{ done: !!t.done_at }">
            <button
              type="button"
              class="tcheck"
              :aria-label="`${t.done_at ? 'Mark not done' : 'Mark done'}: ${t.title}`"
              :aria-pressed="!!t.done_at"
              :disabled="t.read_only"
              @click="emit('toggleTask', t)"
            >
              <HIcon v-if="t.done_at" name="check" :size="9" :stroke-width="3.2" />
            </button>
            <button type="button" class="tname" :title="t.title" @click="emit('openTask', t)">{{ t.title }}</button>
            <span v-if="t.due_at != null && days.length === 1" class="ttime mono">{{ taskTime(t.due_at) }}</span>
          </div>
        </div>
      </div>
    </div>

    <!-- Hours -->
    <div ref="scroller" class="scroll">
      <div class="grid">
        <div class="gutter hours">
          <span v-for="h in 23" :key="h" class="hour-label" :style="{ top: `${h * HOUR}px` }">{{ hhmm(h * 60) }}</span>
        </div>
        <div
          v-for="(c, i) in columns"
          :key="c.day.getTime()"
          class="col"
          :class="{ today: isToday(c.day) }"
          @pointerdown="onPointerDown($event, i)"
          @pointermove="onPointerMove"
          @pointerup="onPointerUp"
        >
          <button
            v-for="b in c.blocks"
            :key="b.event.id"
            type="button"
            class="event"
            :class="{ selected: selected === b.event.id, short: b.height < 34, ro: b.event.read_only }"
            :style="{
              top: `${b.top}px`,
              height: `${b.height}px`,
              left: `calc(${b.left * 100}% + 2px)`,
              width: `calc(${b.width * 100}% - 4px)`,
              '--c': color(b.event),
            }"
            @click="emit('select', b.event)"
          >
            <span class="title">{{ b.event.title }}<template v-if="b.event.recurring"><HIcon name="repeat" :size="11" class="rep" /><span class="sr-only"> · repeats</span></template></span>
            <span class="time">
              {{ timeLabel(new Date(b.event.start * 1000)) }}<template v-if="b.event.location"> · {{ b.event.location }}</template>
            </span>
          </button>
          <button
            v-for="tt in timedTasks(c.day)"
            :key="`task-${tt.task.id}`"
            type="button"
            class="task-block"
            :class="{ done: !!tt.task.done_at }"
            :style="{ top: `${tt.top}px` }"
            :aria-label="`Task: ${tt.task.title}, ${taskTime(tt.task.due_at!)}`"
            @click="emit('openTask', tt.task)"
          >
            <HIcon name="task" :size="12" />
            <span class="title">{{ tt.task.title }}</span>
          </button>
          <div v-if="draft && draft.col === i" class="ghost" :style="{ top: `${draft.from * HOUR / 60}px`, height: `${(draft.to - draft.from) * HOUR / 60 - 2}px` }">
            {{ hhmm(draft.from) }} – {{ hhmm(Math.min(draft.to, 1440)) }}
          </div>
          <div v-if="isToday(c.day)" class="now" :style="{ top: `${nowTop}px` }" aria-hidden="true" />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.taskband {
  display: grid;
  grid-template-columns: var(--gutter) 1fr;
  border-bottom: 1px solid var(--line);
}
.tcols {
  display: grid;
  grid-template-columns: repeat(var(--cols), 1fr);
}
.tcol {
  min-width: 0;
  padding: 6px 4px 8px;
  border-left: 1px solid var(--line);
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.tcount {
  font-size: 11.5px;
  color: var(--faint);
  padding: 0 4px;
}
.titem {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  padding: 2px 4px;
  border-radius: 6px;
  border: 1px dashed var(--line2);
  font-size: 12px;
}
.tcheck {
  flex: 0 0 14px;
  height: 14px;
  padding: 0;
  border-radius: 4px;
  border: 1.5px solid var(--line2);
  background: transparent;
  color: var(--on-accent);
  display: grid;
  place-items: center;
  cursor: pointer;
}
.titem.done .tcheck {
  background: var(--accent);
  border-color: var(--accent);
}
.tname {
  flex: 1;
  min-width: 0;
  padding: 0;
  border: 0;
  background: transparent;
  color: var(--text);
  text-align: left;
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  cursor: pointer;
}
.titem.done .tname {
  color: var(--faint);
  text-decoration: line-through;
}
.ttime {
  font-size: 11px;
  color: var(--faint);
}
.task-block {
  position: absolute;
  left: 2px;
  right: 2px;
  height: 24px;
  z-index: 2;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 8px;
  border-radius: 6px;
  border: 1px solid var(--accent);
  background: var(--panel);
  color: var(--text);
  font-size: 12px;
  text-align: left;
  cursor: pointer;
  overflow: hidden;
}
.task-block svg {
  color: var(--accent);
  flex: none;
}
.task-block .title {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.task-block.done {
  opacity: 0.55;
  text-decoration: line-through;
}
.week {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  --gutter: 56px;
}
.head,
.allday {
  display: grid;
  grid-template-columns: var(--gutter) 1fr;
  border-bottom: 1px solid var(--line);
}
.head {
  grid-template-columns: var(--gutter) repeat(var(--cols), 1fr);
}
.day-name {
  display: flex;
  align-items: baseline;
  gap: 6px;
  padding: 14px 8px 8px;
  color: var(--muted);
  font-size: 12px;
}
.day-name .num {
  padding: 2px 6px;
  border-radius: 8px;
  font-family: var(--font-serif);
  font-size: 22px;
  line-height: 1;
  color: var(--text);
}
.day-name.today .num {
  background: var(--accent);
  color: var(--on-accent);
}
.day-name.today .dow {
  color: var(--accent);
}
.label {
  font-family: var(--font-mono);
  font-size: 10.5px;
  color: var(--faint);
  text-align: right;
  padding: 6px 8px 0 0;
}
.band {
  position: relative;
  display: grid;
  grid-template-columns: repeat(var(--cols), 1fr);
  grid-auto-rows: 24px;
  gap: 2px 0;
  min-height: 30px;
  padding: 3px 0;
}
.band-cell {
  grid-row: 1 / span var(--lanes);
  border: 0;
  border-left: 1px solid var(--line);
  background: transparent;
  cursor: pointer;
}
.band-cell:hover {
  background: var(--hover);
}
.bar {
  position: relative;
  z-index: 1;
  margin: 0 3px;
  padding: 0 8px;
  border: 0;
  border-radius: 6px;
  background: color-mix(in oklab, var(--c) 26%, var(--panel));
  color: var(--text);
  font: inherit;
  font-size: 12.5px;
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  cursor: pointer;
  box-shadow: inset 3px 0 0 var(--c);
}
.bar.before {
  border-top-left-radius: 0;
  border-bottom-left-radius: 0;
  margin-left: 0;
}
.bar.after {
  border-top-right-radius: 0;
  border-bottom-right-radius: 0;
  margin-right: 0;
}
.bar.selected,
.event.selected {
  outline: 2px solid var(--text);
  outline-offset: 1px;
}
.scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overscroll-behavior: contain;
}
.grid {
  position: relative;
  display: grid;
  grid-template-columns: var(--gutter) repeat(var(--cols), 1fr);
  height: calc(var(--hour) * 24);
  background-image: linear-gradient(to bottom, var(--line) 1px, transparent 1px);
  background-size: 100% var(--hour);
}
.hours {
  position: relative;
}
.hour-label {
  position: absolute;
  right: 8px;
  transform: translateY(-50%);
  font-family: var(--font-mono);
  font-size: 10.5px;
  color: var(--faint);
  background: var(--panel);
  padding: 0 2px;
}
.col {
  position: relative;
  border-left: 1px solid var(--line);
  touch-action: pan-y;
  cursor: cell;
}
.col.today {
  background: color-mix(in oklab, var(--accent) 4%, transparent);
}
.event {
  position: absolute;
  z-index: 1;
  display: flex;
  flex-direction: column;
  gap: 1px;
  padding: 4px 7px;
  border: 0;
  border-radius: 7px;
  background: color-mix(in oklab, var(--c) 24%, var(--panel));
  box-shadow:
    inset 3px 0 0 var(--c),
    0 0 0 1px var(--panel);
  color: var(--text);
  font: inherit;
  text-align: left;
  overflow: hidden;
  cursor: pointer;
}
.event:hover {
  background: color-mix(in oklab, var(--c) 34%, var(--panel));
}
.event .title {
  font-size: 12.5px;
  font-weight: 500;
  line-height: 1.25;
  overflow: hidden;
  text-overflow: ellipsis;
}
.event .time {
  font-size: 11.5px;
  color: var(--muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.event.short {
  flex-direction: row;
  align-items: center;
  gap: 6px;
  padding-top: 0;
  padding-bottom: 0;
}
.event.short .title {
  white-space: nowrap;
}
.ghost {
  position: absolute;
  z-index: 2;
  left: 2px;
  right: 4px;
  padding: 4px 7px;
  border-radius: 7px;
  border: 1px dashed var(--accent);
  background: color-mix(in oklab, var(--accent) 18%, transparent);
  color: var(--text);
  font-size: 12px;
  pointer-events: none;
}
.now {
  position: absolute;
  z-index: 3;
  left: -1px;
  right: 0;
  height: 2px;
  background: var(--danger);
  pointer-events: none;
}
.now::before {
  content: '';
  position: absolute;
  left: -5px;
  top: -4px;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: var(--danger);
}
@media (max-width: 720px) {
  .week {
    --gutter: 40px;
  }
  .day-name {
    flex-direction: column;
    align-items: center;
    gap: 2px;
  }
}
.rep {
  display: inline-block;
  margin-left: 4px;
  vertical-align: -1px;
  opacity: 0.75;
}
</style>
