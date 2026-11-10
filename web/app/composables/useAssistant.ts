// What the assistant's answers need in the browser, shared by Ctrl+K and the
// AI app: drawing an answer (as data, never HTML) and running a proposal
// once you confirmed it. Names start with `ai`: composables share one
// auto-import namespace.
import type { AgendaEvent, Calendar } from '~/composables/useAgenda'
import type { AiAnswer, AiMailVerb, AiProposal } from '~/composables/useAi'
import type { Draft } from '~/composables/useCompose'
import type { NoteSummary } from '~/composables/useNotes'
import type { Task } from '~/composables/useTasks'

export type AiEventProposal = Extract<AiProposal, { kind: 'event' }>
export type AiMoveProposal = Extract<AiProposal, { kind: 'move' }>
export type AiEmailProposal = Extract<AiProposal, { kind: 'email' }>
export type AiTaskProposal = Extract<AiProposal, { kind: 'task' }>
export type AiNoteProposal = Extract<AiProposal, { kind: 'note' }>

// ---- Drawing an answer -------------------------------------------------------------

/** One piece of a line: text, **bold**, or a citation: [m3] an email, [e2] an event, [n1] a note, [t4] a task. */
export type AiPart = { text: string; bold?: boolean; ref?: string }

/**
 * The answer as lines of parts. Built as data, not HTML, so nothing the
 * model writes can run as code in the page.
 */
export function aiAnswerLines(text: string) {
  return text.split('\n').map((line) => {
    const bullet = /^\s*[-•*]\s+/.test(line)
    const parts: AiPart[] = []
    for (const piece of line.replace(/^\s*[-•*]\s+/, '').split(/(\[[ment]\d+]|\*\*[^*]+\*\*)/)) {
      if (!piece) continue
      const ref = piece.match(/^\[([ment]\d+)]$/)
      if (ref) parts.push({ text: ref[1]!, ref: ref[1] })
      else if (piece.startsWith('**') && piece.endsWith('**')) parts.push({ text: piece.slice(2, -2), bold: true })
      else parts.push({ text: piece })
    }
    return { bullet, parts }
  })
}

/** What a citation shows: the email's subject or the event's title. */
export function aiRefLabel(answer: AiAnswer | null | undefined, id: string) {
  const m = answer?.messages[id]
  if (m) return m.subject || '(no subject)'
  const e = answer?.events[id]
  if (e) return e.title
  const n = answer?.notes?.[id]
  if (n) return n.title
  const t = answer?.tasks?.[id]
  if (t) return t.title
  return id
}

/** Emails the answer cites, in order, once each (shown as cards under it). */
export function aiCitedMessages(text: string, answer: AiAnswer | null | undefined) {
  const ids = [...text.matchAll(/\[(m\d+)]/g)].map((m) => m[1]!)
  return [...new Set(ids)].flatMap((id) => {
    const hit = answer?.messages[id]
    return hit ? [{ id, hit }] : []
  })
}

/** A note or task the answer cites, as a card. */
export interface AiCitedItem {
  id: string
  kind: 'note' | 'task'
  title: string
  detail: string
  to: { path: string; query: Record<string, string> }
}

/** Notes and tasks the answer cites, in order, once each (cards under it, after the emails). */
export function aiCitedItems(text: string, answer: AiAnswer | null | undefined): AiCitedItem[] {
  const ids = [...new Set([...text.matchAll(/\[([nt]\d+)]/g)].map((m) => m[1]!))]
  return ids.flatMap((id): AiCitedItem[] => {
    const note = answer?.notes?.[id]
    if (note) return [{ id, kind: 'note', title: note.title, detail: aiNoteDetail(note), to: aiNoteLink(note) }]
    const task = answer?.tasks?.[id]
    if (task) return [{ id, kind: 'task', title: task.title, detail: aiTaskDetail(task), to: aiTaskLink(task) }]
    return []
  })
}
function aiNoteDetail(n: NoteSummary) {
  const parts = ['Note']
  if (n.event) parts.push(n.event.title)
  if (n.people.length) parts.push(n.people.map((p) => (p.name || p.email).split(' ')[0]).join(', '))
  if (n.open_todos) parts.push(`${n.open_todos} to-do${n.open_todos > 1 ? 's' : ''} open`)
  return parts.join(' · ')
}
function aiTaskDetail(t: Task) {
  return ['Task', t.done_at ? 'done' : t.due != null ? dueLabel(t) : 'no date'].join(' · ')
}
export function aiNoteLink(n: { id: number }) {
  return { path: '/notes', query: { note: String(n.id) } }
}
/** A task, shown in its list (done ones in Done). */
export function aiTaskLink(t: Task): AiCitedItem['to'] {
  const query: Record<string, string> = t.done_at ? { view: 'done' } : { list: String(t.list) }
  return { path: '/tasks', query: { ...query, task: String(t.id) } }
}

/** Where a citation leads, for notes and tasks (emails and events are handled by the caller). */
export function aiRefLink(answer: AiAnswer | null | undefined, id: string) {
  const n = answer?.notes?.[id]
  if (n) return aiNoteLink(n)
  const t = answer?.tasks?.[id]
  if (t) return aiTaskLink(t)
  return null
}

export function aiEventLink(e: AgendaEvent) {
  return { path: '/agenda', query: { view: 'week', date: isoDate(eventSpan(e).start), event: String(e.id) } }
}

// ---- Proposals ----------------------------------------------------------------------

export function aiDayTime(t: number) {
  const d = new Date(t * 1000)
  return `${d.toLocaleDateString([], { weekday: 'short', day: 'numeric', month: 'short' })} · ${timeLabel(d)}`
}
export function aiProposalSpan(p: AiEventProposal | AiMoveProposal) {
  return `${aiDayTime(p.start)} – ${timeLabel(new Date(p.end * 1000))}`
}
export function aiRecipients(p: AiEmailProposal) {
  return p.to
    .split(/[,;]/)
    .map((x) => x.trim())
    .filter(Boolean)
}
export function aiProposalTitle(p: AiProposal) {
  if (p.kind === 'event') return `Create “${p.title}”`
  if (p.kind === 'move') return `Move “${p.title}”`
  if (p.kind === 'task') return `Add the task “${p.title}”`
  if (p.kind === 'note') return `Write the note “${p.title}”`
  if (p.kind === 'append_note') return `Add to the note “${p.title}”`
  if (p.kind === 'complete_task') return `Mark “${p.title}” as done`
  if (p.kind === 'mail_action') return aiMailTitle(p.action, p.emails.length, p.folder)
  if (p.kind === 'answer_invitation') {
    if (p.answer === 'accepted') return `Accept “${p.title}”`
    if (p.answer === 'declined') return `Decline “${p.title}”`
    return `Answer “${p.title}”: maybe`
  }
  if (p.kind === 'edit_note') return `Edit the note “${p.title}”`
  const to = aiRecipients(p)
  return to.length === 1 ? `Email ${nameIn(to[0])}` : `Email the ${to.length} recipients`
}
const AI_PROPOSAL_APP: Record<AiProposal['kind'], string> = {
  email: 'Mail',
  event: 'Agenda',
  move: 'Agenda',
  task: 'Tasks',
  note: 'Notes',
  append_note: 'Notes',
  complete_task: 'Tasks',
  mail_action: 'Mail',
  answer_invitation: 'Agenda',
  edit_note: 'Notes',
}

/** "Archive 3 emails", "Move 1 email to Clients", "Mark 4 emails as read". */
export function aiMailTitle(action: AiMailVerb, n: number, folder: string | null) {
  const emails = n === 1 ? '1 email' : `${n} emails`
  switch (action) {
    case 'archive':
      return `Archive ${emails}`
    case 'trash':
      return `Move ${emails} to the trash`
    case 'move':
      return `Move ${emails} to ${folder}`
    case 'read':
      return `Mark ${emails} as read`
    case 'unread':
      return `Mark ${emails} as unread`
    case 'star':
      return `Star ${emails}`
    case 'unstar':
      return `Unstar ${emails}`
  }
}

/**
 * Shows what confirmed proposals did, with *Undo* when some can be undone.
 * The toaster comes from the caller (`useToasts()` in its setup).
 */
export function aiReport(
  toast: ReturnType<typeof useToasts>,
  runs: AiRun[],
) {
  if (!runs.length) return
  const undos = runs.flatMap((r) => (r.undo ? [r.undo] : []))
  const undo = async () => {
    try {
      for (const u of undos) await u()
      toast.show('Undone')
    } catch (err) {
      toast.show(apiErrorMessage(err), { tone: 'error' })
    }
  }
  toast.show(
    runs.map((r) => r.said).join(' · '),
    undos.length ? { action: { label: 'Undo', run: () => void undo() }, duration: 8000 } : {},
  )
}

/** What running a proposal did, and how to take it back when it can be. */
export interface AiRun {
  said: string
  /** Undo (mail actions): puts the emails back as they were. */
  undo?: () => Promise<void>
}
/** "Agenda + Mail": which apps the proposals touch. */
export function aiProposalApps(proposals: AiProposal[]) {
  return [...new Set(proposals.map((p) => AI_PROPOSAL_APP[p.kind]))].sort().join(' + ')
}
/** "Due Fri 18 Oct · Havre · Inbox". */
export function aiTaskWhen(p: AiTaskProposal) {
  const due = p.due != null ? `Due ${dueLabel(p)}` : 'No date'
  return `${due} · ${p.list}`
}

/** Other events at the time of an event to create or move. */
export async function aiConflicts(p: AiProposal): Promise<string[]> {
  if (p.kind !== 'event' && p.kind !== 'move') return []
  const others = await fetchEvents(p.start, p.end).catch(() => [] as AgendaEvent[])
  const skip = p.kind === 'move' ? p.event : -1
  return others
    .filter((e) => e.id !== skip && !e.all_day && e.start < p.end && e.end > p.start && e.my_response !== 'declined')
    .map((e) => e.title)
}

/**
 * Runs a proposal you confirmed: creates or moves the event, writes the
 * email, adds or ticks the task, writes, adds to or edits the note, acts on
 * emails, answers an invitation. `send: false` opens the email in compose
 * instead of sending it.
 * Returns what to tell you ("Created “Kick-off”") and, for emails, how to
 * undo it; or null. Each run is noted in Settings › AI › activity.
 */
export async function runAiProposal(p: AiProposal, send: boolean): Promise<AiRun | null> {
  if (p.kind === 'mail_action') return runMailAction(p)
  const said = await runOther(p, send)
  return said ? { said } : null
}

/**
 * Emails: the same calls as Mail's own buttons, one per account and folder.
 * Only the emails still where the assistant saw them are touched (the card
 * may be minutes old).
 */
async function runMailAction(p: Extract<AiProposal, { kind: 'mail_action' }>): Promise<AiRun> {
  const groups = new Map<string, { account: number; folder: string; to: string | null; uids: number[] }>()
  for (const e of p.emails) {
    const key = `${e.account}\n${e.folder}\n${e.to ?? ''}`
    const g = groups.get(key) ?? { account: e.account, folder: e.folder, to: e.to, uids: [] }
    g.uids.push(e.uid)
    groups.set(key, g)
  }
  const undos: (() => Promise<unknown>)[] = []
  let acted = 0
  for (const g of groups.values()) {
    const { uids } = await $fetch<{ uids: number[] }>('/api/mail/present', {
      method: 'POST',
      body: { account: g.account, folder: g.folder, uids: g.uids },
    })
    if (!uids.length) continue
    const sel = { account: g.account, folder: g.folder, uids }
    if (p.action === 'archive' || p.action === 'trash' || p.action === 'move') {
      const out = await applyMailAction(g.account, p.action, g.folder, uids, g.to ?? undefined)
      if (out.result === 'moved') undos.push(() => undoMove(g.account, out.to, g.folder, out.message_ids))
    } else {
      // Read / unread / star / unstar: undoing sets the opposite.
      const flags = (on: boolean) =>
        p.action === 'read' || p.action === 'unread' ? { seen: on === (p.action === 'read') } : { flagged: on === (p.action === 'star') }
      await setFlags(sel, flags(true))
      undos.push(() => setFlags(sel, flags(false)))
    }
    acted += uids.length
  }
  const gone = p.emails.length - acted
  if (!acted) throw new Error('Those emails aren’t where they were any more: nothing was changed.')
  const title = aiMailTitle(p.action, acted, p.folder)
  logConfirmed('Organized mail', title)
  // "Archive 3 emails" → "Archived 3 emails"
  const done = title
    .replace(/^Archive/, 'Archived')
    .replace(/^Move/, 'Moved')
    .replace(/^Mark/, 'Marked')
    .replace(/^Star/, 'Starred')
    .replace(/^Unstar/, 'Unstarred')
  return {
    said: gone ? `${done} (${gone} had already moved)` : done,
    undo: async () => {
      for (const undo of undos) await undo()
    },
  }
}

async function runOther(p: Exclude<AiProposal, { kind: 'mail_action' }>, send: boolean): Promise<string | null> {
  if (p.kind === 'answer_invitation') {
    await $fetch('/api/agenda/invitation/respond', {
      method: 'POST',
      body: { account: p.account, folder: p.folder, uid: p.uid, index: p.index, response: p.answer },
    })
    const said = p.answer === 'accepted' ? 'Accepted' : p.answer === 'declined' ? 'Declined' : 'Answered maybe to'
    logConfirmed('Answered an invitation', `${said} ${p.title}`)
    return `${said} “${p.title}”`
  }
  if (p.kind === 'edit_note') {
    // The note as it is now: the passage must still be there, once.
    const note = await fetchNote(p.note)
    if (note.body.split(p.find).length !== 2) {
      throw new Error(`“${note.title}” changed since: open it to make the change yourself.`)
    }
    // A function as the replacement: `$&` and such in the text stay as they are.
    const body = note.body.replace(p.find, () => p.replace)
    await saveNote(note.id, noteInput(note, { body }))
    logConfirmed('Edited a note', note.title)
    return `Edited “${note.title}”`
  }
  if (p.kind === 'event') {
    const calendars = await $fetch<Calendar[]>('/api/agenda/calendars')
    const writable = calendars.filter((c) => !c.read_only && c.enabled)
    const calendar = writable.find((c) => c.is_default) ?? writable[0]
    if (!calendar) throw new Error('No calendar to add it to.')
    const canInvite = calendar.source === 'microsoft' || calendar.source === 'google'
    // Your default reminder, like an event made in the editor.
    const defaults = await fetchAgendaSettings().catch(() => null)
    const created = await createEvent({
      calendar: calendar.id,
      title: p.title,
      location: p.location,
      description: p.notes,
      start: p.start,
      end: p.end,
      all_day: false,
      attendees: canInvite ? p.guests : [],
      repeat: null,
      time_zone: browserTimeZone(),
      reminder: defaults?.default_reminder ?? null,
    })
    logConfirmed('Created an event', `${created.title} · ${calendar.name}`)
    return `Created “${created.title}”${p.guests.length && !canInvite ? ' (guests not invited: this calendar can’t send invitations)' : ''}`
  }
  if (p.kind === 'move') {
    // The event as it is now (it may have changed since the answer).
    const around = await fetchEvents(p.from_start - 1, p.from_end + 1)
    const e = around.find((x) => x.id === p.event)
    if (!e) throw new Error(`“${p.title}” is no longer at that time: open the Agenda to move it.`)
    // A repeating event: only this occurrence moves.
    await updateEvent(e.id, movedInput(e, p.start, p.end, 'this'), 'this')
    logConfirmed('Moved an event', `${e.title} → ${aiDayTime(p.start)}`)
    return `Moved “${e.title}”`
  }
  if (p.kind === 'task') {
    if (p.from_note) {
      // Like the note's own "Make a task": linked to it, ticked together.
      await noteTodoToTask(p.from_note.note, p.from_note.line)
      logConfirmed('Added a task', `${p.title} · from “${p.from_note.title}”`)
      return `Added “${p.title}” to Tasks`
    }
    const created = await createTask({
      title: p.title,
      notes: p.notes,
      due: p.due,
      due_at: p.due_at,
      links: { person: p.person, event: p.event },
    })
    logConfirmed('Added a task', `${created.title} · ${p.list}`)
    return `Added “${created.title}” to Tasks`
  }
  if (p.kind === 'note') {
    const created = await createNote({ title: p.title, body: p.body, people: p.people, event: p.event })
    logConfirmed('Wrote a note', created.title)
    return `Wrote the note “${created.title}”`
  }
  if (p.kind === 'append_note') {
    // The note as it is now (you may have changed it since the answer), then the text after a blank line.
    const note = await fetchNote(p.note)
    const body = note.body.trimEnd() ? `${note.body.trimEnd()}\n\n${p.text.trim()}\n` : `${p.text.trim()}\n`
    await saveNote(note.id, noteInput(note, { body }))
    logConfirmed('Added to a note', note.title)
    return `Added to “${note.title}”`
  }
  if (p.kind === 'complete_task') {
    await setTaskDone(p.task, true)
    logConfirmed('Ticked a task', p.title)
    return `Ticked “${p.title}”`
  }
  const account = useAccounts().data.value.find((a) => a.can_send)
  if (!account) throw new Error('No account can send yet: add or reconnect one in Settings.')
  const draft = await $fetch<Draft>('/api/drafts', {
    method: 'POST',
    body: { account: account.id, kind: 'new', to: p.to, subject: p.subject },
  })
  await saveDraft(draft.id, { body: p.body })
  refreshNuxtData('drafts')
  if (!send) {
    openDraft(draft.id)
    return null
  }
  await sendDraft(draft.id)
  logConfirmed('Sent an email', `${p.subject} → ${p.to}`)
  return `Sent “${p.subject}”`
}
