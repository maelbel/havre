// Is Havre on a phone-sized screen (720 px wide or less)?
//
// Most of the phone layout is plain CSS (`@media (max-width: 720px)`), which
// is the better tool: nothing to keep in sync. This is for the few places
// where the *markup* differs on a phone (the Settings menu, the full-screen
// search, the dock's "More"), so a component can pick what to render.
//
// One `matchMedia` listener is shared by every caller (`useState`), and it
// follows rotations and window resizes.
export const PHONE_QUERY = '(max-width: 720px)'

export function useIsPhone() {
  const phone = useState('is-phone', () => false)
  if (import.meta.client && !listening) {
    listening = true
    const query = window.matchMedia(PHONE_QUERY)
    phone.value = query.matches
    query.addEventListener('change', (e) => {
      phone.value = e.matches
    })
  }
  return phone
}

// Module-level: the listener is added once for the whole app.
let listening = false
