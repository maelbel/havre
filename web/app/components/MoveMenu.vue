<script setup lang="ts">
import type { Folder, FolderRole } from '~/composables/useMail'
import type { IconName } from './HIcon.vue'

// "Move to…" button with a list of folders.
const props = defineProps<{ folders: Folder[]; current: string }>()
const emit = defineEmits<{ move: [path: string] }>()

const open = ref(false)
const root = ref<HTMLElement | null>(null)
const menu = ref<HTMLElement | null>(null)
// Nested folders are shown with their parents ("Clients / Lyon"), since the
// parent itself may be hidden (it's the current folder).
const targets = computed(() => {
  const trail: string[] = []
  return props.folders
    .map((f) => {
      trail[f.depth] = f.role === 'inbox' ? 'Inbox' : f.name
      return { ...f, label: trail.slice(0, f.depth + 1).join(' / ') }
    })
    .filter((f) => f.path !== props.current)
})

const icons: Record<FolderRole, IconName> = {
  inbox: 'inbox',
  drafts: 'file',
  sent: 'send',
  archive: 'archive',
  junk: 'shield',
  trash: 'trash',
  all: 'apps',
  other: 'folder',
}

async function toggle() {
  open.value = !open.value
  if (open.value) {
    await nextTick()
    menu.value?.querySelector<HTMLButtonElement>('button')?.focus()
  }
}
function pick(path: string) {
  open.value = false
  emit('move', path)
}
// Arrow keys move between items; Escape closes.
function onMenuKey(e: KeyboardEvent) {
  const items = [...(menu.value?.querySelectorAll<HTMLButtonElement>('button') ?? [])]
  const i = items.indexOf(document.activeElement as HTMLButtonElement)
  if (e.key === 'ArrowDown') items[(i + 1) % items.length]?.focus()
  else if (e.key === 'ArrowUp') items[(i - 1 + items.length) % items.length]?.focus()
  else if (e.key === 'Escape') {
    open.value = false
    root.value?.querySelector<HTMLButtonElement>('.trigger')?.focus()
  } else return
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
  <div ref="root" class="move">
    <button
      type="button"
      class="icon-btn trigger"
      aria-label="Move to folder"
      title="Move to…"
      aria-haspopup="menu"
      :aria-expanded="open"
      @click="toggle"
    >
      <HIcon name="folder" :size="17" />
    </button>
    <div v-if="open" ref="menu" class="menu" role="menu" aria-label="Move to" @keydown="onMenuKey">
      <div class="eyebrow head">Move to</div>
      <button
        v-for="f in targets"
        :key="f.path"
        type="button"
        role="menuitem"
        class="item"
        @click="pick(f.path)"
      >
        <HIcon :name="icons[f.role]" :size="15" />
        <span class="label">{{ f.label }}</span>
      </button>
    </div>
  </div>
</template>

<style scoped>
.move {
  position: relative;
}
.menu {
  position: absolute;
  top: calc(100% + 6px);
  left: 0;
  z-index: 20;
  min-width: 220px;
  max-height: 340px;
  overflow: auto;
  padding: 6px;
  border-radius: var(--radius);
  border: 1px solid var(--line2);
  background: var(--raise);
  box-shadow: 0 16px 48px rgba(0, 0, 0, 0.35);
}
.head {
  padding: 6px 10px 8px;
}
.item {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 10px;
  height: 34px;
  padding: 0 10px;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: var(--text);
  text-align: left;
  cursor: pointer;
}
.item:hover,
.item:focus-visible {
  background: var(--hover);
  outline: none;
}
.label {
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.item :deep(svg) {
  flex: 0 0 auto;
  color: var(--muted);
}
</style>
