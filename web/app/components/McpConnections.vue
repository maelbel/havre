<script setup lang="ts">
import type { AiSettings } from '~/composables/useAi'

// Settings › Connections (MCP): other AI apps (Claude Code, Claude
// Desktop…) using Havre's mail and agenda, each with its own key. This
// works without the AI app: it's Havre serving others, not Havre's own AI.
const toast = useToasts()
const saved = ref<AiSettings | null>(null)
const busy = ref(false)
const error = ref<string | null>(null)

onMounted(async () => {
  try {
    saved.value = await fetchAiSettings()
  } catch (err) {
    error.value = apiErrorMessage(err)
  }
})

async function toggle() {
  if (!saved.value) return
  busy.value = true
  try {
    const on = !saved.value.mcp_enabled
    saved.value = await saveAiSettings({ mcp_enabled: on })
    toast.show(on ? 'Other AI apps can connect' : 'MCP connections off')
  } catch (err) {
    error.value = apiErrorMessage(err)
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <div class="mcp-page">
    <header>
      <h2 class="serif">Connections (MCP)</h2>
      <p class="muted">
        Let other AI apps — Claude Code, Claude Desktop, any app that speaks MCP — search and read your mail, see your
        calendar and save drafts. Each gets its own key; nothing is sent without you.
      </p>
    </header>
    <UAlert v-if="error" color="error" variant="soft" icon="i-havre-alert" :title="error" role="alert" />
    <template v-if="saved">
      <USwitch
        :model-value="saved.mcp_enabled"
        :disabled="busy"
        label="Expose Havre over MCP"
        description="At /mcp: other AI apps reach it with a key."
        class="row"
        @update:model-value="toggle"
      />
      <McpSettings v-if="saved.mcp_enabled" embedded />
    </template>
  </div>
</template>

<style scoped>
.mcp-page {
  display: flex;
  flex-direction: column;
  gap: 24px;
  max-width: 720px;
}
h2 {
  margin: 0 0 6px;
  font-weight: 400;
  font-size: 32px;
  letter-spacing: -0.015em;
}
header p {
  margin: 0;
  line-height: 1.6;
}
.muted {
  color: var(--muted);
}
.strong {
  font-weight: 500;
}
.row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 10px;
  margin: 0 -10px;
  border-radius: 10px;
  cursor: pointer;
}
</style>
