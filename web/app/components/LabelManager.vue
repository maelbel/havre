<script setup lang="ts">
import type { Label } from '~/composables/useMail'

// Edit labels: rename, recolor, delete (the messages stay), add.
const emit = defineEmits<{ close: [] }>()
const { state, refresh } = useOrganize()
const toast = useToasts()
const names = reactive<Record<number, string>>({})
watch(
  () => state.value.labels,
  (labels) => {
    for (const l of labels) if (names[l.id] === undefined) names[l.id] = l.name
  },
  { immediate: true },
)
const confirmDelete = ref<number | null>(null)
const fresh = ref('')

async function run(job: () => Promise<unknown>) {
  try {
    await job()
    await refresh()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
function rename(l: Label) {
  const name = names[l.id]?.trim()
  if (!name || name === l.name) return
  run(() => updateLabel(l.id, { name }))
}
function recolor(l: Label, color: string) {
  run(() => updateLabel(l.id, { color }))
}
function remove(l: Label) {
  if (confirmDelete.value !== l.id) {
    confirmDelete.value = l.id
    return
  }
  confirmDelete.value = null
  run(() => deleteLabel(l.id))
}
function add() {
  const name = fresh.value.trim()
  if (!name) return
  run(async () => {
    await createLabel(name, nextLabelColor(state.value.labels.length))
    fresh.value = ''
  })
}
</script>

<template>
  <!-- Esc and the backdrop close it (UModal). -->
  <UModal
    :open="true"
    title="Labels"
    description="Labels are Havre's: they work on every account and follow a message when it moves. Your phone's mail app doesn't see them."
    @update:open="(open) => !open && emit('close')"
  >
    <template #body>
      <ul class="list">
        <li v-for="l in state.labels" :key="l.id">
          <span class="hash mono" :style="{ color: `var(--c-${l.color})` }" aria-hidden="true">#</span>
          <UInput
            v-model="names[l.id]"
            :aria-label="`Name of ${l.name}`"
            variant="ghost"
            class="grow"
            maxlength="40"
            @blur="rename(l)"
            @keydown.enter.prevent="($event.target as HTMLInputElement).blur()"
          />
          <span class="colors" role="radiogroup" :aria-label="`Color of ${l.name}`">
            <button
              v-for="c in LABEL_COLORS"
              :key="c"
              type="button"
              role="radio"
              class="swatch"
              :aria-checked="l.color === c"
              :aria-label="c"
              :style="{ background: `var(--c-${c})` }"
              @click="recolor(l, c)"
            />
          </span>
          <span class="mono faint count">{{ l.count }}</span>
          <UButton
            :color="confirmDelete === l.id ? 'error' : 'neutral'"
            variant="ghost"
            size="xs"
            type="button"
            @click="remove(l)"
          >
            {{ confirmDelete === l.id ? 'Delete?' : 'Delete' }}
          </UButton>
        </li>
      </ul>
    </template>
    <template #footer>
      <form class="add w-full" @submit.prevent="add">
        <UInput v-model="fresh" placeholder="New label" aria-label="New label" maxlength="40" class="grow" />
        <UButton size="sm" type="submit" :disabled="!fresh.trim()">Add</UButton>
      </form>
    </template>
  </UModal>
</template>

<style scoped>
.list {
  list-style: none;
  margin: 0;
  padding: 0;
}
.list li {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 0;
  border-top: 1px solid var(--line);
}
.hash {
  width: 12px;
  text-align: center;
}
.colors {
  display: flex;
  gap: 4px;
}
.swatch {
  width: 16px;
  height: 16px;
  border-radius: 50%;
  border: 2px solid transparent;
  cursor: pointer;
}
.swatch[aria-checked='true'] {
  border-color: var(--text);
}
.count {
  width: 28px;
  text-align: right;
  font-size: 12px;
}
.faint {
  color: var(--faint);
}
.add {
  display: flex;
  gap: 8px;
}
.grow {
  flex: 1;
  min-width: 0;
}
</style>
