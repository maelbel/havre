<script setup lang="ts">
import type { Folder, MessageDetail, Person } from '~/composables/useMail'

const props = defineProps<{
  account: number
  message: MessageDetail | null
  loading: boolean
  error: string | null
  imagesAllowed: boolean
  folders: Folder[]
  inTrash: boolean
  inArchive: boolean
  /** Starred, and the labels it carries (for the toolbar's menus). */
  flagged?: boolean
  labelState?: Record<number, 'all' | 'some'>
  /** Shown from Snoozed (the menu offers "Wake up now"). */
  snoozed?: boolean
  /** "3 of 40": where the open message is in the list (0: not in it). */
  position?: number
  total?: number
}>()
const emit = defineEmits<{
  back: []
  markUnread: []
  loadImages: []
  retry: []
  archive: []
  trash: []
  move: [to: string]
  reply: [kind: 'reply' | 'reply_all' | 'forward', ai?: boolean]
  prev: []
  next: []
  star: []
  label: [label: number, on: boolean]
  snooze: [until: number | null]
}>()

// Deleting from the Trash is permanent: the first click arms the button,
// a second click within 3 seconds confirms.
const confirmDelete = ref(false)
let confirmTimer: ReturnType<typeof setTimeout> | undefined
function onDelete() {
  if (!props.inTrash) return emit('trash')
  if (confirmDelete.value) {
    confirmDelete.value = false
    return emit('trash')
  }
  confirmDelete.value = true
  clearTimeout(confirmTimer)
  confirmTimer = setTimeout(() => (confirmDelete.value = false), 3000)
}
watch(
  () => props.message?.uid,
  () => (confirmDelete.value = false),
)

const files = computed(() => props.message?.attachments.filter((a) => !a.inline) ?? [])

// With the Files app: another version of an attachment came in another
// email ("devis.pdf" then "devis-v2.pdf"): say so, with a link to compare.
const filesOn = useFilesEnabled()
const versionHints = ref<{ key: string; text: string }[]>([])
watch(
  () => [props.message?.uid, props.message?.folder, filesOn.value] as const,
  async () => {
    versionHints.value = []
    const m = props.message
    if (!m || !filesOn.value || !files.value.length) return
    const here = await fetchFilesInMessage({ account: props.account, folder: m.folder, uid: m.uid }).catch(() => [])
    if (props.message?.uid !== m.uid) return // another message opened meanwhile
    const mine = m.date ? new Date(m.date).getTime() : 0
    versionHints.value = here.flatMap((f) => {
      const other = f.versions[0]
      if (!other) return []
      const when = other.date ? new Date(other.date).getTime() : 0
      const what = when < mine ? 'An earlier version' : 'A newer version'
      return [{ key: f.key, text: `${what}, “${other.name}”, came ${other.date ? shortDate(other.date) : 'in another email'}.` }]
    })
  },
  { immediate: true },
)

// A meeting invitation (.ics): the Agenda's card, with Accept / Maybe / Decline.
const invitation = computed(() => {
  const m = props.message
  const file = m?.attachments.find((a) => a.mime === 'text/calendar' || a.filename.toLowerCase().endsWith('.ics'))
  return m && file ? { account: props.account, folder: m.folder, uid: m.uid, index: file.index } : null
})
// AI summary (when AI is on): S or the ✨ button.
const { status: ai } = useAiStatus()
const summarizing = ref(false)
watch(
  () => props.message?.uid,
  () => (summarizing.value = false),
)
function toggleSummary() {
  if (ai.value.enabled && props.message) summarizing.value = !summarizing.value
}
// Tasks (when the app is in the dock): the Task button, or T.
const tasksOn = useTasksEnabled()
const taskMenu = ref<{ toggle: () => void } | null>(null)
function openTask() {
  if (tasksOn.value && props.message) taskMenu.value?.toggle()
}
// Notes (when the app is in the dock): the Note button, or N — opens the
// email's note, or starts one.
const notesOn = useNotesEnabled()
const noting = ref(false)
async function openNote() {
  if (!notesOn.value || !props.message || noting.value) return
  noting.value = true
  try {
    const note = await noteFromMail({ account: props.account, folder: props.message.folder, uid: props.message.uid })
    await navigateTo({ path: '/notes', query: { note: note.id } })
  } catch (err) {
    useToasts().show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    noting.value = false
  }
}
defineExpose({ toggleSummary, openTask, openNote })

/** "Claire" in "Reply to Claire…". */
const firstName = computed(() => {
  const from = props.message?.from
  const name = from?.name?.trim()
  if (name) return name.split(/\s+/)[0]
  return from?.address?.split('@')[0] ?? ''
})

const people = (list: Person[]) => list.map(personLabel).join(', ')

function size(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}
function kind(mime: string, filename: string) {
  if (mime === 'text/calendar') return 'ICS'
  const ext = filename.includes('.') ? filename.split('.').pop() : null
  return (ext && ext.length <= 4 ? ext : mime.split('/')[1] ?? 'file').toUpperCase()
}
</script>

<template>
  <article class="reader" :aria-label="message?.subject || 'Message'" :aria-busy="loading">
    <div class="toolbar">
      <UButton
        icon="i-havre-back"
        color="neutral"
        variant="ghost"
        square
        class="back"
        type="button"
        aria-label="Back to list"
        @click="emit('back')"
      />
      <template v-if="message">
        <UButton
          v-if="!inArchive"
          icon="i-havre-archive"
          color="neutral"
          variant="ghost"
          square
          type="button"
          aria-label="Archive"
          title="Archive (E)"
          @click="emit('archive')"
        />
        <UButton
          icon="i-havre-trash"
          :color="confirmDelete ? 'error' : 'neutral'"
          variant="ghost"
          type="button"
          :aria-label="inTrash ? (confirmDelete ? 'Click again to delete forever' : 'Delete forever') : 'Delete'"
          :title="inTrash ? 'Delete forever' : 'Delete (#)'"
          @click="onDelete"
        >
          <span v-if="confirmDelete" class="confirm">Delete forever?</span>
        </UButton>
        <MoveMenu :folders="folders" :current="message.folder" @move="(to) => emit('move', to)" />
        <UButton
          :icon="flagged ? 'i-havre-star-filled' : 'i-havre-star'"
          :color="flagged ? 'primary' : 'neutral'"
          variant="ghost"
          square
          type="button"
          :aria-label="flagged ? 'Unstar' : 'Star'"
          :aria-pressed="!!flagged"
          title="Star (*)"
          @click="emit('star')"
        />
        <LabelMenu :state="labelState ?? {}" @toggle="(id, on) => emit('label', id, on)" />
        <SnoozeMenu :snoozed="snoozed" @snooze="(until) => emit('snooze', until)" />
        <TaskFromMail v-if="tasksOn" ref="taskMenu" :account="account" :message="message" />
        <UButton
          v-if="notesOn"
          icon="i-havre-note"
          color="neutral"
          variant="ghost"
          square
          type="button"
          aria-label="Note about this email"
          title="Note (N)"
          :disabled="noting"
          @click="openNote"
        />
        <span class="sep" aria-hidden="true" />
        <UButton
          icon="i-havre-reply"
          color="neutral"
          variant="ghost"
          square
          type="button"
          aria-label="Reply"
          title="Reply (R)"
          @click="emit('reply', 'reply')"
        />
        <UButton
          icon="i-havre-reply-all"
          color="neutral"
          variant="ghost"
          square
          type="button"
          aria-label="Reply all"
          title="Reply all (A)"
          @click="emit('reply', 'reply_all')"
        />
        <UButton
          icon="i-havre-forward"
          color="neutral"
          variant="ghost"
          square
          type="button"
          aria-label="Forward"
          title="Forward (F)"
          @click="emit('reply', 'forward')"
        />
        <span class="sep" aria-hidden="true" />
        <UButton
          v-if="ai.enabled"
          icon="i-havre-sparkle"
          :color="summarizing ? 'primary' : 'neutral'"
          :variant="summarizing ? 'soft' : 'ghost'"
          square
          type="button"
          aria-label="Summarize"
          title="Summarize (S)"
          @click="toggleSummary"
        />
        <UButton
          icon="i-havre-unread"
          color="neutral"
          variant="ghost"
          square
          type="button"
          aria-label="Mark as unread"
          title="Mark as unread (U)"
          @click="emit('markUnread')"
        />
        <span class="grow" />
        <template v-if="position && total">
          <span class="pos mono">{{ position }} of {{ total }}</span>
          <UButton
            icon="i-havre-up"
            color="neutral"
            variant="ghost"
            size="sm"
            square
            class="nav"
            aria-label="Previous conversation"
            title="Previous (K)"
            :disabled="position <= 1"
            @click="emit('prev')"
          />
          <UButton
            icon="i-havre-chevron"
            color="neutral"
            variant="ghost"
            size="sm"
            square
            class="nav"
            aria-label="Next conversation"
            title="Next (J)"
            :disabled="position >= total"
            @click="emit('next')"
          />
        </template>
      </template>
    </div>

    <div v-if="error" class="state">
      <p>{{ error }}</p>
      <UButton color="neutral" variant="outline" type="button" @click="emit('retry')">Try again</UButton>
    </div>

    <div v-else-if="loading && !message" class="body skeleton" aria-hidden="true">
      <span class="l1" /><span class="l2" /><span class="l3" /><span class="l3" />
    </div>

    <div v-else-if="message" class="body" :class="{ dim: loading }">
      <h2 class="serif subject">{{ message.subject || '(no subject)' }}</h2>

      <div class="meta">
        <span class="avatar" aria-hidden="true" :style="{ '--c': personColor(personLabel(message.from)) }">
          {{ initials(personLabel(message.from)) }}
        </span>
        <div class="who">
          <div>
            <strong>{{ personLabel(message.from) }}</strong>
            <span v-if="message.from?.name && message.from.address" class="faint addr">{{ message.from.address }}</span>
          </div>
          <div class="faint small">
            <span v-if="message.to.length">to {{ people(message.to) }}</span>
            <span v-if="message.cc.length"> · cc {{ people(message.cc) }}</span>
            <span v-if="message.date"> · {{ new Date(message.date).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' }) }}</span>
          </div>
        </div>
      </div>

      <AiSummary
        v-if="summarizing"
        :account="account"
        :folder="message.folder"
        :uid="message.uid"
        @close="summarizing = false"
        @draft="emit('reply', 'reply', true)"
      />

      <InvitationCard v-if="invitation" :invitation="invitation" />

      <div v-if="message.remote_images_blocked && !imagesAllowed" class="banner">
        <HIcon name="image" :size="16" />
        <span>Remote images are hidden to protect your privacy.</span>
        <UButton color="primary" variant="link" class="p-0" type="button" @click="emit('loadImages')">
          Load images
        </UButton>
      </div>

      <MailFrame v-if="message.html" :html="message.html" :allow-remote-images="imagesAllowed" />
      <div v-else class="text">{{ message.text?.trim() || '(This message has no text.)' }}</div>

      <div v-if="files.length" class="files">
        <div class="eyebrow">
          <HIcon name="paperclip" :size="13" /> {{ files.length }} attachment{{ files.length > 1 ? 's' : '' }}
        </div>
        <div class="file-list">
          <a
            v-for="a in files"
            :key="a.index"
            class="file"
            :href="attachmentUrl(account, message.folder, message.uid, a.index)"
            :download="a.filename"
          >
            <span class="kind mono">{{ kind(a.mime, a.filename) }}</span>
            <span class="fname">{{ a.filename }}</span>
            <span class="faint small">{{ size(a.size) }}</span>
            <HIcon name="download" :size="15" />
          </a>
        </div>
        <p v-for="h in versionHints" :key="h.key" class="version-hint">
          <span class="hint-text">{{ h.text }}</span>
          <NuxtLink :to="filesPageLink(h.key)">Compare in Files →</NuxtLink>
        </p>
      </div>
    </div>

    <div v-if="message && !error" class="replybar">
      <button type="button" class="reply-box" @click="emit('reply', 'reply')">
        <HIcon name="reply" :size="16" />
        <span class="grow">Reply to {{ firstName }}…</span>
        <UKbd value="R" />
      </button>
    </div>

    <p v-if="!message && !loading && !error" class="state faint">Select a conversation.</p>
  </article>
</template>

<style scoped>
.version-hint {
  margin: 10px 0 0;
  display: flex;
  flex-wrap: wrap;
  gap: 6px 10px;
  font-size: 13px;
}
.version-hint a {
  text-decoration: none;
}
.hint-text {
  color: var(--muted);
}
.reader {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  background: var(--panel);
  overflow: auto;
}
.toolbar {
  position: sticky;
  top: 0;
  z-index: 1;
  display: flex;
  gap: 4px;
  min-height: 56px;
  padding: 10px 16px;
  border-bottom: 1px solid var(--line);
  background: var(--panel);
}
.back {
  display: none;
}
.grow {
  flex: 1;
  min-width: 0;
  text-align: left;
}
.pos {
  align-self: center;
  white-space: nowrap;
  margin-right: 6px;
  font-size: 12px;
  color: var(--faint);
}
.nav {
  width: 32px;
  height: 32px;
  align-self: center;
}
.nav:disabled {
  opacity: 0.35;
  cursor: default;
}
.replybar {
  position: sticky;
  bottom: 0;
  width: 100%;
  max-width: 720px;
  padding: 12px 40px 24px;
  background: linear-gradient(transparent, var(--panel) 30%);
}
.reply-box {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 10px;
  height: 46px;
  padding: 0 16px;
  border: 1px solid var(--line2);
  border-radius: 12px;
  background: var(--bg);
  color: var(--faint);
  font-size: 14px;
  cursor: text;
}
.reply-box:hover {
  border-color: var(--faint);
}
.sep {
  width: 1px;
  margin: 8px 6px;
  background: var(--line2);
}
.confirm {
  font-size: 12.5px;
  font-weight: 500;
}
.body {
  flex: 1 0 auto;
  width: 100%;
  max-width: 720px;
  padding: 32px 40px 24px;
  display: flex;
  flex-direction: column;
  gap: 24px;
  transition: opacity 0.15s;
}
.body.dim {
  opacity: 0.6;
}
.subject {
  margin: 0;
  font-size: 32px;
  line-height: 1.15;
  letter-spacing: -0.015em;
}
.meta {
  display: flex;
  align-items: center;
  gap: 12px;
}
.who {
  min-width: 0;
}
.avatar {
  flex: 0 0 38px;
  height: 38px;
  border-radius: 50%;
  display: grid;
  place-items: center;
  font-size: 13px;
  font-weight: 600;
  background: color-mix(in oklab, var(--c, var(--faint)) 20%, transparent);
  color: var(--c, var(--faint));
}
.faint {
  color: var(--faint);
}
.addr {
  margin-left: 6px;
}
.small {
  font-size: 13px;
}
.banner {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px 10px;
  padding: 10px 14px;
  border-radius: var(--radius);
  background: var(--bg);
  border: 1px solid var(--line);
  color: var(--muted);
  font-size: 13px;
}
.text {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font-size: 15px;
  line-height: 1.65;
}
.files {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.files .eyebrow {
  display: flex;
  align-items: center;
  gap: 6px;
}
.file-list {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.file {
  display: inline-flex;
  align-items: center;
  gap: 10px;
  max-width: 100%;
  padding: 8px 12px;
  border: 1px solid var(--line2);
  border-radius: 10px;
  color: var(--text);
  text-decoration: none;
  font-size: 13px;
}
.file:hover {
  background: var(--hover);
}
.kind {
  font-size: 10px;
  padding: 3px 5px;
  border-radius: 4px;
  background: color-mix(in oklab, var(--accent) 16%, transparent);
  color: var(--accent);
}
.fname {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.state {
  padding: 32px 40px;
}
.state p {
  margin: 0 0 12px;
  color: var(--muted);
}
.skeleton span {
  display: block;
  height: 12px;
  border-radius: 5px;
  background: var(--line);
}
.skeleton .l1 {
  height: 28px;
  width: 70%;
}
.skeleton .l2 {
  width: 40%;
}
.skeleton .l3 {
  width: 90%;
}

@media (max-width: 760px) {
  .back {
    display: inline-flex;
  }
  .body {
    padding: 20px 16px 40px;
  }
  .replybar {
    padding: 8px 16px 16px;
  }
  .pos {
    display: none;
  }
  .subject {
    font-size: 24px;
  }
}
</style>
