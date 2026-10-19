<script setup lang="ts">
// A small month to jump to a date. Days with events get a dot.
const props = defineProps<{ selected: Date; now: Date; busy?: Set<string> }>()
const emit = defineEmits<{ pick: [day: Date] }>()

const shown = ref(startOfMonth(props.selected))
watch(
  () => props.selected,
  (d) => {
    if (d.getMonth() !== shown.value.getMonth() || d.getFullYear() !== shown.value.getFullYear()) {
      shown.value = startOfMonth(d)
    }
  },
)
const days = computed(() => {
  const first = startOfWeek(shown.value)
  return Array.from({ length: 42 }, (_, i) => addDays(first, i))
})
const title = computed(() => shown.value.toLocaleDateString([], { month: 'long', year: 'numeric' }))
const initials = computed(() => days.value.slice(0, 7).map((d) => d.toLocaleDateString([], { weekday: 'narrow' })))
const move = (n: number) => (shown.value = new Date(shown.value.getFullYear(), shown.value.getMonth() + n, 1))
</script>

<template>
  <div class="mini">
    <div class="head">
      <span class="title">{{ title }}</span>
      <UButton
        icon="i-havre-back"
        color="neutral"
        variant="ghost"
        size="sm"
        square
        type="button"
        aria-label="Previous month"
        @click="move(-1)"
      />
      <UButton
        icon="i-havre-back"
        color="neutral"
        variant="ghost"
        size="sm"
        square
        class="flip"
        type="button"
        aria-label="Next month"
        @click="move(1)"
      />
    </div>
    <div class="grid">
      <span v-for="(w, i) in initials" :key="i" class="w">{{ w }}</span>
      <button
        v-for="d in days"
        :key="d.getTime()"
        type="button"
        class="d"
        :class="{
          other: d.getMonth() !== shown.getMonth(),
          today: sameDay(d, now),
          sel: sameDay(d, selected),
          busy: busy?.has(isoDate(d)),
        }"
        :aria-label="d.toDateString()"
        @click="emit('pick', d)"
      >
        {{ d.getDate() }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.mini {
  padding: 0 4px;
}
.head {
  display: flex;
  align-items: center;
  gap: 2px;
  margin-bottom: 6px;
}
.title {
  flex: 1;
  padding-left: 4px;
  font-size: 13px;
  font-weight: 500;
  text-transform: capitalize;
}
.flip {
  transform: scaleX(-1);
}
.grid {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
  gap: 1px;
}
.w {
  text-align: center;
  font-size: 10.5px;
  color: var(--faint);
  padding-bottom: 2px;
}
.d {
  position: relative;
  height: 27px;
  border: 0;
  border-radius: 7px;
  background: transparent;
  color: var(--text);
  font: inherit;
  font-size: 12px;
  cursor: pointer;
}
.d:hover {
  background: var(--hover);
}
.d.other {
  color: var(--faint);
}
.d.sel {
  background: var(--sel);
  box-shadow: inset 0 0 0 1px var(--line2);
}
.d.today {
  color: var(--accent);
  font-weight: 700;
}
.d.busy::after {
  content: '';
  position: absolute;
  left: 50%;
  bottom: 3px;
  width: 3px;
  height: 3px;
  margin-left: -1.5px;
  border-radius: 50%;
  background: var(--muted);
}
</style>
