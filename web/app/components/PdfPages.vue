<script setup lang="ts">
// A PDF's pages, drawn by pdf.js (Mozilla's PDF reader, the one in Firefox).
// pdf.js reads the file in a Web Worker (a script running beside the page, so
// a big PDF doesn't freeze it) and draws each page on a <canvas>: nothing the
// PDF contains is run (its forms and scripts are ignored).
//
// This component is only loaded when a PDF preview opens (`LazyPdfPages`),
// and pdf.js itself only when it's mounted: about 1.8 MB that other pages
// never download.
import type { PDFDocumentLoadingTask, PDFDocumentProxy } from 'pdfjs-dist'
// `?url`: the worker file's address in the build, not its code. The
// "legacy" build: the same pdf.js, with what older browsers lack filled in
// (the modern one already needs JavaScript features some browsers don't have).
import workerUrl from 'pdfjs-dist/legacy/build/pdf.worker.min.mjs?url'

const props = withDefaults(
  defineProps<{
    /** The PDF's bytes. */
    data: ArrayBuffer
    /** Only the first page, small (the Files pane). */
    first?: boolean
  }>(),
  { first: false },
)
const emit = defineEmits<{ error: [message: string] }>()

const pdf = shallowRef<PDFDocumentProxy | null>(null)
let task: PDFDocumentLoadingTask | null = null
const pages = ref(0)
/** 1 = the page's width fits the window; zoom multiplies it. */
const zoom = ref(1)
/** The page you're on (the one most in view). */
const current = ref(1)
const root = ref<HTMLElement | null>(null)
const canvases = ref<HTMLCanvasElement[]>([])
/** Pages already drawn at this zoom. */
const drawn = new Set<number>()
let observer: IntersectionObserver | null = null
/** Page 1's shape (width / height): undrawn pages get its size, so the
 *  document is as tall as it will be and scrolling lands where it should. */
let shape = 210 / 297
const pageSize = ref({ width: 0, height: 0 })

onMounted(async () => {
  try {
    const pdfjs = await import('pdfjs-dist/legacy/build/pdf.mjs')
    pdfjs.GlobalWorkerOptions.workerSrc = workerUrl
    task = pdfjs.getDocument({
      // A copy: pdf.js hands the bytes to its worker, which empties the original.
      data: new Uint8Array(props.data.slice(0)),
      standardFontDataUrl: '/pdfjs/standard_fonts/',
      wasmUrl: '/pdfjs/wasm/',
      cMapUrl: '/pdfjs/cmaps/',
      cMapPacked: true,
      enableXfa: false,
    })
    pdf.value = await task.promise
    const first = (await pdf.value.getPage(1)).getViewport({ scale: 1 })
    shape = first.width / first.height
    sizePages()
    pages.value = props.first ? 1 : pdf.value.numPages
    await nextTick()
    watchPages()
  } catch (err) {
    emit('error', err instanceof Error && err.name === 'PasswordException' ? 'This PDF has a password.' : "This PDF can't be read.")
  }
})

onBeforeUnmount(() => {
  observer?.disconnect()
  cancelAnimationFrame(frame)
  // Frees the worker and what it read.
  void task?.destroy()
})

/** Draws pages as they come into view (a 300-page PDF isn't drawn at once). */
function watchPages() {
  observer?.disconnect()
  // Drawing: a little before a page scrolls into view (`rootMargin`).
  observer = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (entry.isIntersecting) void draw(Number((entry.target as HTMLElement).dataset.page))
      }
    },
    { root: props.first ? null : root.value, rootMargin: '400px 0px' },
  )
  for (const c of canvases.value) observer.observe(c)
}

/**
 * The page counter: the page under the middle of the view (the last one
 * when scrolled to the end). `requestAnimationFrame` works it out at most
 * once per frame while scrolling.
 */
let frame = 0
function onScroll() {
  if (frame) return
  frame = requestAnimationFrame(() => {
    frame = 0
    const box = root.value
    if (!box) return
    if (box.scrollTop + box.clientHeight >= box.scrollHeight - 2) {
      current.value = pages.value
      return
    }
    const middle = box.getBoundingClientRect().top + box.clientHeight / 2
    const n = canvases.value.findIndex((c) => c.getBoundingClientRect().bottom + 6 >= middle)
    current.value = n < 0 ? pages.value : n + 1
  })
}

/** The width available, times the zoom. */
function pageWidth() {
  return ((root.value?.clientWidth ?? 600) - (props.first ? 0 : 32)) * zoom.value
}

function sizePages() {
  const width = Math.floor(pageWidth())
  pageSize.value = { width, height: Math.floor(width / shape) }
}

async function draw(n: number) {
  const doc = pdf.value
  const canvas = canvases.value[n - 1]
  if (!doc || !canvas || drawn.has(n)) return
  drawn.add(n)
  const page = await doc.getPage(n)
  // Sharp on high-density screens: more canvas pixels than CSS pixels.
  const base = page.getViewport({ scale: 1 })
  const scale = pageWidth() / base.width
  const ratio = window.devicePixelRatio || 1
  const viewport = page.getViewport({ scale: scale * ratio })
  canvas.width = Math.floor(viewport.width)
  canvas.height = Math.floor(viewport.height)
  canvas.style.width = `${Math.floor(viewport.width / ratio)}px`
  canvas.style.height = `${Math.floor(viewport.height / ratio)}px`
  await page.render({ canvas, viewport }).promise
}

/** A new zoom: every page is drawn again as it comes into view. */
function setZoom(z: number) {
  zoom.value = Math.min(4, Math.max(0.5, Math.round(z * 100) / 100))
  drawn.clear()
  // Every page takes its new size at once (drawn ones are stretched until redrawn).
  sizePages()
  for (const c of canvases.value) {
    c.style.width = ''
    c.style.height = ''
  }
  watchPages()
}

defineExpose({ pages, current, zoom, setZoom })
</script>

<template>
  <div
    ref="root"
    class="pdf"
    :class="{ first }"
    :style="{ '--page-w': `${pageSize.width}px`, '--page-h': `${pageSize.height}px` }"
    @scroll.passive="onScroll"
  >
    <canvas
      v-for="n in pages"
      :key="n"
      :ref="(el) => { if (el) canvases[n - 1] = el as HTMLCanvasElement }"
      :data-page="n"
      class="page"
      role="img"
      :aria-label="`Page ${n}`"
    />
  </div>
</template>

<style scoped>
.pdf {
  overflow: auto;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  padding: 16px;
  min-height: 0;
}
.pdf.first {
  overflow: hidden;
  padding: 0;
}
/* Pages are paper: white in dark mode too. */
.page {
  background: #fff;
  box-shadow: 0 1px 4px rgb(0 0 0 / 0.25);
  max-width: none;
  flex: none;
  /* Before it's drawn: the size it will have (its own style, once drawn, wins). */
  width: var(--page-w);
  height: var(--page-h);
}
.first .page {
  box-shadow: none;
  width: 100%;
}
</style>
