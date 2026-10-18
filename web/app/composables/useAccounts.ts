// Mail accounts (Settings › Accounts). Mirrors `havre_core::store::Account`
// and `havre_core::accounts::AccountView`.
import type { SyncStatus } from './useMail'

export type AccountColor = 'work' | 'personal' | 'lyon' | 'hol'

export interface Account {
  id: number
  name: string
  email: string
  color: AccountColor
  imap_host: string
  imap_port: number
  imap_user: string
  /** How Havre logs in: a password, or OAuth2 with Google or Microsoft. */
  auth: 'password' | OAuthProvider
  /** Your name as recipients see it (may be empty). */
  sender_name: string
  smtp_host: string
  smtp_port: number
  smtp_security: 'tls' | 'starttls'
  /** False for a Microsoft sign-in made before sending existed: reconnect. */
  can_send: boolean
  /** OAuth2 accounts: the sign-in allows the Agenda (reconnect once otherwise). */
  can_calendar: boolean
  /** The sign-in allows Tasks (Microsoft To Do, Google Tasks). */
  can_tasks: boolean
  status: SyncStatus
  /** Unread messages in the inbox. */
  unread: number
}

export interface AccountForm {
  name: string
  email: string
  color: AccountColor
  imap_host: string
  imap_port: number
  imap_user: string
  password: string
  sender_name: string
  smtp_host: string
  smtp_port: number
  smtp_security: 'tls' | 'starttls'
}

export const ACCOUNT_COLORS: { value: AccountColor; label: string }[] = [
  { value: 'work', label: 'Blue' },
  { value: 'personal', label: 'Green' },
  { value: 'lyon', label: 'Amber' },
  { value: 'hol', label: 'Rose' },
]

export function useAccounts() {
  return useFetch<Account[]>('/api/accounts', {
    key: 'accounts',
    lazy: true,
    server: false,
    default: () => [],
  })
}

export function addAccount(form: AccountForm) {
  return $fetch<Account>('/api/accounts', { method: 'POST', body: form })
}

/** Only the fields present are changed. An empty password means "keep it". */
export function updateAccount(id: number, changes: Partial<AccountForm>) {
  return $fetch<Account>(`/api/accounts/${id}`, { method: 'PATCH', body: changes })
}

export function removeAccount(id: number) {
  return $fetch(`/api/accounts/${id}`, { method: 'DELETE' })
}

// ---- OAuth2: Google and Microsoft --------------------------------------------------
// Gmail and Microsoft 365 / Outlook.com want a token instead of your password.
// Like Thunderbird, the IMAP server decides who signs you in, and a window
// opens on their sign-in page. Microsoft also offers a code to type on
// microsoft.com/devicelogin.

export type OAuthProvider = 'google' | 'microsoft'

export const OAUTH_PROVIDERS: Record<OAuthProvider, { name: string; host: string; port: number }> = {
  google: { name: 'Google', host: 'imap.gmail.com', port: 993 },
  microsoft: { name: 'Microsoft', host: 'outlook.office365.com', port: 993 },
}

/** Whose sign-in an IMAP server uses (`imap.gmail.com` → Google). */
export function providerForHost(host: string): OAuthProvider | null {
  const h = host.trim().toLowerCase()
  if (h.endsWith('office365.com') || h.endsWith('outlook.com')) return 'microsoft'
  if (h.endsWith('gmail.com') || h.endsWith('googlemail.com')) return 'google'
  return null
}

/** A sign-in that started: a page to open in a window (`browser`), or
 * (Microsoft) a code to type at microsoft.com/devicelogin (`code`). */
export interface OAuthStart {
  /** Id to poll with. */
  flow: string
  provider: OAuthProvider
  authorize_url?: string
  /** Where the window comes back; must be registered with the provider. */
  redirect_uri?: string
  /** What to type on the Microsoft page, e.g. `F7KQ-2PLM`. */
  user_code?: string
  verification_uri?: string
  /** Seconds between two polls (code mode). */
  interval?: number
}

export type OAuthState =
  | { state: 'pending' }
  | { state: 'connecting' }
  | { state: 'done'; account: { id: number; name: string; email: string } }
  | { state: 'failed'; message: string }

/** The add-account form (OAuth2 chosen), or `{ reconnect: id }`. */
export interface OAuthRequest {
  name?: string
  email?: string
  color?: AccountColor
  imap_host?: string
  imap_port?: number
  imap_user?: string
  reconnect?: number
  sender_name?: string
  provider?: OAuthProvider
  /** `browser` (a sign-in window, default) or `code` (Microsoft only). */
  mode?: 'browser' | 'code'
  /** Where Havre is open: the provider sends the window back there. */
  origin?: string
}

export function startOAuth(body: OAuthRequest) {
  return $fetch<OAuthStart>('/api/accounts/oauth', { method: 'POST', body })
}

export function pollOAuth(flow: string) {
  return $fetch<OAuthState>(`/api/accounts/oauth/${flow}`)
}

/**
 * Who hosts this address: Google (Gmail, Google Workspace), Microsoft
 * (Outlook.com, Microsoft 365), or neither. Known domains answer instantly;
 * for company domains the server looks at their mail servers.
 */
export async function detectOAuthProvider(email: string): Promise<OAuthProvider | null> {
  const known = detectProvider(email)?.oauth
  if (known) return known
  if (!/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(email.trim())) return null
  try {
    const r = await $fetch<{ provider: OAuthProvider | null }>('/api/accounts/detect', {
      query: { email: email.trim() },
    })
    return r.provider
  } catch {
    return null
  }
}

// ---- Provider presets ---------------------------------------------------------
// Most people only know their email and password: guess the rest.

export interface Provider {
  name: string
  host: string
  port: number
  /** Shown under the password field. */
  note?: string
  link?: string
  /** Signs in with OAuth2 by default. */
  oauth?: OAuthProvider
}

const PROVIDERS: Record<string, Provider> = {
  gmail: {
    name: 'Gmail',
    host: 'imap.gmail.com',
    port: 993,
    oauth: 'google',
    note: 'With a password, Gmail needs an app password (Google Account › Security › App passwords).',
    link: 'https://myaccount.google.com/apppasswords',
  },
  outlook: {
    name: 'Outlook',
    host: 'outlook.office365.com',
    port: 993,
    oauth: 'microsoft',
  },
  icloud: {
    name: 'iCloud',
    host: 'imap.mail.me.com',
    port: 993,
    note: 'iCloud needs an app-specific password (appleid.apple.com › Sign-In and Security).',
    link: 'https://appleid.apple.com',
  },
  yahoo: { name: 'Yahoo', host: 'imap.mail.yahoo.com', port: 993, note: 'Yahoo needs an app password.' },
  fastmail: { name: 'Fastmail', host: 'imap.fastmail.com', port: 993, note: 'Fastmail needs an app password.' },
  proton: {
    name: 'Proton Mail',
    host: '127.0.0.1',
    port: 1143,
    note: 'Proton Mail works through Proton Mail Bridge, running on the same machine.',
  },
  orange: { name: 'Orange', host: 'imap.orange.fr', port: 993 },
  free: { name: 'Free', host: 'imap.free.fr', port: 993 },
  sfr: { name: 'SFR', host: 'imap.sfr.fr', port: 993 },
  laposte: { name: 'La Poste', host: 'imap.laposte.net', port: 993 },
  gmx: { name: 'GMX', host: 'imap.gmx.com', port: 993 },
}

const DOMAINS: Record<string, keyof typeof PROVIDERS> = {
  'gmail.com': 'gmail',
  'googlemail.com': 'gmail',
  'outlook.com': 'outlook',
  'outlook.fr': 'outlook',
  'hotmail.com': 'outlook',
  'hotmail.fr': 'outlook',
  'live.com': 'outlook',
  'live.fr': 'outlook',
  'msn.com': 'outlook',
  'icloud.com': 'icloud',
  'me.com': 'icloud',
  'mac.com': 'icloud',
  'yahoo.com': 'yahoo',
  'yahoo.fr': 'yahoo',
  'fastmail.com': 'fastmail',
  'proton.me': 'proton',
  'protonmail.com': 'proton',
  'orange.fr': 'orange',
  'wanadoo.fr': 'orange',
  'free.fr': 'free',
  'sfr.fr': 'sfr',
  'neuf.fr': 'sfr',
  'laposte.net': 'laposte',
  'gmx.com': 'gmx',
  'gmx.fr': 'gmx',
}

/**
 * The provider for an email address, or `null` for a custom domain (then
 * `imap.<domain>` is a reasonable first guess, shown for the user to check).
 */
export function detectProvider(email: string): Provider | null {
  const domain = email.split('@')[1]?.trim().toLowerCase()
  if (!domain) return null
  const key = DOMAINS[domain]
  return key ? PROVIDERS[key]! : null
}

/** The usual outgoing server for an incoming one (same rules as the server). */
export function smtpDefaults(imapHost: string): { host: string; port: number; security: 'tls' | 'starttls' } {
  const h = imapHost.trim().toLowerCase()
  const known: Record<string, { host: string; port: number; security: 'tls' | 'starttls' }> = {
    'outlook.office365.com': { host: 'smtp.office365.com', port: 587, security: 'starttls' },
    'imap.gmail.com': { host: 'smtp.gmail.com', port: 465, security: 'tls' },
    'imap.mail.me.com': { host: 'smtp.mail.me.com', port: 587, security: 'starttls' },
    'imap.mail.yahoo.com': { host: 'smtp.mail.yahoo.com', port: 465, security: 'tls' },
    'imap.gmx.com': { host: 'mail.gmx.com', port: 465, security: 'tls' },
    '127.0.0.1': { host: '127.0.0.1', port: 1025, security: 'starttls' },
  }
  if (known[h]) return known[h]
  return { host: h.startsWith('imap.') ? `smtp.${h.slice(5)}` : h, port: 465, security: 'tls' }
}

export function guessHost(email: string): string {
  const domain = email.split('@')[1]?.trim().toLowerCase()
  return domain ? `imap.${domain}` : ''
}
