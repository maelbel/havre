// What the apps share. Mirrors `crates/server/src/platform.rs`.
import type { AgendaEvent } from './useAgenda'
import type { IconName } from '~/components/HIcon.vue'
import type { Person as Sender } from './useMail'

/** An app's manifest, plus whether it's in the dock. */
export interface HavreApp {
  id: string
  name: string
  description: string
  icon: IconName
  route: string
  provides: string[]
  uses: string[]
  enabled: boolean
  position: number
}

/** The apps, shared by the dock and the App library (one request). */
export function useApps() {
  const apps = useState<HavreApp[]>('apps', () => [])
  const loaded = useState('apps-loaded', () => false)
  async function refresh() {
    apps.value = await $fetch<HavreApp[]>('/api/apps')
    loaded.value = true
  }
  if (!loaded.value && import.meta.client) refresh().catch(() => {})
  return { apps, loaded, refresh }
}

export async function setAppEnabled(id: string, enabled: boolean) {
  useApps().apps.value = await $fetch<HavreApp[]>(`/api/apps/${id}`, { method: 'PATCH', body: { enabled } })
  // AI is on when its app is in the dock: the other apps ask again.
  if (id === 'ai') await useAiStatus().refresh()
}

export async function reorderApps(ids: string[]) {
  useApps().apps.value = await $fetch<HavreApp[]>('/api/apps/order', { method: 'PUT', body: { ids } })
}

// ---- Describing an app (the App library, Settings › Your apps) ------------------

/** What each capability in a manifest means, in words. */
export const APP_CAPABILITIES: Record<string, string> = {
  'mail.unread': 'unread mail',
  'mail.send': 'sending mail',
  'people.senders': 'who writes to you',
  'calendar.events': 'your events',
  'calendar.invitations': 'answering invitations',
  'people.attendees': 'who you meet',
  'people.suggest': 'address suggestions',
  'tasks.due': 'what’s due',
  'tasks.from_mail': 'tasks from emails',
  'notes.for_event': 'meeting notes',
  'notes.for_person': 'notes about people',
  'files.list': 'every attachment, listed',
  'files.versions': 'versions of a file',
  'ai.ask': 'asking in plain language',
  'ai.brief': 'the written brief',
  'ai.summaries': 'summaries and drafts',
}
/** The kind of data a capability shares (the chips under an app). */
const APP_KINDS: Record<string, string> = {
  'mail.unread': 'Mail',
  'mail.send': 'Mail',
  'people.senders': 'People',
  'people.attendees': 'People',
  'people.suggest': 'People',
  'calendar.events': 'Events',
  'calendar.invitations': 'Invitations',
  'tasks.due': 'Tasks',
  'tasks.from_mail': 'Tasks',
  'notes.for_event': 'Events',
  'notes.for_person': 'People',
  'files.list': 'Files',
  'files.versions': 'Files',
  'ai.ask': 'Mail',
  'ai.brief': 'Events',
  'ai.summaries': 'Mail',
}
export function appKinds(app: HavreApp) {
  return [...new Set(app.provides.map((p) => APP_KINDS[p] ?? p))]
}

/** The apps it works with: those providing what it uses, or using what it provides. */
export function appPartners(app: HavreApp, all: HavreApp[]) {
  const names = new Set<string>()
  for (const other of all) {
    if (other.id === app.id) continue
    if (app.uses.some((u) => other.provides.includes(u)) || app.provides.some((p) => other.uses.includes(p)))
      names.add(other.name)
  }
  return [...names]
}

const APP_TINT: Record<string, string> = { mail: 'work', agenda: 'lyon', people: 'personal', tasks: 'hol', notes: 'personal', ai: 'lyon' }
export function appTint(app: Pick<HavreApp, 'id'>) {
  return `var(--c-${APP_TINT[app.id] ?? 'hol'})`
}

/** The line under an app's name: "IMAP · SMTP · 2 accounts". */
export function appDetail(app: HavreApp, s: SystemInfo | null) {
  if (!s) return ''
  const n = (count: number, one: string) => `${count} ${one}${count === 1 ? '' : 's'}`
  if (app.id === 'mail') return `IMAP · SMTP · ${n(s.accounts, 'account')}`
  if (app.id === 'agenda') return n(s.calendars, 'calendar')
  if (app.id === 'people') return `Built in · ${n(s.people, 'contact')}`
  if (app.id === 'tasks') return `To Do · Google · CalDAV · ${s.tasks ?? 0} open`
  if (app.id === 'notes') return `Markdown · ${n(s.notes ?? 0, 'note')}`
  if (app.id === 'files') return `From your mail · ${n(s.files ?? 0, 'file')}`
  if (app.id === 'ai') {
    const ai = useAiStatus().status.value
    if (!ai.enabled) return 'Not set up yet'
    return ai.local ? `On this server · ${ai.model}` : ai.provider === 'claude-code' ? 'Claude Code' : (ai.model ?? '')
  }
  return 'Built in'
}

/** Where an app's settings are (AI has its own page). */
export function appSettingsPath(app: Pick<HavreApp, 'id'>) {
  return app.id === 'ai' ? '/settings/ai' : `/settings/apps/${app.id}`
}

// ---- People -------------------------------------------------------------------

export interface PersonEntry {
  /** The main address, or `~42` (their contact) for someone without one: the page's key. */
  email: string
  /** Every address of this person, the main one first. */
  emails: string[]
  /** Your name for them, else the one they use. */
  name: string
  /** Their details, when you added some. */
  contact: number | null
  company: string
  role: string
  favorite: boolean
  /** No-reply, newsletters…: guessed, or as you said. */
  automated: boolean
  /** Mail from them / mail you sent them / meetings. */
  messages: number
  sent: number
  meetings: number
  /** Unix seconds: the last email either way, or the last meeting. */
  last_contact: number | null
  /** Their latest email waits for your reply (unix seconds). */
  waiting_since: number | null
  /** `YYYY-MM-DD`, or `--MM-DD` without the year. */
  birthday: string | null
  /** An address book has their photo (`personPhoto`). */
  photo: boolean
  phones: Phone[]
}

export interface Phone {
  label: string
  number: string
}

export interface MessageRef {
  account: number
  folder: string
  uid: number
  subject: string
  date: string | null
  unread: boolean
  from?: string | null
  preview?: string | null
}

export interface PersonDetail extends PersonEntry {
  note: string
  /** The address books they're in. */
  books: string[]
  /** The address book that last changed their details, and when. */
  updated_from: string | null
  updated_from_at: number | null
  /** What you said (`null`: guessed from the address). */
  kind: 'person' | 'automated' | null
  last_from: number | null
  last_to: number | null
  last_meeting: number | null
  next_meeting: number | null
  recent_messages: MessageRef[]
  upcoming: AgendaEvent[]
  past: AgendaEvent[]
  files: SharedFile[]
}

export type PeopleSort = 'recent' | 'frequent' | 'name'
export type PeopleFilter = 'everyone' | 'favorites' | 'waiting' | 'automated' | 'all'

/** The People list: searched, sorted, filtered, maybe one company's. */
export function listPeople(q: { q?: string; sort?: PeopleSort; filter?: PeopleFilter; company?: string | null; limit?: number }) {
  const query: Record<string, string | number> = { q: q.q ?? '', sort: q.sort ?? 'recent', filter: q.filter ?? 'everyone' }
  if (q.company) query.company = q.company
  if (q.limit) query.limit = q.limit
  return $fetch<PersonEntry[]>('/api/people', { query })
}

/** Everyone matching, robots included, the most frequent first (suggestions, Ctrl+K). */
export function searchPeople(q: string, limit = 200) {
  return listPeople({ q, sort: 'frequent', filter: 'all', limit })
}

export function fetchPerson(email: string) {
  return $fetch<PersonDetail>(`/api/people/${encodeURIComponent(email)}`)
}

export function fetchCompanies() {
  return $fetch<{ name: string; people: number }[]>('/api/people/companies')
}

/** Only the fields given change; `birthday: null` clears it, `kind: null` goes back to guessing. */
export interface PersonUpdate {
  name?: string
  company?: string
  role?: string
  phones?: Phone[]
  birthday?: string | null
  note?: string
  favorite?: boolean
  kind?: 'person' | 'automated' | null
}
export function updatePerson(email: string, change: PersonUpdate) {
  return $fetch<PersonDetail>(`/api/people/${encodeURIComponent(email)}`, { method: 'PATCH', body: change })
}
/** `other` is the same person as `email`. */
export function mergePerson(email: string, other: string) {
  return $fetch<PersonDetail>(`/api/people/${encodeURIComponent(email)}/merge`, { method: 'POST', body: { email: other } })
}
export function removePersonAddress(email: string, other: string) {
  return $fetch<PersonDetail>(`/api/people/${encodeURIComponent(email)}/addresses/${encodeURIComponent(other)}`, {
    method: 'DELETE',
  })
}

/** Someone new, by name; the address is optional. */
export function createPerson(name: string, email?: string) {
  return $fetch<PersonDetail>('/api/people', { method: 'POST', body: { name, email: email || null } })
}
/** A `.vcf` file into Havre only (nothing is sent to your address books). */
export function importPeople(text: string) {
  return $fetch<{ added: number; updated: number; skipped: number }>('/api/people/import', {
    method: 'POST',
    body: text,
    headers: { 'content-type': 'text/vcard' },
  })
}
export const PEOPLE_EXPORT = '/api/people/export.vcf'
export function personPhoto(p: { email: string }) {
  return `/api/people/${encodeURIComponent(p.email)}/photo`
}
/** Someone without an address: no mail, no meetings to show. */
export function noAddress(p: { emails: string[] }) {
  return p.emails.length === 0
}

// ---- Contacts sync (Settings › People) ----------------------------------------------

export interface ContactBook {
  id: number
  source: 'google' | 'microsoft' | 'carddav'
  name: string
  read_only: boolean
  synced_at: number | null
  error: string | null
  contacts: number
}
export interface ContactSource {
  key: string
  kind: 'google' | 'microsoft' | 'caldav'
  name: string
  enabled: boolean
  /** The sign-in allows contacts (`false`: reconnect the account). */
  allowed: boolean
  account: number | null
  books: ContactBook[]
}
export interface ContactSettings {
  sources: ContactSource[]
  default_book: number | null
}
export function fetchContactSettings() {
  return $fetch<ContactSettings>('/api/contacts/settings')
}
export function setContactSource(key: string, enabled: boolean) {
  return $fetch<ContactSettings>(`/api/contacts/sources/${encodeURIComponent(key)}`, { method: 'PUT', body: { enabled } })
}
export function setDefaultBook(book: number | null) {
  return $fetch<ContactSettings>('/api/contacts/default', { method: 'PUT', body: { book } })
}
export function syncContacts() {
  return $fetch<ContactSettings>('/api/contacts/sync', { method: 'POST' })
}

export interface PeopleToday {
  birthdays: { person: PersonEntry; date: string; days: number; turns: number | null }[]
  keep_in_touch: PersonEntry[]
}
export function fetchPeopleToday() {
  return $fetch<PeopleToday>('/api/people/today', { query: { date: isoDate(new Date()) } })
}

/** "now", "5 min", "3 h", "2 d", "3 wk", "4 mo", "2 y": how long ago (unix seconds). */
export function ago(t: number | null | undefined, now = Date.now()): string {
  if (!t) return ''
  const s = Math.max(0, now / 1000 - t)
  if (s < 60) return 'now'
  if (s < 3600) return `${Math.floor(s / 60)} min`
  if (s < 86_400) return `${Math.floor(s / 3600)} h`
  if (s < 14 * 86_400) return `${Math.floor(s / 86_400)} d`
  if (s < 60 * 86_400) return `${Math.floor(s / (7 * 86_400))} wk`
  if (s < 365 * 86_400) return `${Math.floor(s / (30 * 86_400))} mo`
  return `${Math.floor(s / (365 * 86_400))} y`
}

/** "3 d ago", or "just now". */
export function agoText(t: number | null | undefined) {
  const a = ago(t)
  return a === 'now' ? 'just now' : `${a} ago`
}

/** Days until the next birthday (0: today), or null. 29 February: the 28th in other years. */
export function daysToBirthday(birthday: string | null | undefined, today = new Date()): number | null {
  const m = birthday?.match(/^(?:\d{4}|-)-(\d{2})-(\d{2})$/)
  if (!m) return null
  const start = new Date(today.getFullYear(), today.getMonth(), today.getDate())
  const on = (y: number) => {
    const d = new Date(y, Number(m[1]) - 1, Number(m[2]))
    // `new Date(2027, 1, 29)` rolls over to 1 March: use the 28th then.
    return d.getMonth() === Number(m[1]) - 1 ? d : new Date(y, 1, 28)
  }
  let next = on(start.getFullYear())
  if (next < start) next = on(start.getFullYear() + 1)
  return Math.round((next.getTime() - start.getTime()) / 86_400_000)
}

/** "14 March" or "14 March 1990". */
export function birthdayLabel(birthday: string): string {
  const m = birthday.match(/^(\d{4}|-)-(\d{2})-(\d{2})$/)
  if (!m) return birthday
  const d = new Date(2000, Number(m[2]) - 1, Number(m[3]))
  const day = d.toLocaleDateString([], { day: 'numeric', month: 'long' })
  return m[1] === '-' ? day : `${day} ${m[1]}`
}

export function personName(p: { name: string; email: string; phones?: { number: string }[] }) {
  if (p.name) return p.name
  // Someone without a name nor an address: their number.
  if (p.email.startsWith('~')) return p.phones?.[0]?.number ?? 'Someone'
  return p.email
}

/** "Claire Dubois <claire@x.fr>" → the address. */
export function emailIn(text: string | null | undefined) {
  return text?.match(/[^\s<>,;"]+@[^\s<>,;"]+/)?.[0]?.toLowerCase() ?? null
}

// ---- Mail, across accounts --------------------------------------------------------

export interface UnreadMessage {
  account: number
  folder: string
  uid: number
  subject: string
  from: Sender | null
  date: string | null
}

export function fetchUnread(limit = 8) {
  return $fetch<{ total: number; messages: UnreadMessage[] }>('/api/mail/unread', { query: { limit } })
}

/** The Mail page, opened on one message. */
export function messageLink(m: { account: number; folder: string; uid: number }) {
  return { path: '/mail', query: { account: String(m.account), folder: m.folder, uid: String(m.uid) } }
}

/** The Ctrl+K palette is open (the dock's search button opens it too). */
export function usePalette() {
  return useState('palette-open', () => false)
}

// ---- Related mail (an event's people, a person) ------------------------------------

export interface SharedFile {
  name: string
  mime: string
  size: number
  account: number
  folder: string
  uid: number
  index: number
  date: string | null
}

export interface Related {
  messages: (MessageRef & { from: string | null; preview: string | null })[]
  files: SharedFile[]
}

/** Recent mail and files from an event's guests and organizer. */
export function fetchRelated(eventId: number) {
  return $fetch<Related>(`/api/agenda/events/${eventId}/related`)
}

export function fileLink(f: SharedFile) {
  return `/api/mail/messages/${f.uid}/attachments/${f.index}?account=${f.account}&folder=${encodeURIComponent(f.folder)}`
}

/** "PDF", "DOCX"… from a file name. */
export function fileKind(name: string) {
  const ext = name.split('.').pop() ?? ''
  return ext.length && ext.length <= 4 && ext !== name ? ext.toUpperCase() : 'FILE'
}

// ---- The App library's status card ------------------------------------------------

export interface SystemInfo {
  /** The status card's "Backup" line (null in demo mode). */
  backup: {
    enabled: boolean
    last_at: number | null
    last_error: string | null
    next_at: number | null
    size: number | null
    /** Off-site copies failing for a day or more. */
    offsite_failing?: boolean
    offsite_error?: string | null
  } | null
  version: string
  memory: number | null
  data: number | null
  accounts: number
  calendars: number
  people: number
  /** Tasks still to do. */
  tasks?: number
  notes?: number
  /** Different files found in your mail. */
  files?: number
  /** The AI app's conversations. */
  conversations?: number
}

export function fetchSystem() {
  return $fetch<SystemInfo>('/api/system')
}

export function bytes(n: number) {
  if (n < 1024 * 1024) return `${Math.max(1, Math.round(n / 1024))} KB`
  if (n < 1024 ** 3) return `${Math.round(n / 1024 / 1024)} MB`
  return `${(n / 1024 ** 3).toFixed(1)} GB`
}

/** The organizer's name in "Claire Dubois <claire@x.fr>". */
export function nameIn(who: string | null | undefined) {
  if (!who) return ''
  return who.replace(/\s*<[^>]*>\s*$/, '').trim() || emailIn(who) || who
}

// ---- Backups (Settings › Backups) ------------------------------------------------------

export interface BackupSettings {
  enabled: boolean
  /** "03:00", the server's local time. */
  time: string
  keep: number
}

export interface BackupFile {
  name: string
  size: number
  created_at: number
}

export interface BackupStatus {
  settings: BackupSettings
  dir: string
  backups: BackupFile[]
  last_at: number | null
  last_error: string | null
  next_at: number | null
  key_fingerprint: string
  remote: RemoteStatus
}

/** Settings › Backups › Off-site copy. The secret is only ever sent, never read back (`has_secret`). */
export type RemoteKind = 'webdav' | 's3'
export interface RemoteSettings {
  enabled: boolean
  kind: RemoteKind
  url: string
  bucket: string
  region: string
  prefix: string
  path_style: boolean
  username: string
  keep: number
  has_secret: boolean
  /** A new password or secret key; empty keeps the saved one. */
  secret?: string | null
}
export interface RemoteStatus {
  settings: RemoteSettings
  last_at: number | null
  last_name: string | null
  last_size: number | null
  last_error: string | null
  failing_since: number | null
  pending: string | null
  next_try_at: number | null
  failing_for_a_day: boolean
}

export function saveRemote(settings: RemoteSettings) {
  return $fetch<RemoteStatus>('/api/backups/remote', { method: 'PUT', body: settings })
}

export function testRemote(settings: RemoteSettings) {
  return $fetch<{ message: string }>('/api/backups/remote/test', { method: 'POST', body: settings })
}

export function forgetRemote() {
  return $fetch<RemoteStatus>('/api/backups/remote', { method: 'DELETE' })
}

export function sendOffsite(name: string) {
  return $fetch<{ name: string; size: number }>(`/api/backups/${encodeURIComponent(name)}/offsite`, { method: 'POST' })
}

export function fetchBackups() {
  return $fetch<BackupStatus>('/api/backups')
}

export function saveBackupSettings(settings: BackupSettings) {
  return $fetch<BackupSettings>('/api/backups/settings', { method: 'PATCH', body: settings })
}

export function backupNow() {
  return $fetch<BackupFile>('/api/backups', { method: 'POST' })
}

export function deleteBackup(name: string) {
  return $fetch(`/api/backups/${encodeURIComponent(name)}`, { method: 'DELETE' })
}

/** "Today 03:00", "Yesterday 03:00", "Mon 12 Oct 03:00". */
export function whenShort(unix: number, now = new Date()) {
  const d = new Date(unix * 1000)
  const days = Math.round((startOfDay(d).getTime() - startOfDay(now).getTime()) / DAY_MS)
  const day =
    days === 0 ? 'Today' : days === -1 ? 'Yesterday' : days === 1 ? 'Tomorrow' : d.toLocaleDateString([], { weekday: 'short', day: 'numeric', month: 'short' })
  return `${day} ${timeLabel(d)}`
}
