<script setup lang="ts">
// One account's mailboxes in the Mail sidebar: its Inbox first, then its
// other folders. The page shows one of these per account, inside an
// accordion (or on its own with a single account).
//
// It loads its own folder list, and only once it is shown: Nuxt UI's
// accordion mounts a section's content when it opens, so a closed account
// costs nothing.
const props = defineProps<{
  account: number
  /** The open folder's path, when it belongs to this account. */
  current: string | null
  /** Changes when the server says this account's folders changed. */
  version: number
}>()
const emit = defineEmits<{ select: [path: string] }>()

const id = computed(() => props.account)
const folders = useFolders(id)
watch(
  () => props.version,
  () => folders.refresh(),
)

// The Inbox on top, then the rest in the server's order (a stable sort).
const ordered = computed(() => {
  const list = folders.data.value
  return [...list.filter((f) => f.role === 'inbox'), ...list.filter((f) => f.role !== 'inbox')]
})
</script>

<template>
  <FolderList
    :folders="ordered"
    :current="current"
    :loading="folders.status.value === 'pending'"
    nav-label="Mailboxes"
    @select="(path) => emit('select', path)"
  />
</template>
