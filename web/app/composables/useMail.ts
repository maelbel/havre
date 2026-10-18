// Types mirror the Rust structs in `havre_core::mail` (Rust → JSON).
// Later these will be generated from Rust with `ts-rs` so they never drift.

export interface Person {
  name: string | null
  address: string | null
}

export type FolderRole = 'inbox' | 'drafts' | 'sent' | 'archive' | 'junk' | 'trash' | 'all' | 'other'

export interface Folder {
  /** Exact server name — use it in API calls, never display it. */
  path: string
  /** Human name, decoded. */
  name: string
  depth: number
  role: FolderRole
  unread: number
  total: number
}

export interface MessageSummary {
  uid: number
  from: Person | null
  subject: string
  date: string | null
  unread: boolean
  /** Starred (IMAP \\Flagged: Gmail's star, Outlook's flag). */
  flagged?: boolean
  /** Havre labels on it (ids). */
  labels?: number[]
  /** Snoozed until then (unix seconds): only in the Snoozed list. */
  snoozed_until?: number
  /** The first words (once Havre downloaded the message). */
  preview?: string
  attachments?: boolean
  /** Carries a meeting invitation. */
  invite?: boolean
  /** When the invited meeting starts (unix seconds). */
  invite_at?: number
  /** In "All inboxes": where it is. */
  account?: number
  folder?: string
}

/** Every inbox together ("All inboxes"), newest first. */
export function fetchAllInboxes(offset: number, limit = 50, q?: string) {
  return $fetch<MessagePage>('/api/mail/all', { query: { offset, limit, q: q || undefined } })
}

export interface Attachment {
  index: number
  filename: string
  mime: string
  size: number
  inline: boolean
}

export interface MessageDetail {
  uid: number
  folder: string
  from: Person | null
  to: Person[]
  cc: Person[]
  subject: string
  date: string | null
  text: string | null
  html: string | null
  remote_images_blocked: boolean
  attachments: Attachment[]
  unread: boolean
}

// Every mail call names the account it is about (`account` = its id).

export function useFolders(account: Ref<number | undefined>) {
  return useFetch<Folder[]>('/api/mail/folders', {
    // Reactive: switching account refetches automatically.
    query: { account },
    lazy: true,
    server: false,
    default: () => [],
  })
}

export interface MessagePage {
  messages: MessageSummary[]
  /** Older messages exist: ask again with `before` = the last UID. */
  has_more: boolean
}

/**
 * A page of messages. Read from Havre's local copy (instant), unless
 * `remote` asks the mail server to search inside message bodies.
 */
export function fetchMessages(
  account: number,
  folder: string,
  opts: { before?: number; q?: string; limit?: number; remote?: boolean } = {},
) {
  return $fetch<MessagePage>('/api/mail/messages', {
    query: {
      account,
      folder,
      limit: opts.limit ?? 50,
      before: opts.before,
      q: opts.q || undefined,
      remote: opts.remote ? 1 : undefined,
    },
  })
}

/** Asks the server to sync now, and waits until it's done. */
export function syncNow(account: number, folder?: string) {
  return $fetch('/api/mail/sync', { method: 'POST', query: { account, folder } })
}

export interface SyncStatus {
  state: 'starting' | 'syncing' | 'idle' | 'error'
  /** Unix time (seconds) of the last successful sync. */
  last_success: number | null
  error: string | null
  /** Older messages are still being downloaded. */
  backfilling: boolean
  /** New mail is pushed by the server (IMAP IDLE). */
  push: boolean
}

/** Every event says which account it is about. */
export type MailEvent = { account: number } & (
  | { type: 'folders' }
  | { type: 'folder'; path: string }
  | { type: 'status'; status: SyncStatus }
  | { type: 'accounts' }
  | { type: 'agenda' }
  | { type: 'tasks' }
  | { type: 'reminder'; task: number; title: string }
)

/**
 * Live updates from the server (Server-Sent Events). The browser reconnects
 * by itself if the connection drops.
 */
export function useMailEvents(onEvent: (event: MailEvent) => void) {
  let source: EventSource | null = null
  const route = useRoute()
  onMounted(() => {
    source = new EventSource('/api/events')
    source.onmessage = (message) => {
      try {
        onEvent(JSON.parse(message.data) as MailEvent)
      } catch {
        /* ignore malformed events */
      }
    }
    // The browser retries by itself; but if we were signed out, go sign in.
    source.onerror = async () => {
      const status = await refreshAuthStatus().catch(() => null)
      if (status?.enabled && !status.signed_in) {
        source?.close()
        await navigateTo({ path: '/login', query: { next: route.fullPath } })
      }
    }
  })
  onBeforeUnmount(() => source?.close())
}

export type MailAction = 'archive' | 'trash' | 'move'

export type ActionOutcome = ({ result: 'moved'; to: string } | { result: 'deleted' }) & {
  /** For Undo: where the messages went is known by these. */
  message_ids: string[]
}

export function applyMailAction(account: number, action: MailAction, folder: string, uids: number[], to?: string) {
  return $fetch<ActionOutcome>('/api/mail/actions', {
    method: 'POST',
    body: { account, action, folder, uids, to },
  })
}

export function fetchMessage(account: number, folder: string, uid: number, opts: { images?: boolean } = {}) {
  return $fetch<MessageDetail>(`/api/mail/messages/${uid}`, {
    query: { account, folder, images: opts.images ? 1 : 0 },
  })
}

export function setSeen(account: number, folder: string, uid: number, seen: boolean) {
  return $fetch(`/api/mail/messages/${uid}/seen`, {
    method: 'PUT',
    query: { account, folder },
    body: { seen },
  })
}

export function attachmentUrl(account: number, folder: string, uid: number, index: number) {
  return `/api/mail/messages/${uid}/attachments/${index}?account=${account}&folder=${encodeURIComponent(folder)}`
}

/** Display name of a person: "Claire Dubois", or the address. */
export function personLabel(p: Person | null | undefined): string {
  return p?.name || p?.address || 'Unknown sender'
}

/** API errors carry `{ error: "…" }`; fall back to the generic message. */
export function apiErrorMessage(err: unknown): string {
  const e = err as { data?: { error?: string }; message?: string } | null
  return e?.data?.error ?? e?.message ?? 'Something went wrong.'
}

// ---- Organizing (step 8): stars, labels, snooze, several at once, undo -------------------

export interface Label {
  id: number
  name: string
  /** A design color: work, personal, lyon, hol. */
  color: string
  position: number
  count: number
}

export const LABEL_COLORS = ['lyon', 'work', 'personal', 'hol'] as const

/** Labels and the Starred / Snoozed counts, shared by the sidebar and menus. */
export function useOrganize() {
  const state = useState('organize', () => ({ labels: [] as Label[], starred: 0, snoozed: 0 }))
  const loaded = useState('organize-loaded', () => false)
  async function refresh() {
    state.value = await $fetch<{ labels: Label[]; starred: number; snoozed: number }>('/api/mail/organize')
    loaded.value = true
  }
  if (!loaded.value && import.meta.client) refresh().catch(() => {})
  return { state, refresh }
}

/** A list across accounts. */
export type CrossView = 'inboxes' | 'starred' | 'snoozed' | 'label'

export function fetchView(view: CrossView, opts: { label?: number; offset?: number; limit?: number; q?: string } = {}) {
  return $fetch<MessagePage>('/api/mail/view', {
    query: { view, label: opts.label, offset: opts.offset ?? 0, limit: opts.limit ?? 50, q: opts.q || undefined },
  })
}

/** Messages of one account and folder. */
export interface Selection {
  account: number
  folder: string
  uids: number[]
}

export function setFlags(sel: Selection, flags: { seen?: boolean; flagged?: boolean }) {
  return $fetch('/api/mail/flags', { method: 'POST', body: { ...sel, ...flags } })
}

export function labelMessages(sel: Selection, label: number, on: boolean) {
  return $fetch('/api/mail/labels', { method: 'POST', body: { ...sel, label, on } })
}

/** `until`: unix seconds, or null to wake them up now. */
export function snoozeMessages(sel: Selection, until: number | null) {
  return $fetch('/api/mail/snooze', { method: 'POST', body: { ...sel, until } })
}

/** Moves messages back: `from` where they are now, `to` where they were. */
export function undoMove(account: number, from: string, to: string, messageIds: string[]) {
  return $fetch<{ moved: number }>('/api/mail/undo', {
    method: 'POST',
    body: { account, from, to, message_ids: messageIds },
  })
}

export function createLabel(name: string, color: string) {
  return $fetch<Label>('/api/labels', { method: 'POST', body: { name, color } })
}

export function updateLabel(id: number, changes: { name?: string; color?: string }) {
  return $fetch<Label>(`/api/labels/${id}`, { method: 'PATCH', body: changes })
}

export function deleteLabel(id: number) {
  return $fetch(`/api/labels/${id}`, { method: 'DELETE' })
}

/** Groups selected rows by account and folder (one request each). */
export function groupSelection(rows: { account: number; folder: string; uid: number }[]): Selection[] {
  const groups = new Map<string, Selection>()
  for (const r of rows) {
    const key = `${r.account}\u0000${r.folder}`
    const g = groups.get(key) ?? { account: r.account, folder: r.folder, uids: [] }
    g.uids.push(r.uid)
    groups.set(key, g)
  }
  return [...groups.values()]
}

/** Snooze choices: "Later today", "Tomorrow"… with their time. */
export function snoozeChoices(now = new Date()) {
  const at = (d: Date, h: number) => {
    const x = new Date(d)
    x.setHours(h, 0, 0, 0)
    return x
  }
  const plusDays = (n: number) => {
    const x = new Date(now)
    x.setDate(x.getDate() + n)
    return x
  }
  const out: { label: string; when: Date }[] = []
  // Later today: in 3 hours (rounded to the hour), if still today.
  const later = new Date(now.getTime() + 3 * 3_600_000)
  later.setMinutes(0, 0, 0)
  if (later.getDate() === now.getDate() && later.getHours() < 21) out.push({ label: 'Later today', when: later })
  out.push({ label: 'Tomorrow', when: at(plusDays(1), 8) })
  const day = now.getDay() // 0 Sunday … 6 Saturday
  if (day >= 1 && day <= 4) out.push({ label: 'This weekend', when: at(plusDays(6 - day), 9) })
  out.push({ label: 'Next week', when: at(plusDays(((8 - day) % 7) || 7), 8) })
  return out
}

/** The color for the n-th new label (the design's four, in turn). */
export function nextLabelColor(n: number): string {
  return LABEL_COLORS[n % LABEL_COLORS.length] ?? 'lyon'
}
