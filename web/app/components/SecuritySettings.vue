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
      <button type="button" class="btn" @click="signOut">Sign out</button>
    </div>

    <article class="card">
      <div class="row">
        <div class="info">
          <strong>Password</strong>
          <span class="faint">Changing it signs out every other browser.</span>
        </div>
        <button v-if="!open" type="button" class="btn small" @click="open = true">Change</button>
      </div>
      <form v-if="open" class="pw" @submit.prevent="save">
        <label class="field wide">
          <span>Current password</span>
          <input v-model="current" type="password" autocomplete="current-password" required />
        </label>
        <label class="field">
          <span>New password</span>
          <input v-model="next" type="password" autocomplete="new-password" required minlength="8" />
        </label>
        <label class="field">
          <span>Confirm</span>
          <input v-model="confirm" type="password" autocomplete="new-password" required />
        </label>
        <div v-if="error" class="error wide" role="alert">
          <HIcon name="alert" :size="16" />
          <span>{{ error }}</span>
        </div>
        <div class="actions wide">
          <button type="button" class="btn small" :disabled="busy" @click="open = false">Cancel</button>
          <button type="submit" class="btn btn-primary small" :disabled="busy">Save password</button>
        </div>
      </form>
    </article>

    <article class="card">
      <div class="row">
        <div class="info">
          <strong>Signed-in browsers</strong>
          <span class="faint">Sessions end after 30 days without a visit.</span>
        </div>
        <button v-if="others" type="button" class="btn small" @click="signOutEverywhereElse">
          Sign out the {{ others }} other{{ others > 1 ? 's' : '' }}
        </button>
      </div>
      <ul class="sessions">
        <li v-for="(s, i) in sessions" :key="i">
          <span>{{ describeBrowser(s.user_agent) }}</span>
          <span v-if="s.current" class="badge">This browser</span>
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
.btn.small {
  height: 32px;
  padding: 0 12px;
  font-size: 13px;
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
input {
  height: 38px;
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
.error {
  display: flex;
  gap: 10px;
  padding: 10px 12px;
  border-radius: var(--radius);
  background: color-mix(in oklab, var(--danger) 12%, transparent);
  color: var(--danger);
  font-size: 13px;
}
.error span {
  color: var(--text);
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
.badge {
  padding: 1px 8px;
  border-radius: 999px;
  background: color-mix(in oklab, var(--accent) 16%, transparent);
  color: var(--accent);
  font-size: 11.5px;
}
@media (max-width: 560px) {
  .pw {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
