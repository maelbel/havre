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
}

export type DraftChanges = Partial<Pick<Draft, 'account' | 'to' | 'cc' | 'bcc' | 'subject' | 'body'>>

export interface Contact {
  name: string | null
  address: string
}

/** The draft open in the compose window (one at a time). */
export function useComposer() {
  return useState<{ id: number; minimized: boolean; ai?: boolean } | null>('composer', () => null)
}

/** Unsent drafts, for the "Drafts" list in the sidebar. */
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

export function sendDraft(id: number) {
  return $fetch<{ saved_to_sent: boolean; warning: string | null }>(`/api/drafts/${id}/send`, { method: 'POST' })
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
