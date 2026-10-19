<script setup lang="ts">
import type { IconName } from './HIcon.vue'

// A button with a menu under it (Snooze, Label, Make a task): Nuxt UI's
// UPopover. The menu's content is the default slot; it gets `close` to call
// when an item is chosen. For a plain list of choices, UDropdownMenu is
// simpler (see MoveMenu); this one is for menus with a field in them.
const props = withDefaults(
  defineProps<{ icon: IconName; label: string; title?: string; align?: 'left' | 'right'; text?: string }>(),
  { align: 'left', title: undefined, text: undefined },
)
const open = ref(false)
const menu = ref<HTMLElement | null>(null)
const trigger = ref<{ $el: HTMLElement } | null>(null)

// Opening: the first item (or field) gets the focus, as in a menu.
watch(open, async (now) => {
  if (!now) return
  await nextTick()
  menu.value?.querySelector<HTMLElement>('input, button')?.focus()
})
function toggle() {
  open.value = !open.value
}
function close() {
  open.value = false
  trigger.value?.$el?.focus()
}
defineExpose({ toggle, close })

// Arrow keys move between items (Escape is UPopover's).
function onMenuKey(e: KeyboardEvent) {
  const items = [...(menu.value?.querySelectorAll<HTMLElement>('button, input') ?? [])]
  const i = items.indexOf(document.activeElement as HTMLElement)
  if (e.key === 'ArrowDown') items[(i + 1) % items.length]?.focus()
  else if (e.key === 'ArrowUp') items[(i - 1 + items.length) % items.length]?.focus()
  else return
  e.preventDefault()
  e.stopPropagation()
}
</script>

<template>
  <UPopover v-model:open="open" :content="{ align: props.align === 'right' ? 'end' : 'start', sideOffset: 6 }">
    <UButton
      ref="trigger"
      :icon="havreIcon(props.icon)"
      color="neutral"
      variant="ghost"
      :size="text ? 'sm' : 'md'"
      :square="!text"
      :label="text"
      :aria-label="props.label"
      :title="title ?? props.label"
      aria-haspopup="menu"
    />
    <template #content>
      <div ref="menu" class="menu" role="menu" :aria-label="props.label" @keydown="onMenuKey">
        <slot :close="close" />
      </div>
    </template>
  </UPopover>
</template>

<style scoped>
.menu {
  min-width: 240px;
  max-height: 380px;
  overflow: auto;
  padding: 6px;
}
</style>
