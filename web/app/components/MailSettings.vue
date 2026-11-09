<script setup lang="ts">
// Settings › Your apps › Mail: how long "Undo" stays possible after "Send".
const toast = useToasts()
const undo = ref<number | null>(null)
const choices = [
  { label: 'Off — send at once', value: 0 },
  { label: '5 seconds', value: 5 },
  { label: '10 seconds', value: 10 },
  { label: '20 seconds', value: 20 },
  { label: '30 seconds', value: 30 },
]
onMounted(async () => {
  undo.value = (await fetchMailSettings().catch(() => null))?.undo_seconds ?? null
})
async function save(value: number) {
  const before = undo.value
  undo.value = value
  try {
    await saveMailSettings({ undo_seconds: value })
    toast.show(value ? `Undo send: ${value} seconds` : 'Undo send is off')
  } catch (err) {
    undo.value = before
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
</script>

<template>
  <UFormField
    v-if="undo !== null"
    label="Undo send"
    help="After Send, the message waits this long on the server before it goes: Undo takes it back. Closing the page doesn't stop it."
    class="mail-settings"
  >
    <USelect :model-value="undo" :items="choices" class="w-64" @update:model-value="(v) => save(v as number)" />
  </UFormField>
</template>

<style scoped>
.mail-settings {
  margin: 8px 0 14px;
  max-width: 560px;
}
</style>
