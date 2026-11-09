<script setup lang="ts">
import type { MailSettings } from '~/composables/useCompose'

// Settings › Your apps › Mail: how long "Undo" stays possible after "Send",
// and whether new messages are formatted or plain text.
const toast = useToasts()
const settings = ref<MailSettings | null>(null)
const undoChoices = [
  { label: 'Off — send at once', value: 0 },
  { label: '5 seconds', value: 5 },
  { label: '10 seconds', value: 10 },
  { label: '20 seconds', value: 20 },
  { label: '30 seconds', value: 30 },
]
const formats = [
  { label: 'Rich text (bold, links, lists…)', value: 'rich' },
  { label: 'Plain text', value: 'plain' },
]
onMounted(async () => {
  settings.value = await fetchMailSettings().catch(() => null)
})
async function save(change: Partial<MailSettings>, said: string) {
  if (!settings.value) return
  const before = settings.value
  settings.value = { ...before, ...change }
  try {
    settings.value = await saveMailSettings(settings.value)
    forgetMailFormat()
    toast.show(said)
  } catch (err) {
    settings.value = before
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
</script>

<template>
  <div v-if="settings" class="mail-settings">
    <UFormField
      label="Undo send"
      help="After Send, the message waits this long on the server before it goes: Undo takes it back. Closing the page doesn't stop it."
    >
      <USelect
        :model-value="settings.undo_seconds"
        :items="undoChoices"
        class="w-64"
        @update:model-value="(v) => save({ undo_seconds: v as number }, v ? `Undo send: ${v} seconds` : 'Undo send is off')"
      />
    </UFormField>
    <UFormField label="Write messages in" help="Recipients always get a plain-text version too.">
      <USelect
        :model-value="settings.compose_format"
        :items="formats"
        class="w-64"
        @update:model-value="
          (v) => save({ compose_format: v as MailSettings['compose_format'] }, v === 'plain' ? 'New messages: plain text' : 'New messages: rich text')
        "
      />
    </UFormField>
  </div>
</template>

<style scoped>
.mail-settings {
  display: flex;
  flex-direction: column;
  gap: 16px;
  margin: 8px 0 14px;
  max-width: 560px;
}
</style>
