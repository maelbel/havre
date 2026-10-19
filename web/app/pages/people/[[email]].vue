<script setup lang="ts">
import type { SplitPaneSpec } from '~/composables/useSplit'
import type { PersonEntry, PersonDetail } from '~/composables/usePlatform'
import type { NoteSummary } from '~/composables/useNotes'
import type { Task } from '~/composables/useTasks'

// People: everyone you write to or meet, built from Mail (senders) and
// the Agenda (guests, organizers). Nothing to fill in. /people/<email>
// opens one person: recent mail and meetings, side by side.
const route = useRoute()
const toast = useToasts()
const email = computed(() => ((route.params.email as string) || '').toLowerCase() || null)

const accounts = useAccounts()
const accountMap = computed(() => new Map(accounts.data.value.map((a) => [a.id, a])))

// ---- The list ------------------------------------------------------------------

const query = ref('')
const people = ref<PersonEntry[]>([])
const listLoading = ref(true)
let seq = 0
async function search() {
  const mine = ++seq
  listLoading.value = true
  try {
    const found = await searchPeople(query.value.trim())
    if (mine === seq) people.value = found
  } catch (err) {
    if (mine === seq) toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    if (mine === seq) listLoading.value = false
  }
}
let timer: ReturnType<typeof setTimeout> | undefined
watch(query, () => {
  clearTimeout(timer)
  timer = setTimeout(search, 150)
})
onMounted(search)

// ---- One person ------------------------------------------------------------------

const detail = ref<PersonDetail | null>(null)
const detailError = ref<string | null>(null)
watch(
  email,
  async (e) => {
    detail.value = null
    detailError.value = null
    if (!e) return
    try {
      detail.value = await fetchPerson(e)
    } catch (err) {
      detailError.value = apiErrorMessage(err)
    }
  },
  { immediate: true },
)

// ---- Tasks with this person (when the Tasks app is in the dock) ---------------------
const tasksOn = useTasksEnabled()
const { byId: taskLists } = useTaskLists()
const personTasks = ref<Task[]>([])
async function loadTasks() {
  personTasks.value = []
  if (!tasksOn.value || !email.value) return
  try {
    const found = await fetchTasks({ person: email.value })
    personTasks.value = found.filter((t) => !t.done_at).sort(byDue)
  } catch {
    /* demo or offline */
  }
}
watch([email, tasksOn], loadTasks, { immediate: true })
const newTask = ref('')
const addingTask = ref(false)
async function addTask() {
  const text = newTask.value.trim()
  if (!text || !detail.value || addingTask.value) return
  addingTask.value = true
  try {
    await quickAddTask(text, { person: { email: detail.value.email, name: detail.value.name || null } })
    newTask.value = ''
    await loadTasks()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    addingTask.value = false
  }
}
async function toggleTask(task: Task) {
  try {
    await setTaskDone(task.id, !task.done_at)
    personTasks.value = personTasks.value.filter((t) => t.id !== task.id)
    toast.show('Done', { action: { label: 'Undo', run: () => setTaskDone(task.id, false).then(loadTasks) } })
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
// ---- Notes about this person (when the Notes app is in the dock) -------------------
const notesOn = useNotesEnabled()
const filesOn = useFilesEnabled()
const personNotes = ref<NoteSummary[]>([])
watch(
  [email, notesOn],
  async () => {
    personNotes.value = []
    if (!notesOn.value || !email.value) return
    const asked = email.value
    const found = await fetchNotes({ person: asked }).catch(() => [])
    if (email.value === asked) personNotes.value = found
  },
  { immediate: true },
)
async function noteAbout() {
  if (!detail.value) return
  try {
    const note = await createNote({ title: personName(detail.value), people: [{ email: detail.value.email, name: detail.value.name || null }] })
    await navigateTo({ path: '/notes', query: { note: note.id, edit: 1 } })
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

const firstName = computed(() => (detail.value?.name || detail.value?.email || '').split(/[\s@]/)[0])

useHead({ title: () => (detail.value ? `${personName(detail.value)} · People · Havre` : 'People · Havre') })

const canWrite = computed(() => accounts.data.value.find((a) => a.can_send))
async function write() {
  if (!detail.value || !canWrite.value) return
  const to = detail.value.name ? `${detail.value.name} <${detail.value.email}>` : detail.value.email
  try {
    await compose({ account: canWrite.value.id, to })
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

function eventLink(e: { id: number; start: number; all_day: boolean }) {
  const start = e.all_day ? utcDateToLocal(e.start) : new Date(e.start * 1000)
  return { path: '/agenda', query: { view: 'day', date: isoDate(start), event: String(e.id) } }
}
const counts = (p: PersonEntry) =>
  [p.messages && `${p.messages} mail${p.messages > 1 ? 's' : ''}`, p.meetings && `${p.meetings} meeting${p.meetings > 1 ? 's' : ''}`]
    .filter(Boolean)
    .join(' · ')

// Resizable panes (wide screens): people · person.
const splitPanes: SplitPaneSpec[] = [
  { id: 'list', size: 320, min: 260, max: 480, handle: { label: 'Resize the people list' } },
  { id: 'person', min: 480 },
]
</script>

<template>
  <div class="people" :class="{ open: !!email }">
    <SplitView id="people" :panes="splitPanes">
    <template #list>
    <aside class="list" aria-label="People">
      <div class="list-head">
        <h1 class="serif">People</h1>
        <p class="faint small">From your mail and your meetings.</p>
        <UInput
          v-model="query"
          type="search"
          icon="i-havre-search"
          placeholder="Search people"
          aria-label="Search people"
          class="search"
        />
      </div>
      <ul>
        <li v-for="p in people" :key="p.email">
          <NuxtLink :to="`/people/${encodeURIComponent(p.email)}`" class="row" :class="{ on: p.email === email }">
            <span class="avatar" :style="{ background: personColor(p.email) }">{{ initials(personName(p)) }}</span>
            <span class="who">
              <span class="name">{{ personName(p) }}</span>
              <span class="sub">{{ p.name ? p.email : counts(p) }}</span>
            </span>
          </NuxtLink>
        </li>
      </ul>
      <p v-if="!listLoading && !people.length" class="empty">
        {{ query ? 'Nobody matches.' : 'People appear here as your mail and calendars sync.' }}
      </p>
    </aside>
    </template>

    <template #person>
    <main class="detail">
      <template v-if="detail">
        <UButton
          icon="i-havre-back"
          color="neutral"
          variant="ghost"
          square
          class="back"
          to="/people"
          aria-label="Back to People"
        />
        <header class="card-head">
          <span class="avatar big" :style="{ background: personColor(detail.email) }">{{ initials(personName(detail)) }}</span>
          <div class="id">
            <h2 class="serif">{{ personName(detail) }}</h2>
            <p class="faint">{{ detail.email }}</p>
            <p class="faint small">{{ counts(detail) || 'Nothing in common yet.' }}</p>
          </div>
          <UButton v-if="canWrite" icon="i-havre-pen" type="button" @click="write">Write</UButton>
        </header>

        <div class="cols">
          <section aria-labelledby="p-mail">
            <h3 id="p-mail" class="ph">Recent mail</h3>
            <p v-if="!detail.recent_messages.length" class="empty">No mail from {{ detail.name || 'them' }}.</p>
            <ul>
              <li v-for="m in detail.recent_messages" :key="`${m.account}-${m.folder}-${m.uid}`">
                <NuxtLink :to="messageLink(m)" class="item" :class="{ unread: m.unread }">
                  <span class="acc" :style="{ background: `var(--c-${accountMap.get(m.account)?.color ?? 'work'})` }" />
                  <span class="t">{{ m.subject || '(no subject)' }}</span>
                  <span class="d">{{ shortDate(m.date) }}</span>
                </NuxtLink>
              </li>
            </ul>
            <template v-if="detail.files.length">
              <h3 class="ph later">Shared files</h3>
              <ul>
                <li v-for="f in detail.files" :key="`${f.account}-${f.uid}-${f.index}`">
                  <a :href="fileLink(f)" :download="f.name" class="item">
                    <span class="kind mono" aria-hidden="true">{{ fileKind(f.name) }}</span>
                    <span class="t">{{ f.name }}</span>
                    <span class="d">{{ shortDate(f.date) }}</span>
                  </a>
                </li>
              </ul>
              <!-- With the Files app: all of them, and their versions. -->
              <NuxtLink v-if="filesOn" :to="{ path: '/files', query: { person: detail.email } }" class="to-files">
                All files from {{ (detail.name || detail.email).split(' ')[0] }} in Files →
              </NuxtLink>
            </template>
          </section>

          <section aria-labelledby="p-meet">
            <template v-if="tasksOn">
              <h3 id="p-tasks" class="ph">Tasks<template v-if="personTasks.length"> · {{ personTasks.length }}</template></h3>
              <TaskRow
                v-for="t in personTasks"
                :key="t.id"
                :task="t"
                :list="taskLists.get(t.list)"
                compact
                hide-list
                @toggle="toggleTask(t)"
                @open="navigateTo({ path: '/tasks', query: { task: t.id } })"
              />
              <form @submit.prevent="addTask">
                <UInput
                  v-model="newTask"
                  icon="i-havre-plus"
                  variant="none"
                  :placeholder="`New task with ${firstName}… (“call fri”)`"
                  :aria-label="`New task with ${firstName}`"
                  :disabled="addingTask"
                  class="new-task"
                />
              </form>
            </template>
            <template v-if="notesOn">
              <h3 id="p-notes" class="ph" :class="{ later: tasksOn }">Notes<template v-if="personNotes.length"> · {{ personNotes.length }}</template></h3>
              <ul>
                <li v-for="n in personNotes.slice(0, 5)" :key="n.id">
                  <NuxtLink :to="{ path: '/notes', query: { note: n.id } }" class="item">
                    <span class="t">{{ noteTitle(n) }}</span>
                    <span class="d">{{ noteWhen(n.updated_at) }}</span>
                  </NuxtLink>
                </li>
              </ul>
              <NuxtLink v-if="personNotes.length > 5" :to="{ path: '/notes', query: { person: email } }" class="all-notes">
                All {{ personNotes.length }} notes →
              </NuxtLink>
              <button type="button" class="note-about" @click="noteAbout">
                <HIcon name="plus" :size="14" />Note about {{ firstName }}
              </button>
            </template>
            <h3 id="p-meet" class="ph" :class="{ later: tasksOn || notesOn }">Upcoming meetings</h3>
            <p v-if="!detail.upcoming.length" class="empty">None planned.</p>
            <ul>
              <li v-for="e in detail.upcoming" :key="e.id">
                <NuxtLink :to="eventLink(e)" class="item">
                  <span class="t">{{ e.title }}</span>
                  <span class="d">{{ whenLabel(e) }}</span>
                </NuxtLink>
              </li>
            </ul>
            <template v-if="detail.past.length">
              <h3 class="ph later">Past meetings</h3>
              <ul>
                <li v-for="e in detail.past" :key="e.id">
                  <NuxtLink :to="eventLink(e)" class="item past">
                    <span class="t">{{ e.title }}</span>
                    <span class="d">{{ whenLabel(e) }}</span>
                  </NuxtLink>
                </li>
              </ul>
            </template>
          </section>
        </div>
      </template>
      <p v-else-if="detailError" class="empty center">{{ detailError }}</p>
      <div v-else-if="!email" class="empty center">
        <HIcon name="people" :size="28" />
        <p>Pick someone to see your mail and meetings with them.</p>
      </div>
    </main>
    </template>
    </SplitView>
  </div>
</template>

<style scoped>
.to-files {
  display: inline-block;
  margin-top: 8px;
  font-size: 13px;
  text-decoration: none;
}
/* A dashed line until you type in it. */
.new-task {
  width: 100%;
  margin: 4px 0 0;
  border-radius: 9px;
  border: 1px dashed var(--line2);
}
.new-task:focus-within {
  border-style: solid;
  border-color: var(--accent);
}
.people {
  flex: 1;
  display: flex;
  min-width: 0;
}
.list {
  flex: 0 0 320px;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--line);
  background: var(--bg);
  overflow-y: auto;
}
.list-head {
  position: sticky;
  top: 0;
  z-index: 1;
  padding: 22px 16px 12px;
  background: var(--bg);
}
h1 {
  margin: 0;
  font-size: 28px;
}
.small {
  font-size: 12.5px;
}
.faint {
  color: var(--faint);
  margin: 2px 0 0;
}
.search {
  width: 100%;
  margin-top: 12px;
}
ul {
  margin: 0;
  padding: 0;
  list-style: none;
}
.list ul {
  padding: 0 8px 16px;
}
.row {
  display: flex;
  align-items: center;
  gap: 11px;
  padding: 8px;
  border-radius: 10px;
  color: var(--text);
  text-decoration: none;
}
.row:hover {
  background: var(--hover);
}
.row.on {
  background: var(--sel);
}
.avatar {
  flex: 0 0 34px;
  height: 34px;
  border-radius: 50%;
  display: grid;
  place-items: center;
  font-size: 12.5px;
  font-weight: 600;
  color: #1b1b1b;
}
.avatar.big {
  flex-basis: 64px;
  height: 64px;
  font-size: 22px;
}
.who {
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.name {
  font-size: 14px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sub {
  font-size: 12.5px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.detail {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  padding: 32px 40px 48px;
  background: var(--panel);
}
.back {
  display: none;
}
.card-head {
  display: flex;
  align-items: center;
  gap: 18px;
  margin-bottom: 28px;
}
.id {
  flex: 1;
  min-width: 0;
}
h2 {
  margin: 0;
  font-size: 30px;
  line-height: 1.15;
  overflow-wrap: anywhere;
}
.cols {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  gap: 28px;
}
.ph {
  margin: 0 0 8px;
  font-size: 12px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.08em;
  color: var(--faint);
}
.ph.later {
  margin-top: 20px;
}
.note-about {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 32px;
  margin-top: 4px;
  padding: 0 10px;
  border: 1px dashed var(--line2);
  border-radius: 8px;
  background: transparent;
  color: var(--muted);
  font-size: 13px;
  cursor: pointer;
}
.note-about:hover {
  background: var(--hover);
  color: var(--text);
}
.all-notes {
  display: inline-block;
  margin: 4px 0;
  font-size: 13px;
  text-decoration: none;
}
.item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  margin: 0 -10px;
  border-radius: 9px;
  color: var(--text);
  text-decoration: none;
  font-size: 13.5px;
}
.item:hover {
  background: var(--hover);
}
.item.unread .t {
  font-weight: 600;
}
.item.past {
  color: var(--muted);
}
.item .t {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.item .d {
  flex: 0 0 auto;
  font-size: 12px;
  color: var(--faint);
}
.acc {
  flex: 0 0 7px;
  height: 7px;
  border-radius: 50%;
}
.empty {
  color: var(--faint);
  font-size: 13.5px;
  margin: 4px 0;
}
.list > .empty {
  padding: 0 16px;
}
.empty.center {
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  text-align: center;
}
@media (max-width: 960px) {
  .cols {
    grid-template-columns: minmax(0, 1fr);
  }
}
@media (max-width: 720px) {
  .list {
    flex: 1;
    border-right: 0;
  }
  .detail {
    display: none;
    padding: 16px 16px 40px;
  }
  .people.open .list {
    display: none;
  }
  .people.open .detail {
    display: block;
  }
  .back {
    display: inline-flex;
    margin-bottom: 12px;
  }
  .card-head {
    flex-wrap: wrap;
  }
}
.kind {
  flex: 0 0 auto;
  font-size: 10px;
  padding: 3px 5px;
  border-radius: 4px;
  background: color-mix(in oklab, var(--accent) 16%, transparent);
  color: var(--accent);
}
</style>
