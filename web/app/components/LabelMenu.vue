<script setup lang="ts">
// "Label as…": tick labels on or off for the message(s); type a new name
// and Enter to create one.
const props = withDefaults(
  defineProps<{
    /** For each label id: on every chosen message, on some, or none. */
    state: Record<number, 'all' | 'some'>
    text?: string
  }>(),
  { text: undefined },
)
const emit = defineEmits<{ toggle: [label: number, on: boolean] }>()
const { state: organize, refresh } = useOrganize()
const toast = useToasts()
const filter = ref('')
const shown = computed(() => {
  const f = filter.value.trim().toLowerCase()
  return organize.value.labels.filter((l) => !f || l.name.toLowerCase().includes(f))
})
const canCreate = computed(() => {
  const f = filter.value.trim()
  return f && !organize.value.labels.some((l) => l.name.toLowerCase() === f.toLowerCase())
})
async function create() {
  if (!canCreate.value) {
    const only = shown.value.length === 1 ? shown.value[0] : null
    if (only) emit('toggle', only.id, props.state[only.id] !== 'all')
    return
  }
  try {
    const label = await createLabel(filter.value.trim(), nextLabelColor(organize.value.labels.length))
    filter.value = ''
    await refresh()
    emit('toggle', label.id, true)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
</script>

<template>
  <PopMenu icon="tag" label="Label" title="Label (L)" :text="props.text">
    <div class="eyebrow head">Label as</div>
    <form class="find" @submit.prevent="create">
      <label class="sr-only" for="label-find">Find or create a label</label>
      <input id="label-find" v-model="filter" type="text" placeholder="Find or create…" autocomplete="off" />
    </form>
    <button
      v-for="l in shown"
      :key="l.id"
      type="button"
      role="menuitemcheckbox"
      :aria-checked="state[l.id] === 'all' ? 'true' : state[l.id] === 'some' ? 'mixed' : 'false'"
      class="item"
      @click="emit('toggle', l.id, state[l.id] !== 'all')"
    >
      <span class="box" :class="state[l.id]" aria-hidden="true">
        <HIcon v-if="state[l.id] === 'all'" name="check" :size="11" :stroke-width="2.6" />
        <span v-else-if="state[l.id] === 'some'" class="dash" />
      </span>
      <span class="hash mono" :style="{ color: `var(--c-${l.color})` }" aria-hidden="true">#</span>
      <span class="grow">{{ l.name }}</span>
    </button>
    <button v-if="canCreate" type="button" role="menuitem" class="item" @click="create">
      <HIcon name="plus" :size="14" />
      <span class="grow">Create “{{ filter.trim() }}”</span>
    </button>
    <p v-if="!organize.labels.length && !filter" class="none">No labels yet: type a name to create one.</p>
  </PopMenu>
</template>

<style scoped>
.head {
  padding: 6px 10px 8px;
}
.find {
  padding: 0 4px 6px;
}
.find input {
  width: 100%;
  height: 32px;
  padding: 0 10px;
  border-radius: 8px;
  border: 1px solid var(--line2);
  background: var(--bg);
  color: var(--text);
  font: inherit;
  font-size: 13px;
  outline: none;
}
.find input:focus {
  border-color: var(--accent);
}
.item {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 10px;
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
.box {
  flex: 0 0 15px;
  height: 15px;
  display: grid;
  place-items: center;
  border-radius: 4px;
  border: 1.5px solid var(--line2);
  color: var(--on-accent);
}
.box.all,
.box.some {
  border-color: var(--accent);
  background: var(--accent);
}
.dash {
  width: 7px;
  height: 2px;
  border-radius: 1px;
  background: var(--on-accent);
}
.hash {
  width: 10px;
  text-align: center;
}
.grow {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.none {
  margin: 4px 10px 8px;
  font-size: 12.5px;
  color: var(--faint);
}
</style>
