<script setup lang="ts">
import type { AgendaEvent, Scope } from '~/composables/useAgenda'

// "Only this event / This and the following ones / All events": asked when
// saving or deleting an occurrence of a repeating event. Nothing is picked
// until you choose; on the first occurrence "following" is all of them, so
// it isn't offered.
const props = defineProps<{
  event: AgendaEvent
  mode: 'save' | 'delete'
  /** The choices that make sense for this change (e.g. a new rule: not "only this"). */
  scopes: Scope[]
}>()
const emit = defineEmits<{ choose: [scope: Scope]; cancel: [] }>()

const picked = ref<Scope | undefined>()
const start = computed(() => eventSpan(props.event).start)
const day = (d: Date) => d.toLocaleDateString([], { weekday: 'short', day: 'numeric', month: 'short' })
const short = (d: Date) => d.toLocaleDateString([], { day: 'numeric', month: 'short' })

const how = computed(() => (props.event.repeat ? repeatSummary(props.event.repeat, start.value) : null))

const items = computed(() => {
  const s = start.value
  const hints: Record<Scope, string> =
    props.mode === 'save'
      ? {
          this: `${day(s)} changes; the others stay`,
          following: `From ${short(s)} on; the earlier ones stay`,
          all: 'Every event of the series, past and future',
        }
      : {
          this: `${day(s)} goes; the others stay`,
          following: `Ends the series on ${short(addDays(s, -1))}`,
          all: 'The whole series goes',
        }
  const labels: Record<Scope, string> = {
    this: 'Only this event',
    following: 'This and the following ones',
    all: 'All events',
  }
  return props.scopes.map((scope) => ({ value: scope, label: labels[scope], description: hints[scope] }))
})

// A phone gets a sheet from the bottom, a wider screen a dialog.
const phone = ref(false)
onMounted(() => {
  const query = window.matchMedia('(max-width: 560px)')
  phone.value = query.matches
  query.addEventListener('change', (e) => (phone.value = e.matches))
})

const title = computed(() => `${props.mode === 'save' ? 'Change' : 'Delete'} “${props.event.title}”?`)
const description = computed(() =>
  how.value ? `It repeats ${how.value[0]!.toLowerCase()}${how.value.slice(1)}.` : 'It repeats in a way set in another app.',
)
function confirm() {
  if (picked.value) emit('choose', picked.value)
}
const cards = {
  fieldset: 'gap-2',
  item: 'border-(--line2) rounded-[10px] has-data-[state=checked]:border-primary has-data-[state=checked]:bg-primary/10',
  label: 'font-normal text-[13.5px]',
  description: 'text-[12.5px]',
  wrapper: 'items-start text-start',
}
</script>

<template>
  <UDrawer
    v-if="phone"
    :open="true"
    :title="title"
    :description="description"
    @update:open="(open: boolean) => !open && emit('cancel')"
  >
    <template #body>
      <URadioGroup v-model="picked" :items="items" variant="card" indicator="hidden" aria-label="Which events" :ui="cards" />
    </template>
    <template #footer>
      <div class="foot">
        <UButton color="neutral" variant="outline" size="lg" @click="emit('cancel')">Cancel</UButton>
        <UButton :color="mode === 'delete' ? 'error' : 'primary'" :variant="mode === 'delete' ? 'soft' : 'solid'" size="lg" :disabled="!picked" @click="confirm">
          {{ mode === 'save' ? 'Save' : 'Delete' }}
        </UButton>
      </div>
    </template>
  </UDrawer>
  <UModal
    v-else
    :open="true"
    :title="title"
    :description="description"
    :ui="{ content: 'max-w-[440px]' }"
    @update:open="(open: boolean) => !open && emit('cancel')"
  >
    <template #body>
      <URadioGroup v-model="picked" :items="items" variant="card" indicator="hidden" aria-label="Which events" :ui="cards" />
    </template>
    <template #footer>
      <div class="foot">
        <UButton color="neutral" variant="outline" @click="emit('cancel')">Cancel</UButton>
        <UButton :color="mode === 'delete' ? 'error' : 'primary'" :variant="mode === 'delete' ? 'soft' : 'solid'" :disabled="!picked" @click="confirm">
          {{ mode === 'save' ? 'Save' : 'Delete' }}
        </UButton>
      </div>
    </template>
  </UModal>
</template>

<style scoped>
.foot {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  width: 100%;
}
</style>
