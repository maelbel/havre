<script setup lang="ts">
// "Snooze until…": the message leaves the inbox and comes back, unread and
// on top, at that time (the server does it, even with Havre closed).
const props = withDefaults(defineProps<{ text?: string; snoozed?: boolean }>(), { text: undefined, snoozed: false })
const emit = defineEmits<{ snooze: [until: number | null] }>()

const choices = computed(() => snoozeChoices())
const custom = ref('')
function when(d: Date) {
  const day = sameDay(d, new Date())
    ? 'Today'
    : d.toLocaleDateString([], { weekday: 'short', day: 'numeric', month: 'short' })
  return `${day} · ${timeLabel(d)}`
}
function pick(d: Date, close: () => void) {
  emit('snooze', Math.floor(d.getTime() / 1000))
  close()
}
function pickCustom(close: () => void) {
  const d = new Date(custom.value)
  if (Number.isNaN(d.getTime()) || d.getTime() <= Date.now()) return
  pick(d, close)
}
// The date picker starts at tomorrow morning.
const minimum = computed(() => {
  const d = new Date(Date.now() + 5 * 60_000)
  return `${isoDate(d)}T${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
})
</script>

<template>
  <PopMenu icon="snooze" label="Snooze" title="Snooze (B)" :text="props.text">
    <template #default="{ close }">
      <div class="eyebrow head">Snooze until</div>
      <button v-for="c in choices" :key="c.label" type="button" role="menuitem" class="item" @click="pick(c.when, close)">
        <span class="grow">{{ c.label }}</span>
        <span class="mono faint">{{ when(c.when) }}</span>
      </button>
      <form class="custom" @submit.prevent="pickCustom(close)">
        <label class="sr-only" for="snooze-at">Date and time</label>
        <input id="snooze-at" v-model="custom" type="datetime-local" :min="minimum" />
        <button type="submit" class="btn small" :disabled="!custom">Snooze</button>
      </form>
      <button v-if="snoozed" type="button" role="menuitem" class="item" @click="emit('snooze', null); close()">
        <span class="grow">Wake up now</span>
      </button>
    </template>
  </PopMenu>
</template>

<style scoped>
.head {
  padding: 6px 10px 8px;
}
.item {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 10px;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: var(--text);
  font: inherit;
  font-size: 13.5px;
  text-align: left;
  cursor: pointer;
}
.item:hover,
.item:focus-visible {
  background: var(--hover);
  outline: none;
}
.grow {
  flex: 1;
}
.faint {
  color: var(--faint);
  font-size: 11.5px;
}
.custom {
  display: flex;
  gap: 6px;
  padding: 8px 10px;
  margin-top: 4px;
  border-top: 1px solid var(--line);
}
.custom input {
  flex: 1;
  min-width: 0;
  height: 30px;
  padding: 0 8px;
  border-radius: 7px;
  border: 1px solid var(--line2);
  background: var(--bg);
  color: var(--text);
  font: inherit;
  font-size: 12.5px;
  color-scheme: inherit;
}
.btn.small {
  height: 30px;
  padding: 0 10px;
  font-size: 12.5px;
}
</style>
