<script setup lang="ts">
// Mounted once in the layout: shows event reminders (step 21) as a toast,
// and a system notification in Havre Desktop when it isn't in front. Phones
// get them by push (the server sends those). Also tells the server this
// browser's time zone, so all-day reminders ring at your 9:00.
const toast = useToasts()

onMounted(() => {
  reportTimeZone().catch(() => {
    /* signed out or demo: nothing to tell */
  })
})

/** "in 10 min", "now", "tomorrow"… for the toast. */
function when(start: number, allDay: boolean): string {
  if (allDay) {
    const days = Math.round((utcDateToLocal(start).getTime() - startOfDay(new Date()).getTime()) / 86_400_000)
    return days <= 0 ? 'today' : days === 1 ? 'tomorrow' : `in ${days} days`
  }
  const minutes = Math.round((start * 1000 - Date.now()) / 60_000)
  if (minutes <= 0) return 'now'
  if (minutes < 60) return `in ${minutes} min`
  const hours = Math.round(minutes / 60)
  return hours < 24 ? `in ${hours} h` : `in ${Math.round(hours / 24)} days`
}

useMailEvents((event) => {
  if (event.type !== 'event_reminder') return
  const at = event.all_day ? 'all day' : timeLabel(new Date(event.start * 1000))
  const place = event.location ? `, ${event.location}` : ''
  const line = `${event.title} · ${when(event.start, event.all_day)} · ${at}${place}`
  const day = isoDate(event.all_day ? utcDateToLocal(event.start) : new Date(event.start * 1000))
  toast.show(line, {
    duration: 20_000,
    action: event.join_url
      ? { label: 'Join', run: () => openExternal(event.join_url!) }
      : { label: 'Open', run: () => navigateTo({ path: '/agenda', query: { date: day, event: String(event.event) } }) },
  })
  if (isDesktop() && (document.hidden || !document.hasFocus())) {
    desktopInvoke('notify', { title: event.title, body: `${when(event.start, event.all_day)} · ${at}${place}` }).catch(() => {})
  }
})
</script>

<template>
  <span hidden />
</template>
