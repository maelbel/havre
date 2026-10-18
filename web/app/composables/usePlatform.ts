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
  email: string
  name: string
  messages: number
  meetings: number
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
  recent_messages: MessageRef[]
  upcoming: AgendaEvent[]
  past: AgendaEvent[]
  files: SharedFile[]
}

export function searchPeople(q: string, limit = 200) {
  return $fetch<PersonEntry[]>('/api/people', { query: { q, limit } })
}

export function fetchPerson(email: string) {
  return $fetch<PersonDetail>(`/api/people/${encodeURIComponent(email)}`)
}

export function personName(p: { name: string; email: string }) {
  return p.name || p.email
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
  backup: { enabled: boolean; last_at: number | null; last_error: string | null; next_at: number | null; size: number | null } | null
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
