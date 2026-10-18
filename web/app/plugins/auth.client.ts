// Signed out while using Havre (session expired, signed out from another
// browser, password changed)? Any API answer `401` sends you to the
// sign-in screen. This wraps the global `$fetch`, which `useFetch` uses too.
export default defineNuxtPlugin(() => {
  const original = globalThis.$fetch
  globalThis.$fetch = original.create({
    async onResponseError({ request, response }) {
      if (response.status !== 401 || String(request).includes('/api/auth/')) return
      const status = useAuthStatus()
      if (status.value) status.value = { ...status.value, signed_in: false }
      const route = useRoute()
      if (route.path !== '/login') await navigateTo({ path: '/login', query: { next: route.fullPath } })
    },
  }) as typeof globalThis.$fetch
})
