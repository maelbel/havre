<script setup lang="ts">
import type { Account, OAuthProvider } from '~/composables/useAccounts'

useHead({ title: 'Accounts · Settings · Havre' })
const toast = useToasts()
const route = useRoute()

const accounts = useAccounts()
const health = useFetch<{ demo: boolean; oauth?: Partial<Record<OAuthProvider, boolean>> }>('/api/health', {
  server: false,
  lazy: true,
})
const demo = computed(() => health.data.value?.demo === true)
const oauthConfigured = computed(() => health.data.value?.oauth ?? {})
const authStatus = useAuthStatus()
const authEnabled = computed(() => authStatus.value?.enabled === true)
const tasksOn = useTasksEnabled()
/** OAuth2 accounts signed in before Havre asked for calendar / task access. */
const needsReconnect = (a: Account) =>
  a.status.state === 'error' || !a.can_send || !a.can_calendar || (tasksOn.value && !a.can_tasks)

// OAuth2 sign-in again, for an account whose sign-in expired.
const reconnecting = ref<number | null>(null)
async function onReconnected(account: { name: string }) {
  reconnecting.value = null
  await accounts.refresh()
  toast.show(`${account.name} reconnected`)
}

// Which form is open: 'new', an account id, or nothing.
const editing = ref<'new' | number | null>(route.query.add ? 'new' : null)
const confirmRemove = ref<number | null>(null)

async function onSaved(account: { id: number; name: string }, isNew: boolean) {
  editing.value = null
  await accounts.refresh()
  toast.show(isNew ? `${account.name} connected — downloading your mail…` : `${account.name} saved`)
}

async function remove(account: Account) {
  if (confirmRemove.value !== account.id) {
    confirmRemove.value = account.id
    setTimeout(() => {
      if (confirmRemove.value === account.id) confirmRemove.value = null
    }, 4000)
    return
  }
  confirmRemove.value = null
  try {
    await removeAccount(account.id)
    await accounts.refresh()
    toast.show(`${account.name} removed from Havre`)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

// Status lines update live.
useMailEvents((event) => {
  if (event.type === 'status' || event.type === 'accounts') accounts.refresh()
})

function statusText(a: Account) {
  const s = a.status
  if (s.state === 'error') return s.error ?? 'Sync problem'
  if (s.state === 'starting') return 'Connecting…'
  if (s.state === 'syncing' || s.backfilling) return 'Downloading mail…'
  return s.push ? 'Up to date · new mail arrives instantly' : 'Up to date · checked every 5 minutes'
}
</script>

<template>
  <SettingsFrame>

      <section aria-labelledby="accounts-title">
        <div class="section-head">
          <div>
            <h2 id="accounts-title">Mail accounts</h2>
            <p class="lead">
              Havre keeps a copy of each mailbox on this server. Passwords and sign-ins are stored encrypted.
            </p>
          </div>
          <UButton
            v-if="!demo && editing !== 'new'"
            icon="i-havre-plus"
            type="button"
            @click="editing = 'new'"
          >
            Add account
          </UButton>
        </div>

        <p v-if="demo" class="notice">
          Demo mode (<code>HAVRE_DEMO=1</code>): accounts can't be changed. Remove it from <code>.env</code> to use your
          own mailboxes.
        </p>

        <AccountForm
          v-if="editing === 'new'"
          :oauth-configured="oauthConfigured"
          @saved="(a) => onSaved(a, true)" @cancel="editing = null" />

        <div v-if="accounts.status.value === 'pending' && !accounts.data.value.length" class="card skeleton" />

        <p v-else-if="!accounts.data.value.length && editing !== 'new'" class="empty">
          No account yet. Add one to start reading your mail.
        </p>

        <template v-for="a in accounts.data.value" :key="a.id">
          <AccountForm
            v-if="editing === a.id"
            :account="a"
            :oauth-configured="oauthConfigured"
            @saved="(saved) => onSaved(saved, false)"
            @cancel="editing = null"
          />
          <article v-else class="card">
            <span class="dot" :style="{ background: `var(--c-${a.color})` }" aria-hidden="true" />
            <div class="info">
              <div class="title">
                <strong>{{ a.name }}</strong>
                <span class="faint">{{ a.email }}</span>
              </div>
              <div class="meta">
                <span class="mono faint">
                  {{ a.imap_host }}:{{ a.imap_port }}{{ a.auth !== 'password' ? ' · OAuth2' : '' }}
                </span>
                <span class="status" :class="a.status.state">{{ statusText(a) }}</span>
                <span v-if="!a.can_send" class="status error">Reconnect to allow sending</span>
                <span v-else-if="a.auth !== 'password' && !a.can_calendar" class="status error">
                  Reconnect to show its calendar in the Agenda
                </span>
                <span v-else-if="a.auth !== 'password' && tasksOn && !a.can_tasks" class="status error">
                  Reconnect to sync its tasks ({{ a.auth === 'microsoft' ? 'To Do' : 'Google Tasks' }})
                </span>
              </div>
            </div>
            <div v-if="!demo" class="buttons">
              <UButton
                v-if="a.auth !== 'password' && needsReconnect(a) && reconnecting !== a.id"
                size="sm"
                type="button"
                @click="reconnecting = a.id"
              >
                Reconnect
              </UButton>
              <UButton
                color="neutral"
                variant="outline"
                size="sm"
                :to="{ path: '/mail', query: { account: a.id } }"
              >
                Open
              </UButton>
              <UButton color="neutral" variant="outline" size="sm" type="button" @click="editing = a.id">
                Edit
              </UButton>
              <UButton
                :color="confirmRemove === a.id ? 'error' : 'neutral'"
                variant="outline"
                size="sm"
                type="button"
                :title="confirmRemove === a.id ? 'Your mail stays on the server' : undefined"
                @click="remove(a)"
              >
                {{ confirmRemove === a.id ? 'Remove from Havre?' : 'Remove' }}
              </UButton>
            </div>
            <OAuthSignIn
              v-if="reconnecting === a.id && a.auth !== 'password'"
              class="reconnect"
              :provider="a.auth"
              :configured="!!oauthConfigured[a.auth]"
              :reconnect="a.id"
              :email="a.imap_user"
              @done="onReconnected"
              @cancel="reconnecting = null"
            />
          </article>
        </template>
      </section>


  </SettingsFrame>
</template>

<style scoped>
section {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.section-head {
  display: flex;
  flex-wrap: wrap;
  gap: 16px;
  align-items: flex-end;
  justify-content: space-between;
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
.notice {
  margin: 0;
  padding: 12px 14px;
  border-radius: var(--radius);
  border: 1px solid var(--line2);
  background: var(--bg);
  color: var(--muted);
  font-size: 13.5px;
}
code {
  font-family: var(--font-mono);
  font-size: 12px;
}
.card {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 14px;
  padding: 16px 18px;
  border-radius: var(--radius-lg);
  border: 1px solid var(--line);
  background: var(--raise);
}
.card.skeleton {
  height: 74px;
  opacity: 0.5;
}
.dot {
  flex: 0 0 10px;
  height: 10px;
  border-radius: 50%;
}
.info {
  flex: 1 1 260px;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.title {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 10px;
  align-items: baseline;
}
.meta {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 14px;
  font-size: 12.5px;
}
.faint {
  color: var(--faint);
}
.status {
  color: var(--muted);
}
.status.error {
  color: var(--danger);
}
.reconnect {
  flex: 1 1 100%;
  padding-top: 14px;
  border-top: 1px solid var(--line);
}
.buttons {
  display: flex;
  gap: 6px;
}
.empty {
  margin: 0;
  color: var(--faint);
}
</style>
