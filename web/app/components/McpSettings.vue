<script setup lang="ts">
import type { McpToken } from '~/composables/useAi'

/** `embedded`: inside Settings › AI, under its own switch (no title). */
const props = defineProps<{ embedded?: boolean }>()

// Settings › AI › MCP: let Claude Code (or another AI app that speaks
// MCP) use Havre — search and read your mail, see your calendar, find
// free time, look up people, save drafts. Each app gets its own token.
const toast = useToasts()
const tokens = ref<McpToken[]>([])
const error = ref<string | null>(null)
const name = ref('Claude Code')
const busy = ref(false)
/** Shown once, right after connecting. */
const created = ref<{ token: McpToken; secret: string } | null>(null)
const confirmRevoke = ref<number | null>(null)

const origin = import.meta.client ? window.location.origin : 'http://localhost:8080'
const url = `${origin}/mcp`

async function load() {
  try {
    tokens.value = await fetchMcpTokens()
  } catch (err) {
    error.value = apiErrorMessage(err)
  }
}
onMounted(load)

async function connect() {
  busy.value = true
  error.value = null
  try {
    created.value = await createMcpToken(name.value)
    await load()
  } catch (err) {
    error.value = apiErrorMessage(err)
  } finally {
    busy.value = false
  }
}

async function revoke(t: McpToken) {
  if (confirmRevoke.value !== t.id) {
    confirmRevoke.value = t.id
    setTimeout(() => confirmRevoke.value === t.id && (confirmRevoke.value = null), 3000)
    return
  }
  try {
    await revokeMcpToken(t.id)
    if (created.value?.token.id === t.id) created.value = null
    toast.show(`${t.name} disconnected`)
    await load()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

const command = computed(() =>
  created.value
    ? `claude mcp add --transport http --scope user havre ${url} --header "Authorization: Bearer ${created.value.secret}"`
    : '',
)
const json = computed(() =>
  created.value
    ? JSON.stringify(
        { mcpServers: { havre: { type: 'http', url, headers: { Authorization: `Bearer ${created.value.secret}` } } } },
        null,
        2,
      )
    : '',
)

async function copy(text: string, what: string) {
  try {
    await navigator.clipboard.writeText(text)
    toast.show(`${what} copied`)
  } catch {
    toast.show('Select the text and copy it', { tone: 'error' })
  }
}

function when(seconds: number | null) {
  if (!seconds) return 'never used'
  return `used ${new Date(seconds * 1000).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' })}`
}
</script>

<template>
  <section id="claude-code" :aria-labelledby="props.embedded ? undefined : 'mcp-title'" :aria-label="props.embedded ? 'MCP connections' : undefined">
    <div v-if="!props.embedded" class="section-head">
      <div>
        <h2 id="mcp-title">Claude Code</h2>
        <p class="lead">
          Let Claude Code (or another AI app that speaks MCP) use Havre: search and read your mail, see your calendar,
          find free time, look up people and save drafts. It never sends mail.
        </p>
      </div>
    </div>

    <article class="card">
      <ul v-if="tokens.length" class="tokens">
        <li v-for="t in tokens" :key="t.id">
          <HIcon name="command" :size="15" />
          <span class="tname">{{ t.name }}</span>
          <span class="faint">{{ when(t.last_used_at) }}</span>
          <UButton
            :color="confirmRevoke === t.id ? 'error' : 'neutral'"
            variant="outline"
            size="sm"
            type="button"
            @click="revoke(t)"
          >
            {{ confirmRevoke === t.id ? 'Disconnect?' : 'Disconnect' }}
          </UButton>
        </li>
      </ul>

      <form class="connect" @submit.prevent="connect">
        <UFormField label="Name (to recognize it here)" class="field">
          <UInput v-model="name" aria-label="App name" maxlength="60" class="w-full" />
        </UFormField>
        <UButton size="sm" type="submit" :disabled="busy">Connect an app</UButton>
      </form>

      <div v-if="created" class="created" role="status">
        <p class="ok"><HIcon name="check" :size="14" /> “{{ created.token.name }}” can now use Havre. Copy this now: the key won't be shown again.</p>
        <p class="step">In a terminal, on the computer where you use Claude Code:</p>
        <div class="code">
          <code>{{ command }}</code>
          <UButton
            color="neutral"
            variant="outline"
            size="sm"
            type="button"
            @click="copy(command, 'Command')"
          >
            Copy
          </UButton>
        </div>
        <p class="step">Then ask Claude Code things like “what did Claire say about the budget?” or “draft a reply to the latest email from Thomas”. Other apps (a <code>.mcp.json</code> file, Cursor…):</p>
        <div class="code">
          <pre>{{ json }}</pre>
          <UButton
            color="neutral"
            variant="outline"
            size="sm"
            type="button"
            @click="copy(json, 'Configuration')"
          >
            Copy
          </UButton>
        </div>
        <p class="faint">
          Claude Code must reach {{ url }}: on another computer, use Havre's address on your network (or behind
          HTTPS), not localhost. Anyone with this key can read your mail: keep it like a password, disconnect it here
          if it leaks.
        </p>
      </div>
      <p v-if="error" class="error"><HIcon name="alert" :size="14" /> {{ error }}</p>
    </article>
  </section>
</template>

<style scoped>
section {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
h2 {
  margin: 0 0 4px;
  font-size: 16px;
}
.lead {
  margin: 0;
  color: var(--muted);
  font-size: 13.5px;
}
.card {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 16px 18px;
  border-radius: var(--radius-lg);
  border: 1px solid var(--line);
  background: var(--raise);
}
.tokens {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
}
.tokens li {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 0;
  border-bottom: 1px solid var(--line);
  font-size: 13.5px;
}
.tname {
  font-weight: 500;
}
.tokens .faint {
  flex: 1;
}
.faint {
  color: var(--faint);
  font-size: 12.5px;
  margin: 0;
}
.connect {
  display: flex;
  align-items: flex-end;
  gap: 10px;
  flex-wrap: wrap;
}
.field {
  flex: 1;
  min-width: 200px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.field > span {
  font-size: 12.5px;
  color: var(--muted);
}
.created {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-top: 12px;
  border-top: 1px solid var(--line);
}
.ok {
  display: flex;
  gap: 8px;
  align-items: center;
  margin: 0;
  color: var(--ok);
  font-size: 13.5px;
}
.step {
  margin: 4px 0 0;
  font-size: 13.5px;
}
.code {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 10px 12px;
  border-radius: 10px;
  border: 1px solid var(--line2);
  background: var(--bg);
}
.code code,
.code pre {
  flex: 1;
  min-width: 0;
  margin: 0;
  font-family: var(--font-mono);
  font-size: 12px;
  line-height: 1.5;
  overflow-wrap: anywhere;
  white-space: pre-wrap;
}
.error {
  display: flex;
  gap: 8px;
  margin: 0;
  color: var(--danger);
  font-size: 13px;
}
</style>
