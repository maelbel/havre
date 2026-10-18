// Small display helpers shared by the apps.

/** "09:42" today, "Yesterday", "Mon", or "4 Sep" further back. */
export function shortDate(iso: string | null | undefined, now = new Date()): string {
  if (!iso) return ''
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return ''

  const startOfDay = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime()
  const days = Math.round((startOfDay(now) - startOfDay(d)) / 86_400_000)

  if (days === 0) return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  if (days === 1) return 'Yesterday'
  if (days < 7) return d.toLocaleDateString([], { weekday: 'short' })
  return d.toLocaleDateString([], { day: 'numeric', month: 'short' })
}

/** "Claire Dubois" → "CD", "thomas@x.fr" → "T". */
export function initials(label: string): string {
  const words = label.replace(/@.*/, '').split(/[\s._-]+/).filter(Boolean)
  return words
    .slice(0, 2)
    .map((w) => w[0]!.toUpperCase())
    .join('')
}

/** Stable color per person, picked from the calendar palette. */
export function personColor(key: string): string {
  const palette = ['--c-work', '--c-personal', '--c-lyon', '--c-hol']
  let hash = 0
  for (const ch of key) hash = (hash * 31 + ch.charCodeAt(0)) | 0
  return `var(${palette[Math.abs(hash) % palette.length]})`
}
