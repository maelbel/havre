<script setup lang="ts">
import type { SessionInfo } from '~/composables/useAuth'

// Settings › Security: change Havre's password, see signed-in browsers.
const toast = useToasts()

const current = ref('')
const next = ref('')
const confirm = ref('')
const busy = ref(false)
const error = ref<string | null>(null)
const open = ref(false)

async function save() {
  error.value = null
  if (next.value.length < 8) return void (error.value = 'The new password needs at least 8 characters.')
  if (next.value !== confirm.value) return void (error.value = 'The two new passwords differ.')
  busy.value = true
  try {
    await changePassword(current.value, next.value)
    current.value = next.value = confirm.value = ''
    open.value = false
    await loadSessions()
    toast.show('Password changed — other browsers were signed out')
  } catch (err) {
    error.value = apiErrorMessage(err)
  } finally {
    busy.value = false
  }
}

const sessions = ref<SessionInfo[]>([])
async function loadSessions() {
  try {
    sessions.value = await fetchSessions()
  } catch {
    sessions.value = []
  }
}
const others = computed(() => sessions.value.filter((s) => !s.current).length)

async function signOutEverywhereElse() {
  try {
    await signOutOthers()
    await loadSessions()
    toast.show('Other browsers signed out')
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

function when(seconds: number) {
  const diff = Date.now() / 1000 - seconds
  if (diff < 120) return 'active now'
  if (diff < 3600) return `${Math.round(diff / 60)} min ago`
  if (diff < 86400) return `${Math.round(diff / 3600)} h ago`
  return new Date(seconds * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short' })
}

onMounted(loadSessions)
</script>

<template>
  <section aria-labelledby="security-title">
    <div class="section-head">
      <div>
        <h2 id="security-title">Security</h2>
        <p class="lead">The password that opens this Havre, and the browsers signed in to it.</p>
      </div>
      <UButton color="neutral" variant="outline" type="button" @click="signOut">Sign out</UButton>
    </div>

    <article class="card">
      <div class="row">
        <div class="info">
          <strong>Password</strong>
          <span class="faint">Changing it signs out every other browser.</span>
        </div>
        <UButton v-if="!open" color="neutral" variant="outline" size="sm" type="button" @click="open = true">
          Change
        </UButton>
      </div>
      <form v-if="open" class="pw" @submit.prevent="save">
        <UFormField label="Current password" class="field wide">
          <UInput v-model="current" type="password" autocomplete="current-password" required class="w-full" />
        </UFormField>
        <UFormField label="New password" class="field">
          <UInput
            v-model="next"
            type="password"
            autocomplete="new-password"
            required
            minlength="8"
            class="w-full"
          />
        </UFormField>
        <UFormField label="Confirm" class="field">
          <UInput v-model="confirm" type="password" autocomplete="new-password" required class="w-full" />
        </UFormField>
        <UAlert v-if="error" color="error" variant="soft" icon="i-havre-alert" :title="error" role="alert" class="wide" />
        <div class="actions wide">
          <UButton
            color="neutral"
            variant="outline"
            size="sm"
            type="button"
            :disabled="busy"
            @click="open = false"
          >
            Cancel
          </UButton>
          <UButton size="sm" type="submit" :disabled="busy">Save password</UButton>
        </div>
      </form>
    </article>

    <article class="card">
      <div class="row">
        <div class="info">
          <strong>Signed-in browsers</strong>
          <span class="faint">Sessions end after 30 days without a visit.</span>
        </div>
        <UButton
          v-if="others"
          color="neutral"
          variant="outline"
          size="sm"
          type="button"
          @click="signOutEverywhereElse"
        >
          Sign out the {{ others }} other{{ others > 1 ? 's' : '' }}
        </UButton>
      </div>
      <ul class="sessions">
        <li v-for="(s, i) in sessions" :key="i">
          <span>{{ describeBrowser(s.user_agent) }}</span>
          <UBadge v-if="s.current" variant="soft" size="sm">This browser</UBadge>
          <span class="faint">{{ when(s.last_seen) }}</span>
        </li>
      </ul>
    </article>
  </section>
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
.card {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 16px 18px;
  border-radius: var(--radius-lg);
  border: 1px solid var(--line);
  background: var(--raise);
}
.row {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  align-items: center;
  justify-content: space-between;
}
.info {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.faint {
  color: var(--faint);
  font-size: 12.5px;
}
.pw {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px 14px;
  padding-top: 14px;
  border-top: 1px solid var(--line);
}
.wide {
  grid-column: 1 / -1;
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
.actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
.sessions {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
}
.sessions li {
  display: flex;
  gap: 10px;
  align-items: baseline;
  padding: 8px 0;
  border-top: 1px solid var(--line);
  font-size: 13.5px;
}
.sessions li .faint {
  margin-left: auto;
}
@media (max-width: 560px) {
  .pw {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
