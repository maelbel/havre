<script setup lang="ts">
// Mounted once in the layout: says when a scheduled message couldn't go
// (or went late, Havre having been off), and keeps the Drafts and Scheduled
// lists fresh. Phones get the failures by push (the server sends those).
const toast = useToasts()

useMailEvents((event) => {
  if (event.type !== 'outbox') return
  refreshNuxtData('scheduled')
  refreshNuxtData('drafts')
  const subject = event.subject.trim() || '(no subject)'
  if (!event.sent && event.error) {
    toast.show(`Not sent: ${subject} — ${event.error}`, {
      tone: 'error',
      duration: 15_000,
      action: { label: 'Open', run: () => openDraft(event.draft) },
    })
    if (isDesktop() && (document.hidden || !document.hasFocus())) {
      desktopInvoke('notify', { title: `Not sent: ${subject}`, body: event.error }).catch(() => {})
    }
  } else if (event.sent && event.late) {
    toast.show(`Sent late (Havre was off at its time): ${subject}`)
  } else if (event.sent && event.error) {
    // Sent, but something after failed (no copy in Sent…).
    toast.show(event.error, { tone: 'error' })
  }
})
</script>

<template>
  <span hidden />
</template>
