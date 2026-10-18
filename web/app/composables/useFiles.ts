// Files: every attachment in your mail. Mirrors `/api/files/*`
// (crates/server/src/files.rs). Names start with `file` / `files`:
// composables share one auto-import namespace (see docs/HANDOFF.md).

export type FileKind = 'pdf' | 'doc' | 'sheet' | 'slides' | 'image' | 'archive' | 'other'

/** An email a file is in. */
export interface FileEmail {
  account: number
  folder: string
  uid: number
  subject: string
  date: string | null
  from: string | null
  sent: boolean
}

export interface FileVersion {
  key: string
  name: string
  size: number
  date: string | null
  from: string | null
  sent: boolean
}

/** One file (the newest email with it, flattened in). */
export interface FileItem extends FileEmail {
  /** `account:index:uid:folder`: names one attachment. */
  key: string
  name: string
  mime: string
  size: number
  kind: FileKind
  index: number
  from_address: string | null
  /** Other emails with the same file. */
  copies: FileEmail[]
  /** Other versions (devis.pdf → devis-v2.pdf). */
  versions: FileVersion[]
}

export interface FileList {
  files: FileItem[]
  total: number
  counts: { all: number; received: number; sent: number; kinds: Partial<Record<FileKind, number>> }
  people: { email: string; name: string | null; files: number }[]
}

export interface FilesProgress {
  looked: number
  total: number
  /** 0: everything. */
  depth: number
}

export interface FileQuery {
  q?: string
  /** A kind, or `documents` (PDFs and documents). */
  kind?: string
  person?: string
  sent?: boolean
  limit?: number
}

export function fetchFiles(query: FileQuery = {}) {
  return $fetch<FileList>('/api/files', { query })
}
export function fetchFilesInMessage(m: { account: number; folder: string; uid: number }) {
  return $fetch<FileItem[]>('/api/files/in-message', { query: m })
}
export function fetchFilesProgress() {
  return $fetch<FilesProgress>('/api/files/progress')
}
export function setFilesDepth(depth: number) {
  return $fetch<FilesProgress>('/api/files/settings', { method: 'PUT', body: { depth } })
}

/** Is the Files app in your dock? People, Mail and Ctrl+K only link to it then. */
export function useFilesEnabled() {
  const { apps } = useApps()
  return computed(() => apps.value.some((a) => a.id === 'files' && a.enabled))
}

// ---- Showing a file ------------------------------------------------------------------

export const FILE_KINDS: { id: string; label: string; kinds: FileKind[] }[] = [
  { id: 'documents', label: 'Documents', kinds: ['pdf', 'doc'] },
  { id: 'sheet', label: 'Spreadsheets', kinds: ['sheet'] },
  { id: 'slides', label: 'Presentations', kinds: ['slides'] },
  { id: 'image', label: 'Images', kinds: ['image'] },
  { id: 'archive', label: 'Archives', kinds: ['archive'] },
  { id: 'other', label: 'Other', kinds: ['other'] },
]

const FILE_LOOK: Record<FileKind, { label: string; tint: string }> = {
  pdf: { label: 'PDF', tint: 'var(--danger)' },
  doc: { label: 'Document', tint: 'var(--c-work)' },
  sheet: { label: 'Spreadsheet', tint: 'var(--c-personal)' },
  slides: { label: 'Presentation', tint: 'var(--c-lyon)' },
  image: { label: 'Image', tint: 'var(--c-hol)' },
  archive: { label: 'Archive', tint: 'var(--muted)' },
  other: { label: 'File', tint: 'var(--faint)' },
}
export const fileTint = (k: FileKind) => FILE_LOOK[k].tint
export const fileKindName = (k: FileKind) => FILE_LOOK[k].label

/** "PDF", "XLSX": the extension, short. */
export function fileExt(name: string) {
  const ext = name.includes('.') ? name.split('.').pop()! : ''
  return ext && ext.length <= 4 ? ext.toUpperCase() : 'FILE'
}

/** 182 KB, 1.2 MB. */
export function fileSize(n: number) {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`
  return `${(n / 1024 / 1024).toFixed(1)} MB`
}

/** Mail's download address for a file. */
export function fileUrl(f: { account: number; folder: string; uid: number; index: number }) {
  return attachmentUrl(f.account, f.folder, f.uid, f.index)
}

/** The Files page with this file open. */
export function filesPageLink(key: string, query: Record<string, string> = {}) {
  return { path: '/files', query: { ...query, file: key } }
}

/** Images the server shows inline (safe types): a thumbnail is possible. */
export function fileHasPreview(f: Pick<FileItem, 'mime'>) {
  return ['image/png', 'image/jpeg', 'image/gif', 'image/webp'].includes(f.mime)
}
