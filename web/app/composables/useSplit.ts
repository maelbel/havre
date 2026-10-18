// Resizable panes (SplitGroup, SplitPane, SplitHandle) — the shared parts.
//
// Panes can be resized on wide screens only. Under 1101 px the pages keep
// their own layout (folders hidden, panels over the page, one pane at a time
// on phones), so there the split components render their content untouched.

import type { InjectionKey, Ref } from 'vue'

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

/** What a pane tells its group: how to go back to its default width, fold and unfold. */
export interface SplitPaneControl {
  reset: () => void
  collapse: () => void
  expand: () => void
  collapsed: Ref<boolean>
}

/** What a group shares with its panes and handles. */
export interface SplitContext {
  wide: Ref<boolean>
  panes: Map<string, SplitPaneControl>
  reset: () => void
}
export const SPLIT_KEY: InjectionKey<SplitContext> = Symbol('split')

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
