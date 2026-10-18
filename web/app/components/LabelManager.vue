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
function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') emit('close')
}
</script>

<template>
  <div class="backdrop" @mousedown.self="emit('close')" @keydown="onKey">
    <div class="sheet" role="dialog" aria-label="Labels">
      <header>
        <div>
          <div class="eyebrow">Mail</div>
          <h3 class="serif">Labels</h3>
        </div>
        <button type="button" class="icon-btn" aria-label="Close" @click="emit('close')"><HIcon name="close" :size="15" /></button>
      </header>
      <p class="lead">
        Labels are Havre's: they work on every account and follow a message when it moves. Your phone's mail app
        doesn't see them.
      </p>
      <ul class="list">
        <li v-for="l in state.labels" :key="l.id">
          <span class="hash mono" :style="{ color: `var(--c-${l.color})` }" aria-hidden="true">#</span>
          <input
            v-model="names[l.id]"
            type="text"
            :aria-label="`Name of ${l.name}`"
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
          <button type="button" class="del" :class="{ armed: confirmDelete === l.id }" @click="remove(l)">
            {{ confirmDelete === l.id ? 'Delete?' : 'Delete' }}
          </button>
        </li>
      </ul>
      <form class="add" @submit.prevent="add">
        <input v-model="fresh" type="text" placeholder="New label" aria-label="New label" maxlength="40" />
        <button type="submit" class="btn btn-primary small" :disabled="!fresh.trim()">Add</button>
      </form>
    </div>
  </div>
</template>

<style scoped>
.backdrop {
  position: fixed;
  inset: 0;
  z-index: 60;
  display: grid;
  place-items: center;
  padding: 16px;
  background: rgb(0 0 0 / 0.45);
}
.sheet {
  width: min(520px, 100%);
  max-height: min(80vh, 680px);
  overflow: auto;
  padding: 24px;
  border-radius: 16px;
  border: 1px solid var(--line);
  background: var(--raise);
  box-shadow: 0 24px 60px rgb(0 0 0 / 0.35);
}
header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
}
h3 {
  margin: 4px 0 0;
  font-size: 24px;
  font-weight: 400;
}
.lead {
  margin: 10px 0 16px;
  font-size: 13px;
  line-height: 1.55;
  color: var(--muted);
}
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
.list input,
.add input {
  flex: 1;
  min-width: 0;
  height: 32px;
  padding: 0 10px;
  border-radius: 8px;
  border: 1px solid transparent;
  background: transparent;
  color: var(--text);
  font: inherit;
  font-size: 14px;
  outline: none;
}
.list input:hover,
.list input:focus,
.add input {
  border-color: var(--line2);
  background: var(--bg);
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
.del {
  border: 0;
  background: none;
  color: var(--faint);
  font: inherit;
  font-size: 12.5px;
  cursor: pointer;
}
.del:hover,
.del.armed {
  color: var(--danger);
}
.add {
  display: flex;
  gap: 8px;
  margin-top: 12px;
}
.btn.small {
  height: 32px;
}
</style>
