// Havre's service worker: a script the browser keeps and runs on its own,
// even when no Havre page is open. It does two things:
//
// 1. Push notifications: the server sends an encrypted message through the
//    phone's push service; the browser decrypts it and gives it here as JSON
//    `{title, body, url, tag}`; we show it, and open `url` when it's tapped.
// 2. A small "Havre can't be reached" page when opening Havre offline.
//
// It caches nothing else: your mail, events and notes are never stored in
// the browser. (A new version of this file replaces the old one at the next
// visit: `skipWaiting` + `clients.claim`.)

const OFFLINE_CACHE = 'havre-offline-v1'
const OFFLINE_PAGE = '/offline.html'

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches
      .open(OFFLINE_CACHE)
      .then((cache) => cache.addAll([OFFLINE_PAGE, '/icon-192.png']))
      .then(() => self.skipWaiting()),
  )
})

self.addEventListener('activate', (event) => {
  event.waitUntil(
    (async () => {
      // Old versions of the offline cache.
      for (const name of await caches.keys()) if (name !== OFFLINE_CACHE) await caches.delete(name)
      await self.clients.claim()
    })(),
  )
})

// Opening a page: from the network as usual; only if that fails, the offline page.
self.addEventListener('fetch', (event) => {
  if (event.request.mode !== 'navigate') return
  event.respondWith(fetch(event.request).catch(() => caches.match(OFFLINE_PAGE)))
})

self.addEventListener('push', (event) => {
  let note = { title: 'Havre', body: '', url: '/', tag: undefined }
  try {
    note = { ...note, ...event.data.json() }
  } catch {
    /* not JSON: show the default */
  }
  event.waitUntil(
    self.registration.showNotification(note.title, {
      body: note.body,
      tag: note.tag,
      icon: '/icon-192.png',
      data: { url: note.url },
    }),
  )
})

self.addEventListener('notificationclick', (event) => {
  event.notification.close()
  const url = new URL(event.notification.data?.url || '/', self.location.origin).href
  event.waitUntil(
    (async () => {
      // A Havre window already open: bring it to the front and let its router go there.
      const windows = await self.clients.matchAll({ type: 'window', includeUncontrolled: true })
      for (const client of windows) {
        if (new URL(client.url).origin !== self.location.origin) continue
        await client.focus()
        client.postMessage({ type: 'havre-open', url })
        return
      }
      await self.clients.openWindow(url)
    })(),
  )
})

// The push service renewed this browser's subscription: tell Havre the new one.
self.addEventListener('pushsubscriptionchange', (event) => {
  event.waitUntil(
    (async () => {
      const { key } = await (await fetch('/api/push/key', { credentials: 'same-origin' })).json()
      const subscription = await self.registration.pushManager.subscribe({
        userVisibleOnly: true,
        // base64url → bytes (Safari wants bytes).
        applicationServerKey: Uint8Array.from(
          atob(key.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (key.length % 4)) % 4)),
          (c) => c.charCodeAt(0),
        ),
      })
      await fetch('/api/push/subscribe', {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ ...subscription.toJSON(), label: 'Renewed subscription' }),
      })
    })(),
  )
})
