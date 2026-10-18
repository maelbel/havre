// Small notifications at the bottom of the screen ("Archived", errors…).
export interface Toast {
  id: number
  message: string
  tone: 'info' | 'error'
  action?: { label: string; run: () => void }
}

let nextId = 1

export function useToasts() {
  const toasts = useState<Toast[]>('toasts', () => [])

  function dismiss(id: number) {
    toasts.value = toasts.value.filter((t) => t.id !== id)
  }

  function show(message: string, opts: { tone?: Toast['tone']; action?: Toast['action']; duration?: number } = {}) {
    const toast: Toast = { id: nextId++, message, tone: opts.tone ?? 'info', action: opts.action }
    // Keep at most 3 on screen.
    toasts.value = [...toasts.value.slice(-2), toast]
    setTimeout(() => dismiss(toast.id), opts.duration ?? (toast.tone === 'error' ? 7000 : 4000))
  }

  return { toasts, show, dismiss }
}
