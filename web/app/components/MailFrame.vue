<script setup lang="ts">
// Shows an email's HTML safely.
//
// The server already sanitized it; this is the second layer:
// - `sandbox` without `allow-scripts`: nothing in the email can run.
// - A Content-Security-Policy that only lets images load from Havre itself
//   (embedded images) — plus remote ones when the user allowed them.
// - Links open in a new tab, outside Havre.
const props = defineProps<{ html: string; allowRemoteImages: boolean }>()

const frame = ref<HTMLIFrameElement | null>(null)
const height = ref(200)

const srcdoc = computed(() => {
  const img = props.allowRemoteImages ? "'self' data: https: http:" : "'self' data:"
  return `<!doctype html><html lang="und"><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src ${img}; style-src 'unsafe-inline'; font-src data:">
<base target="_blank">
<style>
  html,body{margin:0}
  body{padding:28px 32px;font:15px/1.6 -apple-system,'Segoe UI',system-ui,sans-serif;color:#1c1b17;background:#fff;overflow-wrap:anywhere}
  img{max-width:100%;height:auto}
  table{max-width:100%}
  a{color:#7a5208}
</style></head><body>${props.html}</body></html>`
})

// Keyboard shortcuts keep working after a click inside the email: keys
// pressed in the iframe are passed on to the page (↓/↑ switch messages
// instead of scrolling the email).
const FORWARDED = new Set(['ArrowDown', 'ArrowUp', 'j', 'k', 'u', 'e', '#', 'Delete', 'Escape', '/', 'c', 'r', 'a', 'f', 's'])
function forwardKeys(doc: Document) {
  doc.addEventListener('keydown', (e) => {
    // Ctrl+K / ⌘K: the command palette.
    const palette = (e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === 'k'
    if (!palette && (!FORWARDED.has(e.key) || e.metaKey || e.ctrlKey || e.altKey)) return
    e.preventDefault()
    window.dispatchEvent(
      new KeyboardEvent('keydown', { key: e.key, ctrlKey: e.ctrlKey, metaKey: e.metaKey, bubbles: true, cancelable: true }),
    )
  })
}

// The iframe must be as tall as the email, so the page scrolls as one.
let observer: ResizeObserver | null = null
function fit() {
  const doc = frame.value?.contentDocument
  if (!doc?.documentElement) return
  forwardKeys(doc)
  height.value = Math.max(120, doc.documentElement.scrollHeight)
  observer?.disconnect()
  observer = new ResizeObserver(() => {
    height.value = Math.max(120, doc.documentElement.scrollHeight)
  })
  observer.observe(doc.body)
}
onBeforeUnmount(() => observer?.disconnect())
</script>

<template>
  <iframe
    ref="frame"
    class="mail-frame"
    title="Email content"
    sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox"
    :srcdoc="srcdoc"
    :style="{ height: `${height}px` }"
    @load="fit"
  />
</template>

<style scoped>
.mail-frame {
  display: block;
  width: 100%;
  border: 1px solid var(--line);
  border-radius: var(--radius-lg);
  background: #fff;
}
</style>
