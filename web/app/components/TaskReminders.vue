<script setup lang="ts">
// Mounted once in the layout: shows task reminders (a toast, and a system
// notification when Havre isn't in front — in Havre Desktop or a browser
// that allowed it), and keeps the dock's "due today" count fresh.
const toast = useToasts()
const enabled = useTasksEnabled()
const due = useState('tasks-due', () => 0)

async function count() {
  if (!enabled.value) return
  try {
    const open = await fetchTasks({ done: false, to: localDay() + 86_400 })
    due.value = open.filter((t) => t.due != null).length
  } catch {
    /* signed out or demo: no badge */
  }
}
let timer: ReturnType<typeof setTimeout> | undefined
function countSoon() {
  clearTimeout(timer)
  timer = setTimeout(count, 500)
}
watch(enabled, count)
let every: ReturnType<typeof setInterval> | undefined
onMounted(() => {
  count()
  // Midnight passes, tasks become due: recount now and then.
  every = setInterval(count, 10 * 60_000)
})
onBeforeUnmount(() => {
  clearTimeout(timer)
  clearInterval(every)
})

function notify(title: string) {
  const away = document.hidden || !document.hasFocus()
  if (!away) return
  if (isDesktop()) {
    desktopInvoke('notify', { title: 'Reminder', body: title }).catch(() => {})
  } else if ('Notification' in window && Notification.permission === 'granted') {
    new Notification('Reminder', { body: title, tag: `havre-task-${title}` })
  }
}

useMailEvents((event) => {
  if (event.type === 'tasks') countSoon()
  if (event.type === 'reminder') {
    toast.show(`Reminder: ${event.title}`, {
      duration: 12_000,
      action: { label: 'Open', run: () => navigateTo({ path: '/tasks', query: { task: event.task } }) },
    })
    notify(event.title)
  }
})
</script>

<template>
  <span hidden />
</template>
