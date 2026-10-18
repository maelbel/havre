<script setup lang="ts">
import type { Account, AccountColor, AccountForm, OAuthProvider } from '~/composables/useAccounts'

// Add an account, or edit one (`account` given).
const props = defineProps<{
  account?: Account
  /** Which OAuth2 providers are set up in .env. */
  oauthConfigured?: Partial<Record<OAuthProvider, boolean>>
}>()
const emit = defineEmits<{ saved: [account: { id: number; name: string }]; cancel: [] }>()

const editing = computed(() => !!props.account)
const form = reactive<AccountForm>({
  name: props.account?.name ?? '',
  email: props.account?.email ?? '',
  color: props.account?.color ?? 'work',
  imap_host: props.account?.imap_host ?? '',
  imap_port: props.account?.imap_port ?? 993,
  imap_user: props.account?.imap_user ?? '',
  password: '',
  sender_name: props.account?.sender_name ?? '',
  smtp_host: props.account?.smtp_host ?? '',
  smtp_port: props.account?.smtp_port ?? 465,
  smtp_security: props.account?.smtp_security ?? 'tls',
})

// Picking TLS / STARTTLS also moves the usual port (465 / 587).
function onSecurity() {
  smtpTouched.value = true
  if (form.smtp_security === 'starttls' && form.smtp_port === 465) form.smtp_port = 587
  if (form.smtp_security === 'tls' && form.smtp_port === 587) form.smtp_port = 465
}

// The outgoing server follows the incoming one until edited.
const smtpTouched = ref(editing.value)
watch(
  () => form.imap_host,
  (host) => {
    if (smtpTouched.value) return
    const smtp = smtpDefaults(host)
    form.smtp_host = smtp.host
    form.smtp_port = smtp.port
    form.smtp_security = smtp.security
  },
  { immediate: !props.account },
)

// Authentication, like Thunderbird: "Normal password" or "OAuth2". OAuth2
// signs in on Google's or Microsoft's page; the IMAP server decides which
// (imap.gmail.com → Google, outlook.office365.com → Microsoft). Addresses
// they host — company domains included — are detected and switch to
// OAuth2, unless the user chose themselves.
type Method = 'password' | 'oauth2'
const method = ref<Method>(props.account && props.account.auth !== 'password' ? 'oauth2' : 'password')
const methodTouched = ref(editing.value)
const oauth = computed(() => method.value === 'oauth2')
/** Who hosts the address (Google, Microsoft), once detected. */
const detected = ref<OAuthProvider | null>(null)
/** Who signs in with OAuth2: the server decides. */
const oauthProvider = computed<OAuthProvider | null>(() => {
  if (props.account && props.account.auth !== 'password') return props.account.auth
  return providerForHost(form.imap_host)
})
const signingIn = ref(false)

// Server settings are filled from the email address until the user edits
// them; the name too.
const serverTouched = ref(editing.value)
const nameTouched = ref(editing.value)
const showAdvanced = ref(false)
const preset = computed(() => detectProvider(form.email))

function fillServer() {
  if (serverTouched.value) return
  const server =
    oauth.value && detected.value
      ? OAUTH_PROVIDERS[detected.value]
      : (preset.value ?? { host: guessHost(form.email), port: 993 })
  form.imap_host = server.host
  form.imap_port = server.port
}

let detectTimer: ReturnType<typeof setTimeout> | undefined
watch(
  () => form.email,
  (email) => {
    if (!serverTouched.value) form.imap_user = ''
    fillServer()
    if (!nameTouched.value) {
      const domain = email.split('@')[1] ?? ''
      form.name = preset.value?.name ?? (domain ? domain.charAt(0).toUpperCase() + domain.slice(1) : '')
    }
    clearTimeout(detectTimer)
    detectTimer = setTimeout(async () => {
      const found = await detectOAuthProvider(email)
      if (form.email !== email) return
      detected.value = found
      if (!methodTouched.value) method.value = found ? 'oauth2' : 'password'
      fillServer()
    }, 400)
  },
)
onBeforeUnmount(() => clearTimeout(detectTimer))
watch(method, fillServer)

// Custom domain: the server is only a guess, so show it.
const unknownDomain = computed(() => form.email.includes('@') && !preset.value && !editing.value)
// OAuth2 with a server that isn't Google's or Microsoft's: show the settings.
const needsOAuthServer = computed(() => oauth.value && !oauthProvider.value && !editing.value)
// An OAuth2 account's server is tied to its sign-in: use Reconnect instead.
const serverLocked = computed(() => editing.value && props.account?.auth !== 'password')
const providerName = computed(() => (oauthProvider.value ? OAUTH_PROVIDERS[oauthProvider.value].name : ''))

const busy = ref(false)
const error = ref<string | null>(null)

async function submit() {
  error.value = null
  if (oauth.value && !editing.value) {
    if (!oauthProvider.value) {
      error.value =
        'OAuth2 works with Gmail and Google Workspace (imap.gmail.com) and with Microsoft 365 and Outlook.com ' +
        '(outlook.office365.com). For another server, use a normal password.'
      return
    }
    signingIn.value = true // OAuthSignIn takes over
    return
  }
  busy.value = true
  try {
    if (props.account) {
      // Send only what changed; an empty password keeps the current one.
      const a = props.account
      const changes: Partial<AccountForm> = {}
      for (const key of [
        'name',
        'email',
        'color',
        'imap_host',
        'imap_user',
        'imap_port',
        'sender_name',
        'smtp_host',
        'smtp_port',
        'smtp_security',
      ] as const) {
        if (form[key] !== a[key]) (changes as Record<string, unknown>)[key] = form[key]
      }
      if (form.password) changes.password = form.password
      emit('saved', await updateAccount(a.id, changes))
    } else {
      emit('saved', await addAccount(form))
    }
  } catch (err) {
    error.value = apiErrorMessage(err)
  } finally {
    busy.value = false
  }
}

const colors: { value: AccountColor; label: string }[] = ACCOUNT_COLORS
</script>

<template>
  <form class="account-form" @submit.prevent="submit">
    <h3>{{ editing ? `Edit ${account!.name}` : 'Add an account' }}</h3>

    <div class="grid" :class="{ locked: signingIn }" :inert="signingIn || undefined">
      <label class="field wide">
        <span>Email address</span>
        <input v-model="form.email" type="email" autocomplete="email" required placeholder="you@example.com" />
      </label>

      <label class="field">
        <span>Authentication</span>
        <select v-model="method" :disabled="editing" @change="methodTouched = true">
          <option value="password">Normal password</option>
          <option value="oauth2">OAuth2</option>
        </select>
      </label>

      <label v-if="!oauth" class="field">
        <span>Password</span>
        <input
          v-model="form.password"
          type="password"
          autocomplete="current-password"
          :required="!editing"
          :placeholder="editing ? 'Unchanged' : ''"
        />
      </label>
      <p v-if="oauth" class="hint oauth-hint">
        <template v-if="editing">Signed in with {{ providerName }}. If it stops working, use <em>Reconnect</em>.</template>
        <template v-else-if="oauthProvider">
          You'll sign in on {{ providerName }}'s page — no password is stored in Havre.
        </template>
        <template v-else>
          OAuth2 is for Google and Microsoft mailboxes: set their server below (imap.gmail.com or
          outlook.office365.com).
        </template>
      </p>
      <small v-else-if="preset?.note" class="hint wide">
        {{ preset.note }}
        <a v-if="preset.link" :href="preset.link" target="_blank" rel="noopener">Open</a>
      </small>

      <label class="field">
        <span>Your name <span class="faint">(seen by recipients)</span></span>
        <input
          v-model="form.sender_name"
          type="text"
          autocomplete="name"
          :placeholder="oauth && !editing ? 'From your sign-in' : 'Maël Belliard'"
        />
      </label>

      <label class="field">
        <span>Name in Havre</span>
        <input v-model="form.name" type="text" required placeholder="Work" @input="nameTouched = true" />
      </label>

      <fieldset class="field wide">
        <legend>Color</legend>
        <div class="swatches">
          <label v-for="c in colors" :key="c.value" class="swatch" :title="c.label">
            <input v-model="form.color" type="radio" name="color" :value="c.value" />
            <span :style="{ background: `var(--c-${c.value})` }" />
            <span class="sr-only">{{ c.label }}</span>
          </label>
        </div>
      </fieldset>
    </div>

    <details
      class="advanced" :open="showAdvanced || (unknownDomain && !oauth) || needsOAuthServer" @toggle="showAdvanced = ($event.target as HTMLDetailsElement).open">
      <summary>
        Server settings
        <span v-if="form.imap_host" class="faint mono">{{ form.imap_host }}:{{ form.imap_port }}</span>
        <span v-if="form.smtp_host" class="faint mono">· {{ form.smtp_host }}:{{ form.smtp_port }}</span>
      </summary>
      <p v-if="unknownDomain && !oauth" class="hint">
        We guessed the server from your address. Check it with your provider if the connection fails.
      </p>
      <div class="grid" :class="{ locked: serverLocked }" :inert="serverLocked || undefined">
        <label class="field">
          <span>IMAP server</span>
          <input v-model="form.imap_host" type="text" required placeholder="imap.example.com" @input="serverTouched = true" />
        </label>
        <label class="field">
          <span>Port</span>
          <input v-model.number="form.imap_port" type="number" min="1" max="65535" required @input="serverTouched = true" />
        </label>
        <label class="field wide">
          <span>User name <span class="faint">(if different from the email)</span></span>
          <input v-model="form.imap_user" type="text" autocomplete="username" :placeholder="form.email" @input="serverTouched = true" />
        </label>
      </div>
      <p class="sub">Outgoing (SMTP) — same login</p>
      <div class="grid three">
        <label class="field">
          <span>SMTP server</span>
          <input v-model="form.smtp_host" type="text" required placeholder="smtp.example.com" @input="smtpTouched = true" />
        </label>
        <label class="field">
          <span>Port</span>
          <input v-model.number="form.smtp_port" type="number" min="1" max="65535" required @input="smtpTouched = true" />
        </label>
        <label class="field">
          <span>Security</span>
          <select v-model="form.smtp_security" @change="onSecurity">
            <option value="tls">TLS (465)</option>
            <option value="starttls">STARTTLS (587)</option>
          </select>
        </label>
      </div>
    </details>

    <OAuthSignIn
      v-if="signingIn && oauthProvider"
      :provider="oauthProvider"
      :configured="!!oauthConfigured?.[oauthProvider]"
      :request="{
        name: form.name,
        email: form.email,
        color: form.color,
        imap_host: form.imap_host,
        imap_port: form.imap_port,
        imap_user: form.imap_user,
        sender_name: form.sender_name,
      }"
      :email="form.imap_user || form.email"
      @done="(a) => emit('saved', a)"
      @cancel="signingIn = false"
    />

    <div v-if="error" class="error" role="alert">
      <HIcon name="alert" :size="16" />
      <span>{{ error }}</span>
    </div>

    <div v-if="!signingIn" class="actions">
      <button type="button" class="btn" :disabled="busy" @click="emit('cancel')">Cancel</button>
      <button type="submit" class="btn btn-primary" :disabled="busy">
        <span v-if="busy" class="spinner" aria-hidden="true" />
        {{
          busy ? 'Checking the connection…' : editing ? 'Save' : 'Connect'
        }}
      </button>
    </div>
  </form>
</template>

<style scoped>
.account-form {
  display: flex;
  flex-direction: column;
  gap: 18px;
  padding: 22px;
  border-radius: var(--radius-lg);
  border: 1px solid var(--line2);
  background: var(--raise);
}
h3 {
  margin: 0;
  font-size: 16px;
}
.grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 14px 16px;
}
.wide {
  grid-column: 1 / -1;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin: 0;
  padding: 0;
  border: 0;
  min-width: 0;
}
.field > span,
legend {
  padding: 0;
  font-size: 12.5px;
  color: var(--muted);
}
input[type='text'],
input[type='email'],
input[type='password'],
input[type='number'],
select {
  height: 40px;
  padding: 0 12px;
  border-radius: 10px;
  border: 1px solid var(--line2);
  background: var(--bg);
  color: var(--text);
  outline: none;
}
input:focus {
  border-color: var(--accent);
}
.hint {
  margin: 0;
  font-size: 12.5px;
  line-height: 1.5;
  color: var(--faint);
}
.faint {
  color: var(--faint);
  font-weight: 400;
}
.swatches {
  display: flex;
  gap: 8px;
  height: 40px;
  align-items: center;
}
.swatch {
  position: relative;
  cursor: pointer;
}
.swatch input {
  position: absolute;
  opacity: 0;
}
.swatch span:first-of-type {
  display: block;
  width: 24px;
  height: 24px;
  border-radius: 50%;
  border: 2px solid transparent;
  box-shadow: 0 0 0 2px var(--raise);
}
.swatch input:checked + span {
  box-shadow:
    0 0 0 2px var(--raise),
    0 0 0 4px var(--text);
}
.swatch input:focus-visible + span {
  outline: 2px solid var(--accent);
  outline-offset: 4px;
}
select {
  appearance: none;
  padding-right: 34px;
  background-image: linear-gradient(45deg, transparent 50%, var(--muted) 50%),
    linear-gradient(135deg, var(--muted) 50%, transparent 50%);
  background-position:
    calc(100% - 18px) 18px,
    calc(100% - 13px) 18px;
  background-size: 5px 5px;
  background-repeat: no-repeat;
  cursor: pointer;
}
select:disabled {
  opacity: 0.7;
  cursor: default;
}
.oauth-hint {
  align-self: end;
  padding-bottom: 4px;
}
.grid.three {
  grid-template-columns: minmax(0, 2fr) minmax(0, 1fr) minmax(0, 1.4fr);
}
.sub {
  margin: 18px 0 10px;
  font-size: 12.5px;
  color: var(--muted);
}
.locked {
  opacity: 0.6;
}
.hint em {
  font-style: normal;
  color: var(--muted);
}
.advanced {
  border-top: 1px solid var(--line);
  padding-top: 14px;
}
.advanced summary {
  display: flex;
  gap: 10px;
  align-items: baseline;
  cursor: pointer;
  font-size: 13px;
  color: var(--muted);
  margin-bottom: 4px;
}
.advanced[open] summary {
  margin-bottom: 14px;
}
.advanced .hint {
  margin-bottom: 12px;
}
.error {
  display: flex;
  gap: 10px;
  padding: 12px 14px;
  border-radius: var(--radius);
  background: color-mix(in oklab, var(--danger) 12%, transparent);
  color: var(--danger);
  font-size: 13.5px;
  line-height: 1.5;
}
.error span {
  color: var(--text);
}
.actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
.spinner {
  width: 14px;
  height: 14px;
  border-radius: 50%;
  border: 2px solid color-mix(in oklab, var(--on-accent) 30%, transparent);
  border-top-color: var(--on-accent);
  animation: spin 0.7s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
@media (max-width: 560px) {
  .grid,
  .grid.three {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
