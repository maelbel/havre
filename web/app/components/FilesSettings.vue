<script setup lang="ts">
import type { FilesProgress } from '~/composables/useFiles'

// Settings › Your apps › Files: how far back Havre looks through your mail
// for files, and how far it has got.
const toast = useToasts()
const progress = ref<FilesProgress | null>(null)
const saving = ref(false)
onMounted(async () => {
  progress.value = await fetchFilesProgress().catch(() => null)
})

const CHOICES = [
  { depth: 2000, label: 'The newest 2 000 emails of each inbox and Sent', note: 'default' },
  { depth: 10000, label: 'The newest 10 000', note: '' },
  { depth: 0, label: 'Everything', note: 'can take days on a big mailbox' },
]
async function choose(depth: number) {
  if (saving.value || progress.value?.depth === depth) return
  saving.value = true
  try {
    progress.value = await setFilesDepth(depth)
    toast.show('Saved: Havre looks a little further every few minutes')
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <div class="files-settings">
    <URadioGroup
      :model-value="progress?.depth"
      :items="CHOICES.map((c) => ({ label: c.label, value: c.depth, description: c.note }))"
      legend="How far back to look"
      variant="card"
      :disabled="saving || !progress"
      :ui="{ legend: 'font-medium mb-2', fieldset: 'gap-2' }"
      @update:model-value="(d) => choose(d as number)"
    />
    <p class="muted small">
      To find files, Havre downloads each email once (a little every few minutes), notes its attachments, then lets the
      copy go. Your mail server keeps the files.
      <template v-if="progress">
        So far: {{ progress.looked.toLocaleString() }} of {{ progress.total.toLocaleString() }} emails.
      </template>
    </p>
  </div>
</template>

<style scoped>
.files-settings {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.grow {
  flex: 1;
}
.faint {
  color: var(--faint);
}
.muted {
  color: var(--muted);
}
.small {
  font-size: 12.5px;
  line-height: 1.55;
}
p {
  margin: 0;
}
</style>
