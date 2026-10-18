<script setup lang="ts">
// The door: first run ("choose a password") or sign in. No dock here.
definePageMeta({ layout: false })

const route = useRoute()
const status = useAuthStatus()
const firstRun = computed(() => status.value?.setup_needed === true)
useHead({ title: computed(() => (firstRun.value ? 'Welcome · Havre' : 'Sign in · Havre')) })

const password = ref('')
const confirm = ref('')
const show = ref(false)
const busy = ref(false)
const error = ref<string | null>(null)
const input = ref<HTMLInputElement | null>(null)

const tooShort = computed(() => firstRun.value && password.value.length > 0 && password.value.length < 8)
const mismatch = computed(() => firstRun.value && confirm.value.length > 0 && confirm.value !== password.value)

async function submit() {
  error.value = null
  if (firstRun.value && (password.value.length < 8 || password.value !== confirm.value)) {
    error.value = password.value.length < 8 ? 'At least 8 characters.' : 'The two passwords differ.'
    return
  }
  busy.value = true
  try {
    if (firstRun.value) await setupPassword(password.value)
    else await signIn(password.value)
    // Back where you were going (only paths on this site: not `//other.site`).
    const asked = route.query.next
    const next = typeof asked === 'string' && /^\/(?![/\\])/.test(asked) ? asked : '/'
    await navigateTo(next)
  } catch (err) {
    error.value = apiErrorMessage(err)
    password.value = firstRun.value ? password.value : ''
    await nextTick()
    input.value?.focus()
  } finally {
    busy.value = false
  }
}

onMounted(() => input.value?.focus())
</script>

<template>
  <main class="door">
    <form class="card" @submit.prevent="submit">
      <div class="logo" aria-hidden="true">
        <HIcon name="logo" :size="26" :stroke-width="1.9" />
      </div>

      <template v-if="firstRun">
        <h1 class="serif">Welcome to Havre</h1>
        <p class="lead">
          Choose the password that protects this Havre. Anyone who can open this page would otherwise read your
          mail.
        </p>
      </template>
      <template v-else>
        <h1 class="serif">Havre</h1>
        <p class="lead">Enter your password to continue.</p>
      </template>

      <div class="field">
        <label for="password">{{ firstRun ? 'New password' : 'Password' }}</label>
        <div class="password">
          <input
            id="password"
            ref="input"
            v-model="password"
            :type="show ? 'text' : 'password'"
            :autocomplete="firstRun ? 'new-password' : 'current-password'"
            required
            :aria-invalid="tooShort || undefined"
          />
          <button
            type="button"
            class="reveal"
            :aria-label="show ? 'Hide password' : 'Show password'"
            :aria-pressed="show"
            @click="show = !show"
          >
            {{ show ? 'Hide' : 'Show' }}
          </button>
        </div>
        <small v-if="firstRun" class="hint" :class="{ warn: tooShort }">
          At least 8 characters. A short sentence is easy to remember and hard to guess.
        </small>
      </div>

      <label v-if="firstRun" class="field">
        <span>Confirm</span>
        <input
          v-model="confirm"
          :type="show ? 'text' : 'password'"
          autocomplete="new-password"
          required
          :aria-invalid="mismatch || undefined"
        />
        <small v-if="mismatch" class="hint warn">The two passwords differ.</small>
      </label>

      <div v-if="error" class="error" role="alert">
        <HIcon name="alert" :size="16" />
        <span>{{ error }}</span>
      </div>

      <button type="submit" class="btn btn-primary submit" :disabled="busy">
        <span v-if="busy" class="spinner" aria-hidden="true" />
        {{ firstRun ? 'Create password' : 'Sign in' }}
      </button>

      <p v-if="!firstRun" class="foot">
        Forgot it? Restart Havre once with <code>HAVRE_RESET_PASSWORD=1</code>: your accounts and mail stay.
      </p>
    </form>
  </main>
</template>

<style scoped>
.door {
  min-height: 100%;
  display: grid;
  place-items: center;
  padding: 32px 16px;
  background:
    radial-gradient(1200px 600px at 50% -10%, color-mix(in oklab, var(--accent) 7%, transparent), transparent),
    var(--bg);
}
.card {
  width: min(400px, 100%);
  display: flex;
  flex-direction: column;
  gap: 18px;
  padding: 36px 32px 28px;
  border-radius: var(--radius-lg);
  border: 1px solid var(--line);
  background: var(--panel);
  box-shadow: 0 30px 80px -40px rgb(0 0 0 / 0.5);
}
.logo {
  width: 44px;
  height: 44px;
  border-radius: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--text);
  color: var(--bg);
}
h1 {
  margin: 4px 0 -8px;
  font-size: 32px;
  line-height: 1.1;
}
.lead {
  margin: 0;
  color: var(--muted);
  font-size: 14px;
  line-height: 1.55;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.field > span,
.field > label {
  font-size: 12.5px;
  color: var(--muted);
}
input {
  width: 100%;
  height: 42px;
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
input[aria-invalid] {
  border-color: var(--danger);
}
.password {
  position: relative;
}
.password input {
  padding-right: 64px;
}
.reveal {
  position: absolute;
  top: 6px;
  right: 6px;
  height: 30px;
  padding: 0 10px;
  border: 0;
  border-radius: 7px;
  background: transparent;
  color: var(--muted);
  font-size: 12.5px;
  cursor: pointer;
}
.reveal:hover {
  background: var(--hover);
  color: var(--text);
}
.hint {
  font-size: 12.5px;
  line-height: 1.5;
  color: var(--faint);
}
.hint.warn {
  color: var(--danger);
}
.error {
  display: flex;
  gap: 10px;
  padding: 11px 13px;
  border-radius: var(--radius);
  background: color-mix(in oklab, var(--danger) 12%, transparent);
  color: var(--danger);
  font-size: 13.5px;
  line-height: 1.5;
}
.error span {
  color: var(--text);
}
.submit {
  height: 42px;
  justify-content: center;
  font-size: 14.5px;
}
.submit:disabled {
  opacity: 0.7;
  cursor: default;
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
.foot {
  margin: 0;
  padding-top: 14px;
  border-top: 1px solid var(--line);
  font-size: 12.5px;
  line-height: 1.55;
  color: var(--faint);
}
code {
  font-family: var(--font-mono);
  font-size: 11.5px;
}
@media (max-width: 440px) {
  .card {
    padding: 28px 20px 22px;
  }
}
</style>
