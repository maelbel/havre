<script setup lang="ts">
import type { Folder, FolderRole } from '~/composables/useMail'
import type { IconName } from './HIcon.vue'

// "Move to…" button with a list of folders (Nuxt UI's UDropdownMenu:
// arrow keys, typing a name's first letters, Escape).
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

// The menu's lines: a heading, then one per folder.
const items = computed(() => [
  { label: 'Move to', type: 'label' as const },
  ...targets.value.map((f) => ({
    label: f.label,
    icon: havreIcon(icons[f.role]),
    onSelect: () => emit('move', f.path),
  })),
])
</script>

<template>
  <UDropdownMenu :items="items" :content="{ align: 'start', sideOffset: 6 }" :ui="{ content: 'max-h-[340px]' }">
    <UButton
      icon="i-havre-folder"
      color="neutral"
      variant="ghost"
      square
      aria-label="Move to folder"
      title="Move to…"
    />
  </UDropdownMenu>
</template>
