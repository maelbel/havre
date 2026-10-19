// The Agenda. Mirrors `/api/agenda/*` (crates/server/src/agenda.rs).
//
// Times travel as unix seconds. All-day events start at midnight UTC of
// their first day and end at midnight UTC of the day after the last one;
// timed events are shown in the browser's time zone.

export interface Calendar {
  id: number
  source: 'local' | 'microsoft' | 'google' | 'caldav'
  account: number | null
  caldav: number | null
  name: string
  color: string
  read_only: boolean
  enabled: boolean
  is_default: boolean
  synced_at: number | null
  error: string | null
}

export interface Attendee {
  name: string | null
  email: string
  status: 'accepted' | 'declined' | 'tentative' | 'needs_action'
}

export interface AgendaEvent {
  id: number
  calendar: number
  title: string
  location: string
  description: string
  start: number
  end: number
  all_day: boolean
  recurring: boolean
  read_only: boolean
  organizer: string | null
  attendees: Attendee[]
  join_url: string | null
  /** iCalendar UID: the same in the invitation email and the calendar. */
  uid: string | null
  /** Your answer when you're a guest; `null` for your own events. */
  my_response: Response | 'needs_action' | null
}

export type Response = 'accepted' | 'tentative' | 'declined'

/**
 * How an answer button looks (Accept / Maybe / Decline): the one you gave is
 * filled in its colour (green, gold, red); otherwise Accept is the main button.
 */
export function responseButton(response: Response, chosen: boolean, main = false) {
  if (chosen) {
    const color = response === 'accepted' ? 'success' : response === 'declined' ? 'error' : 'primary'
    return { color, variant: 'soft' } as const
  }
  return main ? ({ color: 'primary', variant: 'solid' } as const) : ({ color: 'neutral', variant: 'outline' } as const)
}

export const RESPONSE_LABELS: Record<string, string> = {
  accepted: 'Accepted',
  tentative: 'Maybe',
  declined: 'Declined',
  needs_action: 'No answer yet',
}

/** The card above an invitation in Mail. */
export interface InvitationView {
  method: string
  title: string
  start: number
  end: number
  all_day: boolean
  location: string
  organizer: string | null
  occurrences: number
  event: AgendaEvent | null
  my_response: Response | null
  conflicts: AgendaEvent[]
  can_respond: boolean
}

export interface InvitationRef {
  account: number
  folder: string
  uid: number
  index: number
}

export interface EventInput {
  calendar: number
  title: string
  location: string
  description: string
  start: number
  end: number
  all_day: boolean
  attendees: string[]
}

export interface CalDavAccount {
  id: number
  name: string
  url: string
  username: string
}

export function useCalendars() {
  return useFetch<Calendar[]>('/api/agenda/calendars', {
    key: 'calendars',
    lazy: true,
    server: false,
    default: () => [],
  })
}

export function fetchEvents(from: number, to: number) {
  return $fetch<AgendaEvent[]>('/api/agenda/events', { query: { from, to } })
}

export function createEvent(input: EventInput) {
  return $fetch<AgendaEvent>('/api/agenda/events', { method: 'POST', body: input })
}

export function updateEvent(id: number, input: EventInput) {
  return $fetch<AgendaEvent>(`/api/agenda/events/${id}`, { method: 'PATCH', body: input })
}

export function deleteEvent(id: number) {
  return $fetch(`/api/agenda/events/${id}`, { method: 'DELETE' })
}

export function syncAgenda() {
  return $fetch('/api/agenda/sync', { method: 'POST' })
}

export function updateCalendar(id: number, changes: { enabled?: boolean; color?: string; is_default?: boolean }) {
  return $fetch<Calendar>(`/api/agenda/calendars/${id}`, { method: 'PATCH', body: changes })
}

export function useCalDavAccounts() {
  return useFetch<CalDavAccount[]>('/api/agenda/caldav', {
    key: 'caldav',
    lazy: true,
    server: false,
    default: () => [],
  })
}

export function addCalDav(body: { name: string; url: string; username: string; password: string }) {
  return $fetch<CalDavAccount>('/api/agenda/caldav', { method: 'POST', body })
}

export function removeCalDav(id: number) {
  return $fetch(`/api/agenda/caldav/${id}`, { method: 'DELETE' })
}

export function addInvitation(body: { account: number; folder: string; uid: number; index: number }) {
  return $fetch<AgendaEvent[]>('/api/agenda/invitations', { method: 'POST', body })
}

export function fetchInvitation(ref: InvitationRef) {
  return $fetch<InvitationView>('/api/agenda/invitation', { query: ref })
}

/** Accept / Maybe / Decline from Mail. */
export function respondToInvitation(ref: InvitationRef, response: Response) {
  return $fetch<InvitationView>('/api/agenda/invitation/respond', { method: 'POST', body: { ...ref, response } })
}

/** Answer from the Agenda (meetings in Microsoft / Google calendars). */
export function respondToEvent(id: number, response: Response) {
  return $fetch<AgendaEvent>(`/api/agenda/events/${id}/respond`, { method: 'POST', body: { response } })
}

/** An invitation in your mail still to answer or add (the Agenda's "Found in Mail"). */
export interface FoundInvitation {
  account: number
  folder: string
  uid: number
  index: number
  subject: string
  from: string | null
  view: InvitationView
}

export function fetchFoundInMail() {
  return $fetch<FoundInvitation[]>('/api/agenda/found-in-mail')
}

/** ISO 8601 week number (weeks start on Monday; week 1 holds the first Thursday). */
export function isoWeek(d: Date) {
  const t = new Date(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()))
  const day = t.getUTCDay() || 7
  t.setUTCDate(t.getUTCDate() + 4 - day)
  const yearStart = new Date(Date.UTC(t.getUTCFullYear(), 0, 1))
  return Math.ceil(((t.getTime() - yearStart.getTime()) / DAY_MS + 1) / 7)
}

/** Meetings waiting for your answer. */
export function fetchPendingInvitations() {
  return $fetch<AgendaEvent[]>('/api/agenda/invitations/pending')
}

// ---- Dates --------------------------------------------------------------------
// Local dates (the browser's time zone) are plain `Date`s at midnight.

export const DAY_MS = 86_400_000

export function startOfDay(d: Date) {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate())
}

export function addDays(d: Date, n: number) {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() + n)
}

/** Weeks start on Monday. */
export function startOfWeek(d: Date) {
  const day = (d.getDay() + 6) % 7
  return addDays(startOfDay(d), -day)
}

export function startOfMonth(d: Date) {
  return new Date(d.getFullYear(), d.getMonth(), 1)
}

export function sameDay(a: Date, b: Date) {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate()
}

/** `2026-10-14` (local), for URLs and date inputs. */
export function isoDate(d: Date) {
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}

export function parseIsoDate(s: string | undefined | null): Date | null {
  const m = s?.match(/^(\d{4})-(\d{2})-(\d{2})$/)
  return m ? new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])) : null
}

/** `09:30`, from minutes since midnight. */
export function hhmm(minutes: number) {
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${pad(Math.floor(minutes / 60))}:${pad(minutes % 60)}`
}

/** The local day an all-day event's UTC date is (midnight UTC → that date). */
export function utcDateToLocal(seconds: number) {
  const d = new Date(seconds * 1000)
  return new Date(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate())
}

/** The UTC midnight (unix seconds) of a local date, for all-day events. */
export function localDateToUtc(d: Date) {
  return Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()) / 1000
}

/** When an event is shown: local start and end. */
export function eventSpan(e: AgendaEvent): { start: Date; end: Date } {
  if (e.all_day) return { start: utcDateToLocal(e.start), end: utcDateToLocal(e.end) }
  return { start: new Date(e.start * 1000), end: new Date(e.end * 1000) }
}

export function timeLabel(d: Date) {
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}

/** "Tue 14 Oct, 14:00 – 15:30", "Tue 14 – Thu 16 Oct (all day)". */
export function whenLabel(e: AgendaEvent) {
  const { start, end } = eventSpan(e)
  const day = (d: Date) => d.toLocaleDateString([], { weekday: 'short', day: 'numeric', month: 'short' })
  if (e.all_day) {
    const last = addDays(end, -1)
    return sameDay(start, last) ? `${day(start)} · all day` : `${day(start)} – ${day(last)} · all day`
  }
  if (sameDay(start, end)) return `${day(start)}, ${timeLabel(start)} – ${timeLabel(end)}`
  return `${day(start)} ${timeLabel(start)} – ${day(end)} ${timeLabel(end)}`
}

export const SOURCE_LABELS: Record<Calendar['source'], string> = {
  local: 'Havre',
  microsoft: 'Microsoft 365',
  google: 'Google',
  caldav: 'CalDAV',
}
