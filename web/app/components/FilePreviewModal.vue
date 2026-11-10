<script setup lang="ts">
// The preview window for an attachment, in Mail and in Files: the file's
// name and size, Download, and the preview. Large on a computer, the whole
// screen on a phone. `Esc` or Close goes back.
const props = defineProps<{
  /** The file shown; `null` closes the window. */
  file: { url: string; name: string; mime: string; size: number } | null
}>()
const emit = defineEmits<{ close: [] }>()

const phone = useIsPhone()
const open = computed({
  get: () => props.file !== null,
  set: (value: boolean) => {
    if (!value) emit('close')
  },
})
// Kept while the window closes (its animation), so the content doesn't vanish first.
const shown = ref(props.file)
watch(
  () => props.file,
  (f) => {
    if (f) shown.value = f
  },
)
</script>

<template>
  <UModal
    v-model:open="open"
    :fullscreen="phone"
    :title="shown?.name ?? 'Preview'"
    :description="shown ? fileSize(shown.size) : undefined"
    :ui="{ content: phone ? '' : 'max-w-5xl h-[90dvh]', body: 'p-0 sm:p-0 overflow-hidden flex flex-col min-h-0' }"
  >
    <template #body>
      <LazyFilePreview v-if="shown" :url="shown.url" :name="shown.name" :mime="shown.mime" :size="shown.size" class="flex-1 min-h-0" />
    </template>
    <template #footer>
      <div class="flex justify-end gap-2 w-full">
        <slot name="actions" />
        <UButton v-if="shown" icon="i-havre-download" :href="shown.url" :download="shown.name">Download</UButton>
      </div>
    </template>
  </UModal>
</template>
