<script setup lang="ts">
import type { Contact } from '~/composables/useCompose'

// A recipient field: comma-separated addresses, with suggestions from
// People (who wrote to you, who you met), for the address being typed.
// `bare`: a plain form field (the label is outside, e.g. event guests).
const model = defineModel<string>({ required: true })
const props = defineProps<{
  label: string
  autofocus?: boolean
  bare?: boolean
  /** Insert `anna@x.org` rather than `Anna <anna@x.org>`. */
  addressOnly?: boolean
  placeholder?: string
}>()
const fieldId = computed(() => `rcpt-${props.label.toLowerCase().replace(/\W+/g, '-')}`)

const input = ref<HTMLInputElement | null>(null)
const suggestions = ref<Contact[]>([])
const active = ref(0)
let timer: ReturnType<typeof setTimeout> | undefined
let asked = ''

/** The address being typed: what follows the last comma. */
function current() {
  const text = model.value.slice(0, input.value?.selectionStart ?? model.value.length)
  return text.split(/[,;]/).pop()!.trim()
}

function onInput() {
  clearTimeout(timer)
  const q = current()
  if (q.length < 2) return void (suggestions.value = [])
  timer = setTimeout(async () => {
    asked = q
    const found = await searchContacts(q).catch(() => [])
    if (asked !== q) return
    // Leave out who's already in the field.
    suggestions.value = found.filter((c) => !model.value.toLowerCase().includes(c.address.toLowerCase())).slice(0, 6)
    active.value = 0
  }, 150)
}

function pick(c: Contact) {
  const parts = model.value.split(/[,;]/)
  parts[parts.length - 1] = ` ${props.addressOnly ? c.address : contactLabel(c)}`
  model.value = parts.map((p) => p.trim()).filter(Boolean).join(', ') + ', '
  suggestions.value = []
  nextTick(() => input.value?.focus())
}

function onKeydown(e: KeyboardEvent) {
  if (!suggestions.value.length) return
  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    e.preventDefault()
    const n = suggestions.value.length
    active.value = (active.value + (e.key === 'ArrowDown' ? 1 : n - 1)) % n
  } else if (e.key === 'Enter' || e.key === 'Tab') {
    e.preventDefault()
    pick(suggestions.value[active.value]!)
  } else if (e.key === 'Escape') {
    e.stopPropagation()
    suggestions.value = []
  }
}

function onBlur() {
  // Let a click on a suggestion land first.
  setTimeout(() => (suggestions.value = []), 150)
}

onMounted(() => props.autofocus && input.value?.focus())
defineExpose({ focus: () => input.value?.focus() })
</script>

<template>
  <div class="recipients" :class="{ bare }">
    <label v-if="!bare" :for="fieldId">{{ label }}</label>
    <div class="wrap">
      <input
        :id="fieldId"
        ref="input"
        v-model="model"
        type="text"
        autocomplete="off"
        spellcheck="false"
        role="combobox"
        :aria-expanded="suggestions.length > 0"
        :aria-controls="`${fieldId}-list`"
        :placeholder="placeholder"
        @input="onInput"
        @keydown="onKeydown"
        @blur="onBlur"
      />
      <ul v-if="suggestions.length" :id="`${fieldId}-list`" class="suggest" role="listbox">
        <li
          v-for="(c, i) in suggestions"
          :key="c.address"
          role="option"
          :aria-selected="i === active"
          :class="{ active: i === active }"
          @mousedown.prevent="pick(c)"
        >
          <span class="name">{{ c.name || c.address }}</span>
          <span v-if="c.name" class="addr">{{ c.address }}</span>
        </li>
      </ul>
    </div>
    <slot />
  </div>
</template>

<style scoped>
.recipients {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 40px;
  padding: 0 16px;
  border-bottom: 1px solid var(--line);
}
.recipients.bare {
  display: block;
  min-height: 0;
  padding: 0;
  border: 0;
}
.bare input {
  height: 38px;
  padding: 0 10px;
  border-radius: 9px;
  border: 1px solid var(--line2);
  background: var(--bg);
}
.bare input:focus {
  border-color: var(--accent);
}
.bare .suggest {
  left: 0;
}
label {
  flex: 0 0 auto;
  min-width: 34px;
  font-size: 13px;
  color: var(--faint);
}
.wrap {
  position: relative;
  flex: 1;
  min-width: 0;
}
input {
  width: 100%;
  height: 38px;
  border: 0;
  background: transparent;
  color: var(--text);
  outline: none;
  font-size: 14px;
}
.suggest {
  position: absolute;
  z-index: 5;
  top: 100%;
  left: -8px;
  right: 0;
  margin: 2px 0 0;
  padding: 4px;
  list-style: none;
  border-radius: 10px;
  border: 1px solid var(--line2);
  background: var(--raise);
  box-shadow: 0 16px 40px -16px rgb(0 0 0 / 0.5);
}
.suggest li {
  display: flex;
  gap: 8px;
  align-items: baseline;
  padding: 7px 10px;
  border-radius: 7px;
  cursor: pointer;
  font-size: 13.5px;
}
.suggest li.active {
  background: var(--sel);
}
.addr {
  color: var(--faint);
  font-size: 12.5px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
