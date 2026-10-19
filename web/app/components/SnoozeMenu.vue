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
      <UButton v-for="c in choices" :key="c.label" role="menuitem" color="neutral" variant="ghost" block class="justify-start font-normal" @click="pick(c.when, close)">
        <span class="grow text-left">{{ c.label }}</span>
        <span class="mono faint">{{ when(c.when) }}</span>
      </UButton>
      <form class="custom" @submit.prevent="pickCustom(close)">
        <UInput v-model="custom" type="datetime-local" :min="minimum" size="sm" aria-label="Date and time" class="grow" />
        <UButton color="neutral" variant="outline" size="sm" type="submit" :disabled="!custom">
          Snooze
        </UButton>
      </form>
      <UButton v-if="snoozed" role="menuitem" color="neutral" variant="ghost" block class="justify-start font-normal" @click="emit('snooze', null); close()">Wake up now</UButton>
    </template>
  </PopMenu>
</template>

<style scoped>
.head {
  padding: 6px 10px 8px;
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
</style>
