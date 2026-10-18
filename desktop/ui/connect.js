// "Connect to your Havre": sends the address to the app's `connect`
// command (src-tauri/src/lib.rs), which checks it, remembers it and opens it.
const { invoke } = window.__TAURI__.core

const form = document.getElementById('form')
const input = document.getElementById('address')
const error = document.getElementById('error')
const button = document.getElementById('connect')

// Changing server: start from the current one.
invoke('desktop_info')
  .then((info) => {
    if (info.server && !input.value) input.value = info.server
  })
  .catch(() => {})
input.focus()

form.addEventListener('submit', async (event) => {
  event.preventDefault()
  error.hidden = true
  button.disabled = true
  button.textContent = 'Connecting…'
  try {
    await invoke('connect', { address: input.value })
    // The app opens the server in this window.
  } catch (message) {
    error.textContent = String(message)
    error.hidden = false
    button.disabled = false
    button.textContent = 'Connect'
    input.focus()
  }
})
