<script setup lang="ts">
import type { HavreApp, SystemInfo } from '~/composables/usePlatform'

// The App library: every app's manifest (what it does, what it shares
// with the others), and which ones sit in the dock, in which order.
useHead({ title: 'Apps · Havre' })
const toast = useToasts()
const { apps, loaded, refresh } = useApps()
onMounted(() => refresh().catch(() => {}))
const health = useFetch<{ demo: boolean }>('/api/health', { server: false, lazy: true })
const demo = computed(() => !!health.data.value?.demo)

const busy = ref<string | null>(null)

async function toggle(app: HavreApp) {
  busy.value = app.id
  try {
    await setAppEnabled(app.id, !app.enabled)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    busy.value = null
  }
}

async function move(app: HavreApp, by: -1 | 1) {
  const ids = apps.value.map((a) => a.id)
  const i = ids.indexOf(app.id)
  const j = i + by
  if (j < 0 || j >= ids.length) return
  ;[ids[i], ids[j]] = [ids[j]!, ids[i]!]
  busy.value = app.id
  try {
    await reorderApps(ids)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    busy.value = null
  }
}

// What each app shares, its colour and its one-line detail: shared with
// Settings › Your apps (usePlatform.ts).
const partners = (app: HavreApp) => appPartners(app, apps.value)
const kinds = appKinds
const tint = appTint
const CAPABILITIES = APP_CAPABILITIES

const installed = computed(() => apps.value.filter((a) => a.enabled))
const available = computed(() => apps.value.filter((a) => !a.enabled))

// The status card and each app's line ("IMAP · SMTP · 2 accounts").
const system = ref<SystemInfo | null>(null)
onMounted(async () => {
  system.value = await fetchSystem().catch(() => null)
})
const host = import.meta.client ? window.location.host : ''
const accounts = useAccounts()
const troubled = computed(() => accounts.data.value.filter((a) => a.status.state === 'error'))
/** The status card's Backup line, and whether it's a worry. */
const backupLine = computed(() => {
  const b = system.value?.backup
  if (!b) return null
  if (b.last_error) return { text: 'Failed', bad: true }
  if (!b.last_at) return { text: b.enabled && b.next_at ? `Not yet · ${whenShort(b.next_at)}` : 'Never', bad: true }
  const old = Date.now() / 1000 - b.last_at > 2 * 86_400
  return { text: whenShort(b.last_at), bad: old }
})
const detail = (app: HavreApp) => appDetail(app, system.value)
</script>

<template>
  <main class="library">
    <div class="wrap">
      <header class="top">
        <div class="intro">
          <div class="mono faint host">Havre<template v-if="host"> · {{ host }}</template></div>
          <h1 class="serif">Your apps</h1>
          <p class="lead">
            Every app runs on your server and shares the same people, events and files — so they work together from the
            moment you add them.
          </p>
        </div>
        <div class="status">
          <div class="state">
            <span class="dot" :class="{ bad: troubled.length || backupLine?.bad }" aria-hidden="true" />
            {{
              troubled.length
                ? `${troubled.length} account${troubled.length > 1 ? 's' : ''} can't sync`
                : backupLine?.bad
                  ? 'Backups need a look'
                  : 'All systems running'
            }}
          </div>
          <dl v-if="system">
            <dt>Version</dt>
            <dd class="mono">{{ system.version }}</dd>
            <template v-if="system.memory">
              <dt>Memory</dt>
              <dd class="mono">{{ bytes(system.memory) }}</dd>
            </template>
            <template v-if="system.data">
              <dt>Data</dt>
              <dd class="mono">{{ bytes(system.data) }}</dd>
            </template>
            <template v-if="backupLine">
              <dt>Backup</dt>
              <dd class="mono" :class="{ bad: backupLine.bad }">
                <NuxtLink to="/settings/backups" class="plain">{{ backupLine.text }}</NuxtLink>
              </dd>
            </template>
          </dl>
        </div>
      </header>

      <p v-if="demo" class="faint small">Demo mode: the dock can't be changed.</p>

      <section aria-labelledby="installed" class="sec">
        <div class="sec-head">
          <h2 id="installed">Installed</h2>
          <span class="mono faint small">{{ installed.length }}</span>
        </div>
        <ol v-if="loaded" class="grid">
          <li v-for="app in installed" :key="app.id" class="card">
            <div class="row">
              <span class="tile" :style="{ '--t': tint(app) }" aria-hidden="true"><HIcon :name="app.icon" :size="22" /></span>
              <div class="grow">
                <div class="name">{{ app.name }}</div>
                <div class="faint sub">{{ detail(app) }}</div>
              </div>
              <NuxtLink :to="app.route" class="open" :aria-label="`Open ${app.name}`">Open</NuxtLink>
            </div>
            <p class="desc">{{ app.description }}</p>
            <div v-if="kinds(app).length" class="chips" :title="app.provides.map((p) => CAPABILITIES[p] ?? p).join(', ')">
              <span class="faint">Shares</span>
              <span v-for="k in kinds(app)" :key="k" class="chip">{{ k }}</span>
            </div>
            <p v-if="partners(app).length" class="faint small">Works with {{ partners(app).join(' and ') }}.</p>
            <div class="controls">
              <button
                type="button"
                class="icon-btn small"
                :aria-label="`Move ${app.name} up`"
                title="Earlier in the dock"
                :disabled="apps.indexOf(app) === 0 || !!busy || demo"
                @click="move(app, -1)"
              >
                <HIcon name="back" :size="14" />
              </button>
              <button
                type="button"
                class="icon-btn small"
                :aria-label="`Move ${app.name} down`"
                title="Later in the dock"
                :disabled="apps.indexOf(app) === apps.length - 1 || !!busy || demo"
                @click="move(app, 1)"
              >
                <HIcon name="back" :size="14" class="flip" />
              </button>
              <span class="grow" />
              <NuxtLink :to="appSettingsPath(app)" class="settings-link" :aria-label="`${app.name} settings`">Settings</NuxtLink>
              <button type="button" class="link-btn" :disabled="busy === app.id || demo" @click="toggle(app)">
                Remove from dock
              </button>
            </div>
          </li>
        </ol>
      </section>

      <section id="available" aria-labelledby="available-title" class="sec">
        <div class="sec-head">
          <h2 id="available-title">Add to your dock</h2>
          <span class="faint small">One click — no containers, no config files.</span>
        </div>
        <ol class="grid small-grid">
          <li v-for="app in available" :key="app.id" class="card plain">
            <span class="tile sm" :style="{ '--t': tint(app) }" aria-hidden="true"><HIcon :name="app.icon" :size="20" /></span>
            <div>
              <div class="name sm">{{ app.name }}</div>
              <div class="desc">{{ app.description }}</div>
            </div>
            <div class="foot">
              <span class="faint small">{{ kinds(app).join(' · ') }}</span>
              <button type="button" class="add" :aria-label="`Add ${app.name} to your dock`" :disabled="busy === app.id || demo" @click="toggle(app)">Add</button>
            </div>
          </li>
          <li class="card plain dashed">
            <span class="tile sm empty" aria-hidden="true"><HIcon name="plus" :size="18" /></span>
            <div>
              <div class="name sm">Build your own</div>
              <div class="desc">
                A manifest, a Rust module, a Nuxt page. Hidden apps keep working in the background: Mail still answers
                invitations with the Agenda hidden.
              </div>
            </div>
            <div class="foot">
              <span class="faint small">See “Adding an app” in docs/development.md</span>
            </div>
          </li>
        </ol>
      </section>
    </div>
  </main>
</template>

<style scoped>
.library {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  background: var(--panel);
}
.wrap {
  max-width: 1080px;
  margin: 0 auto;
  padding: 56px 40px 64px;
  display: flex;
  flex-direction: column;
  gap: 48px;
}
.top {
  display: flex;
  flex-wrap: wrap;
  gap: 32px;
  align-items: flex-end;
  justify-content: space-between;
}
.intro {
  flex: 1 1 420px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.host {
  font-size: 12px;
}
h1 {
  margin: 0;
  font-size: 48px;
  line-height: 1.05;
  letter-spacing: -0.02em;
}
.lead {
  margin: 0;
  max-width: 520px;
  font-size: 15.5px;
  line-height: 1.6;
  color: var(--muted);
}
.faint {
  color: var(--faint);
}
.small {
  margin: 0;
  font-size: 12.5px;
}
.status {
  flex: 0 1 320px;
  padding: 18px 20px;
  border-radius: 16px;
  border: 1px solid var(--line);
  background: var(--bg);
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.state {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13.5px;
  font-weight: 500;
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--ok);
}
.dot.bad {
  background: var(--danger);
}
dl {
  margin: 0;
  display: grid;
  grid-template-columns: auto 1fr;
  gap: 6px 16px;
  font-size: 13px;
}
dt {
  color: var(--faint);
}
dd {
  margin: 0;
  font-size: 12.5px;
}
dd.bad {
  color: var(--danger);
}
.plain {
  color: inherit;
  text-decoration: none;
}
.plain:hover {
  text-decoration: underline;
}
.sec {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.sec-head {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 10px;
}
h2 {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
}
.grid {
  margin: 0;
  padding: 0;
  list-style: none;
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
  gap: 16px;
}
.small-grid {
  grid-template-columns: repeat(auto-fill, minmax(230px, 1fr));
}
.card {
  padding: 20px;
  border-radius: 16px;
  border: 1px solid var(--line);
  background: var(--raise);
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.card.plain {
  padding: 18px;
  gap: 12px;
  background: transparent;
}
.card.dashed {
  border-style: dashed;
  border-color: var(--line2);
}
.row {
  display: flex;
  align-items: center;
  gap: 12px;
}
.grow {
  flex: 1;
  min-width: 0;
}
.tile {
  flex: 0 0 44px;
  width: 44px;
  height: 44px;
  border-radius: 12px;
  display: grid;
  place-items: center;
  background: color-mix(in oklab, var(--t) 18%, transparent);
  color: var(--t);
}
.tile.sm {
  width: 40px;
  height: 40px;
  flex-basis: 40px;
  border-radius: 11px;
}
.tile.empty {
  background: transparent;
  border: 1px dashed var(--line2);
  color: var(--faint);
}
.name {
  font-size: 16px;
  font-weight: 600;
}
.name.sm {
  font-size: 15px;
}
.sub {
  font-size: 12.5px;
}
.open {
  height: 32px;
  padding: 0 12px;
  border: 1px solid var(--line2);
  border-radius: 8px;
  display: flex;
  align-items: center;
  color: var(--text);
  text-decoration: none;
  font-size: 13px;
}
.open:hover {
  background: var(--hover);
}
.desc {
  margin: 2px 0 0;
  font-size: 13px;
  line-height: 1.5;
  color: var(--muted);
}
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  font-size: 12px;
  color: var(--muted);
}
.chip {
  padding: 2px 8px;
  border-radius: 999px;
  border: 1px solid var(--line2);
}
.controls {
  display: flex;
  align-items: center;
  gap: 2px;
  margin-top: auto;
  padding-top: 10px;
  border-top: 1px solid var(--line);
}
.settings-link {
  margin-right: 12px;
  font-size: 12.5px;
  color: var(--muted);
  text-decoration: none;
}
.settings-link:hover {
  color: var(--text);
}
.icon-btn.small {
  width: 28px;
  height: 28px;
}
.flip {
  transform: scaleX(-1);
}
.link-btn {
  border: 0;
  background: none;
  padding: 0;
  color: var(--faint);
  font: inherit;
  font-size: 12.5px;
  cursor: pointer;
}
.link-btn:hover:not(:disabled) {
  color: var(--danger);
}
.foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-top: auto;
}
.add {
  height: 30px;
  padding: 0 12px;
  border: 0;
  border-radius: 8px;
  background: var(--accent);
  color: var(--on-accent);
  font: inherit;
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
}
.add:hover {
  filter: brightness(1.08);
}
button:disabled {
  opacity: 0.5;
  cursor: default;
}
@media (max-width: 720px) {
  .wrap {
    padding: 28px 16px 48px;
    gap: 32px;
  }
  h1 {
    font-size: 38px;
  }
}
</style>
