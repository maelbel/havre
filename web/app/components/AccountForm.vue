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
      <UFormField label="Email address" class="field wide">
        <UInput
          v-model="form.email"
          type="email"
          autocomplete="email"
          required
          placeholder="you@example.com"
          class="w-full"
        />
      </UFormField>

      <UFormField label="Authentication" class="field">
        <USelect
          v-model="method"
          :disabled="editing"
          :items="[{ label: 'Normal password', value: 'password' }, { label: 'OAuth2', value: 'oauth2' }]"
          class="w-full"
          @change="methodTouched = true"
        />
      </UFormField>

      <UFormField v-if="!oauth" label="Password" class="field">
        <UInput
          v-model="form.password"
          type="password"
          autocomplete="current-password"
          :required="!editing"
          :placeholder="editing ? 'Unchanged' : ''"
          class="w-full"
        />
      </UFormField>
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

      <UFormField label="Your name" hint="seen by recipients" class="field">
        <UInput
          v-model="form.sender_name"
          autocomplete="name"
          :placeholder="oauth && !editing ? 'From your sign-in' : 'Maël Belliard'"
          class="w-full"
        />
      </UFormField>

      <UFormField label="Name in Havre" class="field">
        <UInput v-model="form.name" required placeholder="Work" class="w-full" @input="nameTouched = true" />
      </UFormField>

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
        <UFormField label="IMAP server" class="field">
          <UInput
            v-model="form.imap_host"
            required
            placeholder="imap.example.com"
            class="w-full"
            @input="serverTouched = true"
          />
        </UFormField>
        <UFormField label="Port" class="field">
          <UInput
            v-model.number="form.imap_port"
            type="number"
            min="1"
            max="65535"
            required
            class="w-full"
            @input="serverTouched = true"
          />
        </UFormField>
        <UFormField label="User name" hint="if different from the email" class="field wide">
          <UInput
            v-model="form.imap_user"
            autocomplete="username"
            :placeholder="form.email"
            class="w-full"
            @input="serverTouched = true"
          />
        </UFormField>
      </div>
      <p class="sub">Outgoing (SMTP) — same login</p>
      <div class="grid three">
        <UFormField label="SMTP server" class="field">
          <UInput
            v-model="form.smtp_host"
            required
            placeholder="smtp.example.com"
            class="w-full"
            @input="smtpTouched = true"
          />
        </UFormField>
        <UFormField label="Port" class="field">
          <UInput
            v-model.number="form.smtp_port"
            type="number"
            min="1"
            max="65535"
            required
            class="w-full"
            @input="smtpTouched = true"
          />
        </UFormField>
        <UFormField label="Security" class="field">
          <USelect
            v-model="form.smtp_security"
            :items="[{ label: 'TLS (465)', value: 'tls' }, { label: 'STARTTLS (587)', value: 'starttls' }]"
            class="w-full"
            @change="onSecurity"
          />
        </UFormField>
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

    <UAlert v-if="error" color="error" variant="soft" icon="i-havre-alert" :title="error" role="alert" />

    <div v-if="!signingIn" class="actions">
      <UButton color="neutral" variant="outline" type="button" :disabled="busy" @click="emit('cancel')">
        Cancel
      </UButton>
      <UButton :loading="busy" type="submit" :disabled="busy">
        {{ busy ? 'Checking the connection…' : editing ? 'Save' : 'Connect' }}
      </UButton>
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
/* The colour fieldset; UFormField lays out the others. */
.field {
  margin: 0;
  padding: 0;
  border: 0;
  min-width: 0;
}
legend {
  padding: 0 0 6px;
  font-size: 12.5px;
  color: var(--muted);
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
.actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
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
