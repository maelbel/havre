<script setup lang="ts">
import type { AiAnswer, AiConversation, AiConversationSummary, AiHit, AiSettings, AiStoredTurn } from '~/composables/useAi'
import type { MessageDetail } from '~/composables/useMail'

// The AI app: your conversations on the left, one conversation in the
// middle, and on the right the email the assistant opened (when you click
// one it cites). It answers with the same tools as Ctrl+K and only
// *proposes* changes: each proposal waits for your Confirm. `?c=…` picks the
// conversation.
useHead({ title: 'AI · Havre' })
const route = useRoute()
const router = useRouter()
const toast = useToasts()
const { status: ai, refresh: refreshAi } = useAiStatus()

// ---- The list -------------------------------------------------------------------
const list = ref<AiConversationSummary[]>([])
const loadingList = ref(true)
async function loadList() {
  try {
    list.value = await fetchConversations()
  } catch {
    list.value = []
  } finally {
    loadingList.value = false
  }
}
onMounted(loadList)

const DAY = 86_400_000
const groups = computed(() => {
  const today = startOfDay(new Date()).getTime()
  const label = (ts: number) => {
    const t = ts * 1000
    if (t >= today) return 'Today'
    if (t >= today - DAY) return 'Yesterday'
    if (t >= today - 6 * DAY) return 'This week'
    return 'Earlier'
  }
  const out: { label: string; items: AiConversationSummary[] }[] = []
  for (const c of list.value) {
    const l = label(c.updated_at)
    let g = out.find((x) => x.label === l)
    if (!g) out.push((g = { label: l, items: [] }))
    g.items.push(c)
  }
  return out
})

// What it may read (from Settings › AI), for the box under the list.
const settings = ref<AiSettings | null>(null)
onMounted(async () => {
  settings.value = await fetchAiSettings().catch(() => null)
})

// ---- The open conversation ------------------------------------------------------
const selectedId = computed(() => (route.query.c ? Number(route.query.c) : null))
const current = ref<AiConversation | null>(null)
/** The question just sent, shown while the answer comes. */
const asking = ref<string | null>(null)
const askError = ref<string | null>(null)
const question = ref('')
const composer = ref<HTMLTextAreaElement | null>(null)
const log = ref<HTMLElement | null>(null)

async function open(id: number | null) {
  askError.value = null
  panel.value = null
  if (id == null) {
    current.value = null
    nextTick(() => composer.value?.focus())
    return
  }
  if (current.value?.id === id) return
  try {
    current.value = await fetchConversation(id)
    scrollDown()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
    router.replace({ query: {} })
  }
}

function pick(id: number) {
  router.replace({ query: { c: id } })
}
/** On a phone: the empty conversation is shown (to type a first question). */
const composing = ref(false)
function newConversation() {
  composing.value = true
  router.replace({ query: {} })
  question.value = ''
  nextTick(() => composer.value?.focus())
}

function scrollDown() {
  nextTick(() => log.value?.scrollTo({ top: log.value.scrollHeight, behavior: 'smooth' }))
}

async function send() {
  const q = question.value.trim()
  if (!q || asking.value) return
  asking.value = q
  askError.value = null
  question.value = ''
  scrollDown()
  try {
    const saved = await askInConversation(current.value?.id ?? null, q)
    current.value = saved
    if (selectedId.value !== saved.id) router.replace({ query: { c: saved.id } })
    loadList()
  } catch (err) {
    askError.value = apiErrorMessage(err)
    // The question comes back to the composer, to try again.
    question.value = q
  } finally {
    asking.value = null
    scrollDown()
    nextTick(() => composer.value?.focus())
  }
}
function onComposerKey(e: KeyboardEvent) {
  if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
    e.preventDefault()
    send()
  }
}

// ---- Title and delete -------------------------------------------------------------
const editingTitle = ref<string | null>(null)
async function saveTitle() {
  const title = editingTitle.value?.trim()
  editingTitle.value = null
  if (!current.value || !title || title === current.value.title) return
  try {
    current.value = await renameConversation(current.value.id, title)
    loadList()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
const confirmDelete = ref(false)
watch(selectedId, () => (confirmDelete.value = false))
async function remove() {
  if (!current.value) return
  if (!confirmDelete.value) {
    confirmDelete.value = true
    return
  }
  try {
    await deleteConversation(current.value.id)
    list.value = list.value.filter((c) => c.id !== current.value?.id)
    current.value = null
    router.replace({ query: {} })
    toast.show('Conversation deleted')
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

const when = (ts: number) => whenShort(ts)

// ---- Answers ----------------------------------------------------------------------
const lines = (t: AiStoredTurn) => aiAnswerLines(t.text)
const cited = (t: AiStoredTurn) => aiCitedMessages(t.text, t.answer)
const citedItems = (t: AiStoredTurn) => aiCitedItems(t.text, t.answer)
function openRef(answer: AiAnswer | null, id: string) {
  const m = answer?.messages[id]
  if (m) return showEmail(m)
  const e = answer?.events[id]
  if (e) return router.push(aiEventLink(e))
  const to = aiRefLink(answer, id)
  if (to) router.push(to)
}

// ---- Proposals: confirmed one answer at a time ----------------------------------------
const running = ref<number | null>(null)
const left = (t: AiStoredTurn) => (t.answer?.proposals ?? []).flatMap((p, k) => (t.done.includes(k) ? [] : [{ p, k }]))
async function confirm(t: AiStoredTurn, editEmail = false) {
  const conv = current.value
  if (!conv || running.value != null) return
  running.value = t.id
  const said: string[] = []
  try {
    for (const { p, k } of left(t)) {
      // "Edit email": the email opens in compose instead of being sent.
      const msg = await runAiProposal(p, !(editEmail && p.kind === 'email'))
      if (msg) said.push(msg)
      await markProposalDone(conv.id, t.id, k)
      t.done = [...t.done, k]
    }
    if (said.length) toast.show(said.join(' · '))
  } catch (err) {
    if (said.length) toast.show(said.join(' · '))
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    running.value = null
  }
}

// ---- The email the assistant opened (right panel) ---------------------------------------
const panel = ref<{ hit: AiHit; message: MessageDetail | null; error: string | null } | null>(null)
async function showEmail(hit: AiHit) {
  panel.value = { hit, message: null, error: null }
  try {
    const message = await fetchMessage(hit.account, hit.folder, hit.uid)
    if (panel.value?.hit === hit) panel.value.message = message
  } catch (err) {
    if (panel.value?.hit === hit) panel.value.error = apiErrorMessage(err)
  }
}

// ---- Not ready yet ------------------------------------------------------------------
async function addApp() {
  try {
    await setAppEnabled('ai', true)
    await refreshAi()
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

// `n`: a new conversation (when not typing).
function onKey(e: KeyboardEvent) {
  const typing = (e.target as HTMLElement)?.closest?.('input, textarea, select, [contenteditable]')
  if (typing || e.ctrlKey || e.metaKey || e.altKey) return
  if (e.key === 'n') {
    e.preventDefault()
    newConversation()
  } else if (e.key === 'Escape' && panel.value) panel.value = null
}
onMounted(() => document.addEventListener('keydown', onKey))
onBeforeUnmount(() => document.removeEventListener('keydown', onKey))

const modelLabel = computed(() => (ai.value.local ? 'local model' : ai.value.model || ''))
const showConversation = computed(() => selectedId.value != null || !!asking.value || composing.value)

// Last, once everything `open` uses exists (`immediate` runs it at once).
watch(selectedId, open, { immediate: true })
</script>

<template>
  <div class="ai-app" :class="{ 'has-conv': showConversation, 'has-panel': !!panel }">
    <SplitGroup id="ai">
    <SplitPane id="conversations" :order="1" :size="248" :min="200" :max="360" collapsible>
    <aside class="side" aria-label="Conversations">
      <div class="side-head">
        <h1 class="serif">AI</h1>
        <span class="mono faint tiny">{{ modelLabel }}</span>
      </div>
      <button type="button" class="new btn-primary" @click="newConversation">
        <HIcon name="plus" :size="15" :stroke-width="2" />
        <span class="grow">New conversation</span>
        <kbd class="mono">N</kbd>
      </button>

      <p v-if="!loadingList && !list.length" class="faint small empty-list">Your conversations will be listed here.</p>
      <template v-for="g in groups" :key="g.label">
        <div class="eyebrow group">{{ g.label }}</div>
        <button
          v-for="c in g.items"
          :key="c.id"
          type="button"
          class="conv"
          :aria-current="c.id === selectedId ? 'true' : undefined"
          @click="pick(c.id)"
        >
          <span class="ct">{{ c.title }}</span>
          <span class="cm">{{ c.last || when(c.updated_at) }}</span>
        </button>
      </template>

      <span class="spacer" />

      <section v-if="settings" class="can" aria-label="What the assistant can do">
        <div class="eyebrow">What it can do</div>
        <div class="can-row" :class="{ off: !settings.read_mail }">
          <HIcon :name="settings.read_mail ? 'check' : 'minus'" :size="14" /><span class="grow">Read your mail</span>
        </div>
        <div class="can-row" :class="{ off: !settings.read_mail }">
          <HIcon :name="settings.read_mail ? 'check' : 'minus'" :size="14" /><span class="grow">Write emails</span><span class="faint tiny">you confirm</span>
        </div>
        <div class="can-row" :class="{ off: !settings.read_agenda }">
          <HIcon :name="settings.read_agenda ? 'check' : 'minus'" :size="14" /><span class="grow">Agenda</span><span class="faint tiny">propose</span>
        </div>
        <div class="can-row" :class="{ off: !settings.read_people }">
          <HIcon :name="settings.read_people ? 'check' : 'minus'" :size="14" /><span class="grow">People</span>
        </div>
        <!-- Notes and Tasks: only when their app is in the dock. -->
        <div v-if="settings.notes_app" class="can-row" :class="{ off: !settings.read_notes }">
          <HIcon :name="settings.read_notes ? 'check' : 'minus'" :size="14" /><span class="grow">Notes</span><span class="faint tiny">propose</span>
        </div>
        <div v-if="settings.tasks_app" class="can-row" :class="{ off: !settings.read_tasks }">
          <HIcon :name="settings.read_tasks ? 'check' : 'minus'" :size="14" /><span class="grow">Tasks</span><span class="faint tiny">propose</span>
        </div>
        <div class="can-row off"><HIcon name="minus" :size="14" /><span class="grow">Attachments</span><span class="tiny">off</span></div>
        <NuxtLink to="/settings/ai" class="small to-settings">AI settings →</NuxtLink>
      </section>
    </aside>
    </SplitPane>

    <SplitHandle label="Resize the conversations" fold="conversations" show-label="Show conversations" />
    <SplitPane id="conversation" :order="2" :min="420">
    <main class="conv-main" aria-label="Conversation">
      <!-- Not in the dock, or no model yet -->
      <div v-if="!ai.installed" class="setup">
        <HIcon name="sparkle" :size="26" />
        <h2 class="serif">The AI app isn’t in your dock</h2>
        <p>Add it to ask about your mail and your days, and to get the brief, summaries and drafts in the other apps.</p>
        <button type="button" class="btn btn-primary" @click="addApp">Add AI to your dock</button>
      </div>
      <div v-else-if="!ai.enabled" class="setup">
        <HIcon name="sparkle" :size="26" />
        <h2 class="serif">Choose where the model runs</h2>
        <p>On this server with Ollama (nothing leaves your network), with your Claude plan, or with a provider you choose.</p>
        <NuxtLink to="/settings/ai" class="btn btn-primary">Set up AI</NuxtLink>
      </div>

      <template v-else>
        <header class="bar">
          <button type="button" class="icon-btn back" aria-label="Back to conversations" @click="composing = false; router.replace({ query: {} })">
            <HIcon name="back" :size="18" />
          </button>
          <template v-if="current">
            <input
              v-if="editingTitle !== null"
              v-model="editingTitle"
              class="title-input"
              aria-label="Conversation title"
              autofocus
              @keydown.enter.prevent="saveTitle"
              @keydown.esc="editingTitle = null"
              @blur="saveTitle"
            />
            <button v-else type="button" class="title" title="Rename" @click="editingTitle = current.title">{{ current.title }}</button>
            <span class="mono faint tiny when">{{ when(current.updated_at) }}</span>
            <button
              type="button"
              class="icon-btn"
              :class="{ danger: confirmDelete }"
              :aria-label="confirmDelete ? 'Click again to delete the conversation' : 'Delete conversation'"
              @click="remove"
            >
              <HIcon name="trash" :size="16" />
            </button>
          </template>
          <h2 v-else class="title plain">New conversation</h2>
        </header>

        <div ref="log" class="log">
          <div class="col">
            <div v-if="!current && !asking" class="hello">
              <h2 class="serif">What would you like to know?</h2>
              <div class="ideas">
                <button type="button" @click="question = 'What needs an answer from me today?'; send()">What needs an answer from me today?</button>
                <button type="button" @click="question = 'What’s left this week?'; send()">What’s left this week?</button>
                <button type="button" @click="question = 'Find a free hour next week for a 30-minute call'; send()">Find a free hour next week</button>
              </div>
            </div>

            <template v-for="t in current?.turns ?? []" :key="t.id">
              <div v-if="t.role === 'user'" class="q">{{ t.text }}</div>
              <div v-else class="a">
                <ul v-if="t.answer?.steps.length" class="steps">
                  <li v-for="(s, k) in t.answer.steps" :key="k"><HIcon name="check" :size="12" /> {{ s.label }}</li>
                </ul>
                <div class="text">
                  <p v-for="(line, k) in lines(t)" :key="k" :class="{ bullet: line.bullet }">
                    <template v-for="(part, j) in line.parts" :key="j">
                      <button v-if="part.ref" type="button" class="ref" @click="openRef(t.answer, part.ref)">
                        {{ aiRefLabel(t.answer, part.ref) }}
                      </button>
                      <strong v-else-if="part.bold">{{ part.text }}</strong>
                      <template v-else>{{ part.text }}</template>
                    </template>
                  </p>
                </div>
                <div v-if="cited(t).length" class="cards">
                  <button
                    v-for="x in cited(t)"
                    :key="x.id"
                    type="button"
                    class="card"
                    :class="{ on: panel?.hit === x.hit }"
                    @click="showEmail(x.hit)"
                  >
                    <HIcon name="mail" :size="15" />
                    <span class="cbody">
                      <span class="cs">{{ x.hit.subject || '(no subject)' }}</span>
                      <span class="cf">{{ personLabel(x.hit.from) }} · {{ shortDate(x.hit.date) }}</span>
                    </span>
                    <span class="hint">{{ panel?.hit === x.hit ? 'Shown on the right' : 'Open' }}</span>
                  </button>
                </div>
                <div v-if="citedItems(t).length" class="cards">
                  <NuxtLink v-for="x in citedItems(t)" :key="x.id" :to="x.to" class="card" :class="x.kind">
                    <HIcon :name="x.kind === 'note' ? 'note' : 'task'" :size="15" />
                    <span class="cbody">
                      <span class="cs">{{ x.title }}</span>
                      <span class="cf">{{ x.detail }}</span>
                    </span>
                    <span class="hint">Open</span>
                  </NuxtLink>
                </div>
                <template v-if="t.answer?.proposals.length">
                  <AiPlan :proposals="t.answer.proposals" :done="t.done" />
                  <div v-if="left(t).length" class="confirm-bar">
                    <span class="lock"><HIcon name="lock" :size="13" /> Nothing runs until you confirm</span>
                    <span class="grow" />
                    <button
                      v-if="left(t).some((x) => x.p.kind === 'email')"
                      type="button"
                      class="btn small"
                      :disabled="running != null"
                      @click="confirm(t, true)"
                    >
                      Edit email
                    </button>
                    <button type="button" class="btn btn-primary small" :disabled="running != null" @click="confirm(t)">
                      {{ running === t.id ? 'Running…' : left(t).length > 1 ? `Confirm all ${left(t).length}` : 'Confirm' }}
                    </button>
                  </div>
                </template>
              </div>
            </template>

            <template v-if="asking">
              <div class="q">{{ asking }}</div>
              <p class="thinking"><span class="dot" /><span class="dot" /><span class="dot" /> Looking it up…</p>
            </template>
            <p v-if="askError" class="err" role="alert"><HIcon name="alert" :size="14" /> {{ askError }}</p>
          </div>
        </div>

        <div class="composer">
          <div class="col">
            <label class="box">
              <span class="sr-only">Ask the assistant</span>
              <textarea
                ref="composer"
                v-model="question"
                rows="1"
                placeholder="Ask about your mail, agenda or people…"
                :disabled="!!asking"
                @keydown="onComposerKey"
              />
              <button type="button" class="send btn-primary" aria-label="Send question" :disabled="!!asking || !question.trim()" @click="send">
                <HIcon name="send" :size="16" />
              </button>
            </label>
            <div class="under">
              <span>Answers can be wrong. Emails it reads are treated as data, never as instructions.</span>
              <span class="mono">Ctrl K · quick question</span>
            </div>
          </div>
        </div>
      </template>
    </main>
    </SplitPane>

    <!-- The email the assistant opened -->
    <SplitHandle v-if="panel" label="Resize the opened email" />
    <SplitPane v-if="panel" id="email" :order="3" :size="400" :min="320" :max="720">
    <aside class="panel" aria-label="Opened email">
      <header class="bar panel-bar">
        <span class="badge"><HIcon name="sparkle" :size="12" /> Opened by the assistant</span>
        <span class="grow-x" />
        <NuxtLink :to="messageLink(panel.hit)" class="small">Open in Mail →</NuxtLink>
        <button type="button" class="icon-btn" aria-label="Close email" @click="panel = null">
          <HIcon name="close" :size="16" />
        </button>
      </header>
      <div class="panel-body">
        <h3 class="serif">{{ panel.hit.subject || '(no subject)' }}</h3>
        <div class="from">
          <span class="avatar" :style="{ '--c': personColor(personLabel(panel.hit.from)) }" aria-hidden="true">
            {{ initials(personLabel(panel.hit.from)) }}
          </span>
          <div>
            <div class="strong">{{ personLabel(panel.hit.from) }}</div>
            <div class="faint small">{{ shortDate(panel.hit.date) }}</div>
          </div>
        </div>
        <p v-if="panel.error" class="err"><HIcon name="alert" :size="14" /> {{ panel.error }}</p>
        <p v-else-if="!panel.message" class="faint">Loading…</p>
        <MailFrame v-else-if="panel.message.html" :html="panel.message.html" :allow-remote-images="false" class="frame" />
        <pre v-else class="plain-text">{{ panel.message.text }}</pre>
      </div>
    </aside>
    </SplitPane>
    </SplitGroup>
  </div>
</template>

<style scoped>
.ai-app {
  flex: 1;
  display: flex;
  min-width: 0;
}
.side {
  flex: 0 0 248px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 20px 12px;
  border-right: 1px solid var(--line);
  overflow-y: auto;
  box-sizing: border-box;
}
.side-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  padding: 0 8px 14px;
}
h1 {
  margin: 0;
  font-size: 28px;
}
.tiny {
  font-size: 11.5px;
}
.faint {
  color: var(--faint);
}
.small {
  font-size: 12.5px;
}
.new {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 40px;
  padding: 0 12px;
  margin-bottom: 12px;
  border-radius: 10px;
  font-weight: 500;
}
.new kbd {
  font-size: 11px;
  padding: 1px 6px;
  border-radius: 5px;
  border: 1px solid color-mix(in oklab, var(--on-accent) 30%, transparent);
}
.grow {
  flex: 1;
  text-align: left;
}
.empty-list {
  padding: 8px 10px;
  margin: 0;
}
.group {
  padding: 10px 8px 6px;
}
.conv {
  display: flex;
  flex-direction: column;
  gap: 1px;
  width: 100%;
  padding: 8px 10px;
  border: 0;
  border-radius: 9px;
  background: transparent;
  text-align: left;
  cursor: pointer;
  color: var(--text);
}
.conv:hover {
  background: var(--hover);
}
.conv[aria-current='true'] {
  background: var(--sel);
}
.ct,
.cm {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.ct {
  font-size: 13.5px;
}
.cm {
  font-size: 12px;
  color: var(--faint);
}
.spacer {
  flex: 1;
  min-height: 16px;
}
.can {
  padding: 12px;
  border-radius: 12px;
  border: 1px solid var(--line);
  background: var(--panel);
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.can-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12.5px;
}
.can-row :deep(svg) {
  color: var(--ok);
}
.can-row.off,
.can-row.off :deep(svg) {
  color: var(--faint);
}
.to-settings {
  text-decoration: none;
  margin-top: 2px;
}

/* The conversation */
.conv-main {
  flex: 999 1 480px;
  min-width: 0;
  display: flex;
  flex-direction: column;
  background: var(--panel);
}
.bar {
  display: flex;
  align-items: center;
  gap: 12px;
  height: 56px;
  padding: 0 16px 0 24px;
  border-bottom: 1px solid var(--line);
  flex: none;
  box-sizing: border-box;
}
.back {
  display: none;
}
.title {
  flex: 1;
  min-width: 0;
  padding: 0;
  border: 0;
  background: none;
  color: var(--text);
  font-size: 15px;
  font-weight: 600;
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  cursor: text;
  margin: 0;
}
.title.plain {
  cursor: default;
}
.title-input {
  flex: 1;
  min-width: 0;
  height: 32px;
  padding: 0 8px;
  border: 1px solid var(--accent);
  border-radius: 8px;
  background: var(--bg);
  color: var(--text);
  font-size: 15px;
  font-weight: 600;
  outline: none;
}
.icon-btn.danger {
  color: var(--danger);
  background: color-mix(in oklab, var(--danger) 12%, transparent);
}
.log {
  flex: 1;
  overflow-y: auto;
}
.col {
  max-width: 720px;
  margin: 0 auto;
  padding: 28px 32px 24px;
  display: flex;
  flex-direction: column;
  gap: 22px;
  box-sizing: border-box;
}
.hello {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding-top: 40px;
}
.hello h2 {
  margin: 0;
  font-weight: 400;
  font-size: 30px;
  letter-spacing: -0.01em;
}
.ideas {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.ideas button {
  height: 36px;
  padding: 0 14px;
  border: 1px solid var(--line2);
  border-radius: 999px;
  background: transparent;
  color: var(--text);
  font-size: 13.5px;
  cursor: pointer;
}
.ideas button:hover {
  background: var(--hover);
}
.q {
  align-self: flex-end;
  max-width: 80%;
  padding: 10px 14px;
  border-radius: 14px 14px 4px 14px;
  background: var(--sel);
  font-size: 14.5px;
  white-space: pre-wrap;
}
.a {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.steps {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 3px;
  font-size: 12.5px;
  color: var(--faint);
}
.steps li {
  display: flex;
  align-items: center;
  gap: 6px;
}
.text p {
  margin: 0 0 6px;
  font-size: 15px;
  line-height: 1.6;
}
.text p.bullet {
  position: relative;
  padding-left: 18px;
}
.text p.bullet::before {
  content: '';
  position: absolute;
  left: 4px;
  top: 10px;
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--faint);
}
.ref {
  display: inline;
  padding: 0 4px;
  border: 0;
  border-radius: 5px;
  background: color-mix(in oklab, var(--accent) 14%, transparent);
  color: var(--accent);
  font: inherit;
  cursor: pointer;
}
.cards {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.card {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border-radius: 10px;
  border: 1px solid var(--line);
  background: var(--bg);
  color: var(--text);
  text-align: left;
  cursor: pointer;
}
.card:hover {
  border-color: var(--line2);
}
.card.on {
  border-color: var(--accent);
}
.card :deep(svg) {
  color: var(--c-work);
  flex: none;
}
a.card {
  text-decoration: none;
}
.card.note :deep(svg) {
  color: var(--c-personal);
}
.card.task :deep(svg) {
  color: var(--c-hol);
}
.cbody {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.cs {
  font-size: 13.5px;
  font-weight: 500;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.cf {
  font-size: 12px;
  color: var(--faint);
}
.hint {
  font-size: 12px;
  color: var(--accent);
  flex: none;
}
.confirm-bar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  padding: 10px 12px;
  border-radius: 12px;
  border: 1px solid var(--line);
  background: var(--bg);
}
.lock {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 12.5px;
  color: var(--faint);
}
.thinking {
  display: flex;
  align-items: center;
  gap: 5px;
  margin: 0;
  color: var(--faint);
  font-size: 13.5px;
}
.dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent);
  animation: blink 1.2s infinite ease-in-out;
}
.dot:nth-child(2) {
  animation-delay: 0.15s;
}
.dot:nth-child(3) {
  animation-delay: 0.3s;
  margin-right: 6px;
}
@keyframes blink {
  0%,
  80%,
  100% {
    opacity: 0.25;
  }
  40% {
    opacity: 1;
  }
}
.err {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 0;
  color: var(--danger);
  font-size: 13.5px;
}
.composer {
  flex: none;
}
.composer .col {
  padding: 0 32px 22px;
  gap: 8px;
}
.box {
  display: flex;
  align-items: flex-end;
  gap: 10px;
  padding: 10px 10px 10px 16px;
  border: 1px solid var(--line2);
  border-radius: 14px;
  background: var(--bg);
}
.box:focus-within {
  border-color: var(--accent);
}
.box textarea {
  flex: 1;
  min-width: 0;
  min-height: 26px;
  max-height: 200px;
  padding: 4px 0;
  border: 0;
  outline: none;
  resize: none;
  background: transparent;
  color: var(--text);
  font: inherit;
  font-size: 14.5px;
  field-sizing: content;
}
.send {
  width: 34px;
  height: 34px;
  border-radius: 9px;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  flex: none;
}
.send:disabled {
  opacity: 0.5;
}
.under {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  padding: 0 4px;
  font-size: 12px;
  color: var(--faint);
}
.setup {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  padding: 32px;
  text-align: center;
  color: var(--muted);
}
.setup :deep(svg) {
  color: var(--accent);
}
.setup h2 {
  margin: 0;
  font-weight: 400;
  font-size: 28px;
  color: var(--text);
}
.setup p {
  margin: 0 0 6px;
  max-width: 440px;
  line-height: 1.6;
}
.setup a.btn {
  text-decoration: none;
}

/* The opened email */
.panel {
  flex: 0 1 400px;
  min-width: 300px;
  display: flex;
  flex-direction: column;
  border-left: 1px solid var(--line);
  background: var(--bg);
}
.panel-bar {
  gap: 8px;
  padding: 0 12px 0 20px;
}
.panel-bar a {
  text-decoration: none;
}
.badge {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 3px 9px 3px 7px;
  border-radius: 999px;
  font-size: 12px;
  background: color-mix(in oklab, var(--accent) 14%, transparent);
  color: var(--accent);
}
.grow-x {
  flex: 1;
}
.panel-body {
  flex: 1;
  overflow-y: auto;
  padding: 24px 24px 28px;
  display: flex;
  flex-direction: column;
  gap: 18px;
}
.panel-body h3 {
  margin: 0;
  font-weight: 400;
  font-size: 24px;
  line-height: 1.2;
  letter-spacing: -0.01em;
}
.from {
  display: flex;
  align-items: center;
  gap: 10px;
}
.avatar {
  flex: 0 0 32px;
  height: 32px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 11.5px;
  font-weight: 600;
  background: color-mix(in oklab, var(--c) 20%, transparent);
  color: var(--c);
}
.strong {
  font-weight: 600;
  font-size: 13.5px;
}
.frame {
  border-radius: 10px;
  overflow: hidden;
}
.plain-text {
  margin: 0;
  font: inherit;
  font-size: 14px;
  line-height: 1.65;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

/* Narrower: the email covers the conversation. */
@media (max-width: 1100px) {
  .panel {
    position: fixed;
    z-index: 30;
    right: 0;
    top: 0;
    bottom: 0;
    width: min(420px, 100vw);
    box-shadow: -20px 0 60px -20px rgb(0 0 0 / 0.5);
  }
}

/* Phones: the list, or one conversation full screen. */
@media (max-width: 720px) {
  .ai-app {
    flex-direction: column;
    min-height: 0;
  }
  .side {
    flex: 1;
    border-right: 0;
  }
  .can {
    display: none;
  }
  .conv-main {
    display: none;
  }
  .has-conv .side {
    display: none;
  }
  .has-conv .conv-main {
    display: flex;
    flex: 1;
    min-height: 0;
  }
  .back {
    display: inline-flex;
  }
  .bar {
    padding: 0 8px;
  }
  .col {
    padding: 18px 16px;
  }
  .composer .col {
    padding: 0 12px 12px;
  }
  .under span:last-child {
    display: none;
  }
}
</style>
