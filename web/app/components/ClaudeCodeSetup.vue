<script setup lang="ts">
import type { ClaudeStatus } from '~/composables/useAi'

// Claude Code as Havre's AI, set up from here: Havre finds the `claude`
// command, says how to install it if it's missing, and signs it in to your
// Claude account (it runs `claude auth login` and types in the code you
// get from Anthropic). No terminal, no API key.
const model = defineModel<string>('model', { required: true })
const command = defineModel<string>('command', { required: true })
const props = defineProps<{
  /** AI is on. */
  enabled: boolean
  /** Saves pending changes (the command) before checking. */
  prepare: () => Promise<void>
}>()
const emit = defineEmits<{ enable: [] }>()

const status = ref<ClaudeStatus | null>(null)
const checking = ref(false)
const error = ref<string | null>(null)

async function check() {
  checking.value = true
  error.value = null
  try {
    await props.prepare()
    status.value = await fetchClaudeStatus()
  } catch (err) {
    error.value = apiErrorMessage(err)
  } finally {
    checking.value = false
  }
}
onMounted(check)

// ---- Signing in ---------------------------------------------------------------------

const step = ref<'idle' | 'started' | 'done'>('idle')
const url = ref('')
const code = ref('')
const busy = ref<'start' | 'complete' | 'logout' | null>(null)
const result = ref<{ success: boolean; output: string } | null>(null)

async function signIn() {
  busy.value = 'start'
  error.value = null
  result.value = null
  try {
    url.value = (await startClaudeLogin()).url
    step.value = 'started'
    code.value = ''
  } catch (err) {
    error.value = apiErrorMessage(err)
  } finally {
    busy.value = null
  }
}

async function finish() {
  if (!code.value.trim()) return
  busy.value = 'complete'
  error.value = null
  try {
    result.value = await completeClaudeLogin(code.value.trim())
    step.value = 'done'
    await check()
  } catch (err) {
    error.value = apiErrorMessage(err)
  } finally {
    busy.value = null
  }
}

async function cancel() {
  await cancelClaudeLogin().catch(() => {})
  step.value = 'idle'
  code.value = ''
}

async function signOut() {
  busy.value = 'logout'
  try {
    await claudeLogout()
    await check()
  } catch (err) {
    error.value = apiErrorMessage(err)
  } finally {
    busy.value = null
  }
}

// ---- Installing ---------------------------------------------------------------------

const os = import.meta.client
  ? /Windows/.test(navigator.userAgent)
    ? 'windows'
    : /Mac/.test(navigator.userAgent)
      ? 'mac'
      : 'linux'
  : 'linux'
const installs = [
  { id: 'windows', label: 'Windows (PowerShell)', cmd: 'irm https://claude.ai/install.ps1 | iex' },
  { id: 'mac', label: 'macOS / Linux', cmd: 'curl -fsSL https://claude.ai/install.sh | bash' },
  { id: 'npm', label: 'With Node.js', cmd: 'npm install -g @anthropic-ai/claude-code' },
]
const installTab = ref(os === 'windows' ? 'windows' : 'mac')
const install = computed(() => installs.find((i) => i.id === installTab.value)!)

async function copy(text: string) {
  try {
    await navigator.clipboard.writeText(text)
    useToasts().show('Copied')
  } catch {
    /* select it by hand */
  }
}

const MODELS = [
  { value: '', label: 'Default' },
  { value: 'sonnet', label: 'Sonnet' },
  { value: 'opus', label: 'Opus' },
  { value: 'haiku', label: 'Haiku' },
]
const ready = computed(() => !!status.value?.command && status.value.logged_in)
</script>

<template>
  <div class="cc">
    <ol class="steps">
      <!-- 1. Installed -->
      <li :class="{ ok: status?.command }">
        <span class="mark" aria-hidden="true">
          <span v-if="checking && !status" class="spinner" />
          <HIcon v-else-if="status?.command" name="check" :size="13" :stroke-width="2.4" />
          <template v-else>1</template>
        </span>
        <div class="body">
          <p class="title">
            <template v-if="status?.command">Claude Code {{ status.version }} is installed</template>
            <template v-else-if="status">Install Claude Code</template>
            <template v-else>Looking for Claude Code…</template>
          </p>
          <p v-if="status?.command" class="faint mono path">{{ status.command }}</p>
          <template v-else-if="status">
            <p class="faint">
              On the computer where Havre runs. (With Havre's Docker image it's already there.) Then come back and
              click <em>Check again</em>.
            </p>
            <UTabs
              v-model="installTab"
              :items="installs.map((i) => ({ label: i.label, value: i.id }))"
              :content="false"
              color="neutral"
              size="xs"
              aria-label="How to install"
              class="w-fit"
            />
            <div class="code">
              <code>{{ install.cmd }}</code>
              <UButton color="neutral" variant="outline" size="sm" type="button" @click="copy(install.cmd)">
                Copy
              </UButton>
            </div>
            <UButton
              color="neutral"
              variant="outline"
              size="sm"
              type="button"
              :disabled="checking"
              @click="check"
            >
              {{ checking ? 'Checking…' : 'Check again' }}
            </UButton>
          </template>
        </div>
      </li>

      <!-- 2. Signed in -->
      <li :class="{ ok: status?.logged_in, off: !status?.command }">
        <span class="mark" aria-hidden="true">
          <HIcon v-if="status?.logged_in" name="check" :size="13" :stroke-width="2.4" />
          <template v-else>2</template>
        </span>
        <div class="body">
          <div class="row">
            <p class="title">
              {{ status?.logged_in ? 'Signed in to Claude' : 'Sign in with your Claude account' }}
            </p>
            <template v-if="status?.command && step !== 'started'">
              <UButton
                v-if="status.logged_in"
                color="neutral"
                variant="outline"
                size="sm"
                type="button"
                :disabled="busy !== null"
                @click="signOut"
              >
                Sign out
              </UButton>
              <UButton v-else size="sm" type="button" :disabled="busy !== null" @click="signIn">
                {{ busy === 'start' ? 'Starting…' : 'Sign in' }}
              </UButton>
            </template>
          </div>
          <p v-if="status?.logged_in" class="faint">
            {{ status.auth_method === 'api_key' ? 'With an API key.' : 'Uses your Claude plan (Pro, Max, Team…).' }}
          </p>
          <p v-else-if="status?.command && step === 'idle'" class="faint">
            Pro, Max, Team or Enterprise. Havre opens Anthropic's sign-in page; you paste back the code it shows.
          </p>

          <ol v-if="step === 'started'" class="login">
            <li>
              <span>Sign in at Anthropic</span>
              <UButton
                color="neutral"
                variant="outline"
                size="sm"
                :href="url"
                target="_blank"
                rel="noopener"
              >
                Open the sign-in page
                <HIcon name="external" :size="13" />
              </UButton>
            </li>
            <li>
              <label for="cc-code">Paste the code it shows you</label>
              <form class="code-row" @submit.prevent="finish">
                <UInput
                  v-model="code"
                  id="cc-code"
                  autocomplete="off"
                  spellcheck="false"
                  placeholder="Authentication code"
                />
                <UButton size="sm" type="submit" :disabled="!code.trim() || busy !== null">
                  {{ busy === 'complete' ? 'Signing in…' : 'Finish' }}
                </UButton>
                <UButton
                  color="neutral"
                  variant="outline"
                  size="sm"
                  type="button"
                  :disabled="busy === 'complete'"
                  @click="cancel"
                >
                  Cancel
                </UButton>
              </form>
            </li>
          </ol>
          <div v-if="step === 'done' && result && !result.success" class="fail">
            <p><HIcon name="alert" :size="14" /> The sign-in didn't work. Try again with a new code.</p>
            <details v-if="result.output">
              <summary>What Claude Code said</summary>
              <pre>{{ result.output }}</pre>
            </details>
          </div>
        </div>
      </li>

      <!-- 3. Model -->
      <li :class="{ off: !ready }">
        <span class="mark" aria-hidden="true">3</span>
        <div class="body">
          <p class="title">Model</p>
          <UTabs
            v-model="model"
            :items="MODELS.map((m) => ({ label: m.label, value: m.value }))"
            :content="false"
            color="neutral"
            size="sm"
            aria-label="Claude model"
            class="w-fit"
          />
          <p class="faint">Haiku is fastest and lightest on your plan's limits; Opus is the most capable.</p>
        </div>
      </li>
    </ol>

    <div v-if="ready && !enabled" class="go">
      <span>Claude Code is ready.</span>
      <UButton size="sm" type="button" @click="emit('enable')">Turn on AI with Claude Code</UButton>
    </div>

    <p v-if="error" class="error" role="alert"><HIcon name="alert" :size="14" /> {{ error }}</p>

    <details class="advanced">
      <summary>Advanced</summary>
      <UFormField label="Command" class="field">
        <UInput
          v-model="command"
          placeholder="Found automatically"
          aria-label="Command"
          spellcheck="false"
          class="w-full"
        />
        <template #help>Only if Havre doesn't find Claude Code: the full path to <code>claude</code>.</template>
      </UFormField>
      <UButton color="neutral" variant="outline" size="sm" type="button" :disabled="checking" @click="check">
        {{ checking ? 'Checking…' : 'Check again' }}
      </UButton>
    </details>
  </div>
</template>

<style scoped>
.cc {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.steps {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.steps > li {
  display: flex;
  gap: 12px;
}
.steps > li.off {
  opacity: 0.5;
}
.mark {
  flex: 0 0 24px;
  height: 24px;
  border-radius: 50%;
  display: grid;
  place-items: center;
  border: 1px solid var(--line2);
  font-size: 12px;
  font-weight: 600;
  color: var(--muted);
}
li.ok > .mark {
  border-color: transparent;
  background: color-mix(in oklab, var(--ok) 22%, transparent);
  color: var(--ok);
}
.body {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}
.title {
  margin: 2px 0 0;
  font-weight: 500;
  font-size: 14px;
}
.faint {
  margin: 0;
  color: var(--faint);
  font-size: 12.5px;
}
.mono,
code,
pre {
  font-family: var(--font-mono);
}
.path {
  font-size: 11.5px;
  overflow-wrap: anywhere;
}
.code {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border-radius: 10px;
  border: 1px solid var(--line2);
  background: var(--bg);
}
.code code {
  flex: 1;
  min-width: 0;
  font-size: 12.5px;
  overflow-wrap: anywhere;
}
.body > button {
  align-self: flex-start;
}
.login {
  margin: 4px 0 0;
  padding: 12px 14px 12px 32px;
  border-radius: 10px;
  border: 1px solid var(--line);
  background: var(--bg);
  display: flex;
  flex-direction: column;
  gap: 12px;
  font-size: 13.5px;
}
.login li {
  display: flex;
  flex-direction: column;
  gap: 6px;
  align-items: flex-start;
}
.code-row {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  width: 100%;
}
.fail p {
  display: flex;
  gap: 6px;
  align-items: center;
  margin: 0;
  color: var(--danger);
  font-size: 13px;
}
details summary {
  cursor: pointer;
  font-size: 12.5px;
  color: var(--muted);
}
details pre {
  margin: 6px 0 0;
  padding: 8px 10px;
  border-radius: 8px;
  background: var(--bg);
  font-size: 11.5px;
  white-space: pre-wrap;
}
.go {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 10px 12px;
  border-radius: 10px;
  background: color-mix(in oklab, var(--ok) 10%, transparent);
  font-size: 13.5px;
}
.error {
  display: flex;
  gap: 6px;
  align-items: center;
  margin: 0;
  color: var(--danger);
  font-size: 13px;
}
.advanced {
  padding-top: 10px;
  border-top: 1px solid var(--line);
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.advanced[open] > .field {
  margin: 8px 0;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.field > span {
  font-size: 12.5px;
  color: var(--muted);
}
.spinner {
  width: 12px;
  height: 12px;
  border-radius: 50%;
  border: 2px solid var(--line2);
  border-top-color: var(--text);
  animation: spin 0.7s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
