// Writing mail. Mirrors `/api/drafts*` (crates/core/src/compose.rs).
//
// What you write is a *draft* saved on the server as you type: close the
// window, reload the page, come back tomorrow — it's still there until you
// send or discard it.

export type DraftKind = 'new' | 'reply' | 'reply_all' | 'forward'

export interface DraftAttachment {
  id: number
  filename: string
  mime: string
  size: number
}

export interface Draft {
  id: number
  account: number
  kind: DraftKind
  /** Comma-separated: `Anna <anna@example.com>, bob@example.com` */
  to: string
  cc: string
  bcc: string
  subject: string
  body: string
  attachments: DraftAttachment[]
  updated_at: number
  /** When its copy was last saved in the server's Drafts folder (`null`: never). */
  server_saved_at: number | null
  /** Changed since: the server copy is behind (saved after a pause, or on close). */
  server_pending: boolean
  /** Why the last save to the server failed. */
  server_error: string | null
  /** The formatted text (clean HTML); `null` for a plain-text draft. */
  body_html: string | null
  /** The signature as Havre put it (swapped when "From" changes). */
  signature: string
  /** When it goes (unix seconds): soon (undo send) or later (send later). `null`: a draft. */
  send_at: number | null
  send_state: 'waiting' | 'sending' | 'failed' | null
  send_error: string | null
}

export type DraftChanges = Partial<Pick<Draft, 'account' | 'to' | 'cc' | 'bcc' | 'subject' | 'body'>> & {
  /** Formatted text: the server cleans it and makes its plain text. */
  body_html?: string
  /** Make the draft formatted (`true`) or plain text (`false`). */
  rich?: boolean
}

export interface Contact {
  name: string | null
  address: string
}

/** The draft open in the compose window (one at a time). */
export function useComposer() {
  return useState<{ id: number; minimized: boolean; ai?: boolean } | null>('composer', () => null)
}

/** Every draft (the sidebar's "Unsent" lists those not on the server). */
export function useDrafts() {
  return useFetch<Draft[]>('/api/drafts', { key: 'drafts', lazy: true, server: false, default: () => [] })
}

/**
 * Opens the compose window: a new message, or a reply / forward of
 * `{folder, uid}` (the server fills in recipients, subject, quote).
 */
export async function compose(start: {
  account: number
  kind?: DraftKind
  folder?: string
  uid?: number
  to?: string
  /** Let the AI write the text as soon as it opens. */
  ai?: boolean
}) {
  const { ai, ...body } = start
  const draft = await $fetch<Draft>('/api/drafts', { method: 'POST', body: { kind: 'new', ...body } })
  useComposer().value = { id: draft.id, minimized: false, ai }
  refreshNuxtData('drafts')
  return draft
}

export function openDraft(id: number) {
  useComposer().value = { id, minimized: false }
}

/** A draft from the server's Drafts folder (started in another app), opened to finish it here. */
export async function editServerDraft(at: { account: number; folder: string; uid: number }) {
  const draft = await $fetch<Draft>('/api/drafts/from-message', { method: 'POST', body: at })
  useComposer().value = { id: draft.id, minimized: false }
  refreshNuxtData('drafts')
  return draft
}

/** The window closed: the draft goes to the server now, not after the pause. */
export function closeDraft(id: number) {
  return $fetch<Draft>(`/api/drafts/${id}/close`, { method: 'POST' })
}

/** Not in the server's Drafts folder (yet, or it can't be). */
export function onlyInHavre(d: Draft) {
  // Waiting to be sent: under "Scheduled" instead.
  return (!d.server_saved_at || !!d.server_error) && d.send_at == null
}

export function fetchDraft(id: number) {
  return $fetch<Draft>(`/api/drafts/${id}`)
}

export function saveDraft(id: number, changes: DraftChanges) {
  return $fetch<Draft>(`/api/drafts/${id}`, { method: 'PATCH', body: changes })
}

export async function discardDraft(id: number) {
  await $fetch(`/api/drafts/${id}`, { method: 'DELETE' })
  refreshNuxtData('drafts')
}

export interface SendOutcome {
  saved_to_sent: boolean
  warning: string | null
}
/** "Send": goes after the undo delay (`outcome` is set when there's none); `at`: send later. */
export function sendDraft(id: number, at?: number) {
  return $fetch<{ send_at: number; outcome: SendOutcome | null }>(`/api/drafts/${id}/send`, {
    method: 'POST',
    body: at ? { at } : {},
  })
}
/** Undo send, or "Don't send": a draft again. */
export function unsendDraft(id: number) {
  return $fetch<Draft>(`/api/drafts/${id}/unsend`, { method: 'POST' })
}
/** Messages waiting to be sent, and those that couldn't be. */
export function useScheduled() {
  return useFetch<Draft[]>('/api/mail/scheduled', { key: 'scheduled', lazy: true, server: false, default: () => [] })
}
export interface MailSettings {
  undo_seconds: number
  /** How new messages are written. */
  compose_format: 'rich' | 'plain'
}
/** How new messages are written (asked once per page). */
let format: Promise<'rich' | 'plain'> | null = null
export function mailFormat() {
  format ??= fetchMailSettings()
    .then((s) => s.compose_format)
    .catch(() => 'rich' as const)
  return format
}
/** After Settings changed it. */
export function forgetMailFormat() {
  format = null
}

/** Text as paragraphs of HTML (what the AI writes): `- ` lines become a list. */
export function textToHtml(text: string) {
  const esc = (t: string) => t.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
  return text
    .replace(/\r\n/g, '\n')
    .split(/\n{2,}/)
    .filter((block) => block.trim())
    .map((block) => {
      const lines = block.split('\n')
      if (lines.every((l) => /^\s*[-*] /.test(l))) {
        return `<ul>${lines.map((l) => `<li><p>${esc(l.replace(/^\s*[-*] /, ''))}</p></li>`).join('')}</ul>`
      }
      return `<p>${lines.map(esc).join('<br>')}</p>`
    })
    .join('')
}
export function fetchMailSettings() {
  return $fetch<MailSettings>('/api/mail/settings')
}
export function saveMailSettings(settings: MailSettings) {
  return $fetch<MailSettings>('/api/mail/settings', { method: 'PUT', body: settings })
}

/** "Tomorrow morning", "Monday morning"…: the next such moment, in your time zone. */
export function sendLaterChoices(now = new Date()) {
  const at = (days: number, hour: number) => {
    // `setDate` + `setHours` work in local time: right across a clock change too.
    const d = new Date(now)
    d.setDate(d.getDate() + days)
    d.setHours(hour, 0, 0, 0)
    return d
  }
  // The next Monday (a week later if today is Monday).
  const toMonday = ((8 - now.getDay()) % 7) || 7
  return [
    { label: 'Tomorrow morning', at: at(1, 8) },
    { label: 'Tomorrow afternoon', at: at(1, 13) },
    { label: 'Monday morning', at: at(toMonday, 8) },
  ]
}

export function attachFile(id: number, file: File) {
  return $fetch<DraftAttachment>(`/api/drafts/${id}/attachments`, {
    method: 'POST',
    query: { filename: file.name },
    headers: { 'content-type': file.type || 'application/octet-stream' },
    body: file,
  })
}

export function removeAttachment(id: number, attachment: number) {
  return $fetch(`/api/drafts/${id}/attachments/${attachment}`, { method: 'DELETE' })
}

export function searchContacts(q: string) {
  return $fetch<Contact[]>('/api/contacts', { query: { q } })
}

export function contactLabel(c: Contact) {
  if (!c.name) return c.address
  return /[,;"<>@]/.test(c.name) ? `"${c.name.replace(/"/g, '')}" <${c.address}>` : `${c.name} <${c.address}>`
}
