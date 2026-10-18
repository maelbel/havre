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
    <fieldset :disabled="saving || !progress">
      <legend class="strong">How far back to look</legend>
      <label v-for="c in CHOICES" :key="c.depth" class="choice" :class="{ on: progress?.depth === c.depth }">
        <input type="radio" name="files-depth" :checked="progress?.depth === c.depth" @change="choose(c.depth)" />
        <span class="grow">{{ c.label }}</span>
        <span v-if="c.note" class="faint small">{{ c.note }}</span>
      </label>
    </fieldset>
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
fieldset {
  margin: 0;
  padding: 0;
  border: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
legend {
  padding: 0;
  margin-bottom: 4px;
  font-size: 13.5px;
  font-weight: 500;
}
.choice {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border-radius: 10px;
  border: 1px solid var(--line);
  font-size: 13.5px;
  cursor: pointer;
}
.choice.on {
  border-color: var(--accent);
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
