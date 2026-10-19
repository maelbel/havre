<script setup lang="ts">
import type { SplitPaneSpec } from '~/composables/useSplit'
import type { AiHit } from '~/composables/useAi'
import type { AgendaEvent } from '~/composables/useAgenda'
import type { ActionOutcome, CrossView, MailAction, MessageDetail, MessageSummary, Selection, SyncStatus } from '~/composables/useMail'
import type { PersonDetail } from '~/composables/usePlatform'

useHead({ title: 'Mail · Havre' })
const toast = useToasts()

// What's shown lives in the URL, so a refresh or a shared link reopens it:
//   (nothing) or ?box=all          all inboxes (with two accounts or more)
//   ?box=starred | snoozed         starred / snoozed mail, every account
//   ?box=label&label=3             a label, every account
//   ?account=…&folder=…            one account's folder
//   …&uid=…                        the open message (with ?box=…, account
//                                  and folder say where that message is)
const route = useRoute()
const router = useRouter()

// ---- Accounts ---------------------------------------------------------------
const accounts = useAccounts()
const multi = computed(() => accounts.data.value.length > 1)
type Box = 'all' | 'starred' | 'snoozed' | 'label'
/** A list across accounts (or null: one account's folder). */
const box = computed<Box | null>(() => {
  const b = route.query.box
  if (b === 'starred' || b === 'snoozed') return b
  if (b === 'label' && route.query.label) return 'label'
  return multi.value && (b === 'all' || !route.query.account) ? 'all' : null
})
const labelId = computed(() => (box.value === 'label' ? Number(route.query.label) : null))
/** Rows carry their own account and folder (any list across accounts). */
const allView = computed(() => box.value !== null)
const accountId = computed<number | undefined>(() =>
  route.query.account ? Number(route.query.account) : allView.value ? undefined : accounts.data.value[0]?.id,
)
const noAccounts = computed(() => accounts.status.value === 'success' && accounts.data.value.length === 0)

const folder = computed(() => (route.query.folder as string) || 'INBOX')
const selectedUid = computed(() => (route.query.uid ? Number(route.query.uid) : null))
/** The list being shown: changing it starts a fresh list. */
const listKey = computed(() =>
  box.value ? `${box.value}:${labelId.value ?? ''}` : `${accountId.value}:${folder.value}`,
)

/** `box`: a list across accounts; `false`: one account's folder; unset: as now. */
function go(query: { box?: Box | false; label?: number; account?: number; folder?: string; uid?: number | null }) {
  const nextBox = query.box === undefined ? box.value : query.box || null
  const label = query.label ?? (nextBox === 'label' ? labelId.value : null)
  const next = {
    account: query.account ?? accountId.value,
    folder: query.folder ?? folder.value,
    uid: query.uid === undefined ? selectedUid.value : query.uid,
  }
  // In a list across accounts, account and folder only matter with an open message.
  const where = !nextBox || next.uid
  router.replace({
    query: {
      ...(nextBox && (nextBox !== 'all' || multi.value) ? { box: nextBox } : {}),
      ...(nextBox === 'label' && label ? { label: String(label) } : {}),
      ...(where && next.account && (multi.value || next.account !== accounts.data.value[0]?.id)
        ? { account: String(next.account) }
        : {}),
      ...(where && next.folder !== 'INBOX' ? { folder: next.folder } : {}),
      ...(next.uid ? { uid: String(next.uid) } : {}),
    },
  })
}

// ---- Folders --------------------------------------------------------------
// The Folders section shows the account you're in (all inboxes: the first one).
const folderAccount = computed(() => accountId.value ?? accounts.data.value[0]?.id)
const folderAccountName = computed(() => accounts.data.value.find((a) => a.id === folderAccount.value)?.name ?? '')
const folders = useFolders(folderAccount)
const currentFolder = computed(() => folders.data.value.find((f) => f.path === folder.value))
const { state: organize, refresh: refreshOrganize } = useOrganize()
const labelById = computed(() => new Map(organize.value.labels.map((l) => [l.id, l])))
const folderTitle = computed(() => {
  if (box.value === 'starred') return 'Starred'
  if (box.value === 'snoozed') return 'Snoozed'
  if (box.value === 'label') return labelById.value.get(labelId.value ?? -1)?.name ?? 'Label'
  if (allView.value) return 'All inboxes'
  const f = currentFolder.value
  const name = f ? (f.role === 'inbox' ? 'Inbox' : f.name) : 'Inbox'
  const account = accounts.data.value.find((a) => a.id === accountId.value)
  return multi.value && name === 'Inbox' && account ? account.name : name
})
const isInbox = computed(() => (currentFolder.value ? currentFolder.value.role === 'inbox' : folder.value.toUpperCase() === 'INBOX'))
const otherFolders = computed(() => folders.data.value.filter((f) => f.role !== 'inbox'))
const totalUnread = computed(() => accounts.data.value.reduce((n, a) => n + a.unread, 0))
const folderLabel = (path: string) => {
  const f = folders.data.value.find((x) => x.path === path)
  return f ? (f.role === 'inbox' ? 'Inbox' : f.name) : path
}

// ---- Search ----------------------------------------------------------------
// Instant search in Havre's local copy (sender and subject). "Search inside
// messages" asks the mail server, which also looks at message bodies.
const query = ref('')
const search = ref('') // the debounced value actually sent
const searchRemote = ref(false)
let searchTimer: ReturnType<typeof setTimeout> | undefined
watch(query, (q) => {
  clearTimeout(searchTimer)
  searchTimer = setTimeout(() => {
    searchRemote.value = false
    search.value = q.trim()
  }, 150)
})
// ?q= (from the Ctrl+K palette): search for it.
watch(
  () => route.query.q,
  (q) => {
    if (typeof q !== 'string' || !q) return
    query.value = q
    search.value = q
    const { q: _, ...rest } = route.query
    router.replace({ query: rest })
  },
  { immediate: true },
)
function clearSearch() {
  query.value = ''
  search.value = ''
  searchRemote.value = false
  meaning.value = false
}
function searchServer() {
  searchRemote.value = true
  loadFirstPage()
}

// Search by meaning (AI): every account and folder, closest first.
const { status: ai } = useAiStatus()
const meaning = ref(false)
const meaningHits = ref<AiHit[]>([])
const meaningLoading = ref(false)
const meaningError = ref<string | null>(null)
watch([meaning, search], async ([on, q]) => {
  if (!on || !q) return
  meaningLoading.value = true
  meaningError.value = null
  try {
    const hits = await searchByMeaning(q)
    if (meaning.value && search.value === q) meaningHits.value = hits
  } catch (err) {
    meaningError.value = apiErrorMessage(err)
  } finally {
    meaningLoading.value = false
  }
})
function openHit(h: AiHit) {
  keepSearch = h.account !== accountId.value || h.folder !== folder.value
  go({ box: false, account: h.account, folder: h.folder, uid: h.uid })
}
const isOpen = (h: AiHit) => h.account === accountId.value && h.folder === folder.value && h.uid === selectedUid.value

// ---- Message list, page by page -------------------------------------------
const PAGE = 50
const items = ref<MessageSummary[]>([])
const hasMore = ref(false)
const listStatus = ref<'pending' | 'success' | 'error'>('pending')
const listError = ref<unknown>(null)
const loadingMore = ref(false)
let listRequest = 0 // answers to older requests are ignored

/** The server's name for the current list across accounts. */
function viewName(): CrossView {
  return box.value === 'all' || !box.value ? 'inboxes' : box.value
}

// `keep`: a background reload (live update) — keep as many rows as are
// loaded, and don't show any loading state.
async function loadFirstPage(opts: { keep?: boolean } = {}) {
  const id = ++listRequest
  const account = accountId.value
  if (!allView.value && !account) return // accounts not loaded yet
  if (!opts.keep) {
    listStatus.value = 'pending'
    listError.value = null
  }
  const limit = Math.min(Math.max(PAGE, opts.keep ? items.value.length : 0), 500)
  try {
    const page = box.value
      ? await fetchView(viewName(), { label: labelId.value ?? undefined, offset: 0, limit, q: search.value })
      : await fetchMessages(account!, folder.value, { q: search.value, limit, remote: searchRemote.value })
    if (id !== listRequest) return
    items.value = page.messages
    hasMore.value = page.has_more
    listStatus.value = 'success'
  } catch (err) {
    // A failed background reload is not worth an error screen.
    if (id !== listRequest || opts.keep) return
    listError.value = err
    listStatus.value = 'error'
  }
}

async function loadMore() {
  const last = items.value.at(-1)
  if (!last || !hasMore.value || loadingMore.value) return
  const id = listRequest
  loadingMore.value = true
  try {
    const page = box.value
      ? await fetchView(viewName(), {
          label: labelId.value ?? undefined,
          offset: items.value.length,
          limit: PAGE,
          q: search.value,
        })
      : await fetchMessages(accountId.value!, folder.value, {
          q: search.value,
          before: last.uid,
          limit: PAGE,
          remote: searchRemote.value,
        })
    if (id !== listRequest) return
    const known = new Set(items.value.map(rowKey))
    items.value = [...items.value, ...page.messages.filter((m) => !known.has(rowKey(m)))]
    hasMore.value = page.has_more
  } catch (err) {
    toast.show(`Could not load older messages: ${apiErrorMessage(err)}`, { tone: 'error' })
  } finally {
    loadingMore.value = false
  }
}

// Changing account or folder: start fresh (the skeleton shows while loading).
// (Opening a result of a search by meaning keeps the results.)
let keepSearch = false
watch(listKey, () => {
  if (!keepSearch) clearSearch()
  keepSearch = false
  filter.value = 'all'
  items.value = []
  loadFirstPage()
})
watch(search, () => loadFirstPage())
onMounted(() => loadFirstPage())

// ---- Live updates from the server --------------------------------------------
const syncStatus = ref<SyncStatus | null>(null)
let reloadTimer: ReturnType<typeof setTimeout> | undefined
let accountsTimer: ReturnType<typeof setTimeout> | undefined
/** Stars, labels and snoozes changed: the sidebar's counts follow. */
let organizeTimer: ReturnType<typeof setTimeout> | undefined
function organizeSoon() {
  clearTimeout(organizeTimer)
  organizeTimer = setTimeout(() => refreshOrganize().catch(() => {}), 400)
}

/** Coalesces bursts of events into one quiet reload. */
function reloadSoon() {
  clearTimeout(reloadTimer)
  reloadTimer = setTimeout(() => loadFirstPage({ keep: true }), 150)
}
useMailEvents((event) => {
  // Unread counts (sidebar, dock) follow any account's changes.
  if (event.type === 'accounts' || event.type === 'folders') {
    clearTimeout(accountsTimer)
    accountsTimer = setTimeout(() => accounts.refresh(), 300)
  }
  if (event.type === 'accounts') return
  // Lists across accounts: any folder may hold starred, labelled or snoozed mail.
  if (box.value && event.type === 'folder' && (box.value !== 'all' || /^inbox$/i.test(event.path))) reloadSoon()
  if (event.type === 'folder' || event.type === 'folders') organizeSoon()
  if (event.account === statusAccount.value && event.type === 'status') syncStatus.value = event.status
  if (event.account !== folderAccount.value) return
  if (event.type === 'folders') folders.refresh()
  else if (event.type === 'folder' && !allView.value && event.path === folder.value && !searchRemote.value) reloadSoon()
})
/** Whose sync state the sidebar shows. */
const statusAccount = computed(() => accountId.value ?? accounts.data.value[0]?.id)
watch(
  statusAccount,
  async (account) => {
    syncStatus.value = null
    if (!account) return
    try {
      syncStatus.value = await $fetch<SyncStatus>('/api/mail/status', { query: { account } })
    } catch {
      /* the list shows what's wrong */
    }
  },
  { immediate: true },
)

// Load the next page automatically when the end of the list scrolls into view.
const sentinel = ref<HTMLElement | null>(null)
let observer: IntersectionObserver | null = null
onMounted(() => {
  observer = new IntersectionObserver((entries) => {
    if (entries.some((e) => e.isIntersecting)) loadMore()
  })
})
watch(sentinel, (el, old) => {
  if (old) observer?.unobserve(old)
  if (el) observer?.observe(el)
})
onBeforeUnmount(() => observer?.disconnect())

// ---- Filtering (instant, on what is loaded) ---------------------------------
const filter = ref<'all' | 'unread' | 'invites'>('all')
const visible = computed(() =>
  items.value.filter((m) => filter.value === 'all' || (filter.value === 'unread' ? m.unread : m.invite)),
)

/** Identifies a row (in all inboxes, UIDs repeat across accounts). */
function rowKey(m: MessageSummary) {
  return `${m.account ?? ''}:${m.folder ?? ''}:${m.uid}`
}
/** Where a row's message lives. */
function whereOf(m: MessageSummary) {
  return { account: m.account ?? accountId.value!, folder: m.folder ?? folder.value }
}
function isSelected(m: MessageSummary) {
  if (m.uid !== selectedUid.value) return false
  return !allView.value || (m.account === accountId.value && m.folder === folder.value)
}
function openRow(m: MessageSummary) {
  go(allView.value ? { ...whereOf(m), uid: m.uid } : { uid: m.uid })
}
const selectedIndex = computed(() => visible.value.findIndex(isSelected))

// Day groups, as in the design: Today, Yesterday, the weekday this week,
// then months.
const now = new Date()
function groupOf(date: string | null): { key: string; label: string; day: boolean } {
  if (!date) return { key: 'none', label: 'No date', day: false }
  const d = new Date(date)
  const days = Math.round((startOfDay(now).getTime() - startOfDay(d).getTime()) / DAY_MS)
  if (days <= 0) return { key: 'today', label: 'Today', day: true }
  if (days === 1) return { key: 'yesterday', label: 'Yesterday', day: true }
  if (days < 7) return { key: `d${days}`, label: d.toLocaleDateString([], { weekday: 'long' }), day: true }
  const sameYear = d.getFullYear() === now.getFullYear()
  const month = d.toLocaleDateString([], sameYear ? { month: 'long' } : { month: 'long', year: 'numeric' })
  const thisMonth = sameYear && d.getMonth() === now.getMonth()
  return { key: `${d.getFullYear()}-${d.getMonth()}`, label: thisMonth ? `Earlier in ${month}` : month, day: false }
}
const groups = computed(() => {
  const out: { key: string; label: string; day: boolean; items: MessageSummary[] }[] = []
  for (const m of visible.value) {
    const g = groupOf(m.date)
    const last = out[out.length - 1]
    if (last?.key === g.key) last.items.push(m)
    else out.push({ ...g, items: [m] })
  }
  return out
})
/** The row's time: hours in day groups, the date in month groups. */
function rowTime(m: MessageSummary, day: boolean) {
  if (!m.date) return ''
  return day ? timeLabel(new Date(m.date)) : new Date(m.date).toLocaleDateString([], { day: 'numeric', month: 'short' })
}
/** "Thu 22 Oct · 10:00" for an invitation's chip. */
function inviteLabel(m: MessageSummary) {
  if (!m.invite_at) return 'Invitation'
  const d = new Date(m.invite_at * 1000)
  return `${d.toLocaleDateString([], { weekday: 'short', day: 'numeric', month: 'short' })} · ${timeLabel(d)}`
}
function accountColor(m: MessageSummary) {
  const a = accounts.data.value.find((x) => x.id === m.account)
  return a ? `var(--c-${a.color})` : undefined
}

// ---- Refresh ------------------------------------------------------------------
// Asks the server to sync this folder now, then reloads. The button shows a
// spinner for at least 600 ms, so the feedback is visible even when it's instant.
const manualRefresh = ref(false)
async function refreshList() {
  if (manualRefresh.value) return
  manualRefresh.value = true
  try {
    const targets = allView.value
      ? accounts.data.value.map((a) => ({ account: a.id, folder: 'INBOX' }))
      : [{ account: accountId.value!, folder: folder.value }]
    const sync = Promise.all(targets.map((t) => syncNow(t.account, t.folder))).catch((err) => {
      toast.show(`Sync failed: ${apiErrorMessage(err)}`, { tone: 'error' })
    })
    await Promise.all([sync, new Promise((resolve) => setTimeout(resolve, 600))])
    await loadFirstPage({ keep: true })
    folders.refresh()
  } finally {
    manualRefresh.value = false
  }
}
const listLoading = computed(() => manualRefresh.value || listStatus.value === 'pending')
// A refresh keeps the current list visible and shows a progress bar instead.
const refreshing = computed(() => listLoading.value && items.value.length > 0)

// ---- Opening a message ------------------------------------------------------
const detail = ref<MessageDetail | null>(null)
const detailLoading = ref(false)
const detailError = ref<string | null>(null)
const imagesAllowed = ref(false)
let request = 0

/** Changes the open message's row (the server marked it read, etc.). */
function patchSummary(uid: number, patch: Partial<MessageSummary>) {
  const account = accountId.value
  items.value = items.value.map((m) =>
    m.uid === uid && (!allView.value || m.account === account) ? { ...m, ...patch } : m,
  )
}

async function load(uid: number, images = false) {
  const id = ++request
  detailLoading.value = true
  detailError.value = null
  try {
    const m = await fetchMessage(accountId.value!, folder.value, uid, { images })
    if (id !== request) return
    detail.value = m
    imagesAllowed.value = images
    patchSummary(uid, { unread: false }) // the server marked it as read
  } catch (err) {
    if (id === request) detailError.value = apiErrorMessage(err)
  } finally {
    if (id === request) detailLoading.value = false
  }
}

// When stepping through the list with the keyboard, wait a moment before
// opening: skimming past a message doesn't download it or mark it as read.
let keyboardNav = false
let openTimer: ReturnType<typeof setTimeout> | undefined
watch(
  selectedUid,
  (uid) => {
    clearTimeout(openTimer)
    if (uid) {
      if (keyboardNav) openTimer = setTimeout(() => load(uid), 250)
      else load(uid)
    } else {
      request++
      detail.value = null
      detailError.value = null
    }
    keyboardNav = false
  },
  { immediate: true },
)

/** Selects the next (+1) or previous (-1) message and keeps it in view. */
// The URL updates asynchronously; `cursor` remembers where we are right away,
// so holding ↓ moves one message per key press.
let cursor: string | null = null
watch(selectedUid, () => {
  const open = visible.value[selectedIndex.value]
  if (open && rowKey(open) === cursor) cursor = null
})
function step(delta: 1 | -1) {
  const list = visible.value
  if (!list.length) return
  const i = cursor ? list.findIndex((m) => rowKey(m) === cursor) : selectedIndex.value
  // Nothing selected yet: ↓ starts at the top, ↑ at the bottom.
  const next = i === -1 ? (delta === 1 ? 0 : list.length - 1) : i + delta
  if (next < 0 || next >= list.length) return
  keyboardNav = true
  const target = list[next]!
  cursor = rowKey(target)
  openRow(target)
  const key = cursor
  nextTick(() => {
    document.querySelector(`.row[data-key="${key}"]`)?.scrollIntoView({ block: 'nearest' })
  })
}

async function markUnread() {
  const uid = selectedUid.value
  if (!uid) return
  patchSummary(uid, { unread: true })
  go({ uid: null })
  try {
    await setSeen(accountId.value!, folder.value, uid, false)
  } catch {
    patchSummary(uid, { unread: false })
  }
}

// ---- Archive / delete / move ------------------------------------------------
const inTrash = computed(() => currentFolder.value?.role === 'trash')
const inArchive = computed(() => ['archive', 'all'].includes(currentFolder.value?.role ?? ''))

// ---- Several messages at once -------------------------------------------------
// Tick messages (the avatar becomes a box; Ctrl/⌘-click, Shift-click a range,
// X for the one you're on): the bar above the list acts on all of them.
const checked = ref(new Set<string>())
let anchorKey: string | null = null
const selecting = computed(() => checked.value.size > 0)
const checkedRows = computed(() => visible.value.filter((m) => checked.value.has(rowKey(m))))
function clearChecks() {
  checked.value = new Set()
  anchorKey = null
}
watch(listKey, clearChecks)
// Shift-click on a row's tick: tick everything from the last ticked row
// (the browser's own toggle is cancelled, toggleCheck does it).
function onTickClick(e: MouseEvent, m: MessageSummary) {
  if (!e.shiftKey) return
  e.preventDefault()
  toggleCheck(m, true)
}
function toggleCheck(m: MessageSummary, range = false) {
  const next = new Set(checked.value)
  const key = rowKey(m)
  if (range && anchorKey) {
    const keys = visible.value.map(rowKey)
    const i = keys.indexOf(anchorKey)
    const j = keys.indexOf(key)
    if (i >= 0) for (const k of keys.slice(Math.min(i, j), Math.max(i, j) + 1)) next.add(k)
  } else if (next.has(key)) next.delete(key)
  else next.add(key)
  anchorKey = key
  checked.value = next
}
function checkAll() {
  checked.value = checked.value.size === visible.value.length ? new Set() : new Set(visible.value.map(rowKey))
}
/** A click on a row: Ctrl/⌘ ticks it, Shift ticks a range, otherwise it opens. */
function clickRow(m: MessageSummary, e: MouseEvent) {
  if (e.metaKey || e.ctrlKey) return toggleCheck(m)
  if (e.shiftKey) return toggleCheck(m, true)
  openRow(m)
}

/** Is this the message open in the reader? */
function isOpenRow(m: MessageSummary) {
  const w = whereOf(m)
  return m.uid === selectedUid.value && w.account === accountId.value && w.folder === folder.value
}

/** What an action applies to: the ticked messages, or else the open one. */
function targets(): MessageSummary[] {
  if (selecting.value) return checkedRows.value
  const open = visible.value[selectedIndex.value]
  if (open) return [open]
  const uid = selectedUid.value
  return uid && accountId.value ? [{ uid, account: accountId.value, folder: folder.value } as MessageSummary] : []
}
function where(m: MessageSummary) {
  return { ...whereOf(m), uid: m.uid }
}
/** Changes rows in place (optimistic updates). */
function patchRows(rows: MessageSummary[], change: (m: MessageSummary) => Partial<MessageSummary>) {
  const keys = new Set(rows.map(rowKey))
  items.value = items.value.map((m) => (keys.has(rowKey(m)) ? { ...m, ...change(m) } : m))
}
const many = (n: number, one: string, more: string) => (n === 1 ? one : `${n} ${more}`)

// ---- Archive / delete / move, and Undo -------------------------------------------
type Done = { sel: Selection; outcome: ActionOutcome }
let lastUndo: { list: Done[]; at: number } | null = null

async function act(action: MailAction, to?: string) {
  const rows = targets()
  if (!rows.length) return
  const before = items.value
  const keyBefore = listKey.value
  const openIndex = selectedIndex.value
  const openGone = rows.some(isOpenRow)

  // Optimistic: remove them now and open the next message, like most mail apps.
  const gone = new Set(rows.map(rowKey))
  items.value = items.value.filter((m) => !gone.has(rowKey(m)))
  clearChecks()
  if (openGone) {
    const next = visible.value[openIndex] ?? visible.value[openIndex - 1]
    if (next) openRow(next)
    else go({ uid: null })
  }

  const done: Done[] = []
  try {
    for (const sel of groupSelection(rows.map(where))) {
      done.push({ sel, outcome: await applyMailAction(sel.account, action, sel.folder, sel.uids, to) })
    }
    const n = rows.length
    const first = done[0]?.outcome
    if (first?.result === 'deleted') {
      toast.show(many(n, 'Message deleted permanently', 'messages deleted permanently'))
      return
    }
    const verb =
      action === 'archive' ? 'Archived' : action === 'trash' ? 'Moved to Trash' : `Moved to ${first?.result === 'moved' ? folderLabel(first.to) : 'folder'}`
    const undoable = done.filter((d) => d.outcome.result === 'moved' && d.outcome.message_ids.length)
    lastUndo = undoable.length ? { list: undoable, at: Date.now() } : null
    toast.show(n === 1 ? verb : `${verb}: ${n} messages`, {
      duration: 8000,
      action: undoable.length ? { label: 'Undo', run: () => undo() } : undefined,
    })
  } catch (err) {
    // What was done stays done; the rest comes back.
    if (listKey.value === keyBefore) items.value = done.length ? items.value : before
    if (done.length) loadFirstPage({ keep: true })
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

/** Puts the last archived / moved / deleted messages back (Undo, or Z). */
async function undo() {
  const last = lastUndo
  lastUndo = null
  if (!last || Date.now() - last.at > 5 * 60_000) return
  try {
    let moved = 0
    for (const d of last.list) {
      if (d.outcome.result !== 'moved') continue
      moved += (await undoMove(d.sel.account, d.outcome.to, d.sel.folder, d.outcome.message_ids)).moved
    }
    toast.show(many(moved, 'Message moved back', 'messages moved back'))
    loadFirstPage({ keep: true })
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

// ---- Star, read, labels, snooze --------------------------------------------------------
async function star(on?: boolean) {
  const rows = targets()
  if (!rows.length) return
  const value = on ?? !rows.every((m) => m.flagged)
  const before = rows.map((m) => ({ ...m }))
  patchRows(rows, () => ({ flagged: value }))
  try {
    for (const sel of groupSelection(rows.map(where))) await setFlags(sel, { flagged: value })
    // Unstarred in Starred: it leaves the list.
    if (box.value === 'starred' && !value) loadFirstPage({ keep: true })
    organizeSoon()
  } catch (err) {
    patchRows(before, (m) => ({ flagged: before.find((b) => rowKey(b) === rowKey(m))?.flagged }))
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

async function setRead(seen: boolean) {
  const rows = targets()
  if (!rows.length) return
  patchRows(rows, () => ({ unread: !seen }))
  try {
    for (const sel of groupSelection(rows.map(where))) await setFlags(sel, { seen })
    accounts.refresh()
  } catch (err) {
    loadFirstPage({ keep: true })
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

/** For the label menu: which labels the target messages carry. */
const labelState = computed(() => {
  const rows = selecting.value ? checkedRows.value : visible.value.filter(isSelected)
  const out: Record<number, 'all' | 'some'> = {}
  for (const l of organize.value.labels) {
    const n = rows.filter((m) => m.labels?.includes(l.id)).length
    if (n && n === rows.length) out[l.id] = 'all'
    else if (n) out[l.id] = 'some'
  }
  return out
})

async function toggleLabel(label: number, on: boolean) {
  const rows = targets()
  if (!rows.length) return
  patchRows(rows, (m) => ({
    labels: on ? [...new Set([...(m.labels ?? []), label])] : (m.labels ?? []).filter((x) => x !== label),
  }))
  try {
    for (const sel of groupSelection(rows.map(where))) await labelMessages(sel, label, on)
    if (box.value === 'label' && !on && label === labelId.value) loadFirstPage({ keep: true })
    refreshOrganize()
  } catch (err) {
    loadFirstPage({ keep: true })
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

async function snooze(until: number | null) {
  const rows = targets()
  if (!rows.length) return
  // Snoozed: away from the list (in Snoozed, waking up takes it out too).
  const before = items.value
  const gone = new Set(rows.map(rowKey))
  const openGone = rows.some(isOpenRow)
  const openIndex = selectedIndex.value
  items.value = items.value.filter((m) => !gone.has(rowKey(m)))
  clearChecks()
  if (openGone) {
    const next = visible.value[openIndex] ?? visible.value[openIndex - 1]
    if (next) openRow(next)
    else go({ uid: null })
  }
  const groups = groupSelection(rows.map(where))
  try {
    for (const sel of groups) await snoozeMessages(sel, until)
    refreshOrganize()
    if (until === null) {
      toast.show(many(rows.length, 'Back in the inbox', 'messages back in the inbox'))
      return
    }
    const d = new Date(until * 1000)
    const when = `${sameDay(d, new Date()) ? 'today' : d.toLocaleDateString([], { weekday: 'long', day: 'numeric', month: 'short' })}, ${timeLabel(d)}`
    toast.show(`${many(rows.length, 'Snoozed', `messages snoozed`)} until ${when}`, {
      duration: 8000,
      action: {
        label: 'Undo',
        run: async () => {
          for (const sel of groups) await snoozeMessages(sel, null).catch(() => {})
          refreshOrganize()
          loadFirstPage({ keep: true })
        },
      },
    })
  } catch (err) {
    items.value = before
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

/** The star on a row: that message only, whatever is ticked. */
async function starRow(m: MessageSummary) {
  const value = !m.flagged
  patchRows([m], () => ({ flagged: value }))
  try {
    await setFlags({ ...whereOf(m), uids: [m.uid] }, { flagged: value })
    if (box.value === 'starred' && !value) loadFirstPage({ keep: true })
    organizeSoon()
  } catch (err) {
    patchRows([m], () => ({ flagged: !value }))
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

/** The open message's row (its star, labels). */
const openSummary = computed(() => visible.value[selectedIndex.value] ?? null)
/** "Until Thu 08:00" on a snoozed row. */
function snoozedLabel(m: MessageSummary) {
  if (!m.snoozed_until) return ''
  const d = new Date(m.snoozed_until * 1000)
  const day = sameDay(d, new Date()) ? 'Today' : d.toLocaleDateString([], { weekday: 'short', day: 'numeric', month: 'short' })
  return `Until ${day} · ${timeLabel(d)}`
}
const labelsOpen = ref(false)

// ---- Writing ----------------------------------------------------------------
const composer = useComposer()
const drafts = useDrafts()

/** A new message, from the account being read. */
async function write() {
  const account = accountId.value ?? accounts.data.value.find((a) => a.can_send)?.id ?? accounts.data.value[0]?.id
  if (!account) return
  try {
    await compose({ account })
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

/** Reply, reply all or forward the open message (`ai`: the AI writes it). */
async function reply(kind: 'reply' | 'reply_all' | 'forward', ai = false) {
  if (!accountId.value || !selectedUid.value) return
  try {
    await compose({ account: accountId.value, kind, folder: folder.value, uid: selectedUid.value, ai })
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}

// ---- The open message's sender: the aside (wide screens) ----------------------
const sender = computed(() => emailIn(detail.value?.from?.address ?? null))
const person = ref<PersonDetail | null>(null)
watch(sender, async (email) => {
  person.value = null
  if (!email) return
  try {
    const p = await fetchPerson(email)
    if (sender.value === email) person.value = p
  } catch {
    /* no aside */
  }
})
/** Their next meetings with you (the same meeting in two calendars: once). */
const nextTogether = computed(() => {
  const seen = new Set<string>()
  return (person.value?.upcoming ?? [])
    .filter((e) => {
      const key = `${e.title}|${e.start}`
      if (seen.has(key)) return false
      seen.add(key)
      return true
    })
    .slice(0, 3)
})
function emailPerson() {
  const account = accountId.value
  if (!account || !person.value) return
  compose({ account, to: person.value.email }).catch((err) => toast.show(apiErrorMessage(err), { tone: 'error' }))
}
function schedulePerson() {
  if (!person.value) return
  router.push({ path: '/agenda', query: { new: '1', guest: person.value.email } })
}
function eventWhen(e: AgendaEvent) {
  const d = new Date(e.start * 1000)
  const today = sameDay(d, new Date())
  const day = today ? 'Today' : d.toLocaleDateString([], { weekday: 'short', day: 'numeric', month: 'short' })
  const answer = e.my_response === 'needs_action' ? ' · awaiting reply' : ''
  return `${day}${e.all_day ? '' : ` · ${timeLabel(d)}`}${answer}`
}
function eventLink(e: AgendaEvent) {
  return { path: '/agenda', query: { view: 'week', date: isoDate(new Date(e.start * 1000)), event: String(e.id) } }
}

// ---- Keyboard ---------------------------------------------------------------
// ↓/↑ or j/k next/previous · u unread · e archive · # or Delete delete · / search · Esc close
// c new message · r reply · a reply all · f forward · s summarize (AI) · t make a task
const searchInput = ref<{ inputRef: HTMLInputElement | null } | null>(null)
const reader = ref<{ toggleSummary: () => void; openTask: () => void; openNote: () => void } | null>(null)
function onKey(e: KeyboardEvent) {
  if (e.metaKey || e.ctrlKey || e.altKey) return
  // ↓ in the search box jumps into the results.
  if (e.key === 'ArrowDown' && e.target === searchInput.value?.inputRef) {
    e.preventDefault()
    searchInput.value?.inputRef?.blur()
    return step(1)
  }
  // (Keys forwarded from the email iframe target `window`, not an element.)
  const el = e.target instanceof Element ? e.target : null
  if (el?.closest('input, textarea, select, [role="menu"], [role="dialog"]')) return
  if (e.key === 'ArrowDown' || e.key === 'j') {
    e.preventDefault() // don't scroll the page
    step(1)
  } else if (e.key === 'ArrowUp' || e.key === 'k') {
    e.preventDefault()
    step(-1)
  } else if (e.key === 'u') (selecting.value ? setRead(false) : markUnread())
  else if (e.key === 'e' && (selectedUid.value || selecting.value) && !inArchive.value) act('archive')
  else if ((e.key === '#' || e.key === 'Delete') && (selectedUid.value || selecting.value) && !inTrash.value) act('trash')
  else if (e.key === '*' && (selectedUid.value || selecting.value)) star()
  else if (e.key === 'x') {
    const open = openSummary.value
    if (open) toggleCheck(open)
  } else if (e.key === 'z') undo()
  else if (e.key === '/') {
    e.preventDefault()
    searchInput.value?.inputRef?.focus()
  } else if (e.key === 'Escape') (selecting.value ? clearChecks() : go({ uid: null }))
  else if (e.key === 'c') {
    e.preventDefault()
    write()
  } else if (e.key === 's' && selectedUid.value) {
    e.preventDefault()
    reader.value?.toggleSummary()
  } else if (e.key === 't' && selectedUid.value) {
    e.preventDefault()
    reader.value?.openTask()
  } else if (e.key === 'n' && selectedUid.value) {
    e.preventDefault()
    reader.value?.openNote()
  } else if (selectedUid.value && (e.key === 'r' || e.key === 'a' || e.key === 'f')) {
    e.preventDefault()
    reply(e.key === 'r' ? 'reply' : e.key === 'a' ? 'reply_all' : 'forward')
  }
}
onMounted(() => window.addEventListener('keydown', onKey))
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))

const isMac = import.meta.client && /Mac|iPhone|iPad/.test(navigator.platform)
const notConfigured = computed(() => (listError.value as { statusCode?: number } | null)?.statusCode === 503)

// Resizable panes (wide screens): folders · list · reader.
const splitPanes: SplitPaneSpec[] = [
  { id: 'folders', size: 216, min: 180, max: 320, collapsible: true, handle: { label: 'Resize the folders', fold: true, showLabel: 'Show folders' } },
  { id: 'list', size: 380, min: 300, max: 640, handle: { label: 'Resize the message list' } },
  { id: 'reader', min: 420 },
]
</script>

<template>
  <!-- First run: no account yet. -->
  <main v-if="noAccounts" class="welcome">
    <div class="welcome-card">
      <span class="welcome-icon"><HIcon name="mail" :size="24" /></span>
      <h1 class="serif">Connect your first mailbox</h1>
      <p>
        Havre reads your mail over IMAP and keeps a local copy on this server. Your email and password are
        enough for most providers.
      </p>
      <UButton to="/settings?add=1">Add an account</UButton>
    </div>
  </main>

  <div v-else class="mail" :class="{ 'has-selection': selectedUid }">
    <!-- Mailboxes -->
    <SplitView id="mail" :panes="splitPanes">
    <template #folders>
    <aside class="boxes" aria-label="Mailboxes">
      <div class="boxes-head">
        <h1 class="serif">Mail</h1>
        <span class="mono faint small">{{ accounts.data.value.length }} {{ accounts.data.value.length === 1 ? 'account' : 'accounts' }}</span>
      </div>
      <button type="button" class="compose" title="New message (C)" @click="write()">
        <HIcon name="pen" :size="16" :stroke-width="1.8" />
        <span class="grow">New message</span>
        <UKbd value="C" aria-hidden="true" class="ms-auto bg-transparent ring-(--on-accent)/35 text-(--on-accent)/80" />
      </button>

      <nav class="nav" aria-label="Inboxes">
        <div class="eyebrow sec">Inboxes</div>
        <button
          v-if="multi"
          type="button"
          class="box"
          :class="{ on: allView }"
          :aria-current="allView ? 'page' : undefined"
          @click="go({ box: 'all', uid: null })"
        >
          <HIcon name="inbox" :size="16" />
          <span class="grow">All inboxes</span>
          <span v-if="totalUnread" class="mono n">{{ totalUnread }}</span>
        </button>
        <button
          v-for="a in accounts.data.value"
          :key="a.id"
          type="button"
          class="box"
          :class="{ on: !allView && accountId === a.id && isInbox }"
          :aria-current="!allView && accountId === a.id && isInbox ? 'page' : undefined"
          :title="a.email"
          @click="go({ box: false, account: a.id, folder: 'INBOX', uid: null })"
        >
          <span class="dotbox" aria-hidden="true"><span class="adot" :style="{ background: `var(--c-${a.color})` }" /></span>
          <span class="grow">{{ multi ? a.name : 'Inbox' }}</span>
          <span v-if="a.status.state === 'error'" class="warn" :title="a.status.error ?? 'Sync problem'">!</span>
          <span v-else-if="a.unread" class="mono n">{{ a.unread }}</span>
        </button>
      </nav>

      <nav class="nav" aria-label="Across accounts">
        <div class="eyebrow sec">Folders<span v-if="multi" class="sec-hint"> · {{ folderAccountName }}</span></div>
        <button type="button" class="box" :class="{ on: box === 'starred' }" :aria-current="box === 'starred' ? 'page' : undefined" @click="go({ box: 'starred', uid: null })">
          <HIcon name="star" :size="16" />
          <span class="grow">Starred</span>
          <span v-if="organize.starred" class="mono n">{{ organize.starred }}</span>
        </button>
        <button type="button" class="box" :class="{ on: box === 'snoozed' }" :aria-current="box === 'snoozed' ? 'page' : undefined" @click="go({ box: 'snoozed', uid: null })">
          <HIcon name="snooze" :size="16" />
          <span class="grow">Snoozed</span>
          <span v-if="organize.snoozed" class="mono n">{{ organize.snoozed }}</span>
        </button>
      </nav>
      <div class="tree">
        <FolderList
          :folders="otherFolders"
          :current="allView || isInbox ? null : folder"
          :loading="folders.status.value === 'pending'"
          @select="(path) => go({ box: false, account: folderAccount, folder: path, uid: null })"
        />
        <!-- Havre's labels: every account. -->
        <div class="eyebrow sec sec-row">
          Labels
          <UButton
            color="primary"
            variant="link"
            class="small-link p-0"
            type="button"
            @click="labelsOpen = true"
          >
            {{ organize.labels.length ? 'Edit' : 'Add' }}
          </UButton>
        </div>
        <button
          v-for="l in organize.labels"
          :key="l.id"
          type="button"
          class="box"
          :class="{ on: box === 'label' && labelId === l.id }"
          :aria-current="box === 'label' && labelId === l.id ? 'page' : undefined"
          :aria-label="`Label ${l.name}${l.count ? `, ${l.count}` : ''}`"
          @click="go({ box: 'label', label: l.id, uid: null })"
        >
          <span class="hash mono" :style="{ color: `var(--c-${l.color})` }" aria-hidden="true">#</span>
          <span class="grow">{{ l.name }}</span>
          <span v-if="l.count" class="mono n faint">{{ l.count }}</span>
        </button>
        <!-- Unsent messages, saved as you type. -->
        <template v-if="drafts.data.value.length">
          <div class="eyebrow sec">Unsent</div>
          <button
            v-for="d in drafts.data.value"
            :key="d.id"
            type="button"
            class="draft"
            :class="{ open: composer?.id === d.id }"
            @click="openDraft(d.id)"
          >
            <span class="draft-subject">{{ d.subject || '(no subject)' }}</span>
            <span class="draft-to">{{ d.to ? `To ${d.to}` : 'No recipient yet' }}</span>
          </button>
        </template>
        <NuxtLink to="/settings?add=1" class="add-account">
          <HIcon name="plus" :size="14" />
          Add account
        </NuxtLink>
      </div>
      <SyncStatus :status="syncStatus" />
      <LabelManager v-if="labelsOpen" @close="labelsOpen = false; loadFirstPage({ keep: true })" />
    </aside>
    </template>

    <!-- Message list -->
    <template #list>
    <section class="list" aria-label="Conversations">
      <div class="list-top">
        <UInput
          ref="searchInput"
          v-model="query"
          type="search"
          icon="i-havre-search"
          size="lg"
          :placeholder="allView ? 'Search mail' : `Search ${folderTitle}`"
          :aria-label="`Search ${folderTitle}`"
          class="search"
          @keydown.enter="query.trim() && (search = query.trim())"
          @keydown.esc="clearSearch"
        >
          <template v-if="!query" #trailing>
            <UButton
              color="neutral"
              variant="link"
              size="xs"
              class="p-0"
              title="Search mail, people, events and ask (Ctrl+K)"
              @click="usePalette().value = true"
            >
              <UKbd :value="isMac ? '⌘K' : 'Ctrl K'" />
            </UButton>
          </template>
        </UInput>
        <UButton
          v-if="ai.search && query"
          icon="i-havre-sparkle"
          :color="meaning ? 'primary' : 'neutral'"
          :variant="meaning ? 'soft' : 'outline'"
          size="lg"
          :aria-pressed="meaning"
          title="Search all your mail by meaning, not just words"
          @click="meaning = !meaning"
        >
          By meaning
        </UButton>
      </div>
      <!-- Ticked messages: what to do with them. -->
      <div v-if="selecting" class="bulk" role="toolbar" aria-label="Selected messages">
        <UCheckbox
          :model-value="checked.size === visible.length ? true : 'indeterminate'"
          :title="checked.size === visible.length ? 'Untick all' : 'Tick all'"
          aria-label="Tick all"
          class="all-box"
          @update:model-value="checkAll"
        />
        <span class="count-sel">{{ checked.size }} selected</span>
        <span class="grow" />
        <UButton
          v-if="!inArchive"
          icon="i-havre-archive"
          color="neutral"
          variant="ghost"
          square
          type="button"
          aria-label="Archive selected"
          title="Archive (E)"
          @click="act('archive')"
        />
        <UButton
          icon="i-havre-trash"
          color="neutral"
          variant="ghost"
          square
          type="button"
          :aria-label="inTrash ? 'Delete selected forever' : 'Delete selected'"
          title="Delete (#)"
          @click="act('trash')"
        />
        <UButton
          icon="i-havre-star"
          color="neutral"
          variant="ghost"
          square
          type="button"
          aria-label="Star selected"
          title="Star (*)"
          @click="star()"
        />
        <UButton
          icon="i-havre-unread"
          color="neutral"
          variant="ghost"
          square
          :aria-label="checkedRows.some((m) => m.unread) ? 'Mark selected as read' : 'Mark selected as unread'"
          @click="setRead(checkedRows.some((m) => m.unread))"
        />
        <LabelMenu :state="labelState" @toggle="toggleLabel" />
        <SnoozeMenu :snoozed="box === 'snoozed'" @snooze="snooze" />
        <MoveMenu v-if="!allView" :folders="folders.data.value" :current="folder" @move="(to) => act('move', to)" />
        <UButton
          icon="i-havre-close"
          color="neutral"
          variant="ghost"
          square
          type="button"
          aria-label="Clear selection"
          title="Clear (Esc)"
          @click="clearChecks"
        />
      </div>
      <div v-else class="list-head">
        <h2 v-if="search" class="results">
          {{ meaning ? 'Closest in meaning' : searchRemote ? 'Server results' : 'Results' }} for “{{ search }}”
          <UButton color="primary" variant="link" class="p-0" type="button" @click="clearSearch">
            Clear
          </UButton>
        </h2>
        <h2 v-else>{{ folderTitle }}</h2>
        <UTabs
          v-model="filter"
          :items="[
            { label: 'All', value: 'all' },
            { label: 'Unread', value: 'unread' },
            { label: 'Invites', value: 'invites' },
          ]"
          :content="false"
          color="neutral"
          size="xs"
          aria-label="Filter"
        />
        <UButton
          icon="i-havre-refresh"
          :color="listLoading ? 'primary' : 'neutral'"
          :variant="listLoading ? 'soft' : 'ghost'"
          size="sm"
          :square="!listLoading"
          :loading="listLoading"
          :aria-label="listLoading ? 'Refreshing messages' : 'Refresh'"
          :title="listLoading ? 'Refreshing…' : 'Refresh'"
          :disabled="listLoading"
          @click="refreshList"
        >
          <template v-if="listLoading">{{ search ? 'Searching' : 'Refreshing' }}</template>
        </UButton>
      </div>
      <div class="progress" :class="{ active: refreshing }" role="progressbar" aria-label="Loading messages" :aria-hidden="!refreshing" />

      <div class="rows" :class="{ refreshing }" :aria-busy="listLoading">
        <template v-if="meaning && search">
          <p v-if="meaningError" class="empty">{{ meaningError }}</p>
          <p v-else-if="meaningLoading && !meaningHits.length" class="empty">Searching by meaning…</p>
          <p v-else-if="!meaningHits.length" class="empty">Nothing close to “{{ search }}”.</p>
          <button
            v-for="h in meaningHits"
            :key="`${h.account}-${h.folder}-${h.uid}`"
            type="button"
            class="row"
            :class="{ selected: isOpen(h) }"
            @click="openHit(h)"
          >
            <span class="avatar" aria-hidden="true" :style="{ '--c': personColor(personLabel(h.from)) }">
              {{ initials(personLabel(h.from)) }}
            </span>
            <span class="lines">
              <span class="top">
                <span class="from">{{ personLabel(h.from) }}</span>
                <span class="mono time">{{ shortDate(h.date) }}</span>
              </span>
              <span class="subject">{{ h.subject || '(no subject)' }}</span>
            </span>
          </button>
        </template>

        <template v-else-if="listStatus === 'pending' && !items.length">
          <div v-for="n in 6" :key="n" class="row skeleton" aria-hidden="true">
            <span class="avatar" />
            <span class="lines"><span /><span /><span /></span>
          </div>
        </template>

        <div v-else-if="listStatus === 'error'" class="notice">
          <HIcon name="alert" :size="18" />
          <div>
            <strong>{{ notConfigured ? 'No mailbox connected yet' : 'Could not load your mail' }}</strong>
            <p>{{ apiErrorMessage(listError) }}</p>
            <p v-if="notConfigured" class="hint">
              Copy <code>.env.example</code> to <code>.env</code>, fill in your IMAP account and restart the server.
            </p>
            <UButton
              color="neutral"
              variant="outline"
              type="button"
              :disabled="listLoading"
              @click="refreshList"
            >
              Try again
            </UButton>
          </div>
        </div>

        <p v-else-if="!visible.length" class="empty">
          <template v-if="search">
            No sender or subject in {{ folderTitle }} matches “{{ search }}”.
            <UButton
              v-if="!searchRemote"
              color="primary"
              variant="link"
              class="block p-0"
              type="button"
              @click="searchServer"
            >
              Search inside messages on the server →
            </UButton>
          </template>
          <template v-else-if="filter === 'unread'">No unread messages{{ hasMore ? ' among the ones loaded' : '' }}.</template>
          <template v-else-if="filter === 'invites'">No invitations{{ hasMore ? ' among the ones loaded' : '' }}.</template>
          <template v-else-if="box === 'starred'">No starred mail. Star a message with ☆ or the * key.</template>
          <template v-else-if="box === 'snoozed'">Nothing snoozed. Snooze a message to have it come back later.</template>
          <template v-else-if="box === 'label'">No message with this label yet.</template>
          <template v-else>This folder is empty.</template>
        </p>

        <template v-else>
          <div v-for="g in groups" :key="g.key" role="group" :aria-label="g.label">
            <div class="day eyebrow" aria-hidden="true">{{ g.label }}</div>
            <div
              v-for="m in g.items"
              :key="rowKey(m)"
              class="rowwrap"
              :class="{ selected: isSelected(m), checked: checked.has(rowKey(m)), selecting }"
            >
              <label class="pick" :title="checked.has(rowKey(m)) ? 'Untick' : 'Tick (X)'">
                <input
                  type="checkbox"
                  :checked="checked.has(rowKey(m))"
                  :aria-label="`Select: ${m.subject || '(no subject)'}`"
                  @click="onTickClick($event as MouseEvent, m)"
                  @change="toggleCheck(m)"
                />
                <span class="avatar" aria-hidden="true" :style="{ '--c': personColor(personLabel(m.from)) }">
                  {{ initials(personLabel(m.from)) }}
                </span>
              </label>
              <button
                type="button"
                class="row"
                :data-key="rowKey(m)"
                :class="{ unread: m.unread }"
                :aria-current="isSelected(m) ? 'true' : undefined"
                @click="clickRow(m, $event)"
              >
              <span class="lines">
                <span class="top">
                  <span v-if="m.unread" class="dot" aria-label="Unread" />
                  <span class="from">{{ personLabel(m.from) }}</span>
                  <span v-if="allView && multi" class="acc" :style="{ background: accountColor(m) }" aria-hidden="true" />
                  <HIcon v-if="m.attachments" name="paperclip" :size="13" class="clip" aria-label="Has attachments" />
                  <span class="mono time">{{ rowTime(m, g.day) }}</span>
                </span>
                <span class="subject">{{ m.subject || '(no subject)' }}</span>
                <span v-if="m.preview" class="snippet">{{ m.preview }}</span>
                <span v-if="m.invite || m.labels?.length || m.snoozed_until" class="chips">
                  <span v-if="m.snoozed_until" class="chip snz"><HIcon name="snooze" :size="12" /> {{ snoozedLabel(m) }}</span>
                  <span v-if="m.invite" class="invite"><HIcon name="calendar" :size="12" /> {{ inviteLabel(m) }}</span>
                  <template v-for="id in m.labels ?? []" :key="id">
                    <span v-if="labelById.get(id)" class="chip lab">
                      <span class="mono" :style="{ color: `var(--c-${labelById.get(id)!.color})` }">#</span>{{ labelById.get(id)!.name }}
                    </span>
                  </template>
                </span>
              </span>
              </button>
              <button
                type="button"
                class="star"
                :class="{ on: m.flagged }"
                :aria-label="m.flagged ? 'Unstar' : 'Star'"
                :aria-pressed="!!m.flagged"
                @click="starRow(m)"
              >
                <HIcon name="star" :size="15" />
              </button>
            </div>
          </div>
        </template>

        <!-- Local search covers senders and subjects; offer the server for bodies. -->
        <div v-if="search && !meaning && !searchRemote && visible.length && !hasMore" class="server-search">
          <UButton color="primary" variant="link" class="p-0" type="button" @click="searchServer">
            Search inside messages on the server →
          </UButton>
        </div>

        <!-- "Load more": automatic when scrolled into view, or click. -->
        <div v-if="hasMore && listStatus === 'success' && !(meaning && search)" ref="sentinel" class="more">
          <UButton
            color="neutral"
            variant="outline"
            :loading="loadingMore"
            class="more-btn"
            type="button"
            :disabled="loadingMore"
            @click="loadMore"
          >
            {{ loadingMore ? 'Loading older messages…' : 'Load older messages' }}
          </UButton>
        </div>
      </div>
    </section>
    </template>

    <template #reader>
    <MessageReader
      ref="reader"
      class="reader-pane"
      :account="accountId ?? 0"
      :message="detail"
      :loading="detailLoading"
      :error="detailError"
      :images-allowed="imagesAllowed"
      :folders="folders.data.value"
      :in-trash="inTrash"
      :in-archive="inArchive"
      :flagged="!!openSummary?.flagged"
      :label-state="labelState"
      :snoozed="box === 'snoozed'"
      :position="selectedIndex >= 0 ? selectedIndex + 1 : 0"
      :total="visible.length"
      @star="star()"
      @label="toggleLabel"
      @snooze="snooze"
      @prev="step(-1)"
      @next="step(1)"
      @back="go({ uid: null })"
      @mark-unread="markUnread"
      @archive="act('archive')"
      @trash="act('trash')"
      @move="(to) => act('move', to)"
      @reply="reply"
      @load-images="selectedUid && load(selectedUid, true)"
      @retry="selectedUid && load(selectedUid)"
    />
    </template>
    </SplitView>

    <!-- Who wrote it: what Havre knows about them (wide screens). -->
    <aside v-if="detail && person" class="about" :aria-label="`About ${personName(person)}`">
      <div class="who">
        <span class="big-avatar" aria-hidden="true" :style="{ '--c': personColor(personName(person)) }">
          {{ initials(personName(person)) }}
        </span>
        <div class="serif pname">{{ personName(person) }}</div>
        <div v-if="person.name" class="muted small addr">{{ person.email }}</div>
        <div class="acts">
          <UButton color="neutral" variant="outline" size="xs" type="button" @click="emailPerson">
            Email
          </UButton>
          <UButton color="neutral" variant="outline" size="xs" type="button" @click="schedulePerson">
            Schedule
          </UButton>
          <UButton
            color="neutral"
            variant="outline"
            size="xs"
            :to="`/people/${encodeURIComponent(person.email)}`"
          >
            Profile
          </UButton>
        </div>
      </div>
      <section v-if="nextTogether.length" class="ablock">
        <div class="eyebrow">Next together</div>
        <NuxtLink v-for="e in nextTogether" :key="e.id" :to="eventLink(e)" class="next">
          <span class="bar" :class="{ pending: e.my_response === 'needs_action' }" aria-hidden="true" />
          <span class="ntext">
            <span class="ntitle">{{ e.title }}</span>
            <span class="muted small">{{ eventWhen(e) }}</span>
          </span>
        </NuxtLink>
      </section>
      <section v-if="person.recent_messages.length" class="ablock">
        <div class="eyebrow">Conversations</div>
        <NuxtLink v-for="m in person.recent_messages.slice(0, 5)" :key="`${m.account}-${m.folder}-${m.uid}`" :to="messageLink(m)" class="line">
          <span class="ellip">{{ m.subject || '(no subject)' }}</span>
          <span class="mono faint tiny">{{ shortDate(m.date) }}</span>
        </NuxtLink>
      </section>
      <section v-if="person.files.length" class="ablock">
        <div class="eyebrow">Shared files</div>
        <a v-for="f in person.files.slice(0, 5)" :key="`${f.account}-${f.uid}-${f.index}`" :href="fileLink(f)" :download="f.name" class="line">
          <span class="ellip">{{ f.name }}</span>
          <span class="mono faint tiny">{{ shortDate(f.date) }}</span>
        </a>
      </section>
    </aside>
  </div>
</template>

<style scoped>
.mail {
  flex: 1;
  display: flex;
  min-width: 0;
}

/* Folders */
.boxes {
  flex: 0 0 216px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 20px 12px;
  border-right: 1px solid var(--line);
  background: var(--bg);
  min-height: 0;
}
.boxes-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  padding: 0 8px 14px;
}
.boxes h1 {
  margin: 0;
  font-size: 28px;
}
.small {
  font-size: 11px;
}
.faint {
  color: var(--faint);
}
.muted {
  color: var(--muted);
}
.compose {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 40px;
  padding: 0 12px;
  margin-bottom: 16px;
  border: 0;
  border-radius: 10px;
  background: var(--accent);
  color: var(--on-accent);
  font-weight: 500;
  cursor: pointer;
}
.compose:hover {
  filter: brightness(1.08);
}
.grow {
  flex: 1;
  min-width: 0;
  text-align: left;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.nav {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.sec {
  padding: 6px 8px;
}
.nav + .sec,
.tree .sec {
  padding-top: 18px;
}
.sec-hint {
  text-transform: none;
  letter-spacing: 0;
}
.box {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 34px;
  padding: 0 8px;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: var(--muted);
  font-size: 14px;
  cursor: pointer;
}
.box:hover {
  background: var(--hover);
  color: var(--text);
}
.box.on {
  background: var(--sel);
  color: var(--text);
}
.n {
  font-size: 12px;
}
.dotbox {
  width: 16px;
  display: flex;
  justify-content: center;
}
.adot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
}
.warn {
  color: var(--danger);
  font-weight: 600;
}
.draft {
  display: flex;
  flex-direction: column;
  gap: 1px;
  width: 100%;
  padding: 7px 10px;
  border: 0;
  border-radius: 9px;
  background: transparent;
  text-align: left;
  cursor: pointer;
}
.draft:hover,
.draft.open {
  background: var(--hover);
}
.draft-subject,
.draft-to {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.draft-subject {
  font-size: 13.5px;
}
.draft-to {
  font-size: 12px;
  color: var(--faint);
}

/* List */
.list {
  flex: 1 1 360px;
  max-width: 400px;
  min-width: 300px;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--line);
  background: var(--panel);
}
.list-top {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 16px 16px 8px;
}
.list-top .search {
  flex: 1;
  min-width: 0;
}
.list-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px 8px 20px;
}
.list-head h2 {
  flex: 1;
  min-width: 0;
  margin: 0;
  font-size: 15px;
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.rows {
  flex: 1;
  overflow: auto;
  padding: 0 8px 12px;
  transition: opacity 0.2s;
}
.rows.refreshing {
  opacity: 0.55;
}

/* Refresh button: icon at rest, spinner + label while loading. */
.spinner {
  width: 14px;
  height: 14px;
  border-radius: 50%;
  border: 2px solid color-mix(in oklab, var(--accent) 25%, transparent);
  border-top-color: var(--accent);
  animation: spin 0.7s linear infinite;
}
.progress {
  position: relative;
  height: 2px;
  margin: 0 16px;
  overflow: hidden;
  border-radius: 2px;
  opacity: 0;
  transition: opacity 0.2s;
}
.progress.active {
  opacity: 1;
  background: var(--line);
}
.progress.active::after {
  content: '';
  position: absolute;
  inset: 0 auto 0 0;
  width: 35%;
  border-radius: 2px;
  background: var(--accent);
  animation: slide 1.1s ease-in-out infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
@keyframes slide {
  from {
    transform: translateX(-100%);
  }
  to {
    transform: translateX(290%);
  }
}
@media (prefers-reduced-motion: reduce) {
  .spinner,
  .progress.active::after {
    animation-duration: 2.5s;
  }
}
.row {
  width: 100%;
  display: flex;
  gap: 12px;
  margin-bottom: 2px;
  padding: 12px;
  border: 0;
  border-radius: var(--radius);
  background: transparent;
  text-align: left;
  cursor: pointer;
}
.row:hover {
  background: var(--hover);
}
.row.selected {
  background: var(--sel);
}
/* A message row: the avatar (a tick box on hover), the message, its star. */
.rowwrap {
  position: relative;
  display: flex;
  align-items: flex-start;
  margin-bottom: 2px;
  border-radius: var(--radius);
}
.rowwrap:hover {
  background: var(--hover);
}
.rowwrap.selected {
  background: var(--sel);
}
.rowwrap.checked {
  background: color-mix(in oklab, var(--accent) 12%, transparent);
}
.rowwrap .row {
  flex: 1;
  min-width: 0;
  margin: 0;
  padding: 12px 34px 12px 12px;
  background: transparent;
}
.rowwrap .row:hover {
  background: transparent;
}
.pick {
  position: relative;
  flex: 0 0 auto;
  padding: 12px 0 12px 12px;
  cursor: pointer;
}
.pick .avatar {
  width: 32px;
  box-sizing: border-box;
}
.pick input {
  position: absolute;
  left: 12px;
  top: 12px;
  width: 32px;
  height: 32px;
  margin: 0;
  opacity: 0;
  cursor: pointer;
}
.pick input:focus-visible + .avatar {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}
.rowwrap:hover .pick .avatar,
.rowwrap.selecting .pick .avatar {
  color: transparent;
  background: transparent;
  border: 1.5px solid var(--line2);
  border-radius: 9px;
}
.rowwrap.checked .pick .avatar {
  color: transparent;
  background: var(--accent);
  border: 1.5px solid var(--accent);
  border-radius: 9px;
}
.rowwrap.checked .pick .avatar::after {
  content: '';
  width: 10px;
  height: 5px;
  margin-top: -3px;
  border-left: 2px solid var(--on-accent);
  border-bottom: 2px solid var(--on-accent);
  transform: rotate(-45deg);
}
.star {
  position: absolute;
  right: 8px;
  bottom: 10px;
  width: 26px;
  height: 26px;
  display: grid;
  place-items: center;
  border: 0;
  border-radius: 7px;
  background: transparent;
  color: var(--faint);
  cursor: pointer;
  opacity: 0;
}
.rowwrap:hover .star,
.star:focus-visible,
.star.on {
  opacity: 1;
}
.star:hover {
  color: var(--text);
}
.star.on {
  color: var(--accent);
}
.star.on :deep(path) {
  fill: currentColor;
}
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 7px;
}
.chips .invite {
  margin-top: 0;
}
.chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
  border-radius: 999px;
  font-size: 12px;
  border: 1px solid var(--line2);
  color: var(--muted);
}
.chip.snz {
  border-color: transparent;
  background: color-mix(in oklab, var(--c-work) 16%, transparent);
  color: var(--c-work);
}
.hash {
  width: 16px;
  text-align: center;
}
.sec-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
}
.small-link {
  margin: 0;
  font-family: var(--font-sans);
  font-size: 12px;
  letter-spacing: 0;
  text-transform: none;
}
/* The bar for ticked messages, in place of the list's title. */
.bulk {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 2px;
  margin: 6px 8px 8px;
  padding: 4px 6px 4px 10px;
  border-radius: 10px;
  background: color-mix(in oklab, var(--accent) 12%, var(--panel));
}
.count-sel {
  font-size: 13.5px;
  font-weight: 600;
}
.avatar {
  flex: 0 0 32px;
  height: 32px;
  border-radius: 50%;
  display: grid;
  place-items: center;
  font-size: 11.5px;
  font-weight: 600;
  background: color-mix(in oklab, var(--c, var(--faint)) 20%, transparent);
  color: var(--c, var(--faint));
}
.lines {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.top {
  display: flex;
  align-items: center;
  gap: 8px;
}
.dot {
  flex: 0 0 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent);
}
.from,
.subject {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.from {
  flex: 1;
  min-width: 0;
}
.from {
  font-size: 14px;
  color: var(--text);
}
.subject {
  font-size: 13.5px;
  color: var(--text);
}
.row.unread .from,
.row.unread .subject {
  font-weight: 600;
}
.snippet {
  font-size: 13px;
  color: var(--muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.clip {
  flex: 0 0 auto;
  color: var(--faint);
}
.acc {
  flex: 0 0 6px;
  height: 6px;
  border-radius: 50%;
  opacity: 0.8;
}
.invite {
  align-self: flex-start;
  margin-top: 7px;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 3px 9px 3px 7px;
  border-radius: 999px;
  font-size: 12px;
  background: color-mix(in oklab, var(--accent) 14%, transparent);
  color: var(--accent);
}
.day {
  padding: 14px 12px 6px;
}
.time {
  font-size: 11.5px;
  color: var(--faint);
}
.skeleton .lines span {
  height: 10px;
  border-radius: 4px;
  background: var(--line);
}
.skeleton .lines span:nth-child(1) {
  width: 45%;
}
.skeleton .lines span:nth-child(2) {
  width: 80%;
}
.skeleton .lines span:nth-child(3) {
  width: 60%;
}
.skeleton .avatar {
  background: var(--line);
}
.notice {
  display: flex;
  gap: 12px;
  margin: 8px;
  padding: 16px;
  border-radius: var(--radius-lg);
  border: 1px solid var(--line2);
  background: var(--raise);
  color: var(--danger);
}
.notice div {
  color: var(--text);
}
.notice p {
  margin: 6px 0 0;
  color: var(--muted);
  font-size: 13px;
  line-height: 1.55;
}
.notice .hint {
  margin-bottom: 12px;
}
code {
  font-family: var(--font-mono);
  font-size: 12px;
  padding: 1px 5px;
  border-radius: 4px;
  background: var(--bg);
}
.empty {
  padding: 24px 20px;
  color: var(--faint);
}

/* The sender, beside the message (wide screens only). */
.about {
  flex: 0 1 280px;
  min-width: 240px;
  padding: 24px 20px;
  display: flex;
  flex-direction: column;
  gap: 22px;
  border-left: 1px solid var(--line);
  background: var(--bg);
  overflow: auto;
}
.who {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.big-avatar {
  width: 52px;
  height: 52px;
  border-radius: 50%;
  display: grid;
  place-items: center;
  font-size: 17px;
  font-weight: 600;
  background: color-mix(in oklab, var(--c, var(--faint)) 20%, transparent);
  color: var(--c, var(--faint));
}
.pname {
  font-size: 22px;
  line-height: 1.2;
}
.addr {
  font-size: 13px;
  overflow-wrap: anywhere;
}
.acts {
  display: flex;
  gap: 6px;
}
.mini:hover,
.next:hover,
.line:hover {
  background: var(--hover);
}
.ablock {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.ablock .eyebrow {
  margin-bottom: 4px;
}
.next {
  display: flex;
  gap: 12px;
  padding: 10px;
  margin: 0 -10px;
  border-radius: 10px;
  color: var(--text);
  text-decoration: none;
}
.bar {
  flex: 0 0 4px;
  border-radius: 2px;
  background: var(--accent);
}
.bar.pending {
  background: repeating-linear-gradient(var(--accent) 0 4px, transparent 4px 7px);
}
.ntext {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.ntitle {
  font-size: 13.5px;
  font-weight: 500;
}
.ntext .small {
  font-size: 12.5px;
}
.line {
  display: flex;
  justify-content: space-between;
  gap: 10px;
  padding: 7px 10px;
  margin: 0 -10px;
  border-radius: 8px;
  color: var(--text);
  text-decoration: none;
  font-size: 13px;
}
.ellip {
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.tiny {
  flex: 0 0 auto;
  font-size: 11.5px;
}
@media (max-width: 1499px) {
  .about {
    display: none;
  }
}
@media (max-width: 1100px) {
  .boxes {
    display: none;
  }
}
/* Phones: one pane at a time — the list, or the open message. */
@media (max-width: 760px) {
  .list {
    flex: 1;
    min-width: 0;
    border-right: 0;
  }
  .reader-pane {
    display: none;
  }
  .has-selection .list {
    display: none;
  }
  .has-selection .reader-pane {
    display: flex;
  }
}

/* Search */
.results {
  font-weight: 500;
}
.link {
  margin-left: 6px;
  padding: 0;
  border: 0;
  background: none;
  color: var(--accent);
  font-size: 13px;
  cursor: pointer;
}

.welcome {
  flex: 1;
  display: grid;
  place-items: center;
  padding: 24px;
  background: var(--panel);
}
.welcome-card {
  max-width: 440px;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 14px;
}
.welcome-card h1 {
  margin: 0;
  font-size: 34px;
  line-height: 1.1;
}
.welcome-card p {
  margin: 0 0 6px;
  color: var(--muted);
  line-height: 1.6;
}
.welcome-icon {
  width: 48px;
  height: 48px;
  border-radius: 14px;
  display: grid;
  place-items: center;
  background: color-mix(in oklab, var(--accent) 16%, transparent);
  color: var(--accent);
}
.tree {
  flex: 1;
  min-height: 0;
  overflow: auto;
  margin: 0 -4px;
  padding: 0 4px 12px;
}
.add-account {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 32px;
  margin-top: 10px;
  padding: 0 8px;
  border-radius: var(--radius-sm);
  color: var(--faint);
  font-size: 13px;
  text-decoration: none;
}
.add-account:hover {
  background: var(--hover);
  color: var(--text);
}
.link.block {
  display: block;
  margin: 10px 0 0;
}
.server-search {
  padding: 14px 12px 4px;
  text-align: center;
}

/* Load more */
.more {
  display: flex;
  justify-content: center;
  padding: 12px 0 4px;
}
.more-btn {
  height: 34px;
  font-size: 13px;
  color: var(--muted);
}
.more-btn .spinner {
  width: 12px;
  height: 12px;
}
</style>
