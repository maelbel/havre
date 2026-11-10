// `pnpm test`: the web app's plain functions, with Node's own test runner
// (`--experimental-strip-types` lets Node read the TypeScript file as is).
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { decodeText, PREVIEW_MAX_BYTES, previewKind, previewType } from '../app/utils/preview.ts'

test('the kind comes from the type, else the extension', () => {
  assert.equal(previewKind('devis.pdf', 'application/pdf', 1000), 'pdf')
  assert.equal(previewKind('devis.PDF', 'application/octet-stream', 1000), 'pdf')
  assert.equal(previewKind('photo.jpg', 'image/jpeg', 1000), 'image')
  assert.equal(previewKind('notes.txt', 'text/plain', 1000), 'text')
  assert.equal(previewKind('export.csv', 'application/octet-stream', 1000), 'text')
  assert.equal(previewKind('invite.ics', 'text/calendar', 1000), 'text')
  assert.equal(previewKind('memo.m4a', 'application/octet-stream', 1000), 'audio')
  assert.equal(previewKind('voice.ogg', 'audio/ogg; codecs=opus', 1000), 'audio')
  assert.equal(previewKind('clip.mov', 'video/quicktime', 1000), 'video')
  assert.equal(previewKind('report.docx', 'application/vnd.openxmlformats-officedocument.wordprocessingml.document', 1000), 'none')
  assert.equal(previewKind('archive.zip', 'application/zip', 1000), 'none')
  assert.equal(previewKind('no-extension', 'application/octet-stream', 1000), 'none')
})

test('HTML and SVG are never previewed, nor files too large', () => {
  assert.equal(previewKind('page.html', 'text/html', 1000), 'none')
  assert.equal(previewKind('page.txt', 'text/html', 1000), 'none')
  assert.equal(previewKind('logo.svg', 'image/svg+xml', 1000), 'none')
  assert.equal(previewKind('logo.svg', 'application/octet-stream', 1000), 'none')
  assert.equal(previewKind('big.pdf', 'application/pdf', PREVIEW_MAX_BYTES + 1), 'none')
  assert.equal(previewKind('ok.pdf', 'application/pdf', PREVIEW_MAX_BYTES), 'pdf')
})

test('players get the real type', () => {
  assert.equal(previewType('memo.m4a', 'application/octet-stream'), 'audio/mp4')
  assert.equal(previewType('clip.webm', ''), 'video/webm')
  assert.equal(previewType('voice.ogg', 'audio/ogg'), 'audio/ogg')
  assert.equal(previewType('thing.bin', 'application/octet-stream'), 'application/octet-stream')
})

test('text is UTF-8, else Latin-1', () => {
  const utf8 = new TextEncoder().encode('Réunion à Lyon — café')
  assert.equal(decodeText(utf8), 'Réunion à Lyon — café')
  // "Réunion" as Windows-1252 bytes: é is 0xE9, not valid UTF-8 alone.
  const latin1 = Uint8Array.from([0x52, 0xe9, 0x75, 0x6e, 0x69, 0x6f, 0x6e])
  assert.equal(decodeText(latin1), 'Réunion')
  const bom = Uint8Array.from([0xef, 0xbb, 0xbf, 0x68, 0x69])
  assert.equal(decodeText(bom), 'hi')
})
