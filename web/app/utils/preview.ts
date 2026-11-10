// What kind of preview a file gets (step 27). Plain TypeScript with no Nuxt
// in it, so `web/tests/preview.test.mjs` can run it with Node.
//
// The type a mail app gives an attachment is often a generic
// `application/octet-stream`: the file name's extension decides then.

export type PreviewKind = 'image' | 'pdf' | 'text' | 'audio' | 'video' | 'none'

/** Above this, a preview would download too much: Download only. */
export const PREVIEW_MAX_BYTES = 25 * 1024 * 1024
/** A text preview shows this much; the rest is a download away. */
export const TEXT_MAX_BYTES = 1024 * 1024

const IMAGES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp']
const TEXT_EXT = ['txt', 'text', 'csv', 'tsv', 'md', 'markdown', 'json', 'log', 'xml', 'ics', 'vcf', 'yaml', 'yml', 'ini', 'cfg', 'conf', 'toml', 'sql', 'rtf']
const AUDIO_EXT: Record<string, string> = { mp3: 'audio/mpeg', m4a: 'audio/mp4', aac: 'audio/aac', ogg: 'audio/ogg', oga: 'audio/ogg', opus: 'audio/ogg', wav: 'audio/wav', weba: 'audio/webm', flac: 'audio/flac' }
const VIDEO_EXT: Record<string, string> = { mp4: 'video/mp4', m4v: 'video/mp4', webm: 'video/webm', mov: 'video/quicktime', ogv: 'video/ogg' }
const IMAGE_EXT: Record<string, string> = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp' }

function extension(name: string) {
  const dot = name.lastIndexOf('.')
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : ''
}

/**
 * The preview for a file, or `none`. HTML and SVG are never previewed: they
 * can carry scripts, and showing their source as text isn't what you'd want.
 */
export function previewKind(name: string, mime: string, size: number): PreviewKind {
  const type = (mime || '').toLowerCase().split(';')[0]!.trim()
  const ext = extension(name)
  if (size > PREVIEW_MAX_BYTES) return 'none'
  if (type === 'text/html' || type === 'image/svg+xml' || ['html', 'htm', 'svg', 'xhtml'].includes(ext)) return 'none'
  if (IMAGES.includes(type) || ext in IMAGE_EXT) return 'image'
  if (type === 'application/pdf' || ext === 'pdf') return 'pdf'
  if (type.startsWith('audio/') || ext in AUDIO_EXT) return 'audio'
  if (type.startsWith('video/') || ext in VIDEO_EXT) return 'video'
  if (type.startsWith('text/') || ['application/json', 'application/xml', 'text/calendar'].includes(type) || TEXT_EXT.includes(ext)) {
    return 'text'
  }
  return 'none'
}

/** The type to give the bytes (audio and video players need the real one). */
export function previewType(name: string, mime: string): string {
  const type = (mime || '').toLowerCase().split(';')[0]!.trim()
  if (type && type !== 'application/octet-stream') return type
  const ext = extension(name)
  return AUDIO_EXT[ext] ?? VIDEO_EXT[ext] ?? IMAGE_EXT[ext] ?? (ext === 'pdf' ? 'application/pdf' : 'application/octet-stream')
}

/**
 * Bytes → text: UTF-8 when they are valid UTF-8, else Latin-1 (Windows
 * files often are). A byte-order mark is dropped. `fatal: true` makes the
 * UTF-8 decoder throw on invalid bytes instead of writing "�".
 */
export function decodeText(bytes: Uint8Array): string {
  try {
    return new TextDecoder('utf-8', { fatal: true, ignoreBOM: false }).decode(bytes)
  } catch {
    return new TextDecoder('windows-1252').decode(bytes)
  }
}
