// Tasks. Mirrors `/api/tasks/*` (crates/server/src/tasks.rs).
//
// A task is due on a *day* (`due`: midnight UTC of that date, like all-day
// events) and maybe at a *time* (`due_at`: the moment). Days are compared
// with your local date: "today" is `localDay(new Date())`.

export interface TaskList {
  id: number
  source: 'local' | 'microsoft' | 'google' | 'caldav'
  account: number | null
  caldav: number | null
  /** Who it belongs to: the account's name, or "Havre". */
  owner: string
  name: string
  color: string
  read_only: boolean
  is_default: boolean
  open: number
  synced_at: number | null
  error: string | null
}

export interface MailLink {
  account: number
  message_id: string | null
  folder: string | null
  uid: number | null
  subject: string
  from: string
}
export interface PersonLink {
  email: string
  name: string | null
}
export interface EventLink {
  uid: string
  title: string
  start: number
}
export interface Links {
  mail?: MailLink | null
  person?: PersonLink | null
  event?: EventLink | null
}

export interface Task extends Links {
  id: number
  list: number
  title: string
  notes: string
  due: number | null
  due_at: number | null
  remind_at: number | null
  done_at: number | null
  updated_at: number | null
  read_only: boolean
}

export interface TaskInput {
  list?: number | null
  title: string
  notes?: string
  due?: number | null
  due_at?: number | null
  remind_at?: number | null
  done?: boolean
  links?: Links | null
}

export interface Parsed {
  title: string
  due: number | null
  due_at: number | null
  list: number | null
}

export interface TaskSource {
  key: string
  kind: 'microsoft' | 'google' | 'caldav'
  name: string
  enabled: boolean
  /** The sign-in allows tasks (false: reconnect the account). */
  allowed: boolean
  lists: number
  error: string | null
}

export interface TaskQuery {
  list?: number
  done?: boolean
  from?: number
  to?: number
  person?: string
  message_id?: string
  q?: string
  limit?: number
}

const DAY = 86_400

// ---- Days -----------------------------------------------------------------------

/** Your local date → midnight UTC of that date (how `due` is stored). */
export function localDay(d: Date = new Date()): number {
  return Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()) / 1000
}

/** A stored day → a local Date at midnight (for display). */
export function dayToDate(day: number): Date {
  const u = new Date(day * 1000)
  return new Date(u.getUTCFullYear(), u.getUTCMonth(), u.getUTCDate())
}

/** `2026-10-16` (for `<input type="date">`) ↔ stored day. */
export function dayToIso(day: number | null): string {
  if (day == null) return ''
  return new Date(day * 1000).toISOString().slice(0, 10)
}
export function isoToDay(iso: string): number | null {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(iso)) return null
  return Date.parse(`${iso}T00:00:00Z`) / 1000
}

/** The moment `hh:mm` local time on a stored day. */
export function atTime(day: number, hhmm: string): number | null {
  const m = /^(\d{1,2}):(\d{2})$/.exec(hhmm)
  if (!m) return null
  const d = dayToDate(day)
  d.setHours(Number(m[1]), Number(m[2]), 0, 0)
  return Math.floor(d.getTime() / 1000)
}

export function taskTime(ts: number): string {
  return new Date(ts * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}

/** "Fri 16 Oct" (no comma, as in the design); `long`: "Friday 16 October". */
export function dayText(d: Date, long = false): string {
  const parts = new Intl.DateTimeFormat('en-GB', {
    weekday: long ? 'long' : 'short',
    day: 'numeric',
    month: long ? 'long' : 'short',
  }).formatToParts(d)
  const get = (type: string) => parts.find((p) => p.type === type)?.value ?? ''
  return `${get('weekday')} ${get('day')} ${get('month')}`
}

/** "Today", "Tomorrow", "Yesterday", "Fri 16 Oct". */
export function taskDayLabel(day: number, today = localDay()): string {
  const diff = Math.round((day - today) / DAY)
  if (diff === 0) return 'Today'
  if (diff === 1) return 'Tomorrow'
  if (diff === -1) return 'Yesterday'
  return dayText(dayToDate(day))
}

/** "Today · 17:00", "Tue 13 Oct", "No date". */
export function dueLabel(t: Pick<Task, 'due' | 'due_at'>, today = localDay()): string {
  if (t.due == null) return 'No date'
  const day = taskDayLabel(t.due, today)
  return t.due_at != null ? `${day} · ${taskTime(t.due_at)}` : day
}

export type DueGroup = 'late' | 'today' | 'tomorrow' | 'week' | 'later' | 'none'

/** Which heading a task goes under. */
export function dueGroup(t: Pick<Task, 'due' | 'done_at'>, today = localDay()): DueGroup {
  if (t.due == null) return 'none'
  const diff = Math.round((t.due - today) / DAY)
  if (diff < 0) return t.done_at ? 'today' : 'late'
  if (diff === 0) return 'today'
  if (diff === 1) return 'tomorrow'
  if (diff <= 7) return 'week'
  return 'later'
}

export function isLate(t: Pick<Task, 'due' | 'done_at'>, today = localDay()): boolean {
  return !t.done_at && t.due != null && t.due < today
}

/** What the editor sends to change a task (everything it has). */
export function taskInput(t: Task, change: Partial<TaskInput> = {}): TaskInput {
  return {
    list: t.list,
    title: t.title,
    notes: t.notes,
    due: t.due,
    due_at: t.due_at,
    remind_at: t.remind_at,
    done: !!t.done_at,
    ...change,
  }
}

// ---- The API --------------------------------------------------------------------

export function fetchTasks(query: TaskQuery = {}) {
  return $fetch<Task[]>('/api/tasks', { query })
}
export function fetchTaskLists() {
  return $fetch<TaskList[]>('/api/tasks/lists')
}
export function createTask(input: TaskInput) {
  return $fetch<Task>('/api/tasks', { method: 'POST', body: input })
}
export function updateTask(id: number, input: TaskInput) {
  return $fetch<Task>(`/api/tasks/${id}`, { method: 'PUT', body: input })
}
export function setTaskDone(id: number, done: boolean) {
  return $fetch<Task>(`/api/tasks/${id}/done`, { method: 'PUT', body: { done } })
}
export function deleteTask(id: number) {
  return $fetch(`/api/tasks/${id}`, { method: 'DELETE' })
}
export function quickAddTask(text: string, links?: Links) {
  return $fetch<Task>('/api/tasks/quick', { method: 'POST', body: { text, utc_offset: utcOffset(), links } })
}
export function parseTask(text: string) {
  return $fetch<Parsed>('/api/tasks/parse', { method: 'POST', body: { text, utc_offset: utcOffset() } })
}
/** A task linked to an email (and its sender). `list`/`due` chosen in the menu win over the text. */
export function taskFromMail(
  ref: { account: number; folder: string; uid: number },
  text: string,
  choice: { list?: number | null; due?: number | null } = {},
) {
  return $fetch<Task>('/api/tasks/from-mail', {
    method: 'POST',
    body: { ...ref, text, utc_offset: utcOffset(), list: choice.list ?? undefined, due: choice.due ?? undefined },
  })
}

/** "Re: Devis maintenance Q4" → "Devis maintenance Q4" (a task's title). */
export function subjectToTitle(subject: string): string {
  return subject.replace(/^\s*((re|fw|fwd|tr|aw|wg)\s*(\[\d+])?\s*:\s*)+/i, '').trim()
}
export function locateTaskMail(id: number) {
  return $fetch<{ account: number; folder: string; uid: number }>(`/api/tasks/${id}/mail`)
}
export function createTaskList(name: string) {
  return $fetch<TaskList>('/api/tasks/lists', { method: 'POST', body: { name } })
}
export function updateTaskList(id: number, change: { name?: string; color?: string; is_default?: boolean }) {
  return $fetch<TaskList>(`/api/tasks/lists/${id}`, { method: 'PATCH', body: change })
}
export function deleteTaskList(id: number) {
  return $fetch(`/api/tasks/lists/${id}`, { method: 'DELETE' })
}
export function syncTasks() {
  return $fetch('/api/tasks/sync', { method: 'POST' })
}
export function fetchTaskSources() {
  return $fetch<TaskSource[]>('/api/tasks/sources')
}
export function setTaskSource(key: string, enabled: boolean) {
  return $fetch<TaskSource[]>(`/api/tasks/sources/${encodeURIComponent(key)}`, {
    method: 'PUT',
    body: { enabled },
  })
}

/** Opens a task's email in Mail (wherever it is now). */
export async function openTaskMail(task: Task) {
  const where = await locateTaskMail(task.id)
  await navigateTo({ path: '/mail', query: { account: where.account, folder: where.folder, uid: where.uid } })
}

// ---- Shared state ---------------------------------------------------------------

/**
 * Is the Tasks app in your dock? Mail, Agenda, Today and People only show
 * tasks when it is (add it from the App library).
 */
export function useTasksEnabled() {
  const { apps } = useApps()
  return computed(() => apps.value.some((a) => a.id === 'tasks' && a.enabled))
}

/** The lists, loaded once and shared by every page. */
export function useTaskLists() {
  const lists = useState<TaskList[]>('task-lists', () => [])
  const loaded = useState('task-lists-loaded', () => false)
  async function refresh() {
    lists.value = await fetchTaskLists()
    loaded.value = true
  }
  if (!loaded.value && import.meta.client) refresh().catch(() => {})
  const byId = computed(() => new Map(lists.value.map((l) => [l.id, l])))
  return { lists, loaded, refresh, byId }
}

/** Sorts tasks: by day, then time, then title (no date last). */
export function byDue(a: Task, b: Task): number {
  const da = a.due ?? Infinity
  const db = b.due ?? Infinity
  if (da !== db) return da - db
  const ta = a.due_at ?? a.due ?? 0
  const tb = b.due_at ?? b.due ?? 0
  if (ta !== tb) return ta - tb
  return a.title.localeCompare(b.title)
}
