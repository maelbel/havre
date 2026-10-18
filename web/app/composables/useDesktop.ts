// Havre Desktop (the `desktop/` app): when these pages run inside it, a few
// extras — notifications, the unread count on the tray icon, sign-in pages
// in the real browser, mailto: links. In a browser, none of this happens.
//
// The app injects `window.__TAURI__` (its `withGlobalTauri` setting), so no
// JavaScript package is needed; it only lets this server call four commands
// (see `server_capability` in desktop/src-tauri/src/lib.rs).

interface TauriGlobal {
  core: { invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> }
}

function tauri(): TauriGlobal | null {
  if (!import.meta.client) return null
  return (window as unknown as { __TAURI__?: TauriGlobal }).__TAURI__ ?? null
}

/** Running inside Havre Desktop? */
export function isDesktop() {
  return tauri() !== null
}

export function desktopInvoke<T = void>(command: string, args?: Record<string, unknown>): Promise<T> {
  const t = tauri()
  return t ? t.core.invoke<T>(command, args) : Promise.reject(new Error('Not in Havre Desktop'))
}

/** Opens a page in the browser: the system's in the app, a new tab otherwise. */
export function openExternal(url: string) {
  if (isDesktop()) return desktopInvoke('open_external', { url })
  window.open(url, '_blank', 'noopener')
  return Promise.resolve()
}

export interface MailtoParts {
  to: string
  cc: string
  bcc: string
  subject: string
  body: string
}

/**
 * `mailto:a@x.fr,b@x.fr?cc=c@x.fr&subject=Hi%20there&body=…` (RFC 6068) →
 * its parts. Unknown headers are ignored; addresses stay comma-separated.
 */
export function parseMailto(link: string): MailtoParts {
  const parts: MailtoParts = { to: '', cc: '', bcc: '', subject: '', body: '' }
  if (!/^mailto:/i.test(link)) return parts
  const rest = link.slice(7)
  const q = rest.indexOf('?')
  const decode = (s: string) => {
    try {
      return decodeURIComponent(s.replace(/\+/g, '%2B'))
    } catch {
      return s
    }
  }
  const to = [decode(q < 0 ? rest : rest.slice(0, q))]
  if (q >= 0) {
    for (const pair of rest.slice(q + 1).split('&')) {
      const eq = pair.indexOf('=')
      const key = decode(eq < 0 ? pair : pair.slice(0, eq)).toLowerCase()
      const value = eq < 0 ? '' : decode(pair.slice(eq + 1))
      if (key === 'to') to.push(value)
      else if (key === 'cc' || key === 'bcc' || key === 'subject' || key === 'body') parts[key] = value
    }
  }
  parts.to = to
    .flatMap((t) => t.split(','))
    .map((t) => t.trim())
    .filter(Boolean)
    .join(', ')
  // Bodies use CRLF in links; the editor wants plain newlines.
  parts.body = parts.body.replace(/\r\n/g, '\n')
  return parts
}
