<script setup lang="ts">
import type { MessageDetail } from '~/composables/useMail'

// Mail's "Task" button (or T): a small form to make a to-do from the open
// email. The title starts as the subject, the day is one tap, the list is
// the email's own account; the task stays linked to the email and its
// sender.
const props = defineProps<{ account: number; message: MessageDetail }>()
const toast = useToasts()
const { lists, refresh } = useTaskLists()
const pop = ref<{ toggle: () => void; close: () => void } | null>(null)
defineExpose({ toggle: () => pop.value?.toggle() })

const title = ref('')
const due = ref<number | null>(null)
const picking = ref(false)
const pickIso = ref('')
const list = ref<number | null>(null)
const busy = ref(false)

const writable = computed(() => lists.value.filter((l) => !l.read_only))
const listItems = computed(() => writable.value.map((l) => ({ label: `${l.owner} · ${l.name}`, value: l.id })))
/** The email's account's main list, else the default one. */
function defaultList() {
  const mine = writable.value.filter((l) => l.account === props.account)
  return (mine.find((l) => l.is_default) ?? mine[0] ?? writable.value.find((l) => l.is_default) ?? writable.value[0])?.id ?? null
}
const sameAccount = computed(() => writable.value.find((l) => l.id === list.value)?.account === props.account)

watch(
  () => props.message.uid,
  () => {
    title.value = subjectToTitle(props.message.subject || '') || 'Follow up'
    due.value = null
    picking.value = false
    list.value = defaultList()
  },
  { immediate: true },
)
watch(lists, () => {
  if (list.value == null) list.value = defaultList()
})
onMounted(() => refresh().catch(() => {}))

/** Today, Tomorrow, and the next Friday (the usual "before the weekend"). */
const chips = computed(() => {
  const today = localDay()
  const d = new Date()
  const toFriday = (5 - d.getDay() + 7) % 7 || 7
  const out = [
    { id: 'today', label: 'Today', day: today },
    { id: 'tomorrow', label: 'Tomorrow', day: today + 86_400 },
  ]
  if (toFriday > 1) out.push({ id: 'fri', label: `Fri ${dayToDate(today + toFriday * 86_400).getDate()}`, day: today + toFriday * 86_400 })
  return out
})
function pick(day: number | null) {
  due.value = due.value === day ? null : day
  picking.value = false
}
function pickDay() {
  const day = isoToDay(pickIso.value)
  if (day != null) due.value = day
}
const pickedLabel = computed(() => (due.value != null && !chips.value.some((c) => c.day === due.value) ? taskDayLabel(due.value) : null))

async function add(close: () => void) {
  if (!title.value.trim() || busy.value) return
  busy.value = true
  try {
    const m = props.message
    const task = await taskFromMail({ account: props.account, folder: m.folder, uid: m.uid }, title.value, {
      list: list.value,
      due: due.value,
    })
    close()
    refresh()
    toast.show(`Task added${task.due != null ? ` · ${dueLabel(task)}` : ''}`, {
      action: { label: 'Open', run: () => navigateTo({ path: '/tasks', query: { task: task.id } }) },
    })
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    busy.value = false
  }
}
const sender = computed(() => props.message.from?.name || props.message.from?.address || '')
</script>

<template>
  <PopMenu ref="pop" icon="task" label="Make a task" title="Task (T)">
    <template #default="{ close }">
      <form class="form" aria-label="New task from this email" @submit.prevent="add(close)">
        <UFormField label="Task" class="field">
          <UInput v-model="title" required class="w-full" />
        </UFormField>
        <div class="chips" role="group" aria-label="Due">
          <UButton
            v-for="c in chips"
            :key="c.id"
            :color="due === c.day ? 'primary' : 'neutral'"
            :variant="due === c.day ? 'soft' : 'outline'"
            size="sm"
            class="rounded-full"
            :aria-pressed="due === c.day"
            @click="pick(c.day)"
          >
            {{ c.label }}
          </UButton>
          <UButton
            v-if="!picking"
            :color="pickedLabel ? 'primary' : 'neutral'"
            :variant="pickedLabel ? 'soft' : 'outline'"
            size="sm"
            class="rounded-full"
            :aria-pressed="!!pickedLabel"
            @click="picking = true"
          >
            {{ pickedLabel ?? 'Pick a day…' }}
          </UButton>
          <UInput v-else v-model="pickIso" type="date" size="sm" aria-label="Due day" @change="pickDay" />
        </div>
        <UFormField label="List" orientation="horizontal" class="field">
          <USelect :model-value="list ?? undefined" :items="listItems" class="w-full" @update:model-value="(v) => (list = v as number)" />
        </UFormField>
        <p v-if="sameAccount" class="note">Same account as the email.</p>
        <p class="note link">
          <HIcon name="link" :size="12" />
          Linked to this email{{ sender ? ` and ${sender}` : '' }}
        </p>
        <div class="actions">
          <UButton color="neutral" variant="ghost" size="sm" type="button" @click="close">
            Cancel
          </UButton>
          <UButton size="sm" type="submit" :disabled="busy || !title.trim()">Add task</UButton>
        </div>
      </form>
    </template>
  </PopMenu>
</template>

<style scoped>
.form {
  width: 320px;
  max-width: calc(100vw - 32px);
  padding: 6px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.note {
  margin: -4px 0 0;
  font-size: 12px;
  color: var(--faint);
}
.note.link {
  display: flex;
  align-items: center;
  gap: 6px;
}
.actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>
