<script setup lang="ts">
import type { AgendaEvent } from '~/composables/useAgenda'
import type { Contact } from '~/composables/useCompose'
import type { NotePerson, NoteSummary, NoteView } from '~/composables/useNotes'
import type { PersonLink } from '~/composables/useTasks'

// Notes: views, people and the next meetings on the left, the notes in the
// middle, the open note on the right (Read: the Markdown drawn; Edit: the
// Markdown itself, saved as you type). `?view=…` or `?person=…` picks the
// list, `?note=…` the open note.
useHead({ title: 'Notes · Havre' })
const route = useRoute()
const router = useRouter()
const toast = useToasts()
const tasksOn = useTasksEnabled()

type View = 'all' | 'pinned' | 'meetings' | 'people'
const VIEWS: { id: View; label: string; icon: 'note' | 'pushpin' | 'calendar' | 'people' }[] = [
  { id: 'all', label: 'All notes', icon: 'note' },
  { id: 'pinned', label: 'Pinned', icon: 'pushpin' },
  { id: 'meetings', label: 'Meetings', icon: 'calendar' },
  { id: 'people', label: 'People', icon: 'people' },
]
const personFilter = computed(() => (route.query.person as string | undefined)?.toLowerCase() ?? null)
const view = computed<View | null>(() =>
  personFilter.value ? null : VIEWS.some((v) => v.id === route.query.view) ? (route.query.view as View) : 'all',
)
const selectedId = computed(() => (route.query.note ? Number(route.query.note) : null))
function go(query: Record<string, string | number | undefined>) {
  router.replace({ query: Object.fromEntries(Object.entries(query).filter(([, v]) => v !== undefined)) })
}
function pickView(id: View) {
  go({ view: id === 'all' ? undefined : id, note: selectedId.value ?? undefined })
}
function pickPerson(email: string) {
  go({ person: email, note: selectedId.value ?? undefined })
}
function openNote(id: number) {
  go({ ...route.query, note: id } as Record<string, string | number>)
}
function closeNote() {
  const { note: _, ...rest } = route.query
  router.replace({ query: rest })
}

// ---- The list -------------------------------------------------------------------
const notes = ref<NoteSummary[]>([])
const people = ref<NotePerson[]>([])
const loading = ref(true)
const search = ref('')
const found = ref<NoteSummary[] | null>(null)
async function load() {
  try {
    const [all, who] = await Promise.all([fetchNotes(), fetchNotePeople()])
    notes.value = all
    people.value = who
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    loading.value = false
  }
}
onMounted(load)

// Searching asks the server (it reads every note's whole text).
let searchTimer: ReturnType<typeof setTimeout> | undefined
watch(search, (q) => {
  clearTimeout(searchTimer)
  if (!q.trim()) {
    found.value = null
    return
  }
  searchTimer = setTimeout(async () => {
    found.value = await fetchNotes({ q: q.trim() }).catch(() => [])
  }, 200)
})

const counts = computed(() => ({
  all: notes.value.length,
  pinned: notes.value.filter((n) => n.pinned).length,
  meetings: notes.value.filter((n) => n.event).length,
  people: notes.value.filter((n) => n.people.length).length,
}))
const shown = computed(() => {
  const list = found.value ?? notes.value
  if (personFilter.value) return list.filter((n) => n.people.some((p) => p.email === personFilter.value))
  if (view.value === 'pinned') return list.filter((n) => n.pinned)
  if (view.value === 'meetings') return list.filter((n) => n.event)
  if (view.value === 'people') return list.filter((n) => n.people.length)
  return list
})
const WEEK = 7 * 86_400
const groups = computed(() => {
  const now = Date.now() / 1000
  const pinned = shown.value.filter((n) => n.pinned)
  const recent = shown.value.filter((n) => !n.pinned && now - n.updated_at < WEEK)
  const earlier = shown.value.filter((n) => !n.pinned && now - n.updated_at >= WEEK)
  return [
    { id: 'pinned', label: 'Pinned', items: pinned },
    { id: 'week', label: 'This week', items: recent },
    { id: 'earlier', label: 'Earlier', items: earlier },
  ].filter((g) => g.items.length)
})
const heading = computed(() => {
  if (search.value.trim()) return 'Search'
  if (personFilter.value) {
    const p = people.value.find((x) => x.email === personFilter.value)
    return p?.name || personFilter.value
  }
  return VIEWS.find((v) => v.id === view.value)?.label ?? 'Notes'
})

/** A note changed: its line in the list follows. */
function summarize(v: NoteView): NoteSummary {
  const old = notes.value.find((n) => n.id === v.id)
  const todos = v.blocks.filter((b) => b.kind === 'todo' && !b.done && b.text).length
  // The snippet is the server's; while typing, a rough one from the text.
  const snippet = v.body
    .split('\n')
    .map((l) => l.replace(/^\s*([#>]+|[-*+]\s+(\[[ xX]]\s*)?|\d+[.)]\s+)/, '').replace(/[*_`]/g, '').trim())
    .filter((l) => l && !/^#/.test(l))
    .join(' · ')
    .slice(0, 160)
  return {
    id: v.id,
    title: v.title,
    snippet: snippet || old?.snippet || '',
    pinned: v.pinned,
    updated_at: v.updated_at,
    event: v.event,
    people: v.people,
    mail: v.mail,
    open_todos: todos,
  }
}
function remember(v: NoteView) {
  const s = summarize(v)
  const i = notes.value.findIndex((n) => n.id === v.id)
  if (i >= 0) notes.value.splice(i, 1, s)
  else notes.value.unshift(s)
  notes.value.sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.updated_at - a.updated_at)
}
async function refreshPeople() {
  people.value = await fetchNotePeople().catch(() => people.value)
}

// ---- The open note --------------------------------------------------------------
const note = ref<NoteView | null>(null)
const mode = ref<'read' | 'edit'>('read')
const title = ref('')
const body = ref('')
const saving = ref(false)
const dirty = ref(false)
const busyLine = ref<number | null>(null)
const titleInput = ref<HTMLInputElement | null>(null)
const textarea = ref<HTMLTextAreaElement | null>(null)

async function loadNote(id: number | null) {
  await flush()
  if (id == null) {
    note.value = null
    return
  }
  try {
    const v = await fetchNote(id)
    note.value = v
    title.value = v.title
    body.value = v.body
    dirty.value = false
    // An empty or just-made note opens ready to write (`?edit=1`: made
    // elsewhere, e.g. "Take notes" in the Agenda).
    const fresh = justMade.value === id || route.query.edit === '1'
    mode.value = !v.body.trim() || fresh ? 'edit' : 'read'
    justMade.value = null
    if (route.query.edit) {
      const { edit: _, ...rest } = route.query
      router.replace({ query: rest })
    }
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
    closeNote()
  }
}

// Saved as you type: a moment after the last key, and when you leave.
let saveTimer: ReturnType<typeof setTimeout> | undefined
watch([title, body], () => {
  if (!note.value || (title.value === note.value.title && body.value === note.value.body)) return
  dirty.value = true
  clearTimeout(saveTimer)
  saveTimer = setTimeout(flush, 700)
})
async function flush() {
  clearTimeout(saveTimer)
  const current = note.value
  if (!current || !dirty.value) return
  dirty.value = false
  saving.value = true
  try {
    const v = await saveNote(current.id, noteInput(current, { title: title.value, body: body.value }))
    // Still the open note? Take the server's copy (its blocks, for Read).
    // What was typed meanwhile is in `title`/`body` and saves next.
    if (note.value?.id === v.id) note.value = v
    remember(v)
  } catch (err) {
    dirty.value = true
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    saving.value = false
  }
}
onBeforeUnmount(flush)
if (import.meta.client) {
  const leave = () => {
    if (dirty.value) void flush()
  }
  onMounted(() => window.addEventListener('beforeunload', leave))
  onBeforeUnmount(() => window.removeEventListener('beforeunload', leave))
}

const savedLabel = computed(() => {
  if (saving.value || dirty.value) return 'Saving…'
  if (!note.value) return ''
  const when = noteWhen(note.value.updated_at)
  return /\d:\d/.test(when) ? `Saved · today ${when}` : `Saved · ${when}`
})

async function setMode(m: 'read' | 'edit') {
  if (m === 'read') await flush()
  mode.value = m
  if (m === 'edit') nextTick(() => textarea.value?.focus())
}

/** Saves links or the pin at once (no waiting). */
async function saveLinks(change: Partial<{ people: PersonLink[]; event: null; mail: null }>) {
  const current = note.value
  if (!current) return
  clearTimeout(saveTimer)
  dirty.value = false
  try {
    const v = await saveNote(current.id, noteInput(current, { title: title.value, body: body.value, ...change }))
    note.value = v
    remember(v)
    refreshPeople()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
async function togglePin() {
  const current = note.value
  if (!current) return
  try {
    const v = await pinNote(current.id, !current.pinned)
    note.value = v
    remember({ ...v, title: title.value })
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

async function tick(line: number, done: boolean) {
  const current = note.value
  if (!current) return
  await flush()
  busyLine.value = line
  try {
    const v = await tickNoteTodo(current.id, line, done)
    note.value = v
    body.value = v.body
    dirty.value = false
    remember(v)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    busyLine.value = null
  }
}
async function makeTask(line: number) {
  const current = note.value
  if (!current) return
  await flush()
  busyLine.value = line
  try {
    note.value = await noteTodoToTask(current.id, line)
    toast.show('Added to Tasks')
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    busyLine.value = null
  }
}

// ---- New, delete ----------------------------------------------------------------
const justMade = ref<number | null>(null)
async function newNote() {
  try {
    // A note made while looking at someone's notes is about them.
    const who = personFilter.value ? people.value.find((p) => p.email === personFilter.value) : null
    const v = await createNote({ title: '', body: '', people: who ? [{ email: who.email, name: who.name }] : [] })
    remember(v)
    justMade.value = v.id
    openNote(v.id)
    nextTick(() => setTimeout(() => titleInput.value?.focus(), 50))
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
const confirming = ref(false)
watch(selectedId, () => (confirming.value = false))
async function remove() {
  const current = note.value
  if (!current) return
  if (!confirming.value) {
    confirming.value = true
    return
  }
  clearTimeout(saveTimer)
  dirty.value = false
  const copy = noteInput(current, { title: title.value, body: body.value })
  try {
    await deleteNote(current.id)
    notes.value = notes.value.filter((n) => n.id !== current.id)
    note.value = null
    closeNote()
    refreshPeople()
    toast.show('Note deleted', {
      action: {
        label: 'Undo',
        run: async () => {
          const v = await createNote(copy)
          remember(v)
          refreshPeople()
          openNote(v.id)
        },
      },
    })
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

// ---- Attaching a person -----------------------------------------------------------
const attaching = ref(false)
const who = ref('')
const suggestions = ref<Contact[]>([])
let whoTimer: ReturnType<typeof setTimeout> | undefined
watch(who, (q) => {
  clearTimeout(whoTimer)
  whoTimer = setTimeout(async () => {
    suggestions.value = q.trim().length >= 2 ? (await searchContacts(q.trim()).catch(() => [])).slice(0, 6) : []
  }, 150)
})
function addPerson(p: PersonLink) {
  const current = note.value
  attaching.value = false
  who.value = ''
  if (!current || current.people.some((x) => x.email.toLowerCase() === p.email.toLowerCase())) return
  saveLinks({ people: [...current.people, p] })
}
function addTyped() {
  const typed = who.value.trim()
  const first = suggestions.value[0]
  if (first) addPerson({ email: first.address, name: first.name })
  else if (/^[^\s@]+@[^\s@]+$/.test(typed)) addPerson({ email: typed, name: null })
}
function removePerson(email: string) {
  const current = note.value
  if (current) saveLinks({ people: current.people.filter((p) => p.email !== email) })
}

// ---- Next meetings (from the Agenda) ----------------------------------------------
const upcoming = ref<AgendaEvent[]>([])
onMounted(async () => {
  const now = Math.floor(Date.now() / 1000)
  const events = await fetchEvents(now - 3600, now + 7 * 86_400).catch(() => [])
  upcoming.value = events.filter((e) => !e.all_day && e.end > now).sort((a, b) => a.start - b.start).slice(0, 3)
})
function noteOf(e: AgendaEvent) {
  const uid = e.uid || `havre-event-${e.id}`
  return notes.value.find((n) => n.event?.uid === uid && (!e.recurring || n.event.start === e.start)) ?? null
}
function meetingWhen(e: AgendaEvent) {
  const d = new Date(e.start * 1000)
  const day = sameDay(d, new Date()) ? 'Today' : d.toLocaleDateString('en-GB', { weekday: 'short' })
  return `${day} · ${timeLabel(d)}`
}
async function takeNotes(e: AgendaEvent) {
  const existing = noteOf(e)
  if (existing) return openNote(existing.id)
  try {
    const v = await noteFromEvent(e.id)
    remember(v)
    refreshPeople()
    justMade.value = v.id
    openNote(v.id)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
function agendaDay(start: number) {
  return { path: '/agenda', query: { view: 'day', date: isoDate(new Date(start * 1000)) } }
}

// ---- Keys -------------------------------------------------------------------------
// `n`: new note. `e`: edit the open note. Esc while editing: back to Read.
function onKey(e: KeyboardEvent) {
  const typing = (e.target as HTMLElement)?.closest?.('input, textarea, select, [contenteditable]')
  if (e.key === 'Escape' && mode.value === 'edit' && (e.target as HTMLElement)?.tagName === 'TEXTAREA') {
    e.preventDefault()
    setMode('read')
    return
  }
  if (typing || e.ctrlKey || e.metaKey || e.altKey) return
  if (e.key === 'n') {
    e.preventDefault()
    newNote()
  } else if (e.key === 'e' && note.value && mode.value === 'read') {
    e.preventDefault()
    setMode('edit')
  }
}
onMounted(() => document.addEventListener('keydown', onKey))
onBeforeUnmount(() => document.removeEventListener('keydown', onKey))

const selectedSummary = computed(() => notes.value.find((n) => n.id === selectedId.value))

// Last, once everything it uses exists: `immediate` runs it right away, and
// a `const` used before its line is an error in JavaScript.
watch(selectedId, loadNote, { immediate: true })
</script>

<template>
  <div class="notes-app" :class="{ 'has-note': selectedId != null }">
    <SplitGroup id="notes">
    <SplitPane id="views" :size="232" :min="200" :max="320" collapsible>
    <aside class="side" aria-label="Notes views">
      <div class="side-head">
        <h1 class="serif">Notes</h1>
        <button type="button" class="new btn-primary" @click="newNote">
          <HIcon name="plus" :size="13" :stroke-width="2" />New
        </button>
      </div>
      <nav class="views" aria-label="Views">
        <button
          v-for="v in VIEWS"
          :key="v.id"
          type="button"
          class="row"
          :aria-current="view === v.id ? 'page' : undefined"
          @click="pickView(v.id)"
        >
          <HIcon :name="v.icon" :size="16" class="ic" />
          <span class="name">{{ v.label }}</span>
          <span v-if="counts[v.id]" class="count mono">{{ counts[v.id] }}</span>
        </button>
      </nav>

      <template v-if="people.length">
        <div class="eyebrow section">People</div>
        <button
          v-for="p in people.slice(0, 8)"
          :key="p.email"
          type="button"
          class="row"
          :aria-current="personFilter === p.email ? 'page' : undefined"
          :title="p.email"
          @click="pickPerson(p.email)"
        >
          <span class="avatar small" :style="{ '--c': personColor(p.name || p.email) }" aria-hidden="true">
            {{ initials(p.name || p.email) }}
          </span>
          <span class="name">{{ p.name || p.email }}</span>
          <span class="count mono">{{ p.notes }}</span>
        </button>
      </template>

      <span class="grow" />

      <section v-if="upcoming.length" class="meetings" aria-label="Next meetings">
        <div class="eyebrow">Next meetings</div>
        <div v-for="e in upcoming" :key="`${e.id}-${e.start}`" class="meeting">
          <span class="bar-c" aria-hidden="true" />
          <span class="m">
            <span class="mt">{{ e.title }}</span>
            <span class="mw">{{ meetingWhen(e) }}<template v-if="noteOf(e)"> · has a note</template></span>
          </span>
          <button
            type="button"
            class="mini"
            :aria-label="noteOf(e) ? `Open the note: ${e.title}` : `Take notes: ${e.title}`"
            @click="takeNotes(e)"
          >
            {{ noteOf(e) ? 'Open' : 'Notes' }}
          </button>
        </div>
        <NuxtLink to="/agenda" class="to-agenda">Open Agenda →</NuxtLink>
      </section>
    </aside>
    </SplitPane>

    <SplitHandle label="Resize the views" fold="views" show-label="Show views" />
    <SplitPane id="list" :size="300" :min="260" :max="520">
    <section class="list" aria-label="Notes">
      <header class="bar">
        <h2>{{ heading }}</h2>
        <span class="mono faint">{{ shown.length }}</span>
      </header>
      <div class="find">
        <label class="search">
          <HIcon name="search" :size="14" />
          <span class="sr-only">Search notes</span>
          <input v-model="search" type="search" placeholder="Search notes" />
        </label>
      </div>
      <div class="scroll">
        <p v-if="loading" class="empty">Loading…</p>
        <template v-else>
          <section v-for="g in groups" :key="g.id" :aria-label="g.label">
            <div class="eyebrow group">{{ g.label }}</div>
            <button
              v-for="n in g.items"
              :key="n.id"
              type="button"
              class="item"
              :aria-current="n.id === selectedId ? 'true' : undefined"
              @click="openNote(n.id)"
            >
              <span class="top">
                <span class="title" :class="{ untitled: !n.title.trim() }">{{ noteTitle(n) }}</span>
                <span class="when">{{ noteWhen(n.updated_at) }}</span>
              </span>
              <span v-if="n.snippet" class="snippet">{{ n.snippet }}</span>
              <span v-if="n.event || n.people.length || n.open_todos" class="meta">
                <span v-if="n.event" class="chip-i"><HIcon name="calendar" :size="12" class="c-ev" />{{ noteEventLabel(n.event, true) }}</span>
                <span v-if="n.people.length" class="chip-i">
                  <HIcon name="people" :size="12" class="c-pe" />
                  {{ n.people.map((p) => notePersonLabel(p).split(' ')[0]).slice(0, 3).join(', ') }}
                </span>
                <span v-if="n.open_todos" class="chip-i">
                  <HIcon name="task" :size="12" />{{ n.open_todos }} to-do{{ n.open_todos === 1 ? '' : 's' }}
                </span>
              </span>
            </button>
          </section>
          <p v-if="!groups.length" class="empty">
            {{
              search.trim()
                ? 'No note has these words.'
                : view === 'pinned'
                  ? 'Pin a note to keep it on top.'
                  : view === 'meetings'
                    ? 'No meeting notes yet: open an event in the Agenda and choose Take notes.'
                    : 'No notes yet. Press N or New to start one.'
            }}
          </p>
        </template>
      </div>
    </section>
    </SplitPane>

    <SplitHandle label="Resize the notes list" />
    <SplitPane id="note" :min="420">
    <main class="note" :aria-label="note ? noteTitle({ title }) : 'Note'">
      <template v-if="note">
        <header class="bar note-bar">
          <button type="button" class="icon-btn back" aria-label="Back to notes" @click="closeNote">
            <HIcon name="back" :size="18" />
          </button>
          <div class="mode" role="group" aria-label="Mode">
            <button type="button" :aria-pressed="mode === 'read'" @click="setMode('read')">Read</button>
            <button type="button" :aria-pressed="mode === 'edit'" @click="setMode('edit')">Edit</button>
          </div>
          <span class="saved" aria-live="polite">{{ savedLabel }}</span>
          <span class="grow-x" />
          <button
            type="button"
            class="icon-btn"
            :class="{ on: note.pinned }"
            :aria-pressed="note.pinned"
            :aria-label="note.pinned ? 'Unpin note' : 'Pin note'"
            @click="togglePin"
          >
            <HIcon name="pushpin" :size="16" />
          </button>
          <button type="button" class="icon-btn" aria-label="Download as Markdown" @click="downloadNote({ ...note, title, body })">
            <HIcon name="download" :size="16" />
          </button>
          <button
            type="button"
            class="icon-btn"
            :class="{ danger: confirming }"
            :aria-label="confirming ? 'Click again to delete the note' : 'Delete note'"
            :title="confirming ? 'Click again to delete' : 'Delete'"
            @click="remove"
          >
            <HIcon name="trash" :size="16" />
          </button>
        </header>

        <div class="scroll">
          <article class="page">
            <label class="title-field">
              <span class="sr-only">Title</span>
              <input ref="titleInput" v-model="title" class="serif" placeholder="Untitled note" @keydown.enter.prevent="setMode('edit')" />
            </label>

            <div class="links">
              <NuxtLink v-if="note.event" :to="agendaDay(note.event.start)" class="link-chip">
                <span class="ev-bar" aria-hidden="true" />{{ noteEventLabel(note.event) }}
              </NuxtLink>
              <span v-for="p in note.people" :key="p.email" class="person-chip">
                <NuxtLink :to="`/people/${encodeURIComponent(p.email)}`" class="pc" :title="p.email">
                  <span class="avatar" :style="{ '--c': personColor(notePersonLabel(p)) }" aria-hidden="true">
                    {{ initials(notePersonLabel(p)) }}
                  </span>
                  {{ notePersonLabel(p) }}
                </NuxtLink>
                <button
                  v-if="mode === 'edit'"
                  type="button"
                  class="x"
                  :aria-label="`Remove ${notePersonLabel(p)} from this note`"
                  @click="removePerson(p.email)"
                >
                  <HIcon name="close" :size="12" />
                </button>
              </span>
              <button v-if="note.mail" type="button" class="link-chip" @click="openNoteMail(note.id).catch((e) => toast.show(apiErrorMessage(e), { tone: 'error' }))">
                <HIcon name="mail" :size="14" class="c-pe" />{{ note.mail.subject || '(no subject)' }}
              </button>
              <button
                v-if="mode === 'edit' && note.event"
                type="button"
                class="unlink"
                @click="saveLinks({ event: null })"
              >
                Detach the meeting
              </button>
              <form v-if="attaching" class="attach" @submit.prevent="addTyped">
                <input
                  v-model="who"
                  aria-label="Add a person: name or address"
                  placeholder="Name or address"
                  autofocus
                  @keydown.esc="attaching = false"
                />
                <ul v-if="suggestions.length" class="suggest">
                  <li v-for="c in suggestions" :key="c.address">
                    <button type="button" @mousedown.prevent="addPerson({ email: c.address, name: c.name })">
                      <span>{{ c.name || c.address }}</span><span v-if="c.name" class="faint">{{ c.address }}</span>
                    </button>
                  </li>
                </ul>
              </form>
              <button v-else type="button" class="add-link" @click="attaching = true">
                <HIcon name="plus" :size="13" :stroke-width="1.8" />Person
              </button>
            </div>

            <div v-if="mode === 'read'" class="read" @dblclick="setMode('edit')">
              <NoteBody
                v-if="note.blocks.length"
                :blocks="note.blocks"
                :tasks="tasksOn"
                :busy="busyLine"
                @tick="tick"
                @task="makeTask"
              />
              <p v-else class="empty-note">
                Nothing written yet. <button type="button" class="link-btn" @click="setMode('edit')">Start writing</button>
              </p>
            </div>
            <label v-else class="write">
              <span class="sr-only">Note text, in Markdown</span>
              <textarea ref="textarea" v-model="body" class="mono" spellcheck="true" placeholder="Write in Markdown…" @blur="flush" />
              <span class="hint">
                Markdown: <span class="mono">## heading</span>, <span class="mono">**bold**</span>,
                <span class="mono">- [ ] to-do</span>. Saved as you type · Esc to read.
              </span>
            </label>
          </article>
        </div>
      </template>
      <div v-else class="none">
        <HIcon name="note" :size="28" />
        <p>{{ selectedSummary ? 'Loading…' : 'Pick a note, or start one.' }}</p>
        <button type="button" class="btn" @click="newNote"><HIcon name="plus" :size="14" />New note</button>
      </div>
    </main>
    </SplitPane>
    </SplitGroup>
  </div>
</template>

<style scoped>
.notes-app {
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
  align-items: center;
  gap: 8px;
  padding: 0 4px 14px 8px;
}
h1 {
  margin: 0;
  flex: 1;
  font-size: 28px;
}
.new {
  height: 32px;
  padding: 0 12px;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
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
  color: var(--text);
  text-align: left;
  cursor: pointer;
  font-size: 13.5px;
}
.row:hover {
  background: var(--hover);
}
.row[aria-current='page'] {
  background: var(--sel);
}
.row[aria-current='page'] .ic {
  color: var(--accent);
}
.ic {
  color: var(--muted);
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
.avatar.small {
  width: 20px;
  height: 20px;
}
.grow {
  flex: 1;
  min-height: 16px;
}
.meetings {
  padding: 12px;
  border-radius: 12px;
  border: 1px solid var(--line);
  background: var(--panel);
  display: flex;
  flex-direction: column;
  gap: 9px;
  font-size: 12.5px;
}
.meeting {
  display: flex;
  align-items: center;
  gap: 8px;
}
.bar-c {
  flex: 0 0 3px;
  height: 26px;
  border-radius: 2px;
  background: var(--c-work);
}
.m {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.mt {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.mw {
  color: var(--faint);
}
.mini {
  flex: none;
  height: 28px;
  padding: 0 8px;
  border: 1px solid var(--line2);
  border-radius: 7px;
  background: transparent;
  color: var(--muted);
  font-size: 12px;
  cursor: pointer;
}
.mini:hover {
  background: var(--hover);
  color: var(--text);
}
.to-agenda {
  text-decoration: none;
}

/* The list */
.list {
  flex: 0 1 340px;
  min-width: 280px;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--line);
  background: var(--panel);
}
.bar {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 56px;
  padding: 0 16px 0 20px;
  border-bottom: 1px solid var(--line);
  flex: none;
}
h2 {
  margin: 0;
  flex: 1;
  min-width: 0;
  font-size: 15px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.faint {
  color: var(--faint);
  font-size: 12px;
}
.find {
  padding: 12px 14px 4px;
}
.search {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 36px;
  padding: 0 10px;
  border: 1px solid var(--line2);
  border-radius: 9px;
  background: var(--bg);
  color: var(--faint);
}
.search:focus-within {
  border-color: var(--accent);
}
.search input {
  flex: 1;
  min-width: 0;
  border: 0;
  outline: none;
  background: transparent;
  color: var(--text);
  font-size: 13.5px;
}
.scroll {
  flex: 1;
  overflow-y: auto;
}
.list .scroll {
  padding: 4px 8px 16px;
}
.group {
  padding: 14px 10px 6px;
}
.item {
  display: flex;
  flex-direction: column;
  gap: 4px;
  width: 100%;
  padding: 10px 12px;
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
.top {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
}
.title {
  flex: 1;
  min-width: 0;
  font-size: 14px;
  font-weight: 500;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.title.untitled {
  color: var(--muted);
  font-style: italic;
}
.when {
  flex: none;
  font-size: 12px;
  color: var(--faint);
}
.snippet {
  font-size: 12.5px;
  color: var(--muted);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.meta {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 10px;
  font-size: 12px;
  color: var(--faint);
}
.chip-i {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}
.c-ev {
  color: var(--c-lyon);
}
.c-pe {
  color: var(--c-work);
}
.empty {
  padding: 24px 12px;
  color: var(--faint);
  font-size: 13px;
}

/* The note */
.note {
  flex: 999 1 420px;
  min-width: 0;
  display: flex;
  flex-direction: column;
  background: var(--bg);
}
.note-bar {
  gap: 8px;
  padding: 0 12px 0 24px;
}
.back {
  display: none;
}
.mode {
  display: flex;
  padding: 3px;
  border-radius: 9px;
  border: 1px solid var(--line2);
  background: var(--panel);
}
.mode button {
  height: 26px;
  padding: 0 12px;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--muted);
  font-size: 12.5px;
  cursor: pointer;
}
.mode button[aria-pressed='true'] {
  background: var(--sel);
  color: var(--text);
}
.saved {
  font-size: 12.5px;
  color: var(--faint);
  white-space: nowrap;
}
.grow-x {
  flex: 1;
}
.icon-btn.on {
  color: var(--accent);
}
.icon-btn.danger {
  color: var(--danger);
  background: color-mix(in oklab, var(--danger) 12%, transparent);
}
.page {
  max-width: 720px;
  margin: 0 auto;
  padding: 28px 36px 40px;
  display: flex;
  flex-direction: column;
  gap: 18px;
}
.title-field input {
  width: 100%;
  border: 0;
  outline: none;
  background: transparent;
  color: var(--text);
  font-size: 32px;
  line-height: 1.15;
  letter-spacing: -0.01em;
  padding: 0;
}
.title-field input::placeholder {
  color: var(--faint);
}
.links {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  position: relative;
}
.link-chip {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  height: 32px;
  padding: 0 12px 0 10px;
  border-radius: 8px;
  border: 1px solid var(--line2);
  background: transparent;
  color: var(--text);
  text-decoration: none;
  font-size: 13px;
  cursor: pointer;
  max-width: 100%;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
.link-chip:hover,
.pc:hover {
  background: var(--hover);
}
.ev-bar {
  width: 3px;
  height: 16px;
  border-radius: 2px;
  background: var(--c-lyon);
  flex: none;
}
.person-chip {
  display: inline-flex;
  align-items: center;
  height: 32px;
  border-radius: 999px;
  border: 1px solid var(--line2);
}
.pc {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  height: 100%;
  padding: 0 12px 0 5px;
  border-radius: 999px;
  color: var(--text);
  text-decoration: none;
  font-size: 13px;
}
.x {
  width: 26px;
  height: 26px;
  margin-right: 3px;
  margin-left: -6px;
  border: 0;
  border-radius: 50%;
  background: transparent;
  color: var(--muted);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}
.x:hover {
  background: var(--hover);
  color: var(--text);
}
.add-link,
.unlink {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 32px;
  padding: 0 10px;
  border: 1px dashed var(--line2);
  border-radius: 8px;
  background: transparent;
  color: var(--muted);
  font-size: 13px;
  cursor: pointer;
}
.unlink {
  border-style: solid;
  border-color: transparent;
}
.add-link:hover,
.unlink:hover {
  background: var(--hover);
  color: var(--text);
}
.attach {
  position: relative;
}
.attach input {
  height: 32px;
  width: 220px;
  padding: 0 10px;
  border: 1px solid var(--accent);
  border-radius: 8px;
  background: var(--panel);
  color: var(--text);
  outline: none;
  font-size: 13px;
}
.suggest {
  position: absolute;
  z-index: 5;
  top: 36px;
  left: 0;
  width: 280px;
  margin: 0;
  padding: 4px;
  list-style: none;
  border-radius: 10px;
  border: 1px solid var(--line2);
  background: var(--raise);
  box-shadow: 0 12px 32px rgba(0, 0, 0, 0.3);
}
.suggest button {
  width: 100%;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  padding: 7px 10px;
  border: 0;
  border-radius: 7px;
  background: transparent;
  color: var(--text);
  text-align: left;
  cursor: pointer;
  font-size: 13px;
}
.suggest button:hover {
  background: var(--hover);
}
.read {
  padding-top: 14px;
  border-top: 1px solid var(--line);
}
.empty-note {
  color: var(--faint);
  margin: 0;
}
.link-btn {
  border: 0;
  padding: 0;
  background: none;
  color: var(--accent);
  cursor: pointer;
  font-size: inherit;
}
.write {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
textarea {
  min-height: 420px;
  padding: 16px 18px;
  border: 1px solid var(--line2);
  border-radius: 12px;
  background: var(--panel);
  color: var(--text);
  font-size: 13.5px;
  line-height: 1.7;
  resize: vertical;
  outline: none;
}
textarea:focus {
  border-color: var(--accent);
}
.hint {
  font-size: 12px;
  color: var(--faint);
}
.none {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  color: var(--faint);
}
.none p {
  margin: 0;
}
.none .btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

/* Narrower windows: the note takes the list's place when open. */
@media (max-width: 1100px) {
  .has-note .list {
    display: none;
  }
  .back {
    display: inline-flex;
  }
  .note-bar {
    padding-left: 8px;
  }
}

/* Phones: views as tabs on top, the list, then a note full screen. */
@media (max-width: 720px) {
  .notes-app {
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
  h1 {
    font-size: 26px;
  }
  .new {
    flex: none;
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
  .row .ic,
  .section,
  .side > .row,
  .grow,
  .meetings {
    display: none;
  }
  .row[aria-current='page'] {
    background: var(--text);
    color: var(--bg);
    border-color: var(--text);
  }
  .row[aria-current='page'] .count {
    color: var(--bg);
  }
  .list {
    flex: 1;
    min-height: 0;
    min-width: 0;
    border-right: 0;
  }
  .list .bar {
    display: none;
  }
  .note {
    display: none;
  }
  .has-note .side,
  .has-note .list {
    display: none;
  }
  .has-note .note {
    display: flex;
    flex: 1;
    min-height: 0;
  }
  .note-bar {
    padding: 0 8px;
  }
  .saved {
    display: none;
  }
  .page {
    padding: 16px 20px 28px;
    gap: 14px;
  }
  .title-field input {
    font-size: 28px;
  }
  textarea {
    min-height: 320px;
  }
  .attach input {
    width: 180px;
  }
}
</style>
