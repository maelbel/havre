<script setup lang="ts">
import type { Freq, Repeat, WeekDay } from '~/composables/useAgenda'

// The event editor's Repeat row: how often, on which days, until when.
// `null` is "Doesn't repeat". `start` is the first event (local), which the
// rule follows: its weekday is always one of the days, "the 19th" is its date.
const model = defineModel<Repeat | null>({ required: true })
const props = defineProps<{ start: Date; allDay: boolean }>()

type Choice = 'none' | Freq
const freqItems: { label: string; value: Choice }[] = [
  { label: "Doesn't repeat", value: 'none' },
  { label: 'Every day', value: 'daily' },
  { label: 'Every week', value: 'weekly' },
  { label: 'Every month', value: 'monthly' },
  { label: 'Every year', value: 'yearly' },
]

function patch(changes: Partial<Repeat>) {
  if (model.value) model.value = { ...model.value, ...changes }
}
const clamp = (n: number | null | undefined, min: number, max: number) => Math.min(max, Math.max(min, Math.round(n || min)))

const freq = computed<Choice>({
  get: () => model.value?.freq ?? 'none',
  set: (choice) => {
    if (choice === 'none') return void (model.value = null)
    const was = model.value ?? { freq: choice, interval: 1, days: [], on: null, until: null, count: null }
    // The days and "the third Monday" are kept while trying other choices
    // (only the ones for the chosen frequency are sent: `repeatToSend`).
    const own = weekDayOf(props.start)
    const on = was.on && was.on.day !== own ? { nth: was.on.nth === -1 || place.value.nth > 4 ? -1 : place.value.nth, day: own } : was.on
    model.value = {
      ...was,
      freq: choice,
      // The first event's day is always one of them.
      days: choice === 'weekly' ? WEEK_DAYS.filter((d) => was.days.includes(d) || d === own) : was.days,
      on,
    }
  },
})

const interval = computed({
  get: () => model.value?.interval ?? 1,
  set: (n: number) => patch({ interval: clamp(n, 1, 99) }),
})
const unit = computed(() => {
  const names: Record<Freq, [string, string]> = {
    daily: ['day', 'days'],
    weekly: ['week', 'weeks'],
    monthly: ['month', 'months'],
    yearly: ['year', 'years'],
  }
  const [one, many] = names[model.value?.freq ?? 'daily']
  return interval.value === 1 ? one : many
})

// Weekly: the first event's day can't be unticked (the series starts there).
const ownDay = computed(() => weekDayOf(props.start))
const dayItems = computed(() =>
  WEEK_DAYS.map((d) => ({
    value: d,
    label: WEEK_DAY_NAMES[d],
    short: WEEK_DAY_NAMES[d][0],
    disabled: d === ownDay.value,
  })),
)
const days = computed<WeekDay[]>({
  get: () => (model.value?.days.length ? model.value.days : [ownDay.value]),
  set: (picked) => patch({ days: WEEK_DAYS.filter((d) => picked.includes(d) || d === ownDay.value) }),
})

// Monthly: "the 19th", "the third Monday", "the last Monday".
const place = computed(() => nthOfMonth(props.start))
const byItems = computed(() => {
  const day = weekDayOf(props.start)
  const items = [{ label: `the ${dayNumber(props.start.getDate())}`, value: 'day' }]
  if (place.value.nth <= 4) items.push({ label: nthLabel({ nth: place.value.nth, day }), value: 'nth' })
  if (place.value.last) items.push({ label: nthLabel({ nth: -1, day }), value: 'last' })
  return items
})
const by = computed({
  get: () => (!model.value?.on ? 'day' : model.value.on.nth === -1 ? 'last' : 'nth'),
  set: (choice: string) => {
    const day = weekDayOf(props.start)
    patch({ on: choice === 'day' ? null : { nth: choice === 'last' ? -1 : place.value.nth, day } })
  },
})

// Ends: never, on a date, after N times.
const endItems = [
  { label: 'never', value: 'never' },
  { label: 'on a date', value: 'until' },
  { label: 'after', value: 'count' },
]
const ends = computed({
  get: () => (model.value?.until != null ? 'until' : model.value?.count != null ? 'count' : 'never'),
  set: (choice: string) => {
    if (choice === 'never') patch({ until: null, count: null })
    if (choice === 'count') patch({ until: null, count: 10 })
    if (choice === 'until') {
      const s = props.start
      patch({ count: null, until: localDateToUtc(new Date(s.getFullYear(), s.getMonth() + 3, s.getDate())) })
    }
  },
})
const until = computed({
  get: () => (model.value?.until != null ? isoDate(utcDateToLocal(model.value.until)) : ''),
  set: (iso: string) => {
    const d = parseIsoDate(iso)
    if (d) patch({ until: localDateToUtc(d) })
  },
})
const count = computed({
  get: () => model.value?.count ?? 10,
  set: (n: number) => patch({ count: clamp(n, 1, 730) }),
})

// The first event moved: the rule follows it.
watch(
  () => props.start.getTime(),
  (_, before) => {
    const r = model.value
    if (!r) return
    const day = weekDayOf(props.start)
    if (r.freq === 'weekly' && !r.days.includes(day)) {
      const old = before === undefined ? null : weekDayOf(new Date(before))
      const kept = r.days.length === 1 && r.days[0] === old ? [] : r.days
      patch({ days: WEEK_DAYS.filter((d) => kept.includes(d) || d === day) })
    }
    if (r.freq === 'monthly' && r.on) {
      const last = r.on.nth === -1 && place.value.last
      patch({ on: last || place.value.nth <= 4 ? { nth: last ? -1 : place.value.nth, day } : { nth: -1, day } })
    }
    if (r.until != null && r.until < localDateToUtc(props.start)) patch({ until: localDateToUtc(props.start) })
  },
)

const summary = computed(() => (model.value ? repeatSummary(model.value, props.start) : ''))
const zone = browserTimeZone()

// Nuxt UI's radio and checkbox cards, drawn as pills and round day buttons.
const pills = {
  fieldset: 'gap-1.5',
  item: [
    'items-center rounded-full py-0 px-3 h-8 border-(--line2) text-muted',
    'has-data-[state=checked]:border-primary has-data-[state=checked]:bg-primary/14 has-data-[state=checked]:text-default',
  ].join(' '),
  label: 'font-normal text-inherit text-[12.5px] whitespace-nowrap',
}
const dayPills = {
  fieldset: 'gap-1.5',
  item: [
    'size-[34px] p-0 justify-center items-center rounded-full border-(--line2) text-muted',
    'has-data-[state=checked]:bg-primary has-data-[state=checked]:border-primary has-data-[state=checked]:text-(--on-accent)',
  ].join(' '),
  label: 'font-normal text-inherit text-[12px]',
  wrapper: 'w-auto',
}
</script>

<template>
  <div class="repeat">
    <URadioGroup
      v-model="freq"
      :items="freqItems"
      orientation="horizontal"
      variant="card"
      indicator="hidden"
      size="sm"
      aria-label="Repeat"
      :ui="pills"
    />
    <template v-if="model">
      <div class="line">
        <span class="lbl">Every</span>
        <UInputNumber v-model="interval" :min="1" :max="99" size="sm" class="num" aria-label="Repeat every" />
        <span class="unit">{{ unit }}</span>
      </div>
      <div v-if="model.freq === 'weekly'" class="line">
        <span class="lbl">on</span>
        <UCheckboxGroup
          v-model="days"
          :items="dayItems"
          orientation="horizontal"
          variant="card"
          indicator="hidden"
          aria-label="On these days"
          :ui="dayPills"
        >
          <template #label="{ item }">
            <span aria-hidden="true">{{ (item as { short: string }).short }}</span>
            <span class="sr-only">{{ (item as { label: string }).label }}</span>
          </template>
        </UCheckboxGroup>
      </div>
      <div v-if="model.freq === 'monthly'" class="line">
        <span class="lbl">on</span>
        <URadioGroup
          v-model="by"
          :items="byItems"
          orientation="horizontal"
          variant="card"
          indicator="hidden"
          size="sm"
          aria-label="On"
          :ui="pills"
        />
      </div>
      <div class="line">
        <span class="lbl">ends</span>
        <URadioGroup
          v-model="ends"
          :items="endItems"
          orientation="horizontal"
          variant="card"
          indicator="hidden"
          size="sm"
          aria-label="Ends"
          :ui="pills"
        />
        <DateField v-if="ends === 'until'" v-model="until" label="Last day" size="sm" :min="isoDate(start)" required />
        <template v-if="ends === 'count'">
          <UInputNumber v-model="count" :min="1" :max="730" size="sm" class="num" aria-label="Number of times" />
          <span class="unit">times</span>
        </template>
      </div>
      <p class="summary" aria-live="polite">
        <HIcon name="repeat" :size="14" />
        <span>{{ summary }}</span>
      </p>
      <p v-if="!allDay" class="zone">Kept in your time zone ({{ zone }}): it stays at the same time when the clocks change.</p>
    </template>
  </div>
</template>

<style scoped>
.repeat {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.line {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}
.lbl {
  width: 34px;
  color: var(--faint);
  font-size: 13px;
}
.unit {
  font-size: 13px;
  color: var(--muted);
}
.num {
  width: 104px;
}
.summary {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 0;
  padding: 9px 12px;
  border-radius: 10px;
  background: color-mix(in oklab, var(--accent) 10%, transparent);
  color: var(--text);
  font-size: 13px;
}
.summary :deep(svg) {
  flex: 0 0 auto;
  color: var(--accent);
}
.zone {
  margin: 0;
  font-size: 12px;
  color: var(--faint);
}
</style>
