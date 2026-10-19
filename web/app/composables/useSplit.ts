// Resizable panes (SplitView, on Nuxt UI's USplitter) — the shared parts.
//
// Panes can be resized on wide screens only. Under 1101 px the pages keep
// their own layout (folders hidden, panels over the page, one pane at a time
// on phones), so there SplitView renders the panes' content untouched.

import type { Ref } from 'vue'

/** Where resizable panes start: the pages' own "smaller screen" rules end at 1100 px. */
export const SPLIT_MIN_WIDTH = 1101

// One media query for the whole app, shared by every split (no listener per pane).
let wide: Ref<boolean> | null = null
export function useWideScreen(): Ref<boolean> {
  if (!wide) {
    const query = window.matchMedia(`(min-width: ${SPLIT_MIN_WIDTH}px)`)
    const state = ref(query.matches)
    query.addEventListener('change', (e) => (state.value = e.matches))
    wide = state
  }
  return wide
}

/** One pane of a SplitView (see SplitView.vue). Widths in pixels. */
export interface SplitPaneSpec {
  /** Also the name of the slot holding the pane. */
  id: string
  /** Default width; without it the pane takes what's left. */
  size?: number
  min?: number
  max?: number
  /** Can be folded away (dragged past `min`, or Enter on its handle). */
  collapsible?: boolean
  /** Needed when panes come and go, so the order stays right (defaults to the position). */
  order?: number
  /** The handle on this pane's right. */
  handle?: {
    /** Accessible name, e.g. "Resize the message list". */
    label: string
    /** Enter on the handle folds this pane. */
    fold?: boolean
    /** The tab's name when it's folded, e.g. "Show folders". */
    showLabel?: string
  }
}

/**
 * Where the widths are remembered: this browser's localStorage, which can
 * throw (private windows, blocked storage). Then the panes simply start at
 * their default widths each time.
 */
export const splitStorage = {
  getItem(name: string): string | null {
    try {
      return window.localStorage.getItem(name)
    } catch {
      return null
    }
  },
  setItem(name: string, value: string) {
    try {
      window.localStorage.setItem(name, value)
    } catch {
      // Not remembered: fine.
    }
  },
}
