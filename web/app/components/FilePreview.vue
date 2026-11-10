<script setup lang="ts">
// A preview of an attachment: a PDF's pages, a text, a player, an image.
//
// Attachments are strangers' files. The server only serves common images for
// display and makes every other file a download (with a strict sandbox
// header), so a file can never run as you on Havre's address. Previews keep
// that rule: this component fetches the bytes and shows them itself:
// - text through Vue's `{{ }}`, which never reads it as HTML;
// - audio and video through a `blob:` address (bytes in memory, with the
//   right type), which a player can read but that runs nothing;
// - PDFs drawn by pdf.js (`PdfPages`).
// HTML and SVG files are never previewed (`previewKind`).
const props = withDefaults(
  defineProps<{
    /** The download address (`/api/mail/messages/…/attachments/…`). */
    url: string
    name: string
    mime: string
    size: number
    /** Small, for the Files pane: the first page, the start of a text. */
    compact?: boolean
  }>(),
  { compact: false },
)

const kind = computed(() => previewKind(props.name, props.mime, props.size))
const loading = ref(false)
const error = ref<string | null>(null)
/** The PDF's bytes. */
const pdfData = shallowRef<ArrayBuffer | null>(null)
const text = ref('')
const textCut = ref(false)
/** A `blob:` address for the player; released when the preview goes. */
const mediaUrl = ref<string | null>(null)
const pdfRef = ref<{ pages: number; current: number; zoom: number; setZoom: (z: number) => void } | null>(null)

function release() {
  if (mediaUrl.value) URL.revokeObjectURL(mediaUrl.value)
  mediaUrl.value = null
}

async function load() {
  release()
  pdfData.value = null
  text.value = ''
  error.value = null
  if (kind.value === 'none' || kind.value === 'image') return
  loading.value = true
  try {
    const response = await fetch(props.url, { credentials: 'same-origin' })
    if (!response.ok) throw new Error(`${response.status}`)
    const bytes = await response.arrayBuffer()
    if (kind.value === 'pdf') {
      pdfData.value = bytes
    } else if (kind.value === 'text') {
      const limit = props.compact ? 4096 : TEXT_MAX_BYTES
      textCut.value = bytes.byteLength > limit
      text.value = decodeText(new Uint8Array(bytes, 0, Math.min(limit, bytes.byteLength)))
    } else {
      mediaUrl.value = URL.createObjectURL(new Blob([bytes], { type: previewType(props.name, props.mime) }))
    }
  } catch {
    error.value = "This file couldn't be loaded."
  } finally {
    loading.value = false
  }
}

watch(() => props.url, load, { immediate: true })
onBeforeUnmount(release)

const zoomLabel = computed(() => `${Math.round((pdfRef.value?.zoom ?? 1) * 100)} %`)
</script>

<template>
  <div class="file-preview" :class="[kind, { compact }]">
    <div v-if="kind === 'none'" class="empty">
      <span class="mono ext" aria-hidden="true">{{ fileExt(name) }}</span>
      <p v-if="!compact" class="muted">No preview for this file.</p>
    </div>

    <img v-else-if="kind === 'image'" :src="url" :alt="`Preview of ${name}`" class="image" />

    <div v-else-if="loading" class="empty" role="status">
      <HIcon name="loading" :size="18" class="spin" />
      <span class="muted">Loading the preview…</span>
    </div>

    <div v-else-if="error" class="empty" role="alert">
      <p class="muted">{{ error }}</p>
    </div>

    <template v-else-if="kind === 'pdf' && pdfData">
      <div v-if="!compact && pdfRef?.pages" class="bar" role="toolbar" aria-label="PDF">
        <span class="faint small" aria-live="polite">Page {{ pdfRef.current }} / {{ pdfRef.pages }}</span>
        <div class="zoom">
          <UButton icon="i-havre-minus" color="neutral" variant="ghost" size="xs" aria-label="Zoom out" @click="pdfRef.setZoom(pdfRef.zoom - 0.25)" />
          <span class="small mono">{{ zoomLabel }}</span>
          <UButton icon="i-havre-plus" color="neutral" variant="ghost" size="xs" aria-label="Zoom in" @click="pdfRef.setZoom(pdfRef.zoom + 0.25)" />
          <UButton color="neutral" variant="ghost" size="xs" :disabled="pdfRef.zoom === 1" @click="pdfRef.setZoom(1)">Fit width</UButton>
        </div>
      </div>
      <LazyPdfPages ref="pdfRef" :data="pdfData" :first="compact" class="pages" @error="(m: string) => (error = m)" />
    </template>

    <div v-else-if="kind === 'text'" class="text-wrap">
      <pre class="text mono">{{ text }}</pre>
      <p v-if="textCut" class="faint small cut">The rest of this file is in the download.</p>
    </div>

    <audio v-else-if="kind === 'audio' && mediaUrl" :src="mediaUrl" controls class="audio" :aria-label="name" />
    <video v-else-if="kind === 'video' && mediaUrl" :src="mediaUrl" controls playsinline class="video" :aria-label="name" />
  </div>
</template>

<style scoped>
.file-preview {
  display: flex;
  flex-direction: column;
  min-height: 0;
  height: 100%;
}
.empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  padding: 24px;
  text-align: center;
}
.ext {
  font-size: 28px;
  letter-spacing: 0.04em;
  color: var(--muted);
}
.compact .ext {
  font-size: 34px;
}
.spin {
  animation: spin 1s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
.image {
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
  margin: auto;
}
.bar {
  flex: none;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 6px 12px;
  border-bottom: 1px solid var(--line);
}
.zoom {
  display: flex;
  align-items: center;
  gap: 4px;
}
.zoom .mono {
  min-width: 48px;
  text-align: center;
}
.pages {
  flex: 1;
}
.text-wrap {
  flex: 1;
  overflow: auto;
  min-height: 0;
}
.text {
  margin: 0;
  padding: 16px;
  font-size: 12.5px;
  line-height: 1.55;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  color: var(--text);
}
.compact .text {
  padding: 10px 12px;
  font-size: 11px;
  max-height: 100%;
  overflow: hidden;
}
.cut {
  padding: 0 16px 16px;
}
.audio {
  margin: auto;
  width: min(420px, 90%);
}
/* The whole area; the picture keeps its proportions inside it. */
.video {
  flex: 1;
  width: 100%;
  min-height: 0;
  object-fit: contain;
}
</style>
