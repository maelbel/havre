<script setup lang="ts">
import type { SplitPaneSpec } from '~/composables/useSplit'
import type { FileEmail, FileItem, FileList, FilesProgress } from '~/composables/useFiles'

// Files: every attachment in your mail. Views, kinds and people on the left,
// the files in the middle (newest first, grouped by month), the open file on
// the right. `?view=received|sent`, `?kind=…`, `?person=…` filter the list;
// `?file=<key>` opens one. Files stay on the mail server: downloading one
// goes through Mail.
useHead({ title: 'Files · Havre' })
const route = useRoute()
const router = useRouter()
const toast = useToasts()

const view = computed(() => (route.query.view === 'received' || route.query.view === 'sent' ? route.query.view : 'all'))
const kind = computed(() => (route.query.kind as string | undefined) ?? '')
const person = computed(() => (route.query.person as string | undefined)?.toLowerCase() ?? '')
const selectedKey = computed(() => (route.query.file as string | undefined) ?? null)
function go(change: Record<string, string | undefined>) {
  const query = { ...route.query, ...change } as Record<string, string | undefined>
  router.replace({ query: Object.fromEntries(Object.entries(query).filter(([, v]) => v)) as Record<string, string> })
}
const pickView = (v: string) => go({ view: v === 'all' ? undefined : v, person: undefined })
const pickKind = (k: string) => go({ kind: kind.value === k ? undefined : k })
const pickPerson = (p: string) => go({ person: person.value === p ? undefined : p, view: undefined })
const openFile = (key: string) => go({ file: key })
const closeFile = () => go({ file: undefined })

// ---- The list (filtered by the server) ----------------------------------------------
const list = ref<FileList | null>(null)
const loading = ref(true)
const search = ref('')
async function load() {
  try {
    list.value = await fetchFiles({
      q: search.value.trim() || undefined,
      kind: kind.value || undefined,
      person: person.value || undefined,
      sent: view.value === 'all' ? undefined : view.value === 'sent',
    })
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    loading.value = false
  }
}
let searchTimer: ReturnType<typeof setTimeout> | undefined
watch(search, () => {
  clearTimeout(searchTimer)
  searchTimer = setTimeout(load, 200)
})
watch([view, kind, person], load)
onMounted(load)

// How far Havre has looked through older mail (refreshed now and then).
const progress = ref<FilesProgress | null>(null)
async function loadProgress() {
  progress.value = await fetchFilesProgress().catch(() => null)
}
let progressTimer: ReturnType<typeof setInterval> | undefined
onMounted(() => {
  loadProgress()
  progressTimer = setInterval(() => {
    const before = progress.value?.looked
    loadProgress().then(() => {
      // New files found since: show them.
      if (before != null && progress.value && progress.value.looked !== before) load()
    })
  }, 30_000)
})
onBeforeUnmount(() => clearInterval(progressTimer))
const progressShare = computed(() =>
  progress.value && progress.value.total ? Math.round((progress.value.looked / progress.value.total) * 100) : 100,
)

const files = computed(() => list.value?.files ?? [])
const counts = computed(() => list.value?.counts)
const heading = computed(() => {
  const who = person.value && list.value?.people.find((p) => p.email === person.value)
  if (who) return `From ${who.name || who.email}`
  const v = view.value === 'sent' ? 'Sent' : view.value === 'received' ? 'Received' : 'All files'
  const k = FILE_KINDS.find((x) => x.id === kind.value)
  return k ? `${v} · ${k.label}` : v
})
const kindCount = (ids: string[]) => ids.reduce((n, id) => n + (counts.value?.kinds[id as keyof typeof counts.value.kinds] ?? 0), 0)

/** "This week", "Earlier in October", "September", "September 2025". */
function groupOf(iso: string | null) {
  if (!iso) return 'No date'
  const d = new Date(iso)
  const now = new Date()
  if (now.getTime() - d.getTime() < 7 * 86_400_000) return 'This week'
  const sameYear = d.getFullYear() === now.getFullYear()
  const month = d.toLocaleDateString([], sameYear ? { month: 'long' } : { month: 'long', year: 'numeric' })
  return sameYear && d.getMonth() === now.getMonth() ? `Earlier in ${month}` : month
}
const groups = computed(() => {
  const out: { label: string; files: FileItem[] }[] = []
  for (const f of files.value) {
    const label = groupOf(f.date)
    const last = out[out.length - 1]
    if (last?.label === label) last.files.push(f)
    else out.push({ label, files: [f] })
  }
  return out
})
const who = (e: FileEmail) => (e.sent ? 'You sent' : e.from || 'Unknown sender')

// ---- The open file -------------------------------------------------------------------
const selected = computed(() => files.value.find((f) => f.key === selectedKey.value) ?? null)
const fullDate = (iso: string | null) =>
  iso
    ? new Date(iso).toLocaleString([], { weekday: 'short', day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })
    : ''

// ---- Keys: / search, D download, O open the email, Esc close ----------------------------
const searchInput = ref<{ inputRef: HTMLInputElement | null } | null>(null)
function onKey(e: KeyboardEvent) {
  const typing = (e.target as HTMLElement)?.closest?.('input, textarea, select, [contenteditable]')
  if (typing || e.ctrlKey || e.metaKey || e.altKey) return
  if (e.key === '/') {
    e.preventDefault()
    searchInput.value?.inputRef?.focus()
  } else if (e.key === 'd' && selected.value) {
    e.preventDefault()
    window.location.assign(fileUrl(selected.value))
  } else if (e.key === 'o' && selected.value) {
    e.preventDefault()
    router.push(messageLink(selected.value))
  } else if (e.key === 'Escape' && selected.value) {
    closeFile()
  }
}
onMounted(() => document.addEventListener('keydown', onKey))
onBeforeUnmount(() => document.removeEventListener('keydown', onKey))

// Resizable panes (wide screens): views · list · file.
const splitPanes: SplitPaneSpec[] = [
  { id: 'views', size: 232, min: 200, max: 320, collapsible: true, handle: { label: 'Resize the views', fold: true, showLabel: 'Show views' } },
  { id: 'list', min: 380, handle: { label: 'Resize the file' } },
  { id: 'file', size: 380, min: 320, max: 640 },
]
</script>

<template>
  <div class="files-app" :class="{ 'has-file': selectedKey != null }">
    <SplitView id="files" :panes="splitPanes">
    <template #views>
    <aside class="side" aria-label="Files views">
      <div class="side-head">
        <h1 class="serif">Files</h1>
        <span class="mono faint tiny">from your mail</span>
      </div>
      <nav class="views" aria-label="Views">
        <button
          v-for="v in [
            { id: 'all', label: 'All files', n: counts?.all },
            { id: 'received', label: 'Received', n: counts?.received },
            { id: 'sent', label: 'Sent', n: counts?.sent },
          ]"
          :key="v.id"
          type="button"
          class="row"
          :aria-current="view === v.id && !person ? 'page' : undefined"
          @click="pickView(v.id)"
        >
          <span class="name">{{ v.label }}</span><span class="count mono">{{ v.n ?? '' }}</span>
        </button>
      </nav>
      <div class="section eyebrow">Kinds</div>
      <nav class="views kinds" aria-label="Kinds">
        <template v-for="k in FILE_KINDS" :key="k.id">
          <button
            v-if="kindCount(k.kinds) || kind === k.id"
            type="button"
            class="row"
            :aria-pressed="kind === k.id"
            @click="pickKind(k.id)"
          >
            <span class="dot" :style="{ background: fileTint(k.kinds[0]!) }" aria-hidden="true" />
            <span class="name">{{ k.label }}</span><span class="count mono">{{ kindCount(k.kinds) }}</span>
          </button>
        </template>
      </nav>
      <template v-if="list?.people.length">
        <div class="section eyebrow">People</div>
        <nav class="views people" aria-label="People">
          <button
            v-for="p in list.people"
            :key="p.email"
            type="button"
            class="row"
            :aria-current="person === p.email ? 'page' : undefined"
            :title="p.email"
            @click="pickPerson(p.email)"
          >
            <span class="avatar" :style="{ '--c': personColor(p.name || p.email) }" aria-hidden="true">{{ initials(p.name || p.email) }}</span>
            <span class="name">{{ p.name || p.email }}</span><span class="count mono">{{ p.files }}</span>
          </button>
        </nav>
      </template>
      <span class="grow" />
      <section v-if="progress" class="looked" aria-label="Looked through">
        <div class="looked-top">
          <span class="eyebrow">Looked through</span>
          <span class="mono">{{ progress.looked.toLocaleString() }} / {{ progress.total.toLocaleString() }}</span>
        </div>
        <div class="meter" role="progressbar" :aria-valuenow="progressShare" aria-valuemin="0" aria-valuemax="100" aria-label="Emails looked through">
          <span :style="{ width: `${progressShare}%` }" />
        </div>
        <span v-if="progress.looked < progress.total" class="small muted">
          Inbox and Sent, newest first. A little more every few minutes; older files appear as they're found.
        </span>
        <span v-else class="small muted">Done: every file in {{ progress.depth ? `the newest ${progress.depth.toLocaleString()} emails` : 'your mail' }}.</span>
        <NuxtLink to="/settings/apps/files" class="small">How far back →</NuxtLink>
      </section>
    </aside>
    </template>

    <template #list>
    <section class="list" aria-label="Files">
      <div class="find">
        <UInput
          ref="searchInput"
          v-model="search"
          type="search"
          icon="i-havre-search"
          size="lg"
          placeholder="Search files by name, sender or email"
          aria-label="Search files"
          class="w-full"
        >
          <template #trailing><UKbd value="/" /></template>
        </UInput>
        <div class="list-head">
          <h2>{{ heading }}</h2>
          <span class="faint small">
            <template v-if="list && list.total > files.length">{{ files.length }} of {{ list.total }} · </template>
            Same file in several emails: shown once
          </span>
        </div>
      </div>
      <div class="scroll">
        <p v-if="loading" class="empty">Loading…</p>
        <p v-else-if="!files.length && !search && !kind && !person && view === 'all'" class="empty">
          No files yet. Havre lists the attachments of the mail it has downloaded; it's looking through older mail
          now.
        </p>
        <p v-else-if="!files.length" class="empty">No file matches.</p>
        <template v-for="g in groups" :key="g.label">
          <div class="group eyebrow">{{ g.label }}</div>
          <button
            v-for="f in g.files"
            :key="f.key"
            type="button"
            class="item"
            :aria-current="f.key === selectedKey ? 'true' : undefined"
            @click="openFile(f.key)"
          >
            <span class="tile mono" :style="{ '--t': fileTint(f.kind) }" aria-hidden="true">{{ fileExt(f.name) }}</span>
            <span class="body">
              <span class="top">
                <span class="fname">{{ f.name }}</span>
                <UBadge v-if="f.copies.length" color="neutral" variant="outline" size="sm">{{ f.copies.length + 1 }} emails</UBadge>
                <UBadge v-if="f.versions.length" color="neutral" variant="outline" size="sm">{{ f.versions.length + 1 }} versions</UBadge>
              </span>
              <span class="meta">{{ who(f) }} · {{ f.subject || '(no subject)' }}</span>
            </span>
            <span class="side-meta mono">
              <span>{{ shortDate(f.date) }}</span>
              <span>{{ fileSize(f.size) }}</span>
            </span>
          </button>
        </template>
      </div>
    </section>
    </template>

    <template #file>
    <aside class="file" aria-label="File">
      <template v-if="selected">
        <div class="file-bar">
          <UButton
            icon="i-havre-back"
            color="neutral"
            variant="ghost"
            square
            class="back"
            type="button"
            aria-label="Back to the files"
            @click="closeFile"
          />
          <span class="grow" />
          <UButton
            icon="i-havre-close"
            color="neutral"
            variant="ghost"
            square
            type="button"
            aria-label="Close the file"
            @click="closeFile"
          />
        </div>
        <div class="file-body">
          <div class="preview" :style="{ '--t': fileTint(selected.kind) }">
            <img v-if="fileHasPreview(selected)" :src="fileUrl(selected)" :alt="`Preview of ${selected.name}`" loading="lazy" />
            <span v-else class="mono big-ext" aria-hidden="true">{{ fileExt(selected.name) }}</span>
          </div>
          <div>
            <h2 class="serif file-name">{{ selected.name }}</h2>
            <div class="faint small">{{ fileKindName(selected.kind) }} · {{ fileSize(selected.size) }}</div>
          </div>
          <div class="actions">
            <UButton icon="i-havre-download" :href="fileUrl(selected)" :download="selected.name">
              Download
            </UButton>
            <UButton color="neutral" variant="outline" :to="messageLink(selected)">Open the email</UButton>
          </div>
          <dl class="facts">
            <dt>{{ selected.sent ? 'Sent' : 'From' }}</dt>
            <dd>{{ selected.sent ? 'You' : selected.from || 'Unknown sender' }}</dd>
            <dt>In</dt>
            <dd><NuxtLink :to="messageLink(selected)">{{ selected.subject || '(no subject)' }}</NuxtLink></dd>
            <dt>Date</dt>
            <dd>{{ fullDate(selected.date) }}</dd>
          </dl>
          <section v-if="selected.versions.length" class="more" aria-label="Other versions">
            <div class="eyebrow">Other versions</div>
            <button v-for="v in selected.versions" :key="v.key" type="button" class="mrow" @click="openFile(v.key)">
              <span class="name">{{ v.name }}</span>
              <span class="mono faint small">{{ shortDate(v.date) }} · {{ fileSize(v.size) }}</span>
            </button>
          </section>
          <section v-if="selected.copies.length" class="more" aria-label="Also in">
            <div class="eyebrow">Also in</div>
            <NuxtLink v-for="c in selected.copies" :key="`${c.account}-${c.folder}-${c.uid}`" class="mrow" :to="messageLink(c)">
              <span class="name">{{ c.subject || '(no subject)' }} — {{ who(c) }}</span>
              <span class="mono faint small">{{ shortDate(c.date) }}</span>
            </NuxtLink>
          </section>
          <p class="faint small note">
            Files stay in your mail: Havre lists them and fetches one when you open or download it — no second copy
            on the server.
          </p>
        </div>
      </template>
      <div v-else-if="selectedKey" class="none">
        <p>This file isn't in the list any more.</p>
        <UButton color="neutral" variant="outline" type="button" @click="closeFile">
          Back to the files
        </UButton>
      </div>
      <div v-else class="none">
        <HIcon name="folder" :size="28" />
        <p>Pick a file to see where it came from, its other versions, and to download it.</p>
      </div>
    </aside>
    </template>
    </SplitView>
  </div>
</template>

<style scoped>
.files-app {
  flex: 1;
  display: flex;
  min-width: 0;
}
.side {
  flex: 0 0 232px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 20px 12px;
  border-right: 1px solid var(--line);
  overflow-y: auto;
}
.side-head {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 0 4px 14px 8px;
}
h1 {
  margin: 0;
  flex: 1;
  font-size: 28px;
  font-weight: 400;
}
.tiny {
  font-size: 11px;
}
.views {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.row {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  height: 34px;
  padding: 0 10px;
  border: 0;
  border-radius: 9px;
  background: transparent;
  color: var(--muted);
  text-align: left;
  cursor: pointer;
  font-size: 13.5px;
}
.row:hover {
  background: var(--hover);
  color: var(--text);
}
.row[aria-current='page'],
.row[aria-pressed='true'] {
  background: var(--sel);
  color: var(--text);
}
.name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.count {
  font-size: 11.5px;
  color: var(--faint);
}
.dot {
  flex: none;
  width: 8px;
  height: 8px;
  border-radius: 2px;
}
.section {
  padding: 18px 10px 6px;
}
.avatar {
  flex: none;
  width: 22px;
  height: 22px;
  border-radius: 50%;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 9px;
  font-weight: 600;
  background: color-mix(in oklab, var(--c) 22%, transparent);
  color: var(--c);
}
.grow {
  flex: 1;
  min-height: 16px;
}
.looked {
  margin: 0 4px;
  padding: 12px;
  border-radius: 12px;
  border: 1px solid var(--line);
  background: var(--panel);
  display: flex;
  flex-direction: column;
  gap: 8px;
  line-height: 1.5;
}
.looked-top {
  display: flex;
  justify-content: space-between;
  font-size: 12px;
}
.meter {
  height: 4px;
  border-radius: 2px;
  background: var(--line2);
  overflow: hidden;
}
.meter span {
  display: block;
  height: 100%;
  background: var(--accent);
}
.small {
  font-size: 12px;
}
.muted {
  color: var(--muted);
}
.faint {
  color: var(--faint);
}

/* The list */
.list {
  flex: 1 1 520px;
  min-width: 0;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--line);
  background: var(--panel);
}
.find {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px 20px 12px;
  border-bottom: 1px solid var(--line);
}
.list-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
}
h2 {
  margin: 0;
  font-size: 15px;
  font-weight: 500;
}
.scroll {
  flex: 1;
  overflow-y: auto;
  padding: 4px 12px 16px;
}
.group {
  padding: 14px 8px 6px;
}
.empty {
  padding: 24px 12px;
  color: var(--faint);
  font-size: 13px;
  line-height: 1.55;
}
.item {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
  padding: 10px 8px;
  border: 0;
  border-radius: 10px;
  background: transparent;
  color: var(--text);
  text-align: left;
  cursor: pointer;
}
.item:hover {
  background: var(--hover);
}
.item[aria-current='true'] {
  background: var(--sel);
}
.tile {
  flex: 0 0 36px;
  height: 40px;
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.04em;
  background: color-mix(in oklab, var(--t) 16%, transparent);
  color: var(--t);
}
.body {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.top {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.fname {
  min-width: 0;
  font-size: 14px;
  font-weight: 500;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.meta {
  font-size: 12.5px;
  color: var(--muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.side-meta {
  flex: none;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 2px;
  font-size: 11.5px;
  color: var(--faint);
}

/* The file */
.file {
  flex: 0 1 380px;
  min-width: 300px;
  display: flex;
  flex-direction: column;
  background: var(--bg);
  overflow-y: auto;
}
.file-bar {
  display: flex;
  align-items: center;
  height: 48px;
  padding: 0 8px;
  flex: none;
}
/* After .icon-btn, so it wins: on wide screens the list is beside the file. */
.back {
  display: none;
}
.file-body {
  display: flex;
  flex-direction: column;
  gap: 18px;
  padding: 0 24px 24px;
}
.preview {
  height: 190px;
  border-radius: 12px;
  border: 1px solid var(--line);
  background: color-mix(in oklab, var(--t) 10%, var(--panel));
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
}
.preview img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.big-ext {
  font-size: 22px;
  font-weight: 600;
  letter-spacing: 0.06em;
  color: var(--t);
}
.file-name {
  margin: 0 0 4px;
  font-size: 22px;
  font-weight: 400;
  line-height: 1.25;
  word-break: break-word;
}
.actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.facts {
  margin: 0;
  display: grid;
  grid-template-columns: 60px minmax(0, 1fr);
  gap: 8px 12px;
  font-size: 13px;
}
dt {
  color: var(--faint);
}
dd {
  margin: 0;
  min-width: 0;
  overflow-wrap: anywhere;
}
.more {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.mrow {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  border-radius: 9px;
  border: 1px solid var(--line);
  background: transparent;
  color: var(--text);
  font-size: 13px;
  text-align: left;
  text-decoration: none;
  cursor: pointer;
}
.mrow:hover {
  background: var(--hover);
}
.note {
  margin: 0;
  line-height: 1.5;
}
.none {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  padding: 24px;
  color: var(--faint);
  text-align: center;
  font-size: 13.5px;
}

/* Smaller screens: the list, or the open file. */
@media (max-width: 1100px) {
  .file {
    display: none;
  }
  .has-file .list {
    display: none;
  }
  .has-file .file {
    display: flex;
    flex: 1;
  }
  .back {
    display: inline-flex;
  }
}

/* Phones: views as chips on top, the list, then a file full screen. */
@media (max-width: 720px) {
  .files-app {
    flex-direction: column;
    min-height: 0;
  }
  .side {
    flex: none;
    flex-direction: row;
    gap: 6px;
    padding: 12px 16px 10px;
    border-right: 0;
    border-bottom: 1px solid var(--line);
    overflow-x: auto;
    align-items: center;
  }
  .side-head {
    padding: 0 6px 0 0;
  }
  .side-head .tiny,
  .section,
  .people,
  .grow,
  .looked {
    display: none;
  }
  h1 {
    font-size: 26px;
  }
  .views {
    flex-direction: row;
    gap: 6px;
  }
  .row {
    width: auto;
    height: 34px;
    padding: 0 12px;
    border: 1px solid var(--line2);
    border-radius: 999px;
    white-space: nowrap;
    flex: none;
  }
  .row[aria-current='page'],
  .row[aria-pressed='true'] {
    background: var(--text);
    color: var(--bg);
    border-color: var(--text);
  }
  .row[aria-current='page'] .count,
  .row[aria-pressed='true'] .count {
    color: var(--bg);
  }
  .list {
    flex: 1;
    min-height: 0;
    border-right: 0;
  }
  .find {
    padding: 12px 16px 8px;
  }
  .list-head .small {
    display: none;
  }
  .has-file .side {
    display: none;
  }
  .side-meta span:last-child {
    display: none;
  }
}
</style>
