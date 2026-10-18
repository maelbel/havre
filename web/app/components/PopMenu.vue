<script setup lang="ts">
import type { IconName } from './HIcon.vue'

// A button with a menu under it (Snooze, Label…). The menu's content is the
// default slot; it gets `close` to call when an item is chosen.
const props = withDefaults(
  defineProps<{ icon: IconName; label: string; title?: string; align?: 'left' | 'right'; text?: string }>(),
  { align: 'left', title: undefined, text: undefined },
)
const open = ref(false)
const root = ref<HTMLElement | null>(null)
const menu = ref<HTMLElement | null>(null)

async function toggle() {
  open.value = !open.value
  if (open.value) {
    await nextTick()
    menu.value?.querySelector<HTMLElement>('input, button')?.focus()
  }
}
function close() {
  open.value = false
  root.value?.querySelector<HTMLButtonElement>('.trigger')?.focus()
}
defineExpose({ toggle, close })

// Arrow keys move between items; Escape closes.
function onMenuKey(e: KeyboardEvent) {
  const items = [...(menu.value?.querySelectorAll<HTMLElement>('button, input') ?? [])]
  const i = items.indexOf(document.activeElement as HTMLElement)
  if (e.key === 'ArrowDown') items[(i + 1) % items.length]?.focus()
  else if (e.key === 'ArrowUp') items[(i - 1 + items.length) % items.length]?.focus()
  else if (e.key === 'Escape') close()
  else return
  e.preventDefault()
  e.stopPropagation()
}
function onOutside(e: MouseEvent) {
  if (open.value && !root.value?.contains(e.target as Node)) open.value = false
}
onMounted(() => document.addEventListener('mousedown', onOutside))
onBeforeUnmount(() => document.removeEventListener('mousedown', onOutside))
</script>

<template>
  <div ref="root" class="pop">
    <button
      type="button"
      class="trigger"
      :class="text ? 'text-btn' : 'icon-btn'"
      :aria-label="props.label"
      :title="title ?? props.label"
      aria-haspopup="menu"
      :aria-expanded="open"
      @click="toggle"
    >
      <HIcon :name="icon" :size="text ? 15 : 17" />
      <span v-if="text">{{ text }}</span>
    </button>
    <div v-if="open" ref="menu" class="menu" :class="align" role="menu" :aria-label="props.label" @keydown="onMenuKey">
      <slot :close="close" />
    </div>
  </div>
</template>

<style scoped>
.pop {
  position: relative;
}
.text-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 30px;
  padding: 0 10px;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: var(--muted);
  font: inherit;
  font-size: 13px;
  cursor: pointer;
}
.text-btn:hover {
  background: var(--hover);
  color: var(--text);
}
.menu {
  position: absolute;
  top: calc(100% + 6px);
  left: 0;
  z-index: 30;
  min-width: 240px;
  max-height: 380px;
  overflow: auto;
  padding: 6px;
  border-radius: var(--radius);
  border: 1px solid var(--line2);
  background: var(--raise);
  box-shadow: 0 16px 48px rgba(0, 0, 0, 0.35);
}
.menu.right {
  left: auto;
  right: 0;
}
</style>
