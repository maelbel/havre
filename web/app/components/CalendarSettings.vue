<script setup lang="ts">
// Settings › Calendars: CalDAV servers, and where new events go.
// (Microsoft 365 and Google calendars come with their mail account.)
const toast = useToasts()
const servers = useCalDavAccounts()
const calendars = useCalendars()

const writable = computed(() => calendars.data.value.filter((c) => !c.read_only))
const defaultId = computed(() => calendars.data.value.find((c) => c.is_default)?.id)

async function setDefault(id: number) {
  try {
    await updateCalendar(id, { is_default: true })
    await calendars.refresh()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

const adding = ref(false)
const form = reactive({ url: '', username: '', password: '', name: '' })
const busy = ref(false)
const error = ref<string | null>(null)

async function add() {
  error.value = null
  busy.value = true
  try {
    const added = await addCalDav({ ...form })
    Object.assign(form, { url: '', username: '', password: '', name: '' })
    adding.value = false
    await Promise.all([servers.refresh(), calendars.refresh()])
    toast.show(`${added.name} connected — loading its calendars…`)
  } catch (err) {
    error.value = apiErrorMessage(err)
  } finally {
    busy.value = false
  }
}

const confirmRemove = ref<number | null>(null)
async function remove(id: number, name: string) {
  if (confirmRemove.value !== id) {
    confirmRemove.value = id
    setTimeout(() => confirmRemove.value === id && (confirmRemove.value = null), 4000)
    return
  }
  confirmRemove.value = null
  try {
    await removeCalDav(id)
    await Promise.all([servers.refresh(), calendars.refresh()])
    toast.show(`${name} removed from Havre`)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

const count = (server: number) => calendars.data.value.filter((c) => c.caldav === server).length
</script>

<template>
  <section id="calendars" aria-labelledby="calendars-title">
    <div class="section-head">
      <div>
        <h2 id="calendars-title">Calendars</h2>
        <p class="lead">
          Microsoft 365 and Google calendars come with their mail account. Add other calendars (Nextcloud, iCloud,
          Fastmail…) with CalDAV.
        </p>
      </div>
      <UButton
        v-if="!adding"
        icon="i-havre-plus"
        color="neutral"
        variant="outline"
        type="button"
        @click="adding = true"
      >
        Add CalDAV
      </UButton>
    </div>

    <form v-if="adding" class="card form" @submit.prevent="add">
      <UFormField label="Server address" class="field wide">
        <UInput
          v-model="form.url"
          type="url"
          required
          placeholder="https://cloud.example.com/remote.php/dav"
          class="w-full"
        />
        <template #help>Nextcloud: <code>https://your-cloud/remote.php/dav</code> · iCloud: <code>https://caldav.icloud.com</code> (with an app-specific password) · Fastmail: <code>https://caldav.fastmail.com/dav/</code></template>
      </UFormField>
      <UFormField label="User name" class="field">
        <UInput v-model="form.username" autocomplete="username" required class="w-full" />
      </UFormField>
      <UFormField label="Password" class="field">
        <UInput
          v-model="form.password"
          type="password"
          autocomplete="current-password"
          required
          class="w-full"
        />
      </UFormField>
      <UFormField label="Name" hint="optional" class="field wide">
        <UInput v-model="form.name" placeholder="Nextcloud" class="w-full" />
      </UFormField>
      <UAlert v-if="error" color="error" variant="soft" icon="i-havre-alert" :title="error" role="alert" class="wide" />
      <div class="actions wide">
        <UButton
          color="neutral"
          variant="outline"
          size="sm"
          type="button"
          :disabled="busy"
          @click="adding = false"
        >
          Cancel
        </UButton>
        <UButton size="sm" type="submit" :disabled="busy">
          {{ busy ? 'Finding your calendars…' : 'Connect' }}
        </UButton>
      </div>
    </form>

    <article v-for="s in servers.data.value" :key="s.id" class="card row">
      <div class="info">
        <strong>{{ s.name }}</strong>
        <span class="faint mono small">{{ s.username }} · {{ s.url }}</span>
        <span class="faint small">{{ count(s.id) }} calendar{{ count(s.id) === 1 ? '' : 's' }}</span>
      </div>
      <UButton
        :color="confirmRemove === s.id ? 'error' : 'neutral'"
        variant="outline"
        size="sm"
        type="button"
        @click="remove(s.id, s.name)"
      >
        {{ confirmRemove === s.id ? 'Remove from Havre?' : 'Remove' }}
      </UButton>
    </article>

    <article class="card row">
      <div class="info">
        <strong>New events go to</strong>
        <span class="faint small">When you create an event without choosing.</span>
      </div>
      <USelect
        :model-value="defaultId"
        aria-label="Default calendar"
        :items="[...writable.map((c) => ({ label: `${c.name} · ${SOURCE_LABELS[c.source]}`, value: c.id }))]"
        @update:model-value="(id) => setDefault(id as number)"
      />
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
  max-width: 520px;
}
.card {
  padding: 16px 18px;
  border-radius: var(--radius-lg);
  border: 1px solid var(--line);
  background: var(--raise);
}
.row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.info {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}
.faint {
  color: var(--faint);
}
.small {
  font-size: 12.5px;
}
.mono {
  overflow-wrap: anywhere;
}
.form {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px 14px;
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
.hint {
  font-size: 12px;
  line-height: 1.6;
  color: var(--faint);
}
code {
  font-family: var(--font-mono);
  font-size: 11.5px;
}
.actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
@media (max-width: 560px) {
  .form {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
