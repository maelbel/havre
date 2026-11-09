// The service worker (web/public/sw.js): registered once, in browsers that
// can use it (not in Havre Desktop, which has its own notifications).
//
// While Havre is open and in front, this tells the server every 30 s, so the
// phone doesn't also get a push for what the page shows anyway. And a tapped
// notification, when a Havre window is already open, arrives here as a
// message: the router goes to its page.
export default defineNuxtPlugin(() => {
  if (!pushSupported()) return
  navigator.serviceWorker.register('/sw.js').catch((err) => console.warn('Havre: no service worker', err))

  navigator.serviceWorker.addEventListener('message', (event) => {
    if (event.data?.type !== 'havre-open') return
    const url = new URL(event.data.url)
    if (url.origin === location.origin) navigateTo(url.pathname + url.search)
  })

  const inFront = () => document.visibilityState === 'visible' && document.hasFocus()
  const report = () => {
    if (inFront()) reportInFront().catch(() => {})
  }
  setInterval(report, 30_000)
  window.addEventListener('focus', report)
  document.addEventListener('visibilitychange', report)
})
