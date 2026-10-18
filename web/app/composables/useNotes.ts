// Notes. Mirrors `/api/notes/*` (crates/server/src/notes.rs).
//
// A note is Markdown. The server reads it into blocks (`blocks`): the page
// draws each block itself (no `v-html`), so a note can't put markup in the
// page. Function names start with `note` / end with `Note`: composables
// share one auto-import namespace (see docs/HANDOFF.md).
import type { AgendaEvent } from '~/composables/useAgenda'
import type { EventLink, MailLink, PersonLink } from '~/composables/useTasks'

export type NoteSpan =
  | { kind: 'text' | 'bold' | 'italic' | 'code'; text: string }
  | { kind: 'link'; text: string; href: string }

export type NoteBlock =
  | { kind: 'heading'; line: number; level: 1 | 2 | 3; spans: NoteSpan[] }
  | { kind: 'paragraph' | 'quote'; line: number; spans: NoteSpan[] }
  | { kind: 'bullet'; line: number; depth: number; spans: NoteSpan[] }
  | { kind: 'numbered'; line: number; number: number; spans: NoteSpan[] }
  | { kind: 'todo'; line: number; done: boolean; text: string; spans: NoteSpan[]; task: number | null }
  | { kind: 'code'; line: number; text: string }
  | { kind: 'rule'; line: number }

export interface NoteLinks {
  event: EventLink | null
  people: PersonLink[]
  mail: MailLink | null
}

export interface Note extends NoteLinks {
  id: number
  title: string
  body: string
  pinned: boolean
  created_at: number
  updated_at: number
}

/** A note ready to read: the note and its Markdown in blocks. */
export interface NoteView extends Note {
  blocks: NoteBlock[]
}

/** A note in a list. */
export interface NoteSummary extends NoteLinks {
  id: number
  title: string
  snippet: string
  pinned: boolean
  updated_at: number
  open_todos: number
}

export interface NoteInput extends NoteLinks {
  title: string
  body: string
  pinned: boolean
}

export interface NotePerson {
  email: string
  name: string | null
  notes: number
}

export interface NoteQuery {
  q?: string
  person?: string
  event_uid?: string
  event_start?: number
  message_id?: string
  pinned?: boolean
  limit?: number
}

// ---- The API --------------------------------------------------------------------

export function fetchNotes(query: NoteQuery = {}) {
  return $fetch<NoteSummary[]>('/api/notes', { query })
}
export function fetchNote(id: number) {
  return $fetch<NoteView>(`/api/notes/${id}`)
}
export function fetchNotePeople() {
  return $fetch<NotePerson[]>('/api/notes/people')
}
export function createNote(input: Partial<NoteInput> = {}) {
  return $fetch<NoteView>('/api/notes', { method: 'POST', body: { people: [], ...input } })
}
export function saveNote(id: number, input: NoteInput) {
  return $fetch<NoteView>(`/api/notes/${id}`, { method: 'PUT', body: input })
}
export function pinNote(id: number, pinned: boolean) {
  return $fetch<NoteView>(`/api/notes/${id}/pin`, { method: 'PUT', body: { pinned } })
}
export function deleteNote(id: number) {
  return $fetch(`/api/notes/${id}`, { method: 'DELETE' })
}
/** Ticks a `- [ ]` line (and the task made from it, if any). */
export function tickNoteTodo(id: number, line: number, done: boolean) {
  return $fetch<NoteView>(`/api/notes/${id}/todos/${line}`, { method: 'PUT', body: { done } })
}
/** Makes a task of a `- [ ]` line, linked to the note's meeting and person. */
export function noteTodoToTask(id: number, line: number) {
  return $fetch<NoteView>(`/api/notes/${id}/todos/${line}/task`, {
    method: 'POST',
    body: { utc_offset: utcOffset() },
  })
}
/** A note for a meeting: its title, attendees and a few headings. */
export function noteFromEvent(event: number) {
  return $fetch<NoteView>('/api/notes/from-event', { method: 'POST', body: { event } })
}
/** Where the note's email is now (it may have moved since). */
export function locateNoteMail(id: number) {
  return $fetch<{ account: number; folder: string; uid: number }>(`/api/notes/${id}/mail`)
}
export async function openNoteMail(id: number) {
  const where = await locateNoteMail(id)
  await navigateTo({ path: '/mail', query: { account: where.account, folder: where.folder, uid: where.uid } })
}
/** A note about an email, linked to it and its sender. */
export function noteFromMail(ref: { account: number; folder: string; uid: number }) {
  return $fetch<NoteView>('/api/notes/from-mail', { method: 'POST', body: ref })
}

/** The notes of one meeting (one occurrence, for a repeating one). */
export function fetchEventNotes(event: AgendaEvent) {
  const uid = event.uid || `havre-event-${event.id}`
  return fetchNotes({ event_uid: uid, event_start: event.recurring ? event.start : undefined })
}

/** Opens a meeting's note, making one first if it has none. */
export async function openEventNote(event: AgendaEvent) {
  const existing = await fetchEventNotes(event)
  if (existing[0]) return navigateTo({ path: '/notes', query: { note: existing[0].id } })
  const note = await noteFromEvent(event.id)
  await navigateTo({ path: '/notes', query: { note: note.id, edit: 1 } })
}

// ---- Helpers --------------------------------------------------------------------

/** What `saveNote` sends, from a note and a change. */
export function noteInput(n: Note, change: Partial<NoteInput> = {}): NoteInput {
  return { title: n.title, body: n.body, pinned: n.pinned, event: n.event, people: n.people, mail: n.mail, ...change }
}

export function noteTitle(n: { title: string }) {
  return n.title.trim() || 'Untitled note'
}

/** "12:40" today, "Wed" this week, "29 Sep" before, "Sep 2025" last year. */
export function noteWhen(ts: number, now = new Date()): string {
  const d = new Date(ts * 1000)
  const days = (startOfDay(now).getTime() - startOfDay(d).getTime()) / 86_400_000
  if (days < 1) return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  if (days < 7) return d.toLocaleDateString('en-GB', { weekday: 'short' })
  if (d.getFullYear() === now.getFullYear()) return d.toLocaleDateString('en-GB', { day: 'numeric', month: 'short' })
  return d.toLocaleDateString('en-GB', { month: 'short', year: 'numeric' })
}

/** "Lyon kick-off · Mon 19 Oct, 10:00" */
export function noteEventLabel(e: EventLink, short = false): string {
  const d = new Date(e.start * 1000)
  const day = d.toLocaleDateString('en-GB', short ? { weekday: 'short' } : { weekday: 'short', day: 'numeric', month: 'short' })
  if (short) return `${e.title} · ${day}`
  const time = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  return `${e.title} · ${day.replace(',', '')}, ${time}`
}

export function notePersonLabel(p: PersonLink): string {
  return p.name?.trim() || p.email
}

/** The note as a `.md` file, saved by the browser. */
export function downloadNote(n: Note) {
  const text = `# ${noteTitle(n)}\n\n${n.body}`
  const blob = new Blob([text], { type: 'text/markdown;charset=utf-8' })
  const a = document.createElement('a')
  a.href = URL.createObjectURL(blob)
  // Characters that file systems refuse become dashes.
  a.download = `${noteTitle(n).replace(/[\\/:*?"<>|]+/g, '-').slice(0, 80)}.md`
  a.click()
  setTimeout(() => URL.revokeObjectURL(a.href), 1000)
}

/**
 * Is the Notes app in your dock? Agenda, Mail, Today, People and Ctrl+K
 * only show notes when it is (add it from the App library).
 */
export function useNotesEnabled() {
  const { apps } = useApps()
  return computed(() => apps.value.some((a) => a.id === 'notes' && a.enabled))
}
