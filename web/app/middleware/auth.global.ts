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
  if (!s.enabled) return to.path === '/login' ? navigateTo('/') : undefined
  const free = s.signed_in && !s.setup_needed
  if (to.path === '/login') {
    if (free) return navigateTo(typeof to.query.next === 'string' && /^\/(?![/\\])/.test(to.query.next) ? to.query.next : '/')
    return
  }
  if (!free) return navigateTo({ path: '/login', query: to.fullPath !== '/' ? { next: to.fullPath } : {} })
})
