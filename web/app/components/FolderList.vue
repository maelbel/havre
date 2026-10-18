<script setup lang="ts">
import type { Folder, FolderRole } from '~/composables/useMail'
import type { IconName } from './HIcon.vue'

withDefaults(defineProps<{ folders: Folder[]; current: string | null; loading: boolean; navLabel?: string }>(), {
  navLabel: 'Folders',
})
const emit = defineEmits<{ select: [path: string] }>()

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

const label = (f: Folder) => (f.role === 'inbox' ? 'Inbox' : f.name)
</script>

<template>
  <nav class="folders" :aria-label="navLabel">
    <template v-if="loading && !folders.length">
      <span v-for="n in 5" :key="n" class="folder skeleton" aria-hidden="true" />
    </template>
    <button
      v-for="f in folders"
      :key="f.path"
      type="button"
      class="folder"
      :class="{ active: f.path === current }"
      :style="{ paddingLeft: `${8 + f.depth * 16}px` }"
      :aria-current="f.path === current ? 'page' : undefined"
      @click="emit('select', f.path)"
    >
      <HIcon :name="icons[f.role]" :size="16" />
      <span class="name">{{ label(f) }}</span>
      <span v-if="f.unread && !['sent', 'drafts', 'trash'].includes(f.role)" class="unread mono" :aria-label="`${f.unread} unread`">
        {{ f.unread > 999 ? '999+' : f.unread }}
      </span>
    </button>
  </nav>
</template>

<style scoped>
.folders {
  display: flex;
  flex-direction: column;
  gap: 2px;
  overflow: auto;
}
.folder {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 34px;
  padding: 0 8px;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--muted);
  text-align: left;
  cursor: pointer;
}
.folder:hover {
  background: var(--hover);
  color: var(--text);
}
.folder.active {
  background: var(--sel);
  color: var(--text);
}
.name {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.unread {
  font-size: 11.5px;
  color: var(--muted);
}
.folder.active .unread,
.folder:hover .unread {
  color: var(--text);
}
.skeleton {
  margin: 3px 8px;
  min-height: 26px;
  background: var(--line);
  opacity: 0.5;
}
</style>
