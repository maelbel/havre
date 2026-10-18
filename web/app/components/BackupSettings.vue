<script setup lang="ts">
import type { BackupSettings, BackupStatus } from '~/composables/usePlatform'

// Settings › Backups: the nightly backup, the backups kept, the recovery
// key, how to restore, and exporting the Havre calendar.
const toast = useToasts()
const status = ref<BackupStatus | null>(null)
const error = ref<string | null>(null)
const form = reactive<BackupSettings>({ enabled: true, time: '03:00', keep: 7 })
const running = ref(false)
const confirmDelete = ref<string | null>(null)

async function load() {
  try {
    status.value = await fetchBackups()
    Object.assign(form, status.value.settings)
    error.value = null
  } catch (err) {
    error.value = apiErrorMessage(err)
  }
}
onMounted(load)

async function save() {
  try {
    await saveBackupSettings({ ...form, keep: Number(form.keep) })
    await load()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
    await load()
  }
}

async function runNow() {
  running.value = true
  try {
    const file = await backupNow()
    toast.show(`Backup made (${bytes(file.size)})`)
    await load()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
    await load()
  } finally {
    running.value = false
  }
}

async function remove(name: string) {
  if (confirmDelete.value !== name) {
    confirmDelete.value = name
    return
  }
  confirmDelete.value = null
  try {
    await deleteBackup(name)
    await load()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

/** Old (more than 2 days) or failed: say so. */
const health = computed(() => {
  const s = status.value
  if (!s) return null
  if (s.last_error) return { tone: 'bad', text: `The last backup failed: ${s.last_error}` }
  if (!s.last_at) return { tone: 'warn', text: 'No backup yet.' }
  const age = Date.now() / 1000 - s.last_at
  if (age > 2 * 86_400) return { tone: 'warn', text: `Last backup ${whenShort(s.last_at)}: more than two days ago.` }
  return { tone: 'ok', text: `Last backup ${whenShort(s.last_at)}` }
})

const calendars = useCalendars()
const copy = async (text: string) => {
  await navigator.clipboard?.writeText(text).catch(() => {})
  toast.show('Copied')
}
const restoreDocker = 'docker compose stop havre\ndocker compose run --rm havre havre restore /srv/havre/data/backups/<file>\ndocker compose start havre'
</script>

<template>
  <div class="backups">
    <header>
      <h2 class="serif">Backups</h2>
      <p class="lead">
        Every night, a copy of Havre's database: accounts, labels, drafts, the Havre calendar, settings. Your mail
        itself stays on your mail servers, so it isn't copied: Havre downloads it again after a restore.
      </p>
    </header>

    <p v-if="error" class="error"><HIcon name="alert" :size="14" /> {{ error }}</p>

    <template v-if="status">
      <section class="state" :class="health?.tone" aria-live="polite">
        <span class="dot" aria-hidden="true" />
        <div class="grow">
          <div class="strong">{{ health?.text }}</div>
          <div class="muted small">
            <template v-if="status.backups[0]">{{ bytes(status.backups[0].size) }} · </template>
            <template v-if="status.next_at">next {{ whenShort(status.next_at) }}</template>
            <template v-else>nightly backups are off</template>
          </div>
        </div>
        <button type="button" class="btn" :disabled="running" @click="runNow">
          {{ running ? 'Backing up…' : 'Back up now' }}
        </button>
      </section>

      <fieldset>
        <legend class="eyebrow">Schedule</legend>
        <label class="row">
          <input v-model="form.enabled" type="checkbox" @change="save" />
          <span class="grow"><span class="strong">Every night at</span></span>
          <input v-model="form.time" type="time" class="time mono" aria-label="Backup time" @change="save" />
        </label>
        <label class="row">
          <span class="grow"><span class="strong">Keep the last</span><span class="muted"> — older ones are deleted</span></span>
          <input v-model.number="form.keep" type="number" min="1" max="365" class="num mono" aria-label="Backups to keep" @change="save" />
        </label>
        <p class="note">
          The server's own time zone (with Docker, set <code>TZ</code>, e.g. <code>TZ=Europe/Paris</code>).
        </p>
      </fieldset>

      <section class="block">
        <div class="eyebrow">Kept here</div>
        <p class="path mono">{{ status.dir }}</p>
        <p class="note">
          On the same disk as Havre: a dead disk takes them along. Copy this folder elsewhere too — a NAS, another
          disk, your cloud — or set <code>HAVRE_BACKUP_DIR</code> to such a place (with Docker, mount it at
          <code>/srv/havre/data/backups</code>).
        </p>
        <ol v-if="status.backups.length" class="files">
          <li v-for="f in status.backups" :key="f.name">
            <HIcon name="archive" :size="15" />
            <span class="grow">
              <span class="fname">{{ whenShort(f.created_at) }}</span>
              <span class="mono faint tiny">{{ f.name }}</span>
            </span>
            <span class="mono faint tiny">{{ bytes(f.size) }}</span>
            <a class="mini" :href="`/api/backups/${encodeURIComponent(f.name)}`" :download="f.name">Download</a>
            <button type="button" class="mini del" :class="{ armed: confirmDelete === f.name }" @click="remove(f.name)">
              {{ confirmDelete === f.name ? 'Delete?' : 'Delete' }}
            </button>
          </li>
        </ol>
        <p v-else class="note">None yet: <button type="button" class="link" @click="runNow">make the first one</button>.</p>
      </section>

      <section class="block">
        <div class="eyebrow">Recovery key</div>
        <p class="note">
          Mail passwords and sign-ins are encrypted in the database, and backups don't contain the key — so a backup
          that ends up in the wrong hands gives nothing away. Restoring on a <strong>new server</strong> needs it:
          keep it in your password manager. Its fingerprint (which backups it opens):
          <code class="mono">{{ status.key_fingerprint }}</code>
        </p>
        <p class="note">Show it, on the server:</p>
        <div class="code">
          <code>docker compose exec havre havre key</code>
          <button type="button" class="mini" @click="copy('docker compose exec havre havre key')">Copy</button>
        </div>
        <p class="note faint">Without Docker: <code>havre key</code>, or the <code>secret.key</code> file next to the database.</p>
      </section>

      <section class="block">
        <div class="eyebrow">Restore</div>
        <p class="note">
          Havre must be stopped. The current database is kept next to it (<code>havre.db.before-restore-…</code>).
          On a new server, add <code>--key</code> and the recovery key.
        </p>
        <div class="code">
          <pre>{{ restoreDocker }}</pre>
          <button type="button" class="mini" @click="copy(restoreDocker)">Copy</button>
        </div>
      </section>

      <section v-if="calendars.data.value.length" class="block">
        <div class="eyebrow">Export</div>
        <p class="note">Any calendar as an <code>.ics</code> file, for another calendar app. The Havre calendar lives only here.</p>
        <ul class="exports">
          <li v-for="c in calendars.data.value" :key="c.id">
            <span class="cdot" :style="{ background: c.color }" aria-hidden="true" />
            <span class="grow">{{ c.name }} <span class="faint">· {{ SOURCE_LABELS[c.source] }}</span></span>
            <a class="mini" :href="`/api/agenda/calendars/${c.id}/export.ics`" download>Export .ics</a>
          </li>
        </ul>
      </section>
    </template>
  </div>
</template>

<style scoped>
.backups {
  display: flex;
  flex-direction: column;
  gap: 32px;
}
h2 {
  margin: 0 0 6px;
  font-weight: 400;
  font-size: 32px;
  letter-spacing: -0.015em;
}
.lead {
  margin: 0;
  color: var(--muted);
  line-height: 1.6;
}
.state {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 16px 18px;
  border-radius: 14px;
  border: 1px solid var(--line);
  background: var(--bg);
}
.dot {
  flex: 0 0 9px;
  height: 9px;
  border-radius: 50%;
  background: var(--ok);
}
.state.warn .dot {
  background: var(--accent);
}
.state.bad .dot {
  background: var(--danger);
}
.state.bad .strong {
  color: var(--danger);
}
.grow {
  flex: 1;
  min-width: 0;
}
.strong {
  font-weight: 500;
}
.muted {
  color: var(--muted);
}
.faint {
  color: var(--faint);
}
.small {
  font-size: 13px;
}
.tiny {
  font-size: 11.5px;
}
fieldset {
  margin: 0;
  padding: 0;
  border: 0;
  display: flex;
  flex-direction: column;
}
legend {
  padding: 0;
  margin-bottom: 10px;
}
.row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 10px;
  margin: 0 -10px;
  border-radius: 10px;
  cursor: pointer;
}
.row:hover {
  background: var(--hover);
}
.row input[type='checkbox'] {
  width: 16px;
  height: 16px;
  margin: 0;
  accent-color: var(--accent);
}
.time,
.num {
  height: 30px;
  padding: 0 8px;
  border-radius: 7px;
  border: 1px solid var(--line2);
  background: var(--bg);
  color: var(--text);
  font-size: 12.5px;
  color-scheme: inherit;
}
.num {
  width: 64px;
}
.block {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.note {
  margin: 0;
  font-size: 13.5px;
  line-height: 1.6;
  color: var(--muted);
}
.path {
  margin: 0;
  padding: 8px 12px;
  border-radius: 8px;
  background: var(--bg);
  border: 1px solid var(--line);
  font-size: 12.5px;
  overflow-wrap: anywhere;
}
code {
  font-family: var(--font-mono);
  font-size: 12px;
  padding: 1px 5px;
  border-radius: 4px;
  background: var(--bg);
}
.files,
.exports {
  list-style: none;
  margin: 4px 0 0;
  padding: 0;
}
.files li,
.exports li {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 0;
  border-top: 1px solid var(--line);
  font-size: 13.5px;
}
.files .grow {
  display: flex;
  flex-direction: column;
}
.fname {
  font-weight: 500;
}
.mini {
  display: inline-flex;
  align-items: center;
  height: 28px;
  padding: 0 10px;
  border: 1px solid var(--line2);
  border-radius: 7px;
  background: transparent;
  color: var(--text);
  font: inherit;
  font-size: 12.5px;
  text-decoration: none;
  cursor: pointer;
  white-space: nowrap;
}
.mini:hover {
  background: var(--hover);
}
.del.armed {
  border-color: var(--danger);
  color: var(--danger);
}
.code {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 10px 12px;
  border-radius: 10px;
  border: 1px solid var(--line);
  background: var(--bg);
}
.code code,
.code pre {
  flex: 1;
  margin: 0;
  padding: 0;
  background: none;
  font-family: var(--font-mono);
  font-size: 12.5px;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
.cdot {
  flex: 0 0 9px;
  height: 9px;
  border-radius: 50%;
}
.link {
  border: 0;
  background: none;
  padding: 0;
  color: var(--accent);
  font: inherit;
  cursor: pointer;
}
.error {
  display: flex;
  align-items: center;
  gap: 8px;
  color: var(--danger);
}
</style>
