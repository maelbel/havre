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
  /* One scroll: the frame is as tall as the email, so the email never scrolls
     up and down inside it (only sideways, when it's wider than the pane).
     An email that fixes its own height can't change that. */
  html{overflow-y:hidden!important;overflow-x:auto}
  html,body{height:auto!important;min-height:0!important;max-height:none!important}
  body{overflow:visible!important;padding:28px 32px;font:15px/1.6 -apple-system,'Segoe UI',system-ui,sans-serif;color:#1c1b17;background:#fff;overflow-wrap:anywhere}
  img{max-width:100%;height:auto}
  table{max-width:100%}
  a{color:#7a5208}
</style></head><body><div id="havre-mail">${props.html}</div></body></html>`
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

// The iframe must be as tall as the email, so the reader scrolls as one.
//
// What the email needs: its content's height, plus the sideways scrollbar
// when it's wider than the pane (offsetHeight - clientHeight is that bar's
// thickness), plus the frame's own border — Havre's `box-sizing: border-box`
// takes the border out of the height we set. Forgetting the border made
// every email 2 px too tall for its frame: a second scrollbar inside.
let observer: ResizeObserver | null = null
function measure(doc: Document) {
  const root = doc.documentElement
  const scrollbar = root.offsetHeight - root.clientHeight
  const el = frame.value
  const border = el ? el.offsetHeight - el.clientHeight : 0
  height.value = Math.max(120, Math.ceil(root.scrollHeight + scrollbar + border))
}
function fit() {
  const doc = frame.value?.contentDocument
  if (!doc?.documentElement) return
  forwardKeys(doc)
  measure(doc)
  observer?.disconnect()
  // Follows images that load late, and the pane being resized. The wrapper
  // is measured, not `body`: an email may fix its body's height.
  observer = new ResizeObserver(() => measure(doc))
  observer.observe(doc.getElementById('havre-mail') ?? doc.body)
  observer.observe(doc.documentElement)
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
