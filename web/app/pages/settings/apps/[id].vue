<script setup lang="ts">
import type { HavreApp, SystemInfo } from '~/composables/usePlatform'

// Settings › Your apps › one app: its own settings when it has some (Tasks:
// which accounts sync), otherwise where they are or that there are none —
// and always what it is: what it shares, what it works with, what it keeps,
// where its data lives, its keys. AI has a page of its own (/settings/ai).
const route = useRoute()
const toast = useToasts()
const id = computed(() => String(route.params.id))
if (id.value === 'ai') await navigateTo('/settings/ai', { replace: true })

const { apps, loaded, refresh } = useApps()
onMounted(() => refresh().catch(() => {}))
const app = computed<HavreApp | undefined>(() => apps.value.find((a) => a.id === id.value))
useHead({ title: () => `${app.value?.name ?? 'App'} · Settings · Havre` })

const system = ref<SystemInfo | null>(null)
onMounted(async () => {
  system.value = await fetchSystem().catch(() => null)
})

/** Per app: where its data lives, what it keeps, its keys. Plain words. */
const ABOUT: Record<string, { where: string; keys: [string, string][]; keeps: (s: SystemInfo) => string }> = {
  mail: {
    where: 'Your mail stays on its servers; Havre keeps a copy to be fast and to work offline.',
    keys: [['C', 'write'], ['R', 'reply'], ['S', 'summary (AI)'], ['T', 'task'], ['N', 'note'], ['/', 'search']],
    keeps: (s) => `A copy of the mail of ${plural(s.accounts, 'account')}`,
  },
  agenda: {
    where: 'Events live in their calendar (Microsoft, Google, CalDAV); Havre keeps a copy, synced every few minutes.',
    keys: [['N', 'new event'], ['T', 'today']],
    keeps: (s) => `A copy of ${plural(s.calendars, 'calendar')}`,
  },
  people: {
    where: 'On this server. Nothing to fill in: People is made from your mail and your meetings.',
    keys: [],
    keeps: (s) => `${plural(s.people, 'person', 'people')}, from Mail and Agenda — nothing of its own`,
  },
  tasks: {
    where: 'Each list lives in its app (Microsoft To Do, Google Tasks, CalDAV) or in Havre; the links to emails, people and meetings stay in Havre.',
    keys: [['N', 'quick add'], ['T', 'a task from an email (in Mail)']],
    keeps: (s) => `${plural(s.tasks ?? 0, 'task')} to do`,
  },
  files: {
    where: 'Files stay in your mail, on its servers. Havre keeps only a list of them, and fetches a file when you open or download it.',
    keys: [['/', 'search'], ['D', 'download'], ['O', 'open the email'], ['Esc', 'close']],
    keeps: (s) => `A list of ${plural(s.files ?? 0, 'file')} — not the files`,
  },
  notes: {
    where: 'In Havre’s database only, so in the nightly backups. Each note downloads as a .md file.',
    keys: [['N', 'new note'], ['E', 'edit'], ['Esc', 'read']],
    keeps: (s) => plural(s.notes ?? 0, 'note'),
  },
}
function plural(n: number, one: string, many = `${one}s`) {
  return `${n} ${n === 1 ? one : many}`
}
const about = computed(() => ABOUT[id.value])
const partners = computed(() => (app.value ? appPartners(app.value, apps.value.filter((a) => a.enabled)) : []))

// In the dock, or not.
const busy = ref(false)
const confirmRemove = ref(false)
async function setInDock(on: boolean) {
  if (!app.value) return
  if (!on && !confirmRemove.value) {
    confirmRemove.value = true
    setTimeout(() => (confirmRemove.value = false), 4000)
    return
  }
  busy.value = true
  try {
    await setAppEnabled(app.value.id, on)
    toast.show(on ? `${app.value.name} is in your dock` : `${app.value.name} removed from your dock`)
    if (!on) await navigateTo('/apps')
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    busy.value = false
    confirmRemove.value = false
  }
}
</script>

<template>
  <SettingsFrame>
    <p v-if="loaded && !app" class="faint">There’s no app called “{{ id }}”. <NuxtLink to="/apps">App library →</NuxtLink></p>
    <div v-else-if="app" class="app-page">
      <header class="head">
        <span class="tile" :style="{ '--t': appTint(app) }" aria-hidden="true"><HIcon :name="app.icon" :size="24" /></span>
        <div class="who">
          <h2 class="serif">{{ app.name }}</h2>
          <div class="faint small">{{ appDetail(app, system) }}</div>
        </div>
        <template v-if="app.enabled">
          <UButton color="neutral" variant="outline" :to="app.route">Open</UButton>
          <UButton
            :color="confirmRemove ? 'error' : 'neutral'"
            :variant="confirmRemove ? 'outline' : 'ghost'"
            type="button"
            :disabled="busy"
            @click="setInDock(false)"
          >
            {{ confirmRemove ? 'Click again to remove' : 'Remove from the dock' }}
          </UButton>
        </template>
        <UButton v-else type="button" :disabled="busy" @click="setInDock(true)">Add to your dock</UButton>
      </header>
      <p class="desc">{{ app.description }}</p>

      <section aria-labelledby="app-settings">
        <h3 id="app-settings" class="eyebrow">Settings</h3>
        <TaskSyncSettings v-if="app.id === 'tasks'" />
        <FilesSettings v-else-if="app.id === 'files'" />
        <p v-else-if="app.id === 'mail'" class="none">
          Accounts, their passwords and sending servers are in <NuxtLink to="/settings">Settings › Accounts</NuxtLink>.
        </p>
        <p v-else-if="app.id === 'agenda'" class="none">
          Calendars — which ones show, their colour, where new events go, CalDAV servers to add — are in
          <NuxtLink to="/settings/calendars">Settings › Calendars</NuxtLink>.
        </p>
        <p v-else class="none">{{ app.name }} has no settings of its own.</p>
      </section>

      <section aria-labelledby="app-about">
        <h3 id="app-about" class="eyebrow">About</h3>
        <dl class="facts">
          <template v-if="appKinds(app).length">
            <dt>Shares</dt>
            <dd class="chips">
              <span v-for="k in appKinds(app)" :key="k" class="chip">{{ k }}</span>
              <span class="faint small">{{ app.provides.map((p) => APP_CAPABILITIES[p] ?? p).join(', ') }}</span>
            </dd>
          </template>
          <template v-if="partners.length">
            <dt>Works with</dt>
            <dd>{{ partners.join(', ') }}</dd>
          </template>
          <template v-if="about && system">
            <dt>Keeps</dt>
            <dd>{{ about.keeps(system) }}</dd>
          </template>
          <template v-if="about">
            <dt>Where</dt>
            <dd class="muted">{{ about.where }}</dd>
          </template>
          <dt>Version</dt>
          <dd class="mono small">Built in · Havre {{ system?.version ?? '' }}</dd>
        </dl>
      </section>

      <section v-if="about?.keys.length" aria-labelledby="app-keys">
        <h3 id="app-keys" class="eyebrow">Keys</h3>
        <div class="keys">
          <span v-for="[k, what] in about.keys" :key="k"><UKbd :value="k" /> {{ what }}</span>
        </div>
      </section>
    </div>
  </SettingsFrame>
</template>

<style scoped>
.app-page {
  display: flex;
  flex-direction: column;
  gap: 28px;
  max-width: 720px;
}
.head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 14px;
}
.tile {
  flex: 0 0 52px;
  height: 52px;
  border-radius: 14px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: color-mix(in oklab, var(--t) 18%, transparent);
  color: var(--t);
}
.who {
  flex: 1 1 220px;
  min-width: 0;
}
h2 {
  margin: 0;
  font-weight: 400;
  font-size: 32px;
  letter-spacing: -0.015em;
}
.desc {
  margin: -14px 0 0;
  color: var(--muted);
  line-height: 1.6;
}
section {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
h3 {
  margin: 0;
  font-weight: 400;
}
.none {
  margin: 0;
  padding: 14px 16px;
  border-radius: 12px;
  border: 1px dashed var(--line2);
  color: var(--muted);
  font-size: 13.5px;
  line-height: 1.55;
}
.facts {
  margin: 0;
  display: grid;
  grid-template-columns: 120px minmax(0, 1fr);
  gap: 12px 16px;
  font-size: 13.5px;
}
dt {
  color: var(--faint);
}
dd {
  margin: 0;
}
.chips {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
}
.chip {
  padding: 2px 9px;
  border-radius: 999px;
  border: 1px solid var(--line2);
  font-size: 12.5px;
}
.muted {
  color: var(--muted);
}
.faint {
  color: var(--faint);
}
.small {
  font-size: 12.5px;
}
.keys {
  display: flex;
  flex-wrap: wrap;
  gap: 8px 20px;
  font-size: 13px;
  color: var(--muted);
}
@media (max-width: 720px) {
  .facts {
    grid-template-columns: minmax(0, 1fr);
    gap: 4px;
  }
  dd {
    margin-bottom: 10px;
  }
}
</style>
