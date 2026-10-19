// Small notifications at the bottom of the screen ("Archived", errors…),
// drawn by Nuxt UI's toaster (in `UApp`, app.vue). Havre's pages keep this
// one short call: `useToasts().show('Archived', { action: { label: 'Undo', run } })`.
export interface ToastOptions {
  tone?: 'info' | 'error'
  action?: { label: string; run: () => void }
  /** Milliseconds on screen (errors stay a little longer). */
  duration?: number
}

export function useToasts() {
  const toast = useToast()

  function show(message: string, opts: ToastOptions = {}) {
    const error = opts.tone === 'error'
    toast.add({
      title: message,
      color: error ? 'error' : 'neutral',
      icon: error ? 'i-havre-alert' : undefined,
      duration: opts.duration ?? (error ? 7000 : 4000),
      actions: opts.action
        ? [{ label: opts.action.label, color: 'primary', variant: 'link', onClick: opts.action.run }]
        : undefined,
    })
  }

  return { show, dismiss: toast.remove }
}
