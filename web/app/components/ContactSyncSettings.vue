<script setup lang="ts">
import type { ContactSettings, ContactSource } from '~/composables/usePlatform'

// Settings › Your apps › People: which accounts keep their address book in
// step with People (Google Contacts, Outlook, CardDAV), where details of
// someone in no address book go, and .vcf files in and out.
const toast = useToasts()
const settings = ref<ContactSettings | null>(null)
const busy = ref<string | null>(null)

async function load() {
  try {
    settings.value = await fetchContactSettings()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
onMounted(load)
useMailEvents((event) => {
  if (event.type === 'accounts' || event.type === 'people') load()
})

async function toggle(s: ContactSource) {
  busy.value = s.key
  try {
    settings.value = await setContactSource(s.key, !s.enabled)
    if (!s.enabled) {
      const n = settings.value.sources.find((x) => x.key === s.key)?.books.reduce((sum, b) => sum + b.contacts, 0) ?? 0
      toast.show(`${s.name}: ${n} contact${n === 1 ? '' : 's'} in People`)
    }
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    busy.value = null
  }
}

async function syncNow() {
  busy.value = 'all'
  try {
    settings.value = await syncContacts()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    busy.value = null
  }
}

const KIND: Record<ContactSource['kind'], string> = {
  microsoft: 'Outlook contacts',
  google: 'Google Contacts',
  caldav: 'Address books (CardDAV)',
}
function meta(s: ContactSource) {
  if (!s.allowed) return 'Its sign-in doesn’t allow contacts yet'
  if (busy.value === s.key) return 'Syncing…'
  if (!s.enabled) return `Off — ${KIND[s.kind]} stay${s.kind === 'caldav' ? '' : 's'} where they are`
  const error = s.books.find((b) => b.error)?.error
  if (error) return error
  if (!s.books.length) return 'No address book found yet'
  const n = s.books.reduce((sum, b) => sum + b.contacts, 0)
  const when = s.books[0]?.synced_at
  return `${s.books.map((b) => b.name).join(', ')} · ${n} contact${n === 1 ? '' : 's'}${when ? ` · ${agoText(when)}` : ''}`
}
const bad = (s: ContactSource) => !s.allowed || (s.enabled && s.books.some((b) => b.error))
const color = (s: ContactSource) =>
  ({ microsoft: 'var(--c-work)', google: 'var(--c-personal)', caldav: 'var(--c-hol)' })[s.kind]

// Where details go when you edit someone in no address book.
const HAVRE_ONLY = -1
const books = computed(() =>
  (settings.value?.sources ?? []).filter((s) => s.enabled).flatMap((s) => s.books.filter((b) => !b.read_only)),
)
const destinations = computed(() => [
  { label: 'Havre only', value: HAVRE_ONLY },
  ...books.value.map((b) => ({ label: b.name, value: b.id })),
])
async function setDestination(value: number) {
  try {
    settings.value = await setDefaultBook(value === HAVRE_ONLY ? null : value)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

// .vcf files: read in the browser, sent as text.
const file = ref<HTMLInputElement | null>(null)
async function onFile(event: Event) {
  const input = event.target as HTMLInputElement
  const picked = input.files?.[0]
  input.value = ''
  if (!picked) return
  busy.value = 'import'
  try {
    const r = await importPeople(await picked.text())
    const parts = [`${r.added} added`, `${r.updated} completed`]
    if (r.skipped) parts.push(`${r.skipped} skipped`)
    toast.show(`${picked.name}: ${parts.join(', ')}`)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    busy.value = null
  }
}
</script>

<template>
  <section aria-label="Contacts" class="contact-sync">
    <p class="lead">
      Keep your address books in step with People: their details come into Havre, and what you change here goes back
      to them. Nothing is ever deleted from an address book.
    </p>
    <p v-if="settings && !settings.sources.length" class="empty">
      No account can sync contacts yet: Microsoft and Google accounts (signed in with OAuth2) and CalDAV servers
      (Settings › Calendars) can.
    </p>
    <div v-if="settings?.sources.length" class="card list">
      <div v-for="s in settings.sources" :key="s.key" class="row">
        <span class="dot" :style="{ background: color(s) }" aria-hidden="true" />
        <div class="info">
          <strong>{{ s.name }}</strong>
          <span class="meta" :class="{ bad: bad(s) }">{{ meta(s) }}</span>
        </div>
        <UButton
          v-if="!s.allowed && s.account"
          size="xs"
          color="neutral"
          variant="outline"
          :to="{ path: '/settings', query: { reconnect: s.account } }"
        >
          Reconnect
        </UButton>
        <USwitch
          :model-value="s.enabled"
          :aria-label="`Sync contacts with ${s.name}`"
          :disabled="busy !== null || (!s.allowed && !s.enabled)"
          @update:model-value="toggle(s)"
        />
      </div>
    </div>
    <div v-if="books.length" class="where">
      <UFormField label="When you edit someone in no address book, save to" class="flex-1">
        <USelect
          :model-value="settings?.default_book ?? HAVRE_ONLY"
          :items="destinations"
          class="w-full"
          @update:model-value="(v) => setDestination(v as number)"
        />
      </UFormField>
      <UButton color="neutral" variant="outline" icon="i-havre-refresh" :loading="busy === 'all'" @click="syncNow">
        Sync now
      </UButton>
    </div>
    <div class="files">
      <input ref="file" type="file" accept=".vcf,text/vcard,text/x-vcard" class="sr-only" @change="onFile" />
      <UButton color="neutral" variant="outline" icon="i-havre-upload" :loading="busy === 'import'" @click="file?.click()">
        Import a .vcf file
      </UButton>
      <UButton color="neutral" variant="ghost" icon="i-havre-download" :href="PEOPLE_EXPORT" download="havre-people.vcf">
        Export as .vcf
      </UButton>
      <span class="hint">An import goes into Havre only; it fills what’s missing and never replaces your details.</span>
    </div>
  </section>
</template>

<style scoped>
.contact-sync {
  display: flex;
  flex-direction: column;
  gap: 14px;
  margin-top: 8px;
}
.lead {
  margin: 0;
  color: var(--muted);
  font-size: 13.5px;
  max-width: 640px;
}
.empty {
  margin: 0;
  color: var(--faint);
  font-size: 13.5px;
}
.card {
  border-radius: var(--radius);
  border: 1px solid var(--line);
  background: var(--panel);
}
.row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 16px;
  border-bottom: 1px solid var(--line);
}
.row:last-child {
  border-bottom: 0;
}
.dot {
  flex: 0 0 8px;
  height: 8px;
  border-radius: 2px;
}
.info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  font-size: 13.5px;
}
.info strong {
  font-weight: 500;
}
.meta {
  font-size: 12px;
  color: var(--faint);
}
.meta.bad {
  color: var(--danger);
}
.where {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  gap: 10px;
}
.hint {
  color: var(--faint);
  font-size: 12.5px;
}
.files {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}
</style>
