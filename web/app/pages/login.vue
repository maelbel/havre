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
const input = ref<{ inputRef: HTMLInputElement | null } | null>(null)

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
    // Back where you were going (see safeNext: only paths on this site).
    await navigateTo(safeNext(route.query.next))
  } catch (err) {
    error.value = apiErrorMessage(err)
    password.value = firstRun.value ? password.value : ''
    await nextTick()
    input.value?.inputRef?.focus()
  } finally {
    busy.value = false
  }
}

onMounted(() => input.value?.inputRef?.focus())
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

      <UFormField
        :label="firstRun ? 'New password' : 'Password'"
        :help="firstRun && !tooShort ? 'At least 8 characters. A short sentence is easy to remember and hard to guess.' : undefined"
        :error="tooShort ? 'At least 8 characters. A short sentence is easy to remember and hard to guess.' : undefined"
      >
        <UInput
          ref="input"
          v-model="password"
          :type="show ? 'text' : 'password'"
          :autocomplete="firstRun ? 'new-password' : 'current-password'"
          required
          size="lg"
          class="w-full"
        >
          <template #trailing>
            <UButton
              color="neutral"
              variant="ghost"
              size="sm"
              :aria-label="show ? 'Hide password' : 'Show password'"
              :aria-pressed="show"
              @click="show = !show"
            >
              {{ show ? 'Hide' : 'Show' }}
            </UButton>
          </template>
        </UInput>
      </UFormField>

      <UFormField v-if="firstRun" label="Confirm" :error="mismatch ? 'The two passwords differ.' : undefined">
        <UInput
          v-model="confirm"
          :type="show ? 'text' : 'password'"
          autocomplete="new-password"
          required
          size="lg"
          class="w-full"
        />
      </UFormField>

      <UAlert v-if="error" color="error" variant="soft" icon="i-havre-alert" :title="error" role="alert" />

      <UButton :loading="busy" type="submit" size="lg" block :disabled="busy">
        {{ firstRun ? 'Create password' : 'Sign in' }}
      </UButton>

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
