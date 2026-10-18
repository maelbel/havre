// The login for Havre itself. Mirrors `/api/auth/*` (crates/server/src/auth.rs).
//
// The session lives in an HttpOnly cookie that JavaScript can't read: the
// UI only asks the server "am I signed in?" (`/api/auth/status`).

export interface AuthStatus {
  /** `false` in demo mode: no login. */
  enabled: boolean
  /** First run: no password chosen yet. */
  setup_needed: boolean
  signed_in: boolean
}

export interface SessionInfo {
  created_at: number
  last_seen: number
  user_agent: string
  current: boolean
}

/** Shared across pages; `null` until first asked. */
export function useAuthStatus() {
  return useState<AuthStatus | null>('auth-status', () => null)
}

export async function refreshAuthStatus() {
  const status = useAuthStatus()
  status.value = await $fetch<AuthStatus>('/api/auth/status')
  return status.value
}

export async function setupPassword(password: string) {
  await $fetch('/api/auth/setup', { method: 'POST', body: { password } })
  await refreshAuthStatus()
}

export async function signIn(password: string) {
  await $fetch('/api/auth/login', { method: 'POST', body: { password } })
  await refreshAuthStatus()
}

export async function signOut() {
  await $fetch('/api/auth/logout', { method: 'POST' })
  await refreshAuthStatus()
  await navigateTo('/login')
}

export function changePassword(current: string, next: string) {
  return $fetch('/api/auth/password', { method: 'POST', body: { current, new: next } })
}

export function fetchSessions() {
  return $fetch<SessionInfo[]>('/api/auth/sessions')
}

export function signOutOthers() {
  return $fetch('/api/auth/sessions/others', { method: 'DELETE' })
}

/** "Firefox on Windows", from a user agent string. Good enough for a list. */
export function describeBrowser(userAgent: string): string {
  const ua = userAgent.toLowerCase()
  const browser = ua.includes('edg/')
    ? 'Edge'
    : ua.includes('firefox/')
      ? 'Firefox'
      : ua.includes('chrome/') || ua.includes('chromium/')
        ? 'Chrome'
        : ua.includes('safari/')
          ? 'Safari'
          : ua.includes('curl/')
            ? 'curl'
            : 'Browser'
  const os = ua.includes('windows')
    ? 'Windows'
    : ua.includes('iphone') || ua.includes('ipad')
      ? 'iOS'
      : ua.includes('android')
        ? 'Android'
        : ua.includes('mac os')
          ? 'macOS'
          : ua.includes('linux')
            ? 'Linux'
            : ''
  return os ? `${browser} on ${os}` : browser
}
