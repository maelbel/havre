<script setup lang="ts">
import { Time } from '@internationalized/date'

// A time of day: Nuxt UI's time field (12 or 24 hours, as the browser's
// language has it). The value is `HH:MM`, or '' for none.
const model = defineModel<string>({ required: true })
const props = withDefaults(defineProps<{ label: string; size?: 'sm' | 'md'; required?: boolean; step?: number }>(), {
  size: 'md',
  required: false,
  step: 5,
})

const value = computed({
  get: () => {
    const [h, m] = (model.value || '').split(':').map(Number)
    return model.value && Number.isFinite(h) ? new Time(h, m || 0) : undefined
  },
  set: (t: Time | undefined) => {
    model.value = t ? `${String(t.hour).padStart(2, '0')}:${String(t.minute).padStart(2, '0')}` : ''
  },
})
</script>

<template>
  <UInputTime
    v-model="value"
    :locale="browserLocale()"
    :step="{ minute: props.step }"
    :size="props.size"
    :required="props.required"
    :aria-label="props.label"
  />
</template>
