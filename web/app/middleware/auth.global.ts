// Before every page: no password yet → first-run screen; signed out →
// sign-in screen (then back to where you were going).
export default defineNuxtRouteMiddleware(async (to) => {
  const status = useAuthStatus()
  if (!status.value) {
    try {
      await refreshAuthStatus()
    } catch {
      return // server unreachable: let the page show its own error
    }
  }
  const s = status.value!
  // The server answers `/login` with its folder, `/login/`: both are the sign-in page.
  const onLogin = isLoginPath(to.path)
  if (!s.enabled) return onLogin ? navigateTo('/') : undefined
  const free = s.signed_in && !s.setup_needed
  if (onLogin) {
    if (free) return navigateTo(safeNext(to.query.next))
    return
  }
  if (!free) return navigateTo({ path: '/login', query: to.fullPath !== '/' ? { next: to.fullPath } : {} })
})
