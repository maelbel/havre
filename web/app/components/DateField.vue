<script setup lang="ts">
import { CalendarDate, parseDate } from '@internationalized/date'

// A day: Nuxt UI's date field (type the digits, or ↑ ↓) with a calendar to
// pick from. The value is an ISO day, `2026-10-19`, or '' for none.
const model = defineModel<string>({ required: true })
const props = withDefaults(defineProps<{ label: string; size?: 'sm' | 'md'; min?: string; required?: boolean }>(), {
  size: 'md',
  min: undefined,
  required: false,
})

function toDate(iso: string | undefined) {
  if (!iso) return undefined
  try {
    return parseDate(iso)
  } catch {
    return undefined
  }
}
const value = computed({
  get: () => toDate(model.value),
  set: (d: CalendarDate | undefined) => (model.value = d ? d.toString() : ''),
})
const minValue = computed(() => toDate(props.min))
const open = ref(false)
</script>

<template>
  <UInputDate
    v-model="value"
    :locale="browserLocale()"
    :min-value="minValue"
    :size="props.size"
    :required="props.required"
    :aria-label="props.label"
  >
    <template #trailing>
      <UPopover v-model:open="open">
        <UButton
          icon="i-havre-calendar"
          color="neutral"
          variant="link"
          size="xs"
          square
          :aria-label="`${props.label}: pick in a calendar`"
          class="-me-1.5"
        />
        <template #content>
          <UCalendar
            v-model="value"
            :locale="browserLocale()"
            :min-value="minValue"
            :week-starts-on="1"
            class="p-2"
            @update:model-value="open = false"
          />
        </template>
      </UPopover>
    </template>
  </UInputDate>
</template>
