<script setup lang="ts">
import type { SplitPaneSpec } from '~/composables/useSplit'
import type { PeopleFilter, PeopleSort, PersonEntry, PersonDetail } from '~/composables/usePlatform'
import type { NoteSummary } from '~/composables/useNotes'
import type { Task } from '~/composables/useTasks'

// People: everyone you write to or meet, built from Mail (senders and
// recipients) and the Agenda (guests), plus your address books when you sync
// them (Settings › People). Nothing to fill in, but you can add details,
// favourites, and merge someone's addresses. /people/<email> opens one
// person: their details, where things stand, mail and meetings; someone
// without an address is /people/~42 (their contact).
const route = useRoute()
const toast = useToasts()
const email = computed(() => ((route.params.email as string) || '').toLowerCase() || null)

const accounts = useAccounts()
const accountMap = computed(() => new Map(accounts.data.value.map((a) => [a.id, a])))

// ---- The list ------------------------------------------------------------------

const query = ref('')
const sort = ref<PeopleSort>('recent')
const filter = ref<PeopleFilter>('everyone')
const company = ref<string | null>(null)
const people = ref<PersonEntry[]>([])
const listLoading = ref(true)
const sorts = [
  { label: 'Recent', value: 'recent' },
  { label: 'Frequent', value: 'frequent' },
  { label: 'A–Z', value: 'name' },
]
const filters: { id: PeopleFilter; label: string; icon?: string }[] = [
  { id: 'everyone', label: 'Everyone' },
  { id: 'favorites', label: 'Favourites', icon: 'i-havre-star' },
  { id: 'waiting', label: 'Waiting for you', icon: 'i-havre-reply' },
  { id: 'automated', label: 'Newsletters & robots' },
]
// The companies, for the "By company" menu (loaded once).
const companies = ref<{ label: string; value: string | null }[]>([{ label: 'Any company', value: null }])
onMounted(async () => {
  const list = await fetchCompanies().catch(() => [])
  companies.value = [{ label: 'Any company', value: null }, ...list.map((c) => ({ label: `${c.name} · ${c.people}`, value: c.name }))]
})
let seq = 0
async function search() {
  const mine = ++seq
  listLoading.value = true
  try {
    const found = await listPeople({ q: query.value.trim(), sort: sort.value, filter: filter.value, company: company.value })
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
watch([sort, filter, company], search)
onMounted(search)
/** A row's marks: waiting for you, birthday this week. */
const birthdaySoon = (p: PersonEntry) => {
  const d = daysToBirthday(p.birthday)
  return d != null && d <= 7 ? d : null
}
/** Under the name: their company and role, else their address (or number). */
const subline = (p: PersonEntry) =>
  [p.company, p.role].filter(Boolean).join(' · ') ||
  (noAddress(p) ? (p.name ? (p.phones[0]?.number ?? '') : '') : p.name ? p.email : counts(p))

// An address book changed something: refresh what's on screen (once for a burst).
let refresh: ReturnType<typeof setTimeout> | undefined
useMailEvents((event) => {
  if (event.type !== 'people') return
  clearTimeout(refresh)
  refresh = setTimeout(async () => {
    search()
    if (email.value && !editing.value && !merging.value) {
      detail.value = await fetchPerson(email.value).catch(() => detail.value)
    }
  }, 500)
})
onBeforeUnmount(() => clearTimeout(refresh))

// ---- Someone new, .vcf files ------------------------------------------------------
const adding = ref(false)
const newPerson = reactive({ name: '', email: '' })
async function addPerson() {
  try {
    const p = await createPerson(newPerson.name.trim(), newPerson.email.trim())
    adding.value = false
    newPerson.name = ''
    newPerson.email = ''
    search()
    // The page is made again for the new address: say "edit" in it.
    await navigateTo({ path: `/people/${encodeURIComponent(p.email)}`, query: { edit: '1' } })
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
const vcf = ref<HTMLInputElement | null>(null)
async function importFile(event: Event) {
  const input = event.target as HTMLInputElement
  const picked = input.files?.[0]
  input.value = ''
  if (!picked) return
  try {
    const r = await importPeople(await picked.text())
    toast.show(`${picked.name}: ${r.added} added, ${r.updated} completed${r.skipped ? `, ${r.skipped} skipped` : ''}`)
    search()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
const listMenu = [
  [
    { label: 'New person…', icon: 'i-havre-plus', onSelect: () => (adding.value = true) },
    { label: 'Import a .vcf file…', icon: 'i-havre-upload', onSelect: () => vcf.value?.click() },
    { label: 'Export as .vcf', icon: 'i-havre-download', onSelect: () => window.open(PEOPLE_EXPORT, '_blank') },
  ],
  [{ label: 'Sync with your address books…', icon: 'i-havre-settings', to: '/settings/apps/people' }],
]

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
      const found = await fetchPerson(e)
      // Opened by another of their addresses: show their main one in the address.
      if (found.email !== e && found.emails.includes(e)) {
        await navigateTo(`/people/${encodeURIComponent(found.email)}`, { replace: true })
        return
      }
      detail.value = found
      // Just created (?edit=1): straight to their details.
      if (route.query.edit) {
        editing.value = true
        await navigateTo({ path: route.path }, { replace: true })
      }
    } catch (err) {
      detailError.value = apiErrorMessage(err)
    }
  },
  { immediate: true },
)

// ---- Your details: edit, favourite, merge, robot ------------------------------------
const editing = ref(false)
const merging = ref(false)
/** After a change: the page shows it, the list too. */
async function changed(p: PersonDetail) {
  detail.value = p
  editing.value = false
  merging.value = false
  if (p.email !== email.value) await navigateTo(`/people/${encodeURIComponent(p.email)}`, { replace: true })
  search()
}
async function toggleFavorite() {
  if (!detail.value) return
  try {
    await changed(await updatePerson(detail.value.email, { favorite: !detail.value.favorite }))
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
async function setKind(kind: 'person' | 'automated') {
  if (!detail.value) return
  try {
    const p = await updatePerson(detail.value.email, { kind })
    await changed(p)
    toast.show(kind === 'automated' ? `${personName(p)} is now under Newsletters & robots` : `${personName(p)} is a person again`)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
async function separate(other: string) {
  if (!detail.value) return
  try {
    await changed(await removePersonAddress(detail.value.email, other))
    toast.show(`${other} is someone else again`)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
const moreItems = computed(() => {
  const d = detail.value
  if (!d) return []
  const items: { label: string; icon?: string; onSelect: () => void }[][] = [
    [
      {
        label: noAddress(d) ? 'Add an address…' : 'Merge with another address…',
        icon: 'i-havre-merge',
        onSelect: () => (merging.value = true),
      },
    ],
  ]
  const others = d.emails.filter((e) => e !== d.email)
  if (others.length) items.push(others.map((o) => ({ label: `Separate ${o}`, icon: 'i-havre-close', onSelect: () => separate(o) })))
  if (!noAddress(d)) {
    items.push([
      d.automated
        ? { label: 'This is a person', icon: 'i-havre-people', onSelect: () => setKind('person') }
        : { label: 'Not a person (newsletter, robot)', icon: 'i-havre-archive', onSelect: () => setKind('automated') },
    ])
  }
  return items
})

/** "Waiting for your reply since Tuesday": their latest email, to answer. */
const waitingMail = computed(() => detail.value?.recent_messages.find((m) => m.from) ?? detail.value?.recent_messages[0])
function since(t: number) {
  const d = new Date(t * 1000)
  const days = Math.floor((Date.now() - d.getTime()) / 86_400_000)
  if (days < 1) return 'today'
  if (days < 2) return 'yesterday'
  if (days < 7) return d.toLocaleDateString([], { weekday: 'long' })
  return d.toLocaleDateString([], { day: 'numeric', month: 'long' })
}
const dayLabel = (t: number) => new Date(t * 1000).toLocaleDateString([], { weekday: 'short', day: 'numeric', month: 'short' })

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

const firstName = computed(() => (detail.value ? personName(detail.value) : '').split(/[\s@]/)[0])
/** Where their details come from, and the last change an address book made. */
const fromBooks = computed(() => {
  const d = detail.value
  if (!d?.books.length) return ''
  let text = `In ${d.books.join(' · ')}`
  if (d.updated_from && d.updated_from_at) {
    // "updated from Google Contacts" only says something when there are several books.
    const from = d.books.length > 1 ? ` from ${d.updated_from}` : ''
    text += ` · updated${from} ${agoText(d.updated_from_at)}`
  }
  return text
})

useHead({ title: () => (detail.value ? `${personName(detail.value)} · People · Havre` : 'People · Havre') })

const canWrite = computed(() => (detail.value && noAddress(detail.value) ? undefined : accounts.data.value.find((a) => a.can_send)))

// ---- Phones: chips pick which list shows ----------------------------------------
const phone = useIsPhone()
type Only = 'all' | 'mail' | 'meet' | 'files' | 'tasks' | 'notes'
const only = ref<Only>('all')
// Back to "Everything" for each new person.
watch(email, () => (only.value = 'all'))
const chips = computed(() => {
  const d = detail.value
  if (!d) return []
  const list: { id: Only; label: string }[] = [{ id: 'all', label: 'Everything' }]
  if (!noAddress(d)) {
    list.push({ id: 'mail', label: `Mail · ${d.recent_messages.length}` })
    list.push({ id: 'meet', label: `Meetings · ${d.upcoming.length + d.past.length}` })
  }
  if (d.files.length) list.push({ id: 'files', label: `Files · ${d.files.length}` })
  if (tasksOn.value) list.push({ id: 'tasks', label: `Tasks · ${personTasks.value.length}` })
  if (notesOn.value) list.push({ id: 'notes', label: `Notes · ${personNotes.value.length}` })
  return list
})
/** On a phone, is this list shown? (Everything shows all of them.) */
const shows = (kind: Only) => !phone.value || only.value === 'all' || only.value === kind
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
      <PhoneHeader title="People" />
      <div class="list-head">
        <div class="title-row">
          <h1 class="serif">People</h1>
          <UButton color="neutral" variant="ghost" icon="i-havre-plus" square aria-label="New person" @click="adding = true" />
          <UDropdownMenu :items="listMenu">
            <UButton color="neutral" variant="ghost" icon="i-havre-ellipsis" square aria-label="People options" />
          </UDropdownMenu>
          <input ref="vcf" type="file" accept=".vcf,text/vcard,text/x-vcard" class="sr-only" aria-hidden="true" tabindex="-1" @change="importFile" />
        </div>
        <p class="faint small">From your mail, your meetings and your address books.</p>
        <UInput
          v-model="query"
          type="search"
          icon="i-havre-search"
          placeholder="Name, address or company"
          aria-label="Search people"
          class="search"
        />
        <div class="tools">
          <USelect v-model="sort" :items="sorts" size="sm" aria-label="Sort people" class="w-28" />
          <USelect v-model="company" :items="companies" size="sm" aria-label="Filter by company" class="flex-1 min-w-0" />
        </div>
        <div class="filters" role="tablist" aria-label="Show">
          <UButton
            v-for="f in filters"
            :key="f.id"
            role="tab"
            :aria-selected="filter === f.id"
            :color="filter === f.id ? 'primary' : 'neutral'"
            :variant="filter === f.id ? 'soft' : 'ghost'"
            :icon="f.icon"
            size="xs"
            @click="filter = f.id"
          >
            {{ f.label }}
          </UButton>
        </div>
      </div>
      <ul>
        <li v-for="p in people" :key="p.email">
          <NuxtLink :to="`/people/${encodeURIComponent(p.email)}`" class="row" :class="{ on: p.email === email || p.emails.includes(email ?? '') }">
            <PersonAvatar :person="p" />
            <span class="who">
              <span class="name">
                {{ personName(p) }}
                <HIcon v-if="p.favorite" name="starFilled" :size="12" class="fav" /><span v-if="p.favorite" class="sr-only">, favourite</span>
              </span>
              <span class="sub">{{ subline(p) }}</span>
            </span>
            <span class="meta">
              <span v-if="p.last_contact" class="ago">{{ ago(p.last_contact) }}</span>
              <span class="marks">
                <span v-if="p.waiting_since" class="mark waiting" title="Waiting for your reply"><HIcon name="reply" :size="11" /><span class="sr-only">Waiting for your reply</span></span>
                <span v-if="birthdaySoon(p) != null" class="mark bday" :title="birthdaySoon(p) === 0 ? 'Birthday today' : `Birthday in ${birthdaySoon(p)} days`">
                  <HIcon name="cake" :size="11" /><span class="sr-only">Birthday soon</span>
                </span>
              </span>
            </span>
          </NuxtLink>
        </li>
      </ul>
      <p v-if="!listLoading && !people.length" class="empty">
        {{
          query || company
            ? 'Nobody matches.'
            : filter === 'favorites'
              ? 'No favourites yet: open someone and press ★.'
              : filter === 'waiting'
                ? 'Nobody is waiting for your reply.'
                : filter === 'automated'
                  ? 'No newsletters or robots.'
                  : 'People appear here as your mail and calendars sync.'
        }}
      </p>
    </aside>
    </template>

    <template #person>
    <main class="detail">
      <template v-if="detail">
        <PhoneHeader back="/people" back-label="Back to People">
          <template #actions>
            <UButton v-if="canWrite" color="neutral" variant="ghost" size="lg" square icon="i-havre-pen" :aria-label="`Write to ${firstName}`" @click="write" />
            <UButton
              v-if="!noAddress(detail)"
              color="neutral"
              variant="ghost"
              size="lg"
              square
              icon="i-havre-calendar"
              :aria-label="`Plan a meeting with ${firstName}`"
              :to="{ path: '/agenda', query: { new: 1, guest: detail.email } }"
            />
          </template>
        </PhoneHeader>
        <header class="card-head">
          <PersonAvatar :person="detail" :size="64" />
          <div class="id">
            <h2 class="serif">{{ personName(detail) }}</h2>
            <p v-if="detail.company || detail.role" class="work">{{ [detail.role, detail.company].filter(Boolean).join(' · ') }}</p>
            <p v-if="detail.emails.length" class="faint addresses">
              <span v-for="(e, i) in detail.emails" :key="e">{{ e }}<template v-if="i < detail.emails.length - 1">,&nbsp;</template></span>
            </p>
            <p v-else class="faint">
              No email address ·
              <UButton color="neutral" variant="link" size="sm" class="p-0" @click="merging = true">Add one</UButton>
            </p>
            <p v-if="!noAddress(detail)" class="faint small">
              {{ counts(detail) || 'Nothing in common yet.' }}<template v-if="detail.automated"> · newsletter or robot</template>
            </p>
            <p v-if="fromBooks" class="faint small books">{{ fromBooks }}</p>
          </div>
          <div class="head-actions">
            <UButton v-if="canWrite" icon="i-havre-pen" type="button" class="desk" @click="write">Write</UButton>
            <UButton
              color="neutral"
              :variant="detail.favorite ? 'soft' : 'ghost'"
              :icon="detail.favorite ? 'i-havre-star-filled' : 'i-havre-star'"
              square
              :class="{ 'is-fav': detail.favorite }"
              :aria-label="detail.favorite ? 'Remove from favourites' : 'Add to favourites'"
              :aria-pressed="detail.favorite"
              @click="toggleFavorite"
            />
            <UButton color="neutral" variant="ghost" icon="i-havre-pen" square :aria-label="`Edit ${firstName}'s details`" class="edit" @click="editing = true" />
            <UDropdownMenu :items="moreItems">
              <UButton color="neutral" variant="ghost" icon="i-havre-ellipsis" square aria-label="More" />
            </UDropdownMenu>
          </div>
        </header>

        <!-- What you know: phones, birthday, note (when there's any). -->
        <dl v-if="detail.phones.length || detail.birthday || detail.note" class="facts">
          <div v-if="detail.phones.length">
            <dt><HIcon name="phone" :size="14" /><span class="sr-only">Phones</span></dt>
            <dd>
              <a v-for="ph in detail.phones" :key="ph.number" :href="`tel:${ph.number.replace(/[^+\d]/g, '')}`" class="tel">
                {{ ph.number }}<span v-if="ph.label" class="faint ph-label">{{ ph.label }}</span>
              </a>
            </dd>
          </div>
          <div v-if="detail.birthday">
            <dt><HIcon name="cake" :size="14" /><span class="sr-only">Birthday</span></dt>
            <dd>
              {{ birthdayLabel(detail.birthday) }}
              <span v-if="birthdaySoon(detail) != null" class="soon">{{ birthdaySoon(detail) === 0 ? 'today!' : `in ${birthdaySoon(detail)} days` }}</span>
            </dd>
          </div>
          <div v-if="detail.note">
            <dt><HIcon name="note" :size="14" /><span class="sr-only">Note</span></dt>
            <dd class="note">{{ detail.note }}</dd>
          </div>
        </dl>
        <UButton
          v-else-if="!detail.automated"
          color="neutral"
          variant="link"
          size="sm"
          icon="i-havre-plus"
          class="add-details"
          @click="editing = true"
        >
          Add a phone, birthday, note…
        </UButton>

        <!-- Keeping in touch: where things stand. -->
        <section v-if="!detail.automated && (detail.last_from || detail.last_to || detail.last_meeting || detail.next_meeting)" class="touch" aria-label="Keeping in touch">
          <div v-if="detail.waiting_since" class="waiting-box">
            <HIcon name="reply" :size="15" />
            <span>Waiting for your reply since {{ since(detail.waiting_since) }}</span>
            <UButton v-if="waitingMail" size="xs" :to="messageLink(waitingMail)">Open the email</UButton>
          </div>
          <ul class="stand">
            <li v-if="detail.last_from"><span class="k">Last email from {{ firstName }}</span><span class="v">{{ agoText(detail.last_from) }}</span></li>
            <li v-if="detail.last_to"><span class="k">You last wrote</span><span class="v">{{ agoText(detail.last_to) }}</span></li>
            <li v-if="detail.last_meeting"><span class="k">Last meeting</span><span class="v">{{ dayLabel(detail.last_meeting) }}</span></li>
            <li v-if="detail.next_meeting"><span class="k">Next meeting</span><span class="v">{{ dayLabel(detail.next_meeting) }}</span></li>
          </ul>
        </section>

        <!-- Phones: one list at a time, picked with these chips. -->
        <div v-if="phone" class="chips" role="tablist" aria-label="With them">
          <UButton
            v-for="c in chips"
            :key="c.id"
            role="tab"
            :aria-selected="only === c.id"
            :color="only === c.id ? 'primary' : 'neutral'"
            :variant="only === c.id ? 'solid' : 'outline'"
            size="sm"
            class="rounded-(--radius) shrink-0"
            @click="only = c.id"
          >
            {{ c.label }}
          </UButton>
        </div>

        <div class="cols" :class="{ solo: noAddress(detail) }">
          <section v-if="!noAddress(detail)" aria-labelledby="p-mail">
            <div v-show="shows('mail')">
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
            </div>
            <div v-if="detail.files.length" v-show="shows('files')">
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
            </div>
          </section>

          <section aria-labelledby="p-meet">
            <div v-if="tasksOn" v-show="shows('tasks')">
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
            </div>
            <div v-if="notesOn" v-show="shows('notes')">
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
            </div>
            <div v-if="!noAddress(detail)" v-show="shows('meet')">
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
            </div>
          </section>
        </div>
      </template>
      <UModal v-model:open="adding" title="New person" description="A name is enough: add their phone and the rest next.">
        <template #body>
          <form id="new-person" class="new-person" @submit.prevent="addPerson">
            <UFormField label="Name">
              <UInput v-model="newPerson.name" autofocus class="w-full" />
            </UFormField>
            <UFormField label="Email address" help="Optional">
              <UInput v-model="newPerson.email" type="email" class="w-full" />
            </UFormField>
          </form>
        </template>
        <template #footer>
          <div class="flex justify-end gap-2 w-full">
            <UButton color="neutral" variant="outline" @click="adding = false">Cancel</UButton>
            <UButton type="submit" form="new-person" :disabled="!newPerson.name.trim() && !newPerson.email.trim()">Add</UButton>
          </div>
        </template>
      </UModal>
      <PersonEditor v-if="editing && detail" :person="detail" @saved="changed" @cancel="editing = false" />
      <PersonMerge v-if="merging && detail" :person="detail" @merged="changed" @cancel="merging = false" />
      <p v-if="!detail && detailError" class="empty center">{{ detailError }}</p>
      <div v-else-if="!detail && !email" class="empty center">
        <HIcon name="people" :size="28" />
        <p>Pick someone to see your mail and meetings with them.</p>
      </div>
    </main>
    </template>
    </SplitView>
  </div>
</template>

<style scoped>
.title-row {
  display: flex;
  align-items: center;
  gap: 2px;
}
.title-row h1 {
  flex: 1;
}
.books {
  margin-top: 2px;
}
.new-person {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.tools {
  display: flex;
  gap: 6px;
  margin-top: 8px;
}
.filters {
  display: flex;
  flex-wrap: wrap;
  gap: 2px;
  margin: 8px -4px 0;
}
.fav {
  color: var(--accent);
  vertical-align: -1px;
  margin-left: 3px;
}
.meta {
  flex: none;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 3px;
}
.ago {
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--faint);
}
.marks {
  display: flex;
  gap: 4px;
}
.mark {
  display: grid;
  place-items: center;
  width: 18px;
  height: 18px;
  border-radius: 50%;
}
.mark.waiting {
  color: var(--accent);
  background: color-mix(in oklab, var(--accent) 16%, transparent);
}
.mark.bday {
  color: var(--c-personal);
  background: color-mix(in oklab, var(--c-personal) 18%, transparent);
}
.work {
  margin: 2px 0 0;
  font-size: 14px;
  color: var(--muted);
}
/* Each address stays whole; a very long one may still wrap anywhere. */
.addresses span {
  display: inline-block;
  max-width: 100%;
  overflow-wrap: anywhere;
}
.head-actions {
  display: flex;
  align-items: center;
  gap: 2px;
  margin-left: auto;
  align-self: flex-start;
}
.is-fav {
  color: var(--accent);
}
.facts {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 18px 0 0;
  padding: 14px 16px;
  border: 1px solid var(--line);
  border-radius: var(--radius-lg);
  background: var(--raise);
}
.facts > div {
  display: flex;
  gap: 12px;
  align-items: baseline;
}
.facts dt {
  flex: none;
  color: var(--faint);
  transform: translateY(2px);
}
.facts dd {
  margin: 0;
  display: flex;
  flex-wrap: wrap;
  gap: 4px 16px;
  font-size: 14px;
}
.facts .note {
  white-space: pre-line;
  color: var(--muted);
}
.tel {
  display: inline-flex;
  align-items: baseline;
  gap: 6px;
  color: var(--text);
  text-decoration: none;
}
.ph-label {
  font-size: 12.5px;
}
.tel:hover {
  color: var(--accent);
}
.soon {
  color: var(--c-personal);
  font-size: 13px;
}
.add-details {
  margin-top: 10px;
}
.touch {
  margin: 18px 0 0;
}
.waiting-box {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 10px;
  padding: 10px 14px;
  border-radius: var(--radius);
  border: 1px solid color-mix(in oklab, var(--accent) 45%, var(--line));
  background: color-mix(in oklab, var(--accent) 10%, transparent);
  color: var(--text);
  font-size: 13.5px;
}
.waiting-box > span {
  flex: 1;
}
.waiting-box :deep(svg) {
  color: var(--accent);
}
.stand {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
  gap: 8px;
  margin: 0;
  padding: 0;
  list-style: none;
}
.stand li {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 10px 12px;
  border: 1px solid var(--line);
  border-radius: var(--radius);
}
.stand .k {
  font-size: 11.5px;
  letter-spacing: 0.02em;
  color: var(--faint);
}
.stand .v {
  font-size: 14px;
}
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
  border-radius: 4px;
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
  border-radius: 4px;
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
  border-radius: var(--radius);
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
  flex: 1;
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
  margin-top: 26px;
}
/* Someone without an address: only tasks and notes, one column. */
.cols.solo {
  grid-template-columns: minmax(0, 1fr);
  max-width: 560px;
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
  border-radius: 4px;
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
  border-radius: 4px;
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
  border-radius: var(--radius);
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
  .detail {
    padding: 0 0 40px;
  }
  .detail > :not(.phone-head) {
    margin-left: 16px;
    margin-right: 16px;
  }
  /* The phone header has Back and Write. */
  .desk,
  .list-head h1 {
    display: none;
  }
  /* The phone header has the title: the buttons go to the right. */
  .title-row {
    justify-content: flex-end;
    margin-top: -8px;
  }
  .chips {
    display: flex;
    gap: 6px;
    margin: 0 0 16px;
    padding: 0 16px;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .detail > .chips {
    margin-left: 0;
    margin-right: 0;
  }
  .list-head {
    padding-top: 0;
  }
  h2 {
    font-size: 26px;
    /* A long address as the name breaks anywhere rather than overflowing. */
    overflow-wrap: anywhere;
  }
  /* With a chip picked, the hidden lists leave no gap. */
  .cols {
    gap: 0;
  }
  .cols > section + section {
    margin-top: 28px;
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
