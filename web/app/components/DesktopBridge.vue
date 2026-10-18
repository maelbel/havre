<script setup lang="ts">
import type { Draft } from '~/composables/useCompose'

// Only in Havre Desktop (mounted by app.vue): keeps the tray's unread count
// up to date, notifies new mail when the window isn't in front, and opens
// mailto: links (?compose=mailto:…) as a new message.
const route = useRoute()
const router = useRouter()
const toast = useToasts()
const accounts = useAccounts()

// ---- Unread count and new-mail notifications ------------------------------------------
// The same list as Today's "Unread": its total goes to the tray; messages
// not seen before are notified (the first load only learns what's there).
const seen = new Set<string>()
let first = true
let timer: ReturnType<typeof setTimeout> | undefined

async function check() {
  let unread
  try {
    unread = await fetchUnread(20)
  } catch {
    return // signed out, server restarting…: try again at the next event
  }
  desktopInvoke('set_unread', { count: unread.total }).catch(() => {})
  const fresh = unread.messages.filter((m) => !seen.has(`${m.account}:${m.folder}:${m.uid}`))
  for (const m of unread.messages) seen.add(`${m.account}:${m.folder}:${m.uid}`)
  if (first) {
    first = false
    return
  }
  // You're looking at Havre: no need to tell you.
  if (!fresh.length || (document.hasFocus() && !document.hidden)) return
  if (fresh.length > 3) {
    notify(`${fresh.length} new messages`, fresh.map((m) => personLabel(m.from)).slice(0, 4).join(', '))
  } else {
    for (const m of fresh) notify(personLabel(m.from), m.subject || '(no subject)')
  }
}
function notify(title: string, body: string) {
  desktopInvoke('notify', { title, body }).catch(() => {})
}
function checkSoon() {
  clearTimeout(timer)
  timer = setTimeout(check, 800)
}
useMailEvents((event) => {
  if (event.type === 'folder' || event.type === 'folders' || event.type === 'accounts') checkSoon()
})
// Also every 5 minutes, in case the live connection was down.
let every: ReturnType<typeof setInterval> | undefined
onMounted(() => {
  check()
  every = setInterval(check, 5 * 60_000)
})
onBeforeUnmount(() => {
  clearTimeout(timer)
  clearInterval(every)
})

// ---- mailto: links ---------------------------------------------------------------------
async function composeFrom(link: string) {
  // Wait for the accounts (the link may arrive while the app starts).
  if (accounts.status.value !== 'success') await accounts.refresh()
  const account = accounts.data.value.find((a) => a.can_send)
  if (!account) {
    toast.show('Add an account that can send before writing from a link.', { tone: 'error' })
    return
  }
  const m = parseMailto(link)
  try {
    const draft = await $fetch<Draft>('/api/drafts', {
      method: 'POST',
      body: { account: account.id, kind: 'new', to: m.to },
    })
    if (m.cc || m.bcc || m.subject || m.body) {
      await saveDraft(draft.id, { cc: m.cc, bcc: m.bcc, subject: m.subject, body: m.body })
    }
    refreshNuxtData('drafts')
    openDraft(draft.id)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  }
}
watch(
  () => route.query.compose,
  (value) => {
    if (typeof value !== 'string' || !/^mailto:/i.test(value)) return
    const { compose: _, ...rest } = route.query
    router.replace({ query: rest })
    composeFrom(value)
  },
  { immediate: true },
)
</script>

<template>
  <span hidden />
</template>
