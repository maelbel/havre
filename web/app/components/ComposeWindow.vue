<script setup lang="ts">
import type { Draft, DraftChanges } from '~/composables/useCompose'

// The compose window, docked bottom-right (full screen on phones). It edits
// the draft in `useComposer()`; every change is saved after a short pause.
const composer = useComposer()
const accounts = useAccounts()
const toast = useToasts()

const draft = ref<Draft | null>(null)
const form = reactive({ account: 0, to: '', cc: '', bcc: '', subject: '', body: '' })
const showCc = ref(false)
const showBcc = ref(false)
const loading = ref(false)
const sending = ref(false)
const error = ref<string | null>(null)
const status = ref<'saved' | 'saving' | 'idle'>('idle')
const confirmDiscard = ref(false)
const dragging = ref(false)
const uploading = ref(0)

const toInput = ref<{ focus: () => void } | null>(null)
const bodyInput = ref<HTMLTextAreaElement | null>(null)
const fileInput = ref<HTMLInputElement | null>(null)

const account = computed(() => accounts.data.value.find((a) => a.id === form.account))
const title = computed(() => {
  if (form.subject.trim()) return form.subject.trim()
  return { new: 'New message', reply: 'Reply', reply_all: 'Reply all', forward: 'Forward' }[draft.value?.kind ?? 'new']
})

// ---- Autosave -----------------------------------------------------------------

let saved = { ...form }
let saveTimer: ReturnType<typeof setTimeout> | undefined
let saving: Promise<unknown> = Promise.resolve()

function changes(): DraftChanges {
  const out: DraftChanges = {}
  for (const key of ['to', 'cc', 'bcc', 'subject', 'body'] as const) {
    if (form[key] !== saved[key]) out[key] = form[key]
  }
  if (form.account !== saved.account) out.account = form.account
  return out
}

/** Saves now what changed (waits for a save in progress first). */
async function flush() {
  clearTimeout(saveTimer)
  await saving
  const id = draft.value?.id
  const diff = changes()
  if (!id || !Object.keys(diff).length) return
  const snapshot = { ...form }
  status.value = 'saving'
  saving = saveDraft(id, diff)
    .then(() => {
      saved = snapshot
      status.value = 'saved'
    })
    .catch((err) => {
      status.value = 'idle'
      error.value = apiErrorMessage(err)
    })
  await saving
}

// ---- Loading ------------------------------------------------------------------

let loadedId = 0
watch(
  () => composer.value?.id,
  async (id) => {
    if (!id || id === loadedId) return
    await flush()
    loadedId = id
    loading.value = true
    error.value = null
    confirmDiscard.value = false
    try {
      const d = await fetchDraft(id)
      draft.value = d
      Object.assign(form, { account: d.account, to: d.to, cc: d.cc, bcc: d.bcc, subject: d.subject, body: d.body })
      showCc.value = !!d.cc
      showBcc.value = !!d.bcc
      saved = { ...form }
      status.value = 'idle'
      loading.value = false
      await nextTick()
      // New message or forward: start with the recipients. Reply: type above the quote.
      if (!d.to) toInput.value?.focus()
      else if (bodyInput.value) {
        bodyInput.value.focus()
        bodyInput.value.setSelectionRange(0, 0)
        bodyInput.value.scrollTop = 0
      }
      // "Draft a reply" (from the summary): the AI writes it right away.
      if (composer.value?.ai) {
        composer.value = { ...composer.value, ai: false }
        writeWithAi()
      }
    } catch (err) {
      error.value = apiErrorMessage(err)
    } finally {
      loading.value = false
    }
  },
  { immediate: true },
)

watch(form, () => {
  if (loading.value || !draft.value) return
  clearTimeout(saveTimer)
  saveTimer = setTimeout(flush, 700)
})

// ---- Write with AI ------------------------------------------------------------
// A short instruction ("yes for Thursday, ask for the agenda") → the AI
// writes the text above the quoted message. Undo puts back what was there.
const { status: ai } = useAiStatus()
const aiOpen = ref(false)
const aiInstruction = ref('')
const aiBusy = ref(false)
const aiInput = ref<HTMLInputElement | null>(null)
const aiUndo = ref<string | null>(null)
let aiAbort: AbortController | null = null

/** Where the quoted original (or forwarded message) starts in the body. */
function quoteStart(body: string) {
  const m = body.match(/^(On .+ wrote:|>|-{5,} ?Forwarded message)/m)
  return m?.index ?? body.length
}

function openAi() {
  aiOpen.value = !aiOpen.value
  if (aiOpen.value) nextTick(() => aiInput.value?.focus())
}

async function writeWithAi() {
  if (!draft.value || aiBusy.value) return
  aiAbort = new AbortController()
  aiBusy.value = true
  error.value = null
  const before = form.body
  const quote = before.slice(quoteStart(before))
  try {
    await streamAi(
      '/api/ai/draft',
      { draft: draft.value.id, instruction: aiInstruction.value, body: before },
      (text) => {
        const { text: clean, warning } = splitWarning(text)
        form.body = clean.trim() + (quote ? `\n\n${quote}` : '')
        if (warning) error.value = warning
      },
      aiAbort.signal,
    )
    aiUndo.value = before
    aiOpen.value = false
    aiInstruction.value = ''
    nextTick(() => bodyInput.value?.focus())
  } catch (err) {
    if ((err as Error).name === 'AbortError') form.body = before
    else error.value = (err as Error).message
  } finally {
    aiBusy.value = false
  }
}

function undoAi() {
  if (aiUndo.value === null) return
  form.body = aiUndo.value
  aiUndo.value = null
}
// Typing after the AI wrote: the undo no longer applies.
watch(
  () => form.body,
  () => {
    if (!aiBusy.value && aiUndo.value !== null && document.activeElement === bodyInput.value) aiUndo.value = null
  },
)

function onAiKey(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    e.stopPropagation()
    if (aiBusy.value) aiAbort?.abort()
    else aiOpen.value = false
  }
}

// ---- Actions ------------------------------------------------------------------

async function send() {
  if (!draft.value || sending.value) return
  error.value = null
  sending.value = true
  try {
    await flush()
    const outcome = await sendDraft(draft.value.id)
    toast.show(outcome.warning ?? 'Message sent', outcome.warning ? { tone: 'error' } : undefined)
    finish()
    refreshNuxtData('drafts')
  } catch (err) {
    error.value = apiErrorMessage(err)
  } finally {
    sending.value = false
  }
}

/** Closes the window; the draft stays (in "Drafts"). */
async function close() {
  await flush()
  const empty = !form.to.trim() && !form.subject.trim() && !form.body.trim() && !draft.value?.attachments.length
  if (draft.value && empty) await discardDraft(draft.value.id).catch(() => {})
  else if (draft.value) toast.show('Saved in Drafts')
  finish()
  refreshNuxtData('drafts')
}

async function discard() {
  if (!confirmDiscard.value) {
    confirmDiscard.value = true
    setTimeout(() => (confirmDiscard.value = false), 3000)
    return
  }
  clearTimeout(saveTimer)
  if (draft.value) await discardDraft(draft.value.id).catch(() => {})
  finish()
  toast.show('Draft discarded')
}

function finish() {
  clearTimeout(saveTimer)
  composer.value = null
  draft.value = null
  loadedId = 0
}

function toggleMinimize() {
  if (composer.value) composer.value = { ...composer.value, minimized: !composer.value.minimized }
}

// ---- Attachments --------------------------------------------------------------

async function addFiles(files: FileList | File[] | null) {
  if (!files || !draft.value) return
  for (const file of Array.from(files)) {
    uploading.value++
    try {
      const added = await attachFile(draft.value.id, file)
      draft.value.attachments.push(added)
    } catch (err) {
      toast.show(`${file.name}: ${apiErrorMessage(err)}`, { tone: 'error' })
    } finally {
      uploading.value--
    }
  }
  if (fileInput.value) fileInput.value.value = ''
}

async function dropAttachment(id: number) {
  if (!draft.value) return
  await removeAttachment(draft.value.id, id).catch(() => {})
  draft.value.attachments = draft.value.attachments.filter((a) => a.id !== id)
}

function onDrop(e: DragEvent) {
  dragging.value = false
  addFiles(e.dataTransfer?.files ?? null)
}

function size(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

// ---- Keyboard -----------------------------------------------------------------

function onKeydown(e: KeyboardEvent) {
  if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
    e.preventDefault()
    send()
  } else if (e.key === 'Escape') {
    e.preventDefault()
    close()
  }
}

// Leaving the page: save what's left.
function onUnload() {
  const id = draft.value?.id
  const diff = changes()
  if (id && Object.keys(diff).length) {
    fetch(`/api/drafts/${id}`, {
      method: 'PATCH',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(diff),
      keepalive: true,
    })
  }
}
onMounted(() => window.addEventListener('pagehide', onUnload))
onBeforeUnmount(() => window.removeEventListener('pagehide', onUnload))

const isMac = import.meta.client && /Mac|iPhone|iPad/.test(navigator.platform)
</script>

<template>
  <section
    v-if="composer"
    class="composer"
    :class="{ minimized: composer.minimized, dragging }"
    role="dialog"
    aria-label="Write a message"
    @keydown="onKeydown"
    @dragover.prevent="dragging = true"
    @dragleave.self="dragging = false"
    @drop.prevent="onDrop"
  >
    <header @click="composer.minimized && toggleMinimize()">
      <h2>{{ title }}</h2>
      <span v-if="status === 'saving'" class="status">Saving…</span>
      <span v-else-if="status === 'saved'" class="status">Saved</span>
      <button
        type="button"
        class="icon-btn small"
        :aria-label="composer.minimized ? 'Open' : 'Minimize'"
        @click.stop="toggleMinimize"
      >
        <HIcon :name="composer.minimized ? 'chevron' : 'minus'" :size="16" />
      </button>
      <button type="button" class="icon-btn small" aria-label="Close (kept in Drafts)" @click.stop="close">
        <HIcon name="close" :size="16" />
      </button>
    </header>

    <template v-if="!composer.minimized">
      <div v-if="loading" class="loading"><span class="spinner" aria-hidden="true" /></div>
      <form v-else class="body" @submit.prevent="send">
        <div class="row">
          <label for="compose-from">From</label>
          <select id="compose-from" v-model.number="form.account">
            <option v-for="a in accounts.data.value" :key="a.id" :value="a.id">
              {{ a.sender_name ? `${a.sender_name} <${a.email}>` : a.email }}
            </option>
          </select>
        </div>
        <p v-if="account && !account.can_send" class="warn">
          This account must be reconnected before it can send (Microsoft asks to allow sending).
          <NuxtLink to="/settings">Settings › Reconnect</NuxtLink>
        </p>

        <RecipientInput ref="toInput" v-model="form.to" label="To">
          <span class="cc-toggles">
            <button v-if="!showCc" type="button" class="link" @click="showCc = true">Cc</button>
            <button v-if="!showBcc" type="button" class="link" @click="showBcc = true">Bcc</button>
          </span>
        </RecipientInput>
        <RecipientInput v-if="showCc" v-model="form.cc" label="Cc" />
        <RecipientInput v-if="showBcc" v-model="form.bcc" label="Bcc" />

        <div class="row">
          <label for="compose-subject" class="sr-only">Subject</label>
          <input id="compose-subject" v-model="form.subject" type="text" class="subject" placeholder="Subject" />
        </div>

        <div v-if="aiOpen" class="ai-bar" @keydown="onAiKey">
          <HIcon name="sparkle" :size="15" />
          <input
            ref="aiInput"
            v-model="aiInstruction"
            type="text"
            aria-label="What should the AI write?"
            :placeholder="draft?.kind === 'new' ? 'What is the message about?' : 'What do you want to answer? (e.g. yes for Thursday)'"
            :disabled="aiBusy"
            @keydown.enter.prevent="writeWithAi"
          />
          <button v-if="aiBusy" type="button" class="btn small" @click="aiAbort?.abort()">Stop</button>
          <button v-else type="button" class="btn btn-primary small" @click="writeWithAi">Write</button>
        </div>
        <p v-if="aiUndo !== null" class="ai-undo">
          Written by AI — check it before sending.
          <button type="button" class="link" @click="undoAi">Undo</button>
        </p>

        <textarea
          ref="bodyInput"
          v-model="form.body"
          class="text"
          aria-label="Message text"
          placeholder="Write your message…"
          spellcheck="true"
        />

        <ul v-if="draft?.attachments.length || uploading" class="files">
          <li v-for="a in draft?.attachments" :key="a.id" class="file">
            <HIcon name="paperclip" :size="14" />
            <span class="fname">{{ a.filename }}</span>
            <span class="fsize">{{ size(a.size) }}</span>
            <button type="button" class="remove" :aria-label="`Remove ${a.filename}`" @click="dropAttachment(a.id)">
              <HIcon name="close" :size="13" />
            </button>
          </li>
          <li v-if="uploading" class="file uploading"><span class="spinner small" aria-hidden="true" /> Adding…</li>
        </ul>

        <div v-if="error" class="error" role="alert">
          <HIcon name="alert" :size="16" />
          <span>{{ error }}</span>
        </div>

        <footer>
          <button type="submit" class="btn btn-primary" :disabled="sending || (account && !account.can_send)">
            <span v-if="sending" class="spinner on-accent" aria-hidden="true" />
            <HIcon v-else name="send" :size="15" />
            {{ sending ? 'Sending…' : 'Send' }}
          </button>
          <kbd class="hint mono">{{ isMac ? '⌘' : 'Ctrl' }} Enter</kbd>
          <button type="button" class="icon-btn" aria-label="Attach files" title="Attach files" @click="fileInput?.click()">
            <HIcon name="paperclip" :size="17" />
          </button>
          <button
            v-if="ai.enabled"
            type="button"
            class="icon-btn"
            :class="{ on: aiOpen }"
            aria-label="Write with AI"
            title="Write with AI"
            @click="openAi"
          >
            <HIcon name="sparkle" :size="17" />
          </button>
          <input ref="fileInput" type="file" multiple hidden @change="addFiles(($event.target as HTMLInputElement).files)" />
          <span class="spacer" />
          <button
            type="button"
            class="icon-btn"
            :class="{ danger: confirmDiscard }"
            :aria-label="confirmDiscard ? 'Click again to discard' : 'Discard draft'"
            :title="confirmDiscard ? 'Click again to discard' : 'Discard draft'"
            @click="discard"
          >
            <HIcon name="trash" :size="17" />
            <span v-if="confirmDiscard" class="confirm">Discard?</span>
          </button>
        </footer>
      </form>
      <div v-if="dragging" class="dropzone" aria-hidden="true">Drop files to attach them</div>
    </template>
  </section>
</template>

<style scoped>
.composer {
  position: fixed;
  z-index: 50;
  right: 24px;
  bottom: 0;
  width: min(620px, calc(100vw - 48px));
  height: min(680px, calc(100vh - 64px));
  display: flex;
  flex-direction: column;
  border-radius: 14px 14px 0 0;
  border: 1px solid var(--line2);
  border-bottom: 0;
  background: var(--panel);
  box-shadow: 0 30px 80px -20px rgb(0 0 0 / 0.55);
  overflow: hidden;
}
.composer.minimized {
  width: 320px;
  height: auto;
}
header {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 44px;
  padding: 0 8px 0 16px;
  border-bottom: 1px solid var(--line);
  background: var(--raise);
}
.minimized header {
  cursor: pointer;
  border-bottom: 0;
}
h2 {
  flex: 1;
  min-width: 0;
  margin: 0;
  font-size: 13.5px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.status {
  font-size: 12px;
  color: var(--faint);
  margin-right: 4px;
}
.icon-btn.small {
  width: 30px;
  height: 30px;
}
.loading {
  flex: 1;
  display: grid;
  place-items: center;
}
.body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.row {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 40px;
  padding: 0 16px;
  border-bottom: 1px solid var(--line);
}
.row label {
  min-width: 34px;
  font-size: 13px;
  color: var(--faint);
}
select {
  flex: 1;
  min-width: 0;
  height: 38px;
  border: 0;
  background: transparent;
  color: var(--text);
  font: inherit;
  font-size: 14px;
  outline: none;
  cursor: pointer;
}
select option {
  background: var(--raise);
}
.subject {
  flex: 1;
  height: 40px;
  border: 0;
  background: transparent;
  color: var(--text);
  font-size: 14.5px;
  font-weight: 500;
  outline: none;
}
.cc-toggles {
  display: flex;
  gap: 10px;
}
.link {
  border: 0;
  background: none;
  padding: 0;
  color: var(--muted);
  font-size: 13px;
  cursor: pointer;
}
.link:hover {
  color: var(--text);
}
.warn {
  margin: 0;
  padding: 8px 16px;
  font-size: 12.5px;
  color: var(--danger);
  border-bottom: 1px solid var(--line);
}
.text {
  flex: 1;
  min-height: 120px;
  padding: 14px 16px;
  border: 0;
  resize: none;
  background: transparent;
  color: var(--text);
  font: inherit;
  font-size: 14.5px;
  line-height: 1.6;
  outline: none;
}
.files {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin: 0;
  padding: 8px 16px;
  list-style: none;
  border-top: 1px solid var(--line);
}
.file {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  max-width: 100%;
  height: 30px;
  padding: 0 4px 0 10px;
  border-radius: 8px;
  border: 1px solid var(--line2);
  background: var(--raise);
  font-size: 12.5px;
  color: var(--muted);
}
.file.uploading {
  padding-right: 10px;
}
.fname {
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 220px;
}
.fsize {
  color: var(--faint);
}
.remove {
  display: inline-flex;
  width: 22px;
  height: 22px;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--faint);
  cursor: pointer;
}
.remove:hover {
  background: var(--hover);
  color: var(--text);
}
.error {
  display: flex;
  gap: 10px;
  margin: 0 16px 8px;
  padding: 10px 12px;
  border-radius: var(--radius);
  background: color-mix(in oklab, var(--danger) 12%, transparent);
  color: var(--danger);
  font-size: 13px;
  line-height: 1.5;
}
.error span {
  color: var(--text);
}
footer {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 12px 12px 16px;
  border-top: 1px solid var(--line);
}
footer .btn-primary {
  height: 36px;
  padding: 0 16px;
}
footer .btn-primary:disabled {
  opacity: 0.6;
  cursor: default;
}
.hint {
  font-size: 11px;
  color: var(--faint);
}
.spacer {
  flex: 1;
}
.icon-btn.danger {
  width: auto;
  padding: 0 10px;
  gap: 6px;
  color: var(--danger);
  background: color-mix(in oklab, var(--danger) 12%, transparent);
}
.confirm {
  font-size: 12.5px;
}
.dropzone {
  position: absolute;
  inset: 44px 0 0;
  display: grid;
  place-items: center;
  border: 2px dashed var(--accent);
  border-radius: 0 0 0 0;
  background: color-mix(in oklab, var(--panel) 85%, transparent);
  color: var(--accent);
  font-size: 14px;
  pointer-events: none;
}
.spinner {
  width: 16px;
  height: 16px;
  border-radius: 50%;
  border: 2px solid var(--line2);
  border-top-color: var(--accent);
  animation: spin 0.7s linear infinite;
}
.spinner.small {
  width: 12px;
  height: 12px;
}
.spinner.on-accent {
  width: 14px;
  height: 14px;
  border-color: color-mix(in oklab, var(--on-accent) 30%, transparent);
  border-top-color: var(--on-accent);
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
@media (max-width: 720px) {
  .composer:not(.minimized) {
    inset: 0;
    width: 100%;
    height: 100%;
    border-radius: 0;
  }
  .composer.minimized {
    right: 12px;
    width: calc(100vw - 24px);
  }
  .hint {
    display: none;
  }
}
.ai-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 8px 12px 0;
  padding: 6px 6px 6px 12px;
  border-radius: 12px;
  border: 1px solid color-mix(in oklab, var(--accent) 45%, var(--line2));
  background: color-mix(in oklab, var(--accent) 7%, var(--bg));
  color: var(--accent);
}
.ai-bar input {
  flex: 1;
  min-width: 0;
  height: 32px;
  border: 0;
  background: transparent;
  color: var(--text);
  outline: none;
  font-size: 14px;
}
.ai-bar .btn.small {
  height: 30px;
  padding: 0 12px;
  font-size: 13px;
}
.ai-undo {
  margin: 6px 16px 0;
  font-size: 12.5px;
  color: var(--muted);
}
.icon-btn.on {
  color: var(--accent);
  background: color-mix(in oklab, var(--accent) 14%, transparent);
}
</style>
