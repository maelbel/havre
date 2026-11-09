// Push notifications on this device (Settings › Notifications) and the
// service worker (web/public/sw.js). Mirrors crates/server/src/push.rs.
//
// Turning them on: the browser asks permission, then `pushManager.subscribe`
// returns a subscription (an address at the phone's push service and two
// keys) that we give to the server. Havre Desktop has its own notifications
// and doesn't use this.

export interface PushDevice {
  id: number
  label: string
  this_device: boolean
  created_at: number
  last_ok_at: number | null
  last_error: string | null
}

export interface PushSettings {
  new_mail: boolean
  mail_details: boolean
  reminders: boolean
}

/** Can this browser get push notifications? (Not Havre Desktop, not plain HTTP.) */
export function pushSupported() {
  return (
    import.meta.client &&
    !isDesktop() &&
    window.isSecureContext &&
    'serviceWorker' in navigator &&
    'PushManager' in window &&
    'Notification' in window
  )
}

/** On an iPhone or iPad, push needs Havre added to the Home Screen first. */
export function needsHomeScreen() {
  if (!import.meta.client) return false
  const ios = /iphone|ipad|ipod/i.test(navigator.userAgent)
  const standalone = window.matchMedia('(display-mode: standalone)').matches
  return ios && !standalone
}

/** base64url → bytes: what `subscribe()` takes as the server's key (Safari wants bytes, not text). */
function keyBytes(key: string): Uint8Array<ArrayBuffer> {
  const text = atob(key.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (key.length % 4)) % 4))
  return Uint8Array.from(text, (c) => c.charCodeAt(0))
}

async function registration() {
  return navigator.serviceWorker.ready
}

/** This browser's subscription, if notifications are on here. */
export async function currentSubscription(): Promise<PushSubscription | null> {
  if (!pushSupported()) return null
  return (await registration()).pushManager.getSubscription()
}

/** Asks the browser, subscribes, and tells the server. */
export async function enablePush(): Promise<void> {
  const permission = await Notification.requestPermission()
  if (permission !== 'granted') {
    throw new Error(
      permission === 'denied'
        ? 'Notifications are blocked for Havre in this browser. Allow them in the site settings, then try again.'
        : 'Notifications weren’t allowed.',
    )
  }
  const { key } = await $fetch<{ key: string }>('/api/push/key')
  const reg = await registration()
  let subscription: PushSubscription
  try {
    subscription = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: keyBytes(key) })
  } catch (err) {
    // The browser couldn't register with its push service (Google's for Chrome, Mozilla's for Firefox…).
    const what = err instanceof Error ? err.message : String(err)
    throw new Error(
      /push service|registration failed/i.test(what)
        ? `This browser couldn't reach its push service (${what.replace(/^Registration failed - /i, '')}). Check the connection and try again; in a private window, push doesn't work.`
        : what,
    )
  }
  await $fetch('/api/push/subscribe', {
    method: 'POST',
    body: { ...subscription.toJSON(), label: describeBrowser(navigator.userAgent) },
  })
}

/** Unsubscribes this browser and tells the server. */
export async function disablePush(): Promise<void> {
  const subscription = await currentSubscription()
  if (!subscription) return
  await $fetch('/api/push/subscribe', { method: 'DELETE', body: { endpoint: subscription.endpoint } }).catch(() => {})
  await subscription.unsubscribe()
}

export async function fetchPushDevices() {
  const endpoint = (await currentSubscription())?.endpoint
  // In a header, not the address: the endpoint shouldn't end up in logs.
  return $fetch<PushDevice[]>('/api/push/devices', { headers: endpoint ? { 'X-Push-Endpoint': endpoint } : {} })
}
export function removePushDevice(id: number) {
  return $fetch(`/api/push/devices/${id}`, { method: 'DELETE' })
}
export function fetchPushSettings() {
  return $fetch<PushSettings>('/api/push/settings')
}
export function savePushSettings(change: Partial<PushSettings>) {
  return $fetch<PushSettings>('/api/push/settings', { method: 'PATCH', body: change })
}
export async function sendTestPush() {
  const subscription = await currentSubscription()
  if (!subscription) throw new Error('Turn notifications on for this device first.')
  await $fetch('/api/push/test', { method: 'POST', body: { endpoint: subscription.endpoint } })
}

/** Tells the server Havre is open and in front here, so it pushes nothing to this device. */
export async function reportInFront() {
  const subscription = await currentSubscription()
  if (subscription) await $fetch('/api/push/seen', { method: 'PUT', body: { endpoint: subscription.endpoint } })
}
