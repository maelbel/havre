<script setup lang="ts">
import type { AiAnswer, AiHit, AiProposal } from '~/composables/useAi'
import type { AiEmailProposal } from '~/composables/useAssistant'
import type { IconName } from './HIcon.vue'
import type { NoteSummary } from '~/composables/useNotes'
import type { FileItem } from '~/composables/useFiles'
import type { PersonEntry } from '~/composables/usePlatform'
import type { Parsed, Task } from '~/composables/useTasks'

// Ctrl+K (⌘K on a Mac), anywhere: go somewhere, do something, find a
// person or a message — or, when AI is on, ask Havre in plain language.
// The assistant only proposes changes; you confirm them here.
const open = usePalette()
const q = ref('')
const input = ref<{ $el: HTMLElement } | null>(null)
/** The search field inside UCommandPalette, to give it the focus. */
const focusSearch = () => input.value?.$el?.querySelector<HTMLInputElement>('input')?.focus()
const router = useRouter()
const toast = useToasts()
const { toggle: toggleTheme } = useTheme()
const { status: ai } = useAiStatus()
const accounts = useAccounts()
const isMac = import.meta.client && /Mac|iPhone|iPad/.test(navigator.platform)

interface Item {
  id: string
  group: string
  label: string
  hint?: string
  icon: IconName
  keywords?: string
  run: () => void | Promise<void>
}

function close() {
  open.value = false
}
function show() {
  open.value = true
}
// Opened (by the shortcut or the dock button): start fresh.
watch(open, (v) => {
  if (!v) return
  mode.value = 'search'
  q.value = ''
  nextTick(focusSearch)
})

function onGlobalKey(e: KeyboardEvent) {
  if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === 'k') {
    e.preventDefault()
    if (open.value) close()
    else show()
  }
}
onMounted(() => window.addEventListener('keydown', onGlobalKey))
onBeforeUnmount(() => window.removeEventListener('keydown', onGlobalKey))

const go = (path: string) => () => {
  router.push(path)
  close()
}

async function newMessage() {
  close()
  const account = accounts.data.value.find((a) => a.can_send)
  if (!account) return void router.push('/settings')
  await compose({ account: account.id }).catch((err) => toast.show(apiErrorMessage(err), { tone: 'error' }))
}

const tasksOn = useTasksEnabled()
const notesOn = useNotesEnabled()
const filesOn = useFilesEnabled()
const commands = computed<Item[]>(() => [
  ...(filesOn.value
    ? [{ id: 'files', group: 'Go to', label: 'Files', icon: 'folder' as IconName, keywords: 'attachments documents pdf', run: go('/files') }]
    : []),
  ...(notesOn.value
    ? [
        { id: 'notes', group: 'Go to', label: 'Notes', icon: 'note' as IconName, keywords: 'markdown meeting notes', run: go('/notes') },
        { id: 'new-note', group: 'Actions', label: 'New note', hint: 'type “note Lyon ideas”', icon: 'note' as IconName, keywords: 'write add create', run: () => { q.value = 'note ' } },
      ]
    : []),
  ...(tasksOn.value
    ? [
        { id: 'tasks', group: 'Go to', label: 'Tasks', icon: 'task' as IconName, keywords: 'todo to-do', run: go('/tasks') },
        { id: 'new-task', group: 'Actions', label: 'New task', hint: 'type “task call Thomas fri”', icon: 'task' as IconName, keywords: 'todo add create', run: () => { q.value = 'task ' } },
      ]
    : []),
  { id: 'today', group: 'Go to', label: 'Today', icon: 'logo', keywords: 'home brief', run: go('/') },
  { id: 'mail', group: 'Go to', label: 'Mail', icon: 'mail', keywords: 'inbox', run: go('/mail') },
  { id: 'agenda', group: 'Go to', label: 'Agenda', icon: 'calendar', keywords: 'calendar events', run: go('/agenda') },
  { id: 'people', group: 'Go to', label: 'People', icon: 'people', keywords: 'contacts', run: go('/people') },
  { id: 'apps', group: 'Go to', label: 'Apps', icon: 'apps', keywords: 'library dock', run: go('/apps') },
  { id: 'settings', group: 'Go to', label: 'Settings', icon: 'settings', keywords: 'accounts preferences', run: go('/settings') },
  ...(ai.value.installed
    ? [
        { id: 'ai', group: 'Go to', label: 'AI', icon: 'sparkle' as IconName, keywords: 'assistant ask chat conversations', run: go('/ai') },
        { id: 'ai-settings', group: 'Go to', label: 'Settings › AI', icon: 'sparkle' as IconName, keywords: 'ai model ollama claude', run: go('/settings/ai') },
      ]
    : []),
  { id: 'mcp-settings', group: 'Go to', label: 'Settings › Connections (MCP)', icon: 'command', keywords: 'mcp connect claude code ai', run: go('/settings/connections') },
  { id: 'write', group: 'Actions', label: 'New message', hint: 'C in Mail', icon: 'pen', keywords: 'write compose email', run: newMessage },
  { id: 'event', group: 'Actions', label: 'New event', hint: 'N in Agenda', icon: 'plus', keywords: 'meeting create', run: go('/agenda?new=1') },
  {
    id: 'theme',
    group: 'Actions',
    label: 'Switch light / dark theme',
    icon: 'sun',
    keywords: 'dark light mode',
    run: () => {
      toggleTheme()
      close()
    },
  },
])

// ---- Search -------------------------------------------------------------------------

const people = ref<PersonEntry[]>([])
const mails = ref<AiHit[]>([])
const foundTasks = ref<Task[]>([])

// "task call Thomas fri" (or todo, tâche): a new task, read like quick add.
const TASK_PREFIX = /^(task|todo|to-do|tâche|tache)\s+/i
const taskText = computed(() => (tasksOn.value && TASK_PREFIX.test(q.value) ? q.value.replace(TASK_PREFIX, '').trim() : ''))
const taskPreview = ref<Parsed | null>(null)
let taskTimer: ReturnType<typeof setTimeout> | undefined
watch(taskText, (text) => {
  clearTimeout(taskTimer)
  if (!text) return void (taskPreview.value = null)
  taskTimer = setTimeout(async () => {
    taskPreview.value = await parseTask(text).catch(() => null)
    // The same task may exist already: look for its title.
    const title = taskPreview.value?.title
    foundTasks.value = title ? await fetchTasks({ q: title, done: false, limit: 4 }).catch(() => []) : []
  }, 150)
})
async function addTask() {
  const text = taskText.value
  if (!text) return
  close()
  try {
    const task = await quickAddTask(text)
    toast.show(`Task added${task.due != null ? ` · ${dueLabel(task)}` : ''}`, {
      action: { label: 'Open', run: () => router.push({ path: '/tasks', query: { task: task.id } }) },
    })
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
// "note Lyon ideas": a new note with that title.
const NOTE_PREFIX = /^(note)\s+/i
const noteText = computed(() => (notesOn.value && NOTE_PREFIX.test(q.value) ? q.value.replace(NOTE_PREFIX, '').trim() : ''))
const foundNotes = ref<NoteSummary[]>([])
const foundFiles = ref<FileItem[]>([])
async function addNote() {
  const title = noteText.value
  if (!title) return
  close()
  try {
    const note = await createNote({ title })
    await router.push({ path: '/notes', query: { note: note.id, edit: 1 } })
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

let seq = 0
let timer: ReturnType<typeof setTimeout> | undefined
watch(q, (text) => {
  clearTimeout(timer)
  const query = text.trim()
  if (query.length < 2) {
    people.value = []
    mails.value = []
    foundTasks.value = []
    foundNotes.value = []
    foundFiles.value = []
    return
  }
  timer = setTimeout(async () => {
    const mine = ++seq
    const typingTask = !!taskText.value
    // Typing "note lyon": look for notes about "lyon".
    const noteQuery = noteText.value || query
    const [p, m, t, n, f] = await Promise.all([
      searchPeople(query, 4).catch(() => []),
      $fetch<AiHit[]>('/api/mail/search', { query: { q: query, limit: 5 } }).catch(() => []),
      tasksOn.value && !typingTask ? fetchTasks({ q: query, done: false, limit: 4 }).catch(() => []) : Promise.resolve(null),
      notesOn.value && noteQuery.length >= 2 ? fetchNotes({ q: noteQuery, limit: 4 }).catch(() => []) : Promise.resolve([]),
      filesOn.value && !typingTask && !noteText.value
        ? fetchFiles({ q: query, limit: 4 }).then((l) => l.files).catch(() => [])
        : Promise.resolve([]),
    ])
    if (mine !== seq) return
    people.value = p
    mails.value = m
    if (t) foundTasks.value = t
    foundNotes.value = n
    foundFiles.value = f
  }, 120)
})

const items = computed<Item[]>(() => {
  const query = q.value.trim().toLowerCase()
  const out: Item[] = []
  const ask: Item | null =
    ai.value.enabled && query
      ? {
          id: 'ask',
          group: 'Ask',
          label: `Ask AI: “${q.value.trim()}”`,
          hint: 'Tab',
          icon: 'sparkle',
          run: () => startChat(q.value.trim()),
        }
      : null
  // "task …" goes first.
  if (taskText.value) {
    const p = taskPreview.value
    out.push({
      id: 'create-task',
      group: 'Create',
      label: `New task “${p?.title ?? taskText.value}”`,
      hint: p?.due != null ? dueLabel(p) : 'No date',
      icon: 'task',
      run: addTask,
    })
  }
  // "note …" too.
  if (noteText.value) {
    out.push({
      id: 'create-note',
      group: 'Create',
      label: `New note “${noteText.value}”`,
      icon: 'note',
      run: addNote,
    })
  }
  // A question goes first.
  if (ask && /\?$|^(what|when|who|where|how|why|find|summari[sz]e|schedule|plan|write|am i|do i|is there|quand|qui|quoi|comment|où|est-ce)\b/.test(query)) out.push(ask)
  const words = query.split(/\s+/).filter(Boolean)
  for (const c of commands.value) {
    const text = `${c.label} ${c.keywords ?? ''}`.toLowerCase()
    if (words.every((w) => text.includes(w))) out.push(c)
  }
  for (const p of people.value) {
    out.push({
      id: `p:${p.email}`,
      group: 'People',
      label: personName(p),
      hint: p.name ? p.email : undefined,
      icon: 'people',
      run: go(`/people/${encodeURIComponent(p.email)}`),
    })
  }
  for (const t of foundTasks.value) {
    out.push({
      id: `t:${t.id}`,
      group: 'Tasks',
      label: t.title,
      hint: isLate(t) ? 'Overdue' : dueLabel(t),
      icon: 'task',
      run: go(`/tasks?task=${t.id}`),
    })
  }
  for (const n of foundNotes.value) {
    out.push({
      id: `n:${n.id}`,
      group: 'Notes',
      label: noteTitle(n),
      hint: n.event ? noteEventLabel(n.event, true) : noteWhen(n.updated_at),
      icon: 'note',
      run: go(`/notes?note=${n.id}`),
    })
  }
  for (const f of foundFiles.value) {
    out.push({
      id: `f:${f.key}`,
      group: 'Files',
      label: f.name,
      hint: `${f.sent ? 'You sent' : f.from || ''} · ${shortDate(f.date)}`,
      icon: 'folder',
      run: () => {
        router.push(filesPageLink(f.key))
        close()
      },
    })
  }
  for (const m of mails.value) {
    out.push({
      id: `m:${m.account}:${m.folder}:${m.uid}`,
      group: 'Mail',
      label: m.subject || '(no subject)',
      hint: personLabel(m.from),
      icon: 'mail',
      run: () => {
        router.push(messageLink(m))
        close()
      },
    })
  }
  if (query) {
    out.push({
      id: 'search-mail',
      group: 'Mail',
      label: `Search mail for “${q.value.trim()}”`,
      icon: 'search',
      run: go(`/mail?q=${encodeURIComponent(q.value.trim())}`),
    })
  }
  if (ask && !out.includes(ask)) out.push(ask)
  return out
})

/**
 * The results as UCommandPalette groups, in the order above. Havre already
 * chose and ordered them (some come from the server), so the palette doesn't
 * filter them again (`ignoreFilter`).
 */
const groups = computed(() => {
  const out: { id: string; label: string; ignoreFilter: true; items: object[] }[] = []
  for (const item of items.value) {
    let g = out[out.length - 1]
    if (!g || g.label !== item.group) out.push((g = { id: `${item.group}-${out.length}`, label: item.group, ignoreFilter: true, items: [] }))
    g.items.push({
      id: item.id,
      label: item.label,
      suffix: item.hint,
      icon: havreIcon(item.icon),
      class: item.id === 'ask' ? 'ask' : undefined,
      onSelect: (e: Event) => {
        e.preventDefault()
        item.run()
      },
    })
  }
  return out
})

// Tab: ask the assistant what's typed.
function onSearchKey(e: KeyboardEvent) {
  if (e.key === 'Tab' && ai.value.enabled && q.value.trim()) {
    e.preventDefault()
    startChat(q.value.trim())
  }
}

// Esc (or the backdrop): back from a chat to the search, otherwise close.
function onOpenChange(value: boolean) {
  if (value) return show()
  if (mode.value === 'chat' && !chatBusy.value) {
    mode.value = 'search'
    nextTick(focusSearch)
  } else close()
}

// ---- The assistant ----------------------------------------------------------------------

const mode = ref<'search' | 'chat'>('search')
interface ChatEntry {
  role: 'user' | 'assistant'
  text: string
  answer?: AiAnswer
  /** The stored turn (for "done" marks). */
  turn?: number
  error?: string
}
/** Chats are kept as conversations: "Continue in the AI app" opens this one. */
const conversation = ref<number | null>(null)
const chat = ref<ChatEntry[]>([])
const chatInput = ref('')
const chatBusy = ref(false)
const chatInputEl = ref<{ textareaRef: HTMLTextAreaElement | null } | null>(null)
const chatLog = ref<HTMLElement | null>(null)

function startChat(question: string) {
  mode.value = 'chat'
  chat.value = []
  conversation.value = null
  send(question)
}

async function send(question: string) {
  if (!question.trim() || chatBusy.value) return
  chat.value.push({ role: 'user', text: question.trim() })
  chatInput.value = ''
  chatBusy.value = true
  scrollDown()
  try {
    const saved = await askInConversation(conversation.value, question.trim())
    conversation.value = saved.id
    const last = saved.turns[saved.turns.length - 1]
    const answer = last?.answer ?? undefined
    chat.value.push({ role: 'assistant', text: last?.text ?? '', answer, turn: last?.id })
  } catch (err) {
    chat.value.push({ role: 'assistant', text: '', error: apiErrorMessage(err) })
  } finally {
    chatBusy.value = false
    scrollDown()
    nextTick(() => (pending.value ? confirmBtn() : chatInputEl.value?.textareaRef)?.focus())
  }
}
function confirmBtn() {
  return document.querySelector<HTMLButtonElement>('.confirmbar .confirm')
}

function scrollDown() {
  nextTick(() => chatLog.value?.scrollTo({ top: chatLog.value.scrollHeight, behavior: 'smooth' }))
}

// Drawing answers and running proposals: shared with the AI app (useAssistant.ts).
const parse = (entry: ChatEntry) => aiAnswerLines(entry.text)
const refLabel = (entry: ChatEntry, id: string) => aiRefLabel(entry.answer, id)
const cited = (entry: ChatEntry) => aiCitedMessages(entry.text, entry.answer)
const citedItems = (entry: ChatEntry) => aiCitedItems(entry.text, entry.answer)
function openRef(entry: ChatEntry, id: string) {
  const m = entry.answer?.messages[id]
  const e = entry.answer?.events[id]
  const to = m ? messageLink(m) : e ? aiEventLink(e) : aiRefLink(entry.answer, id)
  if (to) router.push(to)
  close()
}

// ---- Proposals: what the assistant wants to do, reviewed before running ----------------
// Each answer's proposals show as numbered steps; nothing runs until you click
// "Confirm" (all of them) or "Edit email" (opens the email in compose).
const done = ref(new Set<AiProposal>())
const dismissed = ref(new Set<ChatEntry>())
const running = ref(false)

/** The last answer still waiting for a decision (the confirm bar acts on it). */
const pending = computed(() => {
  const last = chat.value[chat.value.length - 1]
  if (!last || last.role !== 'assistant' || dismissed.value.has(last)) return null
  const left = (last.answer?.proposals ?? []).filter((p) => !done.value.has(p))
  return left.length ? { entry: last, left } : null
})
const pendingEmail = computed(() => pending.value?.left.find((p): p is AiEmailProposal => p.kind === 'email') ?? null)

/** Runs a proposal, and remembers it in the conversation (the AI app shows it done). */
async function run(p: AiProposal, send: boolean) {
  const msg = await runAiProposal(p, send)
  const entry = chat.value.find((c) => c.answer?.proposals.includes(p))
  if (entry?.turn && conversation.value) markProposalDone(conversation.value, entry.turn, entry.answer!.proposals.indexOf(p))
  return msg
}

/** Confirm: runs every step left, in order; stops at the first failure. */
async function confirmAll() {
  const todo = pending.value?.left ?? []
  if (!todo.length || running.value) return
  running.value = true
  const said: string[] = []
  try {
    for (const p of todo) {
      const msg = await run(p, true)
      done.value.add(p)
      if (msg) said.push(msg)
    }
    toast.show(said.join(' · '))
  } catch (err) {
    if (said.length) toast.show(said.join(' · '))
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    running.value = false
  }
}

/** Edit email: the other steps run, the email opens in compose to change and send. */
async function editEmail() {
  const email = pendingEmail.value
  if (!email || running.value) return
  running.value = true
  try {
    for (const p of pending.value!.left) if (p !== email) {
      await run(p, true)
      done.value.add(p)
    }
    await run(email, false)
    done.value.add(email)
    close()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    running.value = false
  }
}

function cancelPlan() {
  if (pending.value) dismissed.value.add(pending.value.entry)
  nextTick(() => chatInputEl.value?.textareaRef?.focus())
}

function onPlanKey(e: KeyboardEvent) {
  if (e.key === 'Enter' && !e.shiftKey && pending.value && !(e.target instanceof HTMLTextAreaElement)) {
    e.preventDefault()
    confirmAll()
  }
}
</script>

<template>
  <UModal
    :open="open"
    title="Command palette"
    :ui="{
      header: 'sr-only',
      body: 'p-0 sm:p-0 flex flex-1 flex-col min-h-0 overflow-hidden',
      // `sm:` too: Nuxt UI's own height limit is an `sm:` class, which would win otherwise.
      content: `max-w-[680px] sm:top-[12vh] sm:translate-y-0 ${mode === 'chat' ? 'h-[70vh] sm:h-[70vh]' : 'max-h-[72vh] sm:max-h-[72vh]'}`,
    }"
    @update:open="onOpenChange"
  >
    <template #body>
      <template v-if="mode === 'search'">
        <UCommandPalette
          ref="input"
          v-model:search-term="q"
          :groups="groups"
          icon="i-havre-search"
          :placeholder="ai.enabled ? 'Go to, search, or ask AI anything…' : 'Go to, search people and mail…'"
          :close="false"
          class="flex-1 min-h-0"
          @keydown="onSearchKey"
        >
          <template #empty>Nothing matches.</template>
        </UCommandPalette>
        <footer class="foot">
          <span><UKbd value="↑" /><UKbd value="↓" /> move</span>
          <span><UKbd value="Enter" /> open</span>
          <span v-if="ai.enabled"><UKbd value="Tab" /> ask AI</span>
          <span class="spacer" />
          <span><UKbd :value="isMac ? '⌘' : 'Ctrl'" /><UKbd value="K" /></span>
        </footer>
      </template>

      <div v-else class="chat" @keydown="onPlanKey">
        <header class="chat-head">
          <UButton
            icon="i-havre-back"
            color="neutral"
            variant="ghost"
            square
            aria-label="Back to search"
            @click="mode = 'search'; nextTick(focusSearch)"
          />
          <HIcon name="sparkle" :size="15" class="spark" />
          <span>Ask AI</span>
          <span class="model">{{ ai.model }}{{ ai.local ? ' · local' : '' }}</span>
          <span class="spacer" />
          <NuxtLink
            v-if="conversation"
            :to="{ path: '/ai', query: { c: conversation } }"
            class="continue"
            @click="close"
          >
            Continue in the AI app →
          </NuxtLink>
          <UButton
            icon="i-havre-close"
            color="neutral"
            variant="ghost"
            square
            type="button"
            aria-label="Close"
            @click="close"
          />
        </header>
        <div ref="chatLog" class="log" aria-live="polite">
          <div v-for="(c, n) in chat" :key="n" class="turn" :class="c.role">
            <p v-if="c.role === 'user'" class="q">{{ c.text }}</p>
            <template v-else>
              <ul v-if="c.answer?.steps.length" class="steps">
                <li v-for="(s, k) in c.answer.steps" :key="k"><HIcon name="check" :size="12" /> {{ s.label }}</li>
              </ul>
              <p v-if="c.error" class="err"><HIcon name="alert" :size="14" /> {{ c.error }}</p>
              <div v-else class="a">
                <p v-for="(line, k) in parse(c)" :key="k" :class="{ bullet: line.bullet }">
                  <template v-for="(part, j) in line.parts" :key="j">
                    <button
                      v-if="part.ref"
                      type="button"
                      class="ref"
                      :title="refLabel(c, part.ref)"
                      @click="openRef(c, part.ref)"
                    >
                      {{ refLabel(c, part.ref) }}
                    </button>
                    <strong v-else-if="part.bold">{{ part.text }}</strong>
                    <template v-else>{{ part.text }}</template>
                  </template>
                </p>
              </div>
              <div v-if="cited(c).length" class="cards">
                <button
                  v-for="x in cited(c)"
                  :key="x.id"
                  type="button"
                  class="card"
                  @click="router.push(messageLink(x.hit!)); close()"
                >
                  <HIcon name="mail" :size="14" />
                  <span class="ct">{{ x.hit!.subject || '(no subject)' }}</span>
                  <span class="cf">{{ personLabel(x.hit!.from) }} · {{ shortDate(x.hit!.date) }}</span>
                </button>
              </div>
              <div v-if="citedItems(c).length" class="cards">
                <button v-for="x in citedItems(c)" :key="x.id" type="button" class="card" @click="router.push(x.to); close()">
                  <HIcon :name="x.kind === 'note' ? 'note' : 'task'" :size="14" />
                  <span class="ct">{{ x.title }}</span>
                  <span class="cf">{{ x.detail }}</span>
                </button>
              </div>
              <AiPlan
                v-if="c.answer?.proposals.length"
                :proposals="c.answer.proposals"
                :done="c.answer.proposals.flatMap((p, k) => (done.has(p) ? [k] : []))"
                :off="dismissed.has(c)"
              />
            </template>
          </div>
          <div v-if="chatBusy" class="turn assistant">
            <p class="thinking"><span class="dot" /><span class="dot" /><span class="dot" /> Looking it up…</p>
          </div>
        </div>
        <div v-if="pending" class="confirmbar">
          <span class="lockline"><HIcon name="lock" :size="13" /> Nothing runs until you confirm · {{ ai.local ? 'local model' : ai.model }}</span>
          <span class="spacer" />
          <UButton color="neutral" variant="ghost" size="sm" :disabled="running" @click="cancelPlan">Cancel</UButton>
          <UButton v-if="pendingEmail" color="neutral" variant="outline" size="sm" :disabled="running" @click="editEmail">
            Edit email
          </UButton>
          <UButton size="sm" class="confirm" :loading="running" :disabled="running" @click="confirmAll">
            {{ pending.left.length > 1 ? (pending.left.length === 2 ? 'Confirm both' : `Confirm all ${pending.left.length}`) : 'Confirm' }}
            <template #trailing><UKbd value="↵" /></template>
          </UButton>
        </div>
        <UChatPrompt
          v-else
          ref="chatInputEl"
          v-model="chatInput"
          placeholder="Ask a follow-up…"
          aria-label="Ask a follow-up"
          :disabled="chatBusy"
          :autofocus="false"
          :maxrows="5"
          variant="outline"
          class="ask"
          @submit="send(chatInput)"
        >
          <UChatPromptSubmit :status="chatBusy ? 'submitted' : 'ready'" :disabled="chatBusy || !chatInput.trim()">
            Ask
          </UChatPromptSubmit>
        </UChatPrompt>
        <p v-if="!pending" class="disclaimer">AI answers can be wrong. Nothing is created, moved or sent until you confirm.</p>
      </div>
    </template>
  </UModal>
</template>

<style scoped>
/* The chat fills the dialog: header, the answers (scrolling), the prompt. */
.chat {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.backdrop {
  position: fixed;
  inset: 0;
  z-index: 80;
  display: flex;
  justify-content: center;
  align-items: flex-start;
  padding: 12vh 16px 16px;
  background: rgb(0 0 0 / 0.4);
}
.foot {
  display: flex;
  gap: 14px;
  padding: 8px 14px;
  border-top: 1px solid var(--line);
  font-size: 12px;
  color: var(--faint);
}
.spacer {
  flex: 1;
}
.chat-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border-bottom: 1px solid var(--line);
  font-weight: 500;
}
.continue {
  font-size: 12.5px;
  text-decoration: none;
  margin-right: 4px;
}
.spark {
  color: var(--accent);
}
.model {
  font-size: 12px;
  font-weight: 400;
  color: var(--faint);
}
.log {
  flex: 1;
  overflow-y: auto;
  padding: 14px 18px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.turn.user {
  align-self: flex-end;
  max-width: 85%;
}
.q {
  margin: 0;
  padding: 8px 12px;
  border-radius: 12px 12px 4px 12px;
  background: var(--sel);
  font-size: 14px;
  white-space: pre-wrap;
}
.steps {
  margin: 0 0 6px;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: 12px;
  color: var(--faint);
}
.steps li {
  display: flex;
  align-items: center;
  gap: 6px;
}
.a p {
  margin: 0 0 6px;
  font-size: 14px;
  line-height: 1.55;
}
.a p.bullet {
  padding-left: 14px;
  position: relative;
}
.a p.bullet::before {
  content: '•';
  position: absolute;
  left: 2px;
  color: var(--accent);
}
.ref {
  display: inline;
  padding: 0 6px;
  margin: 0 1px;
  border-radius: 6px;
  border: 1px solid color-mix(in oklab, var(--accent) 40%, transparent);
  background: color-mix(in oklab, var(--accent) 10%, transparent);
  color: var(--text);
  font: inherit;
  font-size: 13px;
  cursor: pointer;
}
.err {
  display: flex;
  gap: 8px;
  align-items: flex-start;
  margin: 0;
  font-size: 13.5px;
  color: var(--danger);
}
.cards {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 4px;
}
.card {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: 0 8px;
  align-items: center;
  padding: 8px 10px;
  border-radius: 10px;
  border: 1px solid var(--line);
  background: var(--bg);
  color: var(--muted);
  text-align: left;
  cursor: pointer;
}
.card:hover {
  border-color: var(--line2);
}
.ct {
  color: var(--text);
  font-size: 13.5px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.cf {
  grid-column: 2;
  font-size: 12px;
  color: var(--faint);
}
.confirmbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
  padding: 14px 20px;
  border-top: 1px solid var(--line);
  background: var(--panel);
}
.lockline {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12.5px;
  color: var(--faint);
}
.thinking {
  display: flex;
  align-items: center;
  gap: 4px;
  margin: 0;
  font-size: 13px;
  color: var(--faint);
}
.dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--accent);
  animation: blink 1s infinite;
}
.dot:nth-child(2) {
  animation-delay: 0.2s;
}
.dot:nth-child(3) {
  animation-delay: 0.4s;
  margin-right: 6px;
}
@keyframes blink {
  50% {
    opacity: 0.25;
  }
}
.ask {
  width: auto;
  margin: 10px 12px 0;
}
.disclaimer {
  margin: 0;
  padding: 0 14px 8px;
  font-size: 11.5px;
  color: var(--faint);
}
@media (max-width: 720px) {
  /* Key hints are for keyboards. */
  .foot {
    display: none;
  }
}
</style>
