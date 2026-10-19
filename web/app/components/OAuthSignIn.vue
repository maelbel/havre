<script setup lang="ts">
import type { OAuthProvider, OAuthRequest, OAuthStart } from '~/composables/useAccounts'

// OAuth2 sign-in (Google or Microsoft), like Thunderbird:
// 1. a window opens on the provider's sign-in page,
// 2. you sign in and accept; the provider sends the window back to Havre
//    (/api/oauth/{provider}/callback), which closes it,
// 3. meanwhile we ask the server how it's going; once it has opened the
//    mailbox, the account is added.
// Microsoft only — fallback: a code to type at microsoft.com/devicelogin
// (no window needed, works whatever address Havre is opened at).
const props = defineProps<{
  provider: OAuthProvider
  /** Is this provider set up in .env? Otherwise, explain how. */
  configured: boolean
  /** The add-account form. */
  request?: OAuthRequest
  /** Signing in again to an existing account. */
  reconnect?: number
  /** Shown as a reminder of which address to sign in with. */
  email?: string
}>()
const emit = defineEmits<{ done: [account: { id: number; name: string }]; cancel: [] }>()

type Step = 'starting' | 'waiting' | 'blocked' | 'connecting' | 'failed'
const mode = ref<'browser' | 'code'>('browser')
const step = ref<Step>('starting')
const started = ref<OAuthStart | null>(null)
const error = ref<string | null>(null)
const copied = ref(false)
let timer: ReturnType<typeof setTimeout> | undefined
let stopped = false
let popup: Window | null = null
const desktopApp = isDesktop()

const name = computed(() => OAUTH_PROVIDERS[props.provider].name)
/** Microsoft can also sign in with a code; Google can't (not for mail). */
const canUseCode = computed(() => props.provider === 'microsoft')
/** Where the provider must send the window back: register it with them. */
const callbackUrl = computed(
  () => started.value?.redirect_uri ?? `${location.origin}/api/oauth/${props.provider}/callback`,
)

function openWindow(url = '') {
  const width = 520
  const height = 680
  const left = Math.round(window.screenX + (window.outerWidth - width) / 2)
  const top = Math.round(window.screenY + (window.outerHeight - height) / 2)
  return window.open(url, 'havre-oauth', `popup,width=${width},height=${height},left=${left},top=${top}`)
}

async function start(next: 'browser' | 'code' = mode.value) {
  clearTimeout(timer)
  mode.value = next
  error.value = null
  started.value = null
  step.value = 'starting'
  // Open the window right away, while the click still counts as a user
  // action — otherwise the browser blocks it. It gets its address below.
  closePopup()
  // In Havre Desktop, sign-in happens in your browser (where your passwords
  // and passkeys are): no pop-up, the page is opened once we have it.
  const desktop = isDesktop()
  if (next === 'browser' && !desktop) popup = openWindow()
  try {
    const body: OAuthRequest = props.reconnect ? { reconnect: props.reconnect } : { ...props.request }
    started.value = await startOAuth({ ...body, provider: props.provider, mode: next, origin: location.origin })
    if (next === 'browser' && desktop) {
      await openExternal(started.value.authorize_url!)
      step.value = 'waiting'
    } else if (next === 'browser') {
      if (popup && !popup.closed) {
        popup.location.href = started.value.authorize_url!
        popup.focus()
        step.value = 'waiting'
      } else {
        step.value = 'blocked'
      }
    } else {
      step.value = 'waiting'
    }
    schedule()
  } catch (err) {
    closePopup()
    fail(apiErrorMessage(err))
  }
}

/** The pop-up was blocked or closed: open it again (this click allows it). */
function reopen() {
  if (!started.value?.authorize_url) return
  if (isDesktop()) {
    openExternal(started.value.authorize_url)
    step.value = 'waiting'
    return
  }
  popup = openWindow(started.value.authorize_url)
  if (popup) step.value = 'waiting'
}

function closePopup() {
  try {
    popup?.close()
  } catch {
    // Already gone.
  }
  popup = null
}

function schedule(delay?: number) {
  if (stopped || !started.value) return
  clearTimeout(timer)
  const interval = mode.value === 'code' ? Math.max(started.value.interval ?? 5, 2) * 1000 : 1500
  timer = setTimeout(poll, delay ?? interval)
}

async function poll() {
  if (!started.value) return
  try {
    const result = await pollOAuth(started.value.flow)
    if (stopped) return
    if (result.state === 'pending') return schedule()
    if (result.state === 'connecting') {
      step.value = 'connecting'
      return schedule()
    }
    if (result.state === 'failed') return fail(result.message)
    step.value = 'connecting'
    emit('done', result.account)
  } catch (err) {
    if (!stopped) {
      error.value = apiErrorMessage(err)
      schedule()
    }
  }
}

function fail(message: string) {
  step.value = 'failed'
  error.value = message
}

// The sign-in window says "I'm back": check right away.
function onMessage(event: MessageEvent) {
  if (event.origin === location.origin && event.data?.type === 'havre-oauth') schedule(0)
}

async function copy() {
  if (!started.value?.user_code) return
  try {
    await navigator.clipboard.writeText(started.value.user_code)
    copied.value = true
    setTimeout(() => (copied.value = false), 1500)
  } catch {
    // Clipboard needs https (or localhost): the code is selectable anyway.
  }
}

function cancel() {
  closePopup()
  emit('cancel')
}

onMounted(() => {
  window.addEventListener('message', onMessage)
  if (props.configured) start()
})
onBeforeUnmount(() => {
  stopped = true
  clearTimeout(timer)
  window.removeEventListener('message', onMessage)
})
</script>

<template>
  <div class="ms">
    <template v-if="!configured && provider === 'microsoft'">
      <p class="lead">
        <strong>OAuth2 with Microsoft isn't set up on this Havre yet.</strong>
        Microsoft 365 and Outlook.com mailboxes no longer accept passwords from mail apps.
      </p>
      <ol class="steps">
        <li>
          In <a href="https://entra.microsoft.com" target="_blank" rel="noopener">Microsoft Entra</a> ›
          App registrations, create an app named <em>Havre</em>.
        </li>
        <li>
          In <em>Authentication</em>, add the platform <em>Mobile and desktop applications</em> with the
          redirect URI <code class="uri">{{ callbackUrl }}</code>, and turn on
          <em>Allow public client flows</em>.
        </li>
        <li>
          In <em>API permissions</em>, add <em>Office 365 Exchange Online › IMAP.AccessAsUser.All</em> and
          <em>SMTP.Send</em>, and <em>Microsoft Graph › Calendars.ReadWrite</em> (all delegated). Your admin may
          need to grant consent.
        </li>
        <li>
          Copy its <em>Application (client) ID</em> into <code>HAVRE_MICROSOFT_CLIENT_ID</code> in
          <code>.env</code>, then restart Havre.
        </li>
      </ol>
      <p class="faint">docs/providers.md has the details (section “Microsoft 365 and Outlook”).</p>
    </template>

    <template v-else-if="!configured">
      <p class="lead">
        <strong>OAuth2 with Google isn't set up on this Havre yet.</strong>
        Until it is, Gmail works with a normal password if you create an app password.
      </p>
      <ol class="steps">
        <li>
          In <a href="https://console.cloud.google.com" target="_blank" rel="noopener">Google Cloud console</a>,
          create a project named <em>Havre</em> and enable the <em>Gmail API</em> and the
          <em>Google Calendar API</em>.
        </li>
        <li>
          In <em>Google Auth Platform</em>, set up the consent screen, and add the scopes
          <code>https://mail.google.com/</code> and <code>…/auth/calendar</code> under <em>Data access</em>.
        </li>
        <li>
          In <em>Clients</em>, create a <em>Web application</em> client with the authorized redirect URI
          <code class="uri">{{ callbackUrl }}</code>.
        </li>
        <li>
          Copy its <em>Client ID</em> and <em>Client secret</em> into <code>HAVRE_GOOGLE_CLIENT_ID</code> and
          <code>HAVRE_GOOGLE_CLIENT_SECRET</code> in <code>.env</code>, then restart Havre.
        </li>
      </ol>
      <p class="faint">docs/providers.md has the details (section “Gmail and Google Workspace”).</p>
    </template>

    <template v-else-if="step === 'starting'">
      <p class="status"><span class="spinner" aria-hidden="true" /> Contacting {{ name }}…</p>
    </template>

    <template v-else-if="step === 'connecting'">
      <p class="status" role="status">
        <span class="spinner" aria-hidden="true" /> Signed in — opening your mailbox…
      </p>
    </template>

    <template v-else-if="mode === 'browser' && (step === 'waiting' || step === 'blocked')">
      <p v-if="step === 'waiting'" class="status" role="status">
        <span class="spinner" aria-hidden="true" />
        <span>
          Sign in<template v-if="email"> as <strong>{{ email }}</strong></template> in the {{ name }}
          {{ desktopApp ? 'page that opened in your browser' : 'window' }}, then accept.{{ desktopApp ? ' Havre continues here by itself.' : '' }}
        </span>
      </p>
      <p v-else class="lead">Your browser blocked the {{ name }} window.</p>
      <div class="code-row">
        <UButton
          trailing-icon="i-havre-external"
          :color="step === 'blocked' ? 'primary' : 'neutral'"
          :variant="step === 'blocked' ? 'solid' : 'outline'"
          size="sm"
          @click="reopen"
        >
          {{ step === 'blocked' ? `Open the ${name} window` : 'Open it again' }}
        </UButton>
        <UButton
          v-if="canUseCode"
          color="neutral"
          variant="outline"
          size="sm"
          type="button"
          @click="start('code')"
        >
          Use a code instead
        </UButton>
      </div>
      <p v-if="provider === 'microsoft'" class="faint">
        Microsoft says the redirect URI doesn't match? Add <code class="uri">{{ callbackUrl }}</code> to the
        app registration (Authentication › Mobile and desktop applications), or use a code.
      </p>
      <p v-else class="faint">
        Google says <em>redirect_uri_mismatch</em>? Add <code class="uri">{{ callbackUrl }}</code> to the
        OAuth client's authorized redirect URIs.
      </p>
    </template>

    <template v-else-if="mode === 'code' && started?.user_code && step === 'waiting'">
      <p class="lead">
        Open the Microsoft page, enter this code, then sign in<template v-if="email">
          as <strong>{{ email }}</strong></template
        >.
      </p>
      <div class="code-row">
        <output class="code mono" aria-label="Code to enter">{{ started.user_code }}</output>
        <UButton
          :icon="havreIcon(copied ? 'check' : 'copy')"
          color="neutral"
          variant="outline"
          size="sm"
          type="button"
          @click="copy"
        >
          {{ copied ? 'Copied' : 'Copy' }}
        </UButton>
        <UButton size="sm" :href="started.verification_uri" target="_blank" rel="noopener">
          Open microsoft.com
          <HIcon name="external" :size="14" />
        </UButton>
      </div>
      <p class="status" role="status">
        <span class="spinner" aria-hidden="true" />
        Waiting for you to approve on Microsoft’s page…
      </p>
    </template>

    <UAlert v-if="error" color="error" variant="soft" icon="i-havre-alert" :title="error" role="alert" />

    <div class="actions">
      <UButton
        v-if="configured && canUseCode && step === 'failed' && mode === 'browser'"
        color="neutral"
        variant="outline"
        type="button"
        @click="start('code')"
      >
        Use a code instead
      </UButton>
      <UButton color="neutral" variant="outline" type="button" @click="cancel">Cancel</UButton>
      <UButton v-if="configured && step === 'failed'" type="button" @click="start()">Try again</UButton>
    </div>
  </div>
</template>

<style scoped>
.ms {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.lead {
  margin: 0;
  font-size: 14px;
  line-height: 1.55;
  color: var(--muted);
}
.lead strong {
  color: var(--text);
  font-weight: 600;
}
.steps {
  margin: 0;
  padding-left: 20px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 13.5px;
  line-height: 1.55;
  color: var(--muted);
}
em {
  font-style: normal;
  color: var(--text);
}
code {
  font-family: var(--font-mono);
  font-size: 12px;
}
.uri {
  word-break: break-all;
  user-select: all;
}
.faint {
  margin: 0;
  font-size: 12.5px;
  color: var(--faint);
}
.code-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}
.code {
  padding: 8px 16px;
  border-radius: 10px;
  border: 1px dashed var(--line2);
  background: var(--bg);
  font-size: 22px;
  letter-spacing: 0.12em;
  user-select: all;
}
.status {
  display: flex;
  align-items: center;
  gap: 10px;
  margin: 0;
  font-size: 13px;
  color: var(--muted);
}
.spinner {
  flex: 0 0 auto;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  border: 2px solid var(--line2);
  border-top-color: var(--accent);
  animation: spin 0.7s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
.actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>
