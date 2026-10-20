<script setup lang="ts">
import type { Task } from '~/composables/useTasks'
import type { AgendaEvent, Calendar } from '~/composables/useAgenda'

// The month view: six weeks, a few events per day ("+3 more" opens the day).
const props = defineProps<{
  month: Date
  events: AgendaEvent[]
  calendars: Map<number, Calendar>
  now: Date
  selected: number | null
  /** Tasks due (when the Tasks app is in the dock): "2 tasks" on their day. */
  tasks?: Task[]
}>()
const emit = defineEmits<{
  select: [event: AgendaEvent]
  openDay: [day: Date]
  create: [range: { start: Date; end: Date; allDay: boolean }]
}>()

const SHOWN = 3

const days = computed(() => {
  const first = startOfWeek(startOfMonth(props.month))
  return Array.from({ length: 42 }, (_, i) => addDays(first, i))
})

/** Events of each day, all-day first, then by time. */
const byDay = computed(() => {
  const map = new Map<number, AgendaEvent[]>()
  for (const day of days.value) map.set(day.getTime(), [])
  for (const event of props.events) {
    const { start, end } = eventSpan(event)
    const last = event.all_day ? addDays(end, -1) : startOfDay(new Date(Math.max(end.getTime() - 1, start.getTime())))
    for (let d = startOfDay(start); d <= last; d = addDays(d, 1)) map.get(d.getTime())?.push(event)
  }
  return map
})

const color = (e: AgendaEvent) => props.calendars.get(e.calendar)?.color ?? 'var(--accent)'
const weekdays = computed(() => days.value.slice(0, 7).map((d) => d.toLocaleDateString([], { weekday: 'short' })))

/** Tasks still to do on that day. */
function openTasks(d: Date) {
  const day = localDay(d)
  return (props.tasks ?? []).filter((t) => t.due === day && !t.done_at).length
}
</script>

<template>
  <div class="month">
    <div class="weekdays">
      <span v-for="w in weekdays" :key="w">{{ w }}</span>
    </div>
    <div class="cells">
      <div
        v-for="d in days"
        :key="d.getTime()"
        class="cell"
        :class="{ other: d.getMonth() !== month.getMonth(), today: sameDay(d, now) }"
        @dblclick.self="emit('create', { start: d, end: addDays(d, 1), allDay: true })"
      >
        <button type="button" class="num" :aria-label="d.toDateString()" @click="emit('openDay', d)">
          {{ d.getDate() }}
        </button>
        <button
          v-for="e in byDay.get(d.getTime())!.slice(0, SHOWN)"
          :key="e.id"
          type="button"
          class="chip"
          :class="{ allday: e.all_day, selected: selected === e.id }"
          :style="{ '--c': color(e) }"
          @click="emit('select', e)"
        >
          <span v-if="!e.all_day" class="t">{{ timeLabel(new Date(e.start * 1000)) }}</span>
          {{ e.title }}<template v-if="e.recurring"><HIcon name="repeat" :size="10" class="rep" /><span class="sr-only"> · repeats</span></template>
        </button>
        <button
          v-if="openTasks(d)"
          type="button"
          class="more tasks"
          :aria-label="`${openTasks(d)} task${openTasks(d) > 1 ? 's' : ''} due — open the day`"
          @click="emit('openDay', d)"
        >
          <HIcon name="task" :size="11" />
          {{ openTasks(d) }} task{{ openTasks(d) > 1 ? 's' : '' }}
        </button>
        <button
          v-if="byDay.get(d.getTime())!.length > SHOWN"
          type="button"
          class="more"
          @click="emit('openDay', d)"
        >
          +{{ byDay.get(d.getTime())!.length - SHOWN }} more
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.month {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.weekdays {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
  border-bottom: 1px solid var(--line);
}
.weekdays span {
  padding: 10px 10px 8px;
  font-size: 12.5px;
  color: var(--muted);
}
.cells {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: repeat(7, minmax(0, 1fr));
  grid-template-rows: repeat(6, minmax(0, 1fr));
}
.cell {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-height: 0;
  padding: 4px 4px 6px;
  border-right: 1px solid var(--line);
  border-bottom: 1px solid var(--line);
  overflow: hidden;
}
.cell:nth-child(7n) {
  border-right: 0;
}
.cell.other {
  background: color-mix(in oklab, var(--bg) 50%, transparent);
}
.cell.other .num {
  color: var(--faint);
}
.num {
  align-self: flex-start;
  min-width: 26px;
  height: 26px;
  padding: 0 6px;
  border: 0;
  border-radius: 13px;
  background: transparent;
  color: var(--text);
  font: inherit;
  font-size: 13px;
  cursor: pointer;
}
.num:hover {
  background: var(--hover);
}
.today .num {
  background: var(--accent);
  color: var(--on-accent);
  font-weight: 600;
}
.chip {
  display: block;
  width: 100%;
  padding: 2px 6px;
  border: 0;
  border-radius: 5px;
  background: transparent;
  color: var(--text);
  font: inherit;
  font-size: 12px;
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  cursor: pointer;
}
.chip::before {
  content: '';
  display: inline-block;
  width: 7px;
  height: 7px;
  margin-right: 6px;
  border-radius: 50%;
  background: var(--c);
  vertical-align: 1px;
}
.chip.allday {
  background: color-mix(in oklab, var(--c) 26%, var(--panel));
}
.chip.allday::before {
  display: none;
}
.chip:hover {
  background: var(--hover);
}
.chip.selected {
  outline: 2px solid var(--text);
}
.t {
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 11px;
  margin-right: 4px;
}
.more {
  align-self: flex-start;
  padding: 0 6px;
  border: 0;
  background: none;
  color: var(--muted);
  font: inherit;
  font-size: 12px;
  cursor: pointer;
}
.more.tasks {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  color: var(--accent);
}
.more:hover {
  color: var(--text);
}
.rep {
  display: inline-block;
  margin-left: 3px;
  vertical-align: -1px;
  opacity: 0.75;
}
</style>
