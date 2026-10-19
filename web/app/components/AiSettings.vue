<script setup lang="ts">
import type { AiProvider, AiSettings, AiSettingsUpdate } from '~/composables/useAi'

// Settings › Your apps › AI, as in the design: where the model runs
// (one card each, the chosen one opens its settings), what it may read,
// how it behaves, and the activity log. Switches save at once; text
// fields save with their Save button.
const toast = useToasts()
const { status, refresh: refreshStatus } = useAiStatus()

const saved = ref<AiSettings | null>(null)
const error = ref<string | null>(null)
const busy = ref<string | null>(null)
const testReply = ref<string | null>(null)
const form = reactive({ url: '', model: '', apiKey: '', embedUrl: '', embedModel: '', briefTime: '07:30' })
const models = ref<string[]>([])
const host = import.meta.client ? window.location.hostname : 'this server'

function fill(s: AiSettings) {
  saved.value = s
  Object.assign(form, {
    url: s.url,
    model: s.model,
    apiKey: '',
    embedUrl: s.embed_url,
    embedModel: s.embed_model,
    briefTime: s.brief_time,
  })
}
onMounted(async () => {
  try {
    fill(await fetchAiSettings())
    checkHealth()
  } catch (err) {
    error.value = apiErrorMessage(err)
  }
})

async function update(changes: AiSettingsUpdate, what = 'save') {
  busy.value = what
  error.value = null
  testReply.value = null
  try {
    fill(await saveAiSettings(changes))
    return true
  } catch (err) {
    error.value = apiErrorMessage(err)
    return false
  } finally {
    busy.value = null
  }
}

// ---- Where the model runs -------------------------------------------------------------
// Three cards: this server (Ollama), Claude Code (your plan), or an external
// provider (OpenAI-compatible or Anthropic, with a key).
type Where = 'local' | 'claude' | 'external'
const where = computed<Where>(() => {
  const p = saved.value?.provider
  return p === 'ollama' ? 'local' : p === 'claude-code' ? 'claude' : 'external'
})
async function choose(w: Where) {
  if (w === where.value) return
  const provider: AiProvider = w === 'local' ? 'ollama' : w === 'claude' ? 'claude-code' : 'openai'
  models.value = []
  if (await update({ provider, url: '', model: '' }, 'where')) checkHealth()
}
async function chooseExternal(provider: AiProvider) {
  if (provider === saved.value?.provider) return
  models.value = []
  await update({ provider, url: '', model: '' }, 'where')
}
// The three choices, as cards.
const whereItems: { value: Where; label: string; description: string }[] = [
  { value: 'local', label: 'On this server', description: `Runs through Ollama. Your mail and calendar never leave ${host}.` },
  {
    value: 'claude',
    label: 'Claude, with your Claude plan',
    description: 'Through Claude Code, signed in with your Claude account (Pro, Max, Team…). No API key.',
  },
  {
    value: 'external',
    label: 'External provider',
    description: 'Mistral, Anthropic, or any OpenAI-compatible API. More capable, needs an API key.',
  },
]
const providerInfo = computed(() => AI_PROVIDERS.find((p) => p.value === saved.value?.provider)!)

const dirty = computed(() => {
  const s = saved.value
  if (!s) return false
  return form.url !== s.url || form.model !== s.model || form.apiKey !== ''
})
async function saveModel() {
  const ok = await update(
    { url: form.url, model: form.model, ...(form.apiKey ? { api_key: form.apiKey } : {}) },
    'model',
  )
  if (ok) {
    toast.show('Saved')
    checkHealth()
  }
}
async function loadModels() {
  busy.value = 'models'
  error.value = null
  try {
    if (dirty.value) await saveModel()
    models.value = await fetchAiModels()
    if (!models.value.length) error.value = 'The server offers no models. With Ollama: `ollama pull llama3.1`.'
  } catch (err) {
    error.value = apiErrorMessage(err)
  } finally {
    busy.value = null
  }
}
async function test() {
  busy.value = 'test'
  error.value = null
  testReply.value = null
  try {
    if (dirty.value) await saveModel()
    testReply.value = (await testAi()).reply || 'Connected.'
  } catch (err) {
    error.value = apiErrorMessage(err)
  } finally {
    busy.value = null
  }
}

// "● running" on the local card.
const health = ref<{ ok: boolean; error?: string } | null>(null)
async function checkHealth() {
  if (saved.value?.provider !== 'ollama') return (health.value = null)
  health.value = await aiHealth().catch(() => ({ ok: false }))
}

// ---- The app: in the dock or not ----------------------------------------------------------
// There's no on/off switch: AI is on while its app is in the dock.
const confirmRemove = ref(false)
async function removeApp() {
  if (!confirmRemove.value) {
    confirmRemove.value = true
    setTimeout(() => (confirmRemove.value = false), 4000)
    return
  }
  try {
    await setAppEnabled('ai', false)
    toast.show('AI removed from your dock. Your conversations and settings are kept.')
    await navigateTo('/apps')
  } catch (err) {
    error.value = apiErrorMessage(err)
  }
}
async function addApp() {
  try {
    await setAppEnabled('ai', true)
    fill(await fetchAiSettings())
  } catch (err) {
    error.value = apiErrorMessage(err)
  }
}
async function enableWithClaude() {
  fill(await fetchAiSettings())
  await refreshStatus()
  toast.show('AI is ready, with Claude Code')
}
async function saveIfDirty() {
  if (dirty.value) await update({ url: form.url, model: form.model })
}
watch(
  () => form.model,
  (m) => {
    // Claude Code's model buttons save at once.
    if (where.value === 'claude' && saved.value && m !== saved.value.model) update({ model: m })
  },
)

// ---- What it can read --------------------------------------------------------------------
async function setRead(key: 'read_mail' | 'read_agenda' | 'read_people' | 'read_notes' | 'read_tasks', value: boolean) {
  await update({ [key]: value }, key)
}
const searchOpen = ref(false)
async function setSearch(on: boolean) {
  if (on && !form.embedModel) {
    searchOpen.value = true
    form.embedModel = where.value === 'external' && saved.value?.provider === 'openai' ? 'text-embedding-3-small' : 'nomic-embed-text'
    return
  }
  await update({ index_mail: on, embed_model: form.embedModel, embed_url: form.embedUrl }, 'index')
}
async function saveSearch() {
  if (await update({ embed_model: form.embedModel, embed_url: form.embedUrl, index_mail: !!form.embedModel }, 'index')) {
    searchOpen.value = false
    toast.show('Search by meaning set up: indexing starts in the background')
  }
}

// ---- How it behaves ------------------------------------------------------------------------
async function setBrief(on: boolean) {
  await update({ brief_enabled: on }, 'brief')
}
async function saveBriefTime() {
  if (form.briefTime && form.briefTime !== saved.value?.brief_time) await update({ brief_time: form.briefTime }, 'brief')
}

const activityOpen = ref(false)

// Indexing progress while this page is open.
let poll: ReturnType<typeof setInterval> | undefined
onMounted(() => (poll = setInterval(() => status.value.search && refreshStatus(), 5000)))
onBeforeUnmount(() => clearInterval(poll))
</script>

<template>
  <div v-if="saved" class="ai">
    <header class="head">
      <div class="intro">
        <h2 class="serif">AI</h2>
        <p>
          The AI app: its own tab, the brief in Today, summaries and drafts in Mail, Ask in Ctrl+K. Remove it and all of
          that goes away; nothing else changes.
        </p>
      </div>
      <UButton
        v-if="saved.installed"
        :color="confirmRemove ? 'error' : 'neutral'"
        variant="outline"
        type="button"
        @click="removeApp"
      >
        {{ confirmRemove ? 'Click again to remove' : 'Remove from the dock' }}
      </UButton>
    </header>

    <div v-if="!saved.installed" class="notice">
      <span>The AI app isn’t in your dock, so nothing here runs.</span>
      <UButton size="sm" type="button" @click="addApp">Add it</UButton>
    </div>
    <div v-else-if="!saved.enabled" class="notice">
      <HIcon name="sparkle" :size="15" />
      <span>Almost there: choose where the model runs, and a model.</span>
    </div>

    <UAlert v-if="error" color="error" variant="soft" icon="i-havre-alert" :title="error" role="alert" />

    <!-- Where -->
    <fieldset>
      <legend>Where the model runs</legend>

      <URadioGroup
        :model-value="where"
        :items="whereItems"
        variant="card"
        :disabled="!!busy"
        aria-label="Where the model runs"
        :ui="{ fieldset: 'gap-2.5', item: 'rounded-[14px] p-4 has-data-[state=checked]:bg-(--raise)' }"
        @update:model-value="(w) => choose(w as Where)"
      >
        <template #label="{ item }">
          <span class="choice-title">
            {{ item.label }}
            <UBadge v-if="item.value === 'local'" color="success" variant="soft" size="sm">Recommended</UBadge>
          </span>
        </template>
        <template #description="{ item }">
          <span class="choice-text">
            <span class="muted">{{ item.description }}</span>
            <span v-if="item.value !== 'local'" class="warnline">
              <HIcon name="alert" :size="13" />The content the assistant reads is sent to
              {{ item.value === 'claude' ? 'Anthropic' : 'that provider' }}.
            </span>
            <span v-if="item.value === 'local' && where === 'local'" class="meta mono">
              <span>model: {{ saved.model || 'none chosen' }}</span>
              <span v-if="health" :class="health.ok ? 'okc' : 'warnc'">● {{ health.ok ? 'running' : 'not reachable' }}</span>
            </span>
          </span>
        </template>
      </URadioGroup>

      <!-- The chosen one's settings -->
      <div v-if="where === 'local'" class="config">
        <UFormField label="Ollama address">
          <UInput v-model="form.url" type="url" :placeholder="providerInfo.url" aria-label="Address" spellcheck="false" class="w-full" />
        </UFormField>
        <UFormField label="Model">
          <div class="with-btn">
            <UInput
              v-model="form.model"
              list="ai-models"
              placeholder="e.g. llama3.1:8b"
              aria-label="Model"
              spellcheck="false"
              class="grow"
            />
            <UButton color="neutral" variant="outline" :loading="busy === 'models'" :disabled="!!busy" @click="loadModels">
              List models
            </UButton>
          </div>
        </UFormField>
        <p v-if="health && !health.ok" class="faint small">
          {{ health.error || 'Ollama doesn’t answer.' }} Install it from ollama.com, then <code>ollama pull llama3.1</code>.
        </p>
      </div>
      <div v-else-if="where === 'claude'" class="config">
        <ClaudeCodeSetup
          v-model:model="form.model"
          v-model:command="form.url"
          :enabled="saved.enabled"
          :prepare="saveIfDirty"
          @enable="enableWithClaude"
        />
      </div>
      <div v-else class="config">
        <UTabs
          :model-value="saved.provider === 'anthropic' ? 'anthropic' : 'openai'"
          :items="[
            { label: 'OpenAI-compatible', value: 'openai' },
            { label: 'Anthropic', value: 'anthropic' },
          ]"
          :content="false"
          color="neutral"
          size="sm"
          aria-label="API"
          class="w-fit"
          @update:model-value="(v) => chooseExternal(v as AiProvider)"
        />
        <UFormField label="Address">
          <UInput v-model="form.url" type="url" :placeholder="providerInfo.url" aria-label="Address" spellcheck="false" class="w-full" />
        </UFormField>
        <UFormField label="API key">
          <UInput
            v-model="form.apiKey"
            type="password"
            autocomplete="off"
            :placeholder="saved.has_key ? 'Saved — type to replace' : 'sk-…'"
            aria-label="API key"
            class="w-full"
          />
        </UFormField>
        <UFormField label="Model">
          <div class="with-btn">
            <UInput
              v-model="form.model"
              list="ai-models"
              placeholder="e.g. mistral-small-latest"
              aria-label="Model"
              spellcheck="false"
              class="grow"
            />
            <UButton color="neutral" variant="outline" :loading="busy === 'models'" :disabled="!!busy" @click="loadModels">
              List models
            </UButton>
          </div>
        </UFormField>
      </div>
      <datalist id="ai-models">
        <option v-for="m in models" :key="m" :value="m" />
      </datalist>

      <div v-if="where !== 'claude'" class="row-actions">
        <p v-if="testReply" class="okline"><HIcon name="check" :size="14" /> {{ testReply }}</p>
        <span class="spacer" />
        <UButton color="neutral" variant="outline" size="sm" :loading="busy === 'test'" :disabled="!!busy" @click="test">
          Test
        </UButton>
        <UButton size="sm" :disabled="!!busy || !dirty" @click="saveModel">Save</UButton>
      </div>
    </fieldset>

    <!-- Read -->
    <fieldset>
      <legend>What it can read</legend>
      <div class="row">
        <UCheckbox
          :model-value="saved.read_mail"
          :disabled="!!busy"
          label="Mail"
          description="summaries, drafts, search"
          class="grow"
          @update:model-value="setRead('read_mail', !saved.read_mail)"
        />
        <span v-if="status.search" class="mono faint">{{ (status.indexed ?? 0).toLocaleString() }} indexed</span>
      </div>
      <div class="row">
        <UCheckbox
          :model-value="saved.read_agenda"
          :disabled="!!busy"
          label="Agenda"
          description="free time, preparation notes"
          class="grow"
          @update:model-value="setRead('read_agenda', !saved.read_agenda)"
        />
      </div>
      <div class="row">
        <UCheckbox
          :model-value="saved.read_people"
          :disabled="!!busy"
          label="People"
          description="who someone is, past conversations"
          class="grow"
          @update:model-value="setRead('read_people', !saved.read_people)"
        />
      </div>
      <!-- Notes and Tasks: only when their app is in the dock. -->
      <div v-if="saved.notes_app" class="row">
        <UCheckbox
          :model-value="saved.read_notes"
          :disabled="!!busy"
          label="Notes"
          description="search and read them, propose new notes"
          class="grow"
          @update:model-value="setRead('read_notes', !saved.read_notes)"
        />
      </div>
      <div v-if="saved.tasks_app" class="row">
        <UCheckbox
          :model-value="saved.read_tasks"
          :disabled="!!busy"
          label="Tasks"
          description="what’s due and done, propose new tasks"
          class="grow"
          @update:model-value="setRead('read_tasks', !saved.read_tasks)"
        />
      </div>
      <div class="row">
        <UCheckbox
          :model-value="saved.index_mail && !!saved.embed_model"
          :disabled="!!busy || !saved.read_mail"
          label="Search by meaning"
          description="find mail by what it says, not just its words"
          class="grow"
          @update:model-value="setSearch(!(saved.index_mail && !!saved.embed_model))"
        />
        <UButton color="primary" variant="link" size="sm" @click="searchOpen = !searchOpen">
          {{ searchOpen ? 'Close' : 'Settings' }}
        </UButton>
      </div>
      <div v-if="searchOpen" class="sub-config">
        <UFormField label="Embedding model">
          <UInput
            v-model="form.embedModel"
            list="ai-models"
            placeholder="nomic-embed-text"
            aria-label="Embedding model"
            spellcheck="false"
            class="w-full"
          />
        </UFormField>
        <UFormField label="Embeddings address">
          <UInput
            v-model="form.embedUrl"
            type="url"
            placeholder="Same server (Claude: Ollama on this machine)"
            aria-label="Embeddings address"
            spellcheck="false"
            class="w-full"
          />
        </UFormField>
        <p class="faint small">The newest 20 000 messages are indexed in the background; recent ones are downloaded to read their text.</p>
        <div class="row-actions">
          <span class="spacer" />
          <UButton size="sm" type="button" :disabled="!!busy || !form.embedModel" @click="saveSearch">
            Save and index
          </UButton>
        </div>
      </div>
      <p v-if="status.search && status.total" class="progress faint small">
        <span class="bar"><span :style="{ width: `${Math.min(100, ((status.indexed ?? 0) / status.total) * 100)}%` }" /></span>
        {{ status.indexed }} / {{ status.total }} messages
        <template v-if="status.index_error"> · <span class="warnc">{{ status.index_error }}</span></template>
      </p>
    </fieldset>

    <!-- Behaviour -->
    <fieldset class="behave">
      <legend>How it behaves</legend>
      <div class="always">
        <HIcon name="shield" :size="18" />
        <div>
          <div class="strong">Always asks before acting</div>
          <div class="muted small">It never sends, accepts, moves or deletes anything on its own. Instructions found inside emails are ignored.</div>
        </div>
      </div>
      <div class="row">
        <UCheckbox
          :model-value="saved.brief_enabled"
          :disabled="!!busy"
          label="Morning brief"
          description="weekdays at"
          class="grow"
          @update:model-value="setBrief(!saved.brief_enabled)"
        />
        <TimeField v-model="form.briefTime" label="Brief time" size="sm" @update:model-value="saveBriefTime" />
      </div>
      <p class="faint small">
        Other AI apps (Claude Code…) using Havre: <NuxtLink to="/settings/connections">Settings › Connections</NuxtLink>
        — that works without the AI app.
      </p>
    </fieldset>

    <div class="foot faint">
      Every action the assistant takes is logged.
      <UButton color="primary" variant="link" size="sm" class="p-0" @click="activityOpen = true">View activity →</UButton>
    </div>
    <AiActivity v-if="activityOpen" @close="activityOpen = false" />
  </div>
  <UAlert v-else-if="error" color="error" variant="soft" icon="i-havre-alert" :title="error" role="alert" />
</template>

<style scoped>
.ai {
  display: flex;
  flex-direction: column;
  gap: 36px;
}
.head {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-start;
  gap: 16px;
}
.intro {
  flex: 1 1 360px;
}
h2 {
  margin: 0 0 6px;
  font-weight: 400;
  font-size: 32px;
  letter-spacing: -0.015em;
}
.intro p {
  margin: 0;
  color: var(--muted);
  line-height: 1.6;
}
fieldset {
  margin: 0;
  padding: 0;
  border: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-width: 0;
}
legend {
  padding: 0;
  margin-bottom: 10px;
  font-family: var(--font-mono);
  font-size: 11px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--faint);
}
.choice-text {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.choice-title {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  font-weight: 600;
}
.muted {
  color: var(--muted);
  font-size: 13.5px;
}
.meta {
  display: flex;
  flex-wrap: wrap;
  gap: 6px 16px;
  margin-top: 6px;
  font-size: 12px;
  color: var(--faint);
}
.okc {
  color: var(--ok);
}
.warnc {
  color: var(--danger);
}
.warnline {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 6px;
  font-size: 12.5px;
  color: var(--danger);
}
.config {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 4px 0 0;
}
.sub-config {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 4px 0 12px 28px;
}
.with-btn {
  display: flex;
  gap: 8px;
}
.row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 0;
}
.grow {
  flex: 1;
}
.strong {
  font-weight: 500;
}
.faint {
  color: var(--faint);
}
.mono {
  font-family: var(--font-mono);
  font-size: 12px;
}
.small {
  font-size: 12.5px;
  margin: 0;
}
.behave {
  gap: 6px;
}
.behave .row + .row {
  margin-top: 0;
}
.always {
  display: flex;
  gap: 12px;
  padding: 14px 16px;
  border-radius: 12px;
  background: var(--bg);
  border: 1px solid var(--line);
  color: var(--ok);
}
.always > div {
  color: var(--text);
}
.row-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}
.spacer {
  flex: 1;
}
.okline {
  display: flex;
  gap: 8px;
  align-items: center;
  margin: 0;
  color: var(--ok);
  font-size: 13px;
}
.progress {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
}
.bar {
  width: 140px;
  height: 6px;
  border-radius: 3px;
  background: var(--line2);
  overflow: hidden;
}
.bar span {
  display: block;
  height: 100%;
  background: var(--accent);
}
.notice {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 14px;
  border-radius: 12px;
  border: 1px solid var(--line2);
  background: var(--raise);
  font-size: 13.5px;
}
.notice span {
  flex: 1;
}
.foot {
  font-size: 13px;
}
@media (max-width: 640px) {
  .config {
    padding-left: 16px;
  }
}
</style>
