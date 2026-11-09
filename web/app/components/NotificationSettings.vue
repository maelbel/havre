<script setup lang="ts">
import type { PushDevice, PushSettings } from '~/composables/usePush'

// Settings › Notifications: push notifications on this device (Turn on,
// Send a test), what Havre sends, and the devices that get them.
const toast = useToasts()

const supported = ref(false)
const homeScreen = ref(false)
const on = ref(false)
const busy = ref<'on' | 'off' | 'test' | null>(null)
const devices = ref<PushDevice[]>([])
const settings = ref<PushSettings | null>(null)

async function load() {
  supported.value = pushSupported()
  homeScreen.value = needsHomeScreen()
  try {
    on.value = !!(await currentSubscription())
  } catch {
    on.value = false
  }
  try {
    ;[devices.value, settings.value] = await Promise.all([fetchPushDevices(), fetchPushSettings()])
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
onMounted(load)

async function turnOn() {
  busy.value = 'on'
  try {
    await enablePush()
    toast.show('Notifications are on for this device')
    await load()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    busy.value = null
  }
}

async function turnOff() {
  busy.value = 'off'
  try {
    await disablePush()
    await load()
  } finally {
    busy.value = null
  }
}

async function test() {
  busy.value = 'test'
  try {
    await sendTestPush()
    toast.show('Sent. It should show in a few seconds.')
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    busy.value = null
  }
}

async function change(part: Partial<PushSettings>) {
  try {
    settings.value = await savePushSettings(part)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

async function remove(d: PushDevice) {
  try {
    if (d.this_device) await disablePush()
    else await removePushDevice(d.id)
    await load()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

function since(seconds: number) {
  return new Date(seconds * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })
}
</script>

<template>
  <section aria-labelledby="notifications-title">
    <div class="section-head">
      <div>
        <h2 id="notifications-title">Notifications</h2>
        <p class="lead">
          New mail and reminders on your phone or computer, even when Havre is closed. They're encrypted on your
          server: the phone's push service (Google, Apple, Mozilla) only carries them.
        </p>
      </div>
    </div>

    <article class="card">
      <div class="row">
        <div class="info">
          <strong>This device</strong>
          <span v-if="!supported" class="faint">
            This browser can't get push notifications{{ isDesktop() ? ': Havre Desktop shows its own.' : ' (it needs HTTPS).' }}
          </span>
          <span v-else-if="homeScreen" class="faint">
            On an iPhone or iPad, add Havre to the Home Screen first (Share › Add to Home Screen), then open it from
            there.
          </span>
          <span v-else-if="on" class="faint">Notifications are on. Nothing is sent while Havre is open here.</span>
          <span v-else class="faint">Your browser will ask for permission.</span>
        </div>
        <div v-if="supported && !homeScreen" class="buttons">
          <template v-if="on">
            <UButton color="neutral" variant="outline" size="sm" type="button" :loading="busy === 'test'" :disabled="!!busy" @click="test">
              Send a test
            </UButton>
            <UButton color="neutral" variant="ghost" size="sm" type="button" :disabled="!!busy" @click="turnOff">Turn off</UButton>
          </template>
          <UButton v-else size="sm" type="button" icon="i-havre-bell" :loading="busy === 'on'" :disabled="!!busy" @click="turnOn">
            Turn on notifications
          </UButton>
        </div>
      </div>
    </article>

    <article v-if="settings" class="card">
      <div class="info">
        <strong>What to send</strong>
        <span class="faint">To every device below.</span>
      </div>
      <USwitch
        :model-value="settings.new_mail"
        label="New mail in your inboxes"
        description="Unread mail that just arrived; more than three at once make one notification."
        @update:model-value="(v) => change({ new_mail: v })"
      />
      <USwitch
        :model-value="settings.mail_details"
        :disabled="!settings.new_mail"
        label="Show the sender and subject"
        description="Off: just “New email”, for a lock screen others can see."
        class="indent"
        @update:model-value="(v) => change({ mail_details: v })"
      />
      <USwitch
        :model-value="settings.event_reminders"
        label="Event reminders"
        description="When an event's reminder is due (set in each event, default in Settings › Calendars)."
        @update:model-value="(v) => change({ event_reminders: v })"
      />
      <USwitch
        :model-value="settings.reminders"
        label="Task reminders"
        @update:model-value="(v) => change({ reminders: v })"
      />
    </article>

    <article class="card">
      <div class="info">
        <strong>Devices</strong>
        <span class="faint">A device stops getting them when you remove it here or turn them off on it.</span>
      </div>
      <p v-if="!devices.length" class="faint empty">No device yet.</p>
      <ul v-else class="devices">
        <li v-for="d in devices" :key="d.id">
          <HIcon name="bell" :size="15" />
          <span class="what">
            <span>{{ d.label || 'A browser' }}</span>
            <span class="faint">
              Since {{ since(d.created_at) }}<template v-if="d.last_error"> · <span class="bad">{{ d.last_error }}</span></template>
            </span>
          </span>
          <UBadge v-if="d.this_device" variant="soft" size="sm">This device</UBadge>
          <UButton color="neutral" variant="ghost" size="sm" type="button" :aria-label="`Remove ${d.label || 'this device'}`" @click="remove(d)">
            Remove
          </UButton>
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
  max-width: 62ch;
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
.buttons {
  display: flex;
  gap: 6px;
}
.faint {
  color: var(--faint);
  font-size: 12.5px;
}
.indent {
  padding-left: 28px;
}
.empty {
  margin: 0;
}
.devices {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
}
.devices li {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 0;
  border-top: 1px solid var(--line);
  font-size: 13.5px;
}
.devices li:first-child {
  border-top: 0;
}
.what {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.bad {
  color: var(--danger);
}
</style>
