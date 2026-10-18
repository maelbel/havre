<script setup lang="ts">
import { SplitterResizeHandle } from 'reka-ui'
import { SPLIT_KEY } from '~/composables/useSplit'

// The handle between two panes of a SplitGroup: drag it, or Tab to it and
// use ← →, Home, End (Reka does that). Double-click: default widths again.
// With `fold`, Enter folds that pane to a thin strip, and its tab brings it back.
const props = defineProps<{
  /** Accessible name, e.g. "Resize the message list". */
  label: string
  /** The id of the pane this handle can fold (the one beside it). */
  fold?: string
  /** The tab's name when that pane is folded, e.g. "Show folders". */
  showLabel?: string
}>()

const ctx = inject(SPLIT_KEY, null)
const wide = computed(() => !!ctx?.wide.value)
const folded = computed(() => (props.fold ? !!ctx?.panes.get(props.fold)?.collapsed.value : false))

function toggleFold() {
  const pane = props.fold ? ctx?.panes.get(props.fold) : undefined
  if (!pane) return
  if (pane.collapsed.value) pane.expand()
  else pane.collapse()
}
</script>

<template>
  <SplitterResizeHandle
    v-if="wide"
    class="split-handle"
    :aria-label="props.label"
    @dblclick="ctx?.reset()"
    @keydown.enter.prevent="toggleFold"
  >
    <!-- mousedown.stop: Reka listens on <body> to start a drag; a click on the tab mustn't. -->
    <button
      v-if="folded"
      type="button"
      class="split-show"
      :aria-label="props.showLabel ?? 'Show the pane'"
      :title="props.showLabel"
      @mousedown.stop
      @click.stop="toggleFold"
    >
      <HIcon name="chevron" :size="14" />
    </button>
  </SplitterResizeHandle>
</template>

<style>
/* Takes no room (10 px wide, pulled back 5 px on each side): the panes'
   borders stay the visible line. Those 10 px are where Reka lets you grab it,
   and being a real box there is what lets it get the double-click. */
.split-handle {
  position: relative;
  flex: 0 0 10px;
  margin: 0 -5px;
  z-index: 2;
}
/* Not the app's focus ring around the 10 px box: the line below shows focus. */
.split-handle:focus-visible {
  outline: none;
}
/* Hover, drag, keyboard focus: a 2 px accent line over the border. */
.split-handle::after {
  content: '';
  position: absolute;
  top: 0;
  bottom: 0;
  left: 4px;
  width: 2px;
  background: transparent;
  transition: background 0.12s;
}
.split-handle[data-resize-handle-state='hover']::after,
.split-handle[data-resize-handle-state='drag']::after,
.split-handle:focus-visible::after {
  background: var(--accent);
}
/* Keyboard focus: the line itself, a bit thicker (a ring around a 2 px line
   reads as three lines). */
.split-handle:focus-visible::after {
  left: 3px;
  width: 4px;
  border-radius: 2px;
}
.split-show {
  position: absolute;
  top: 16px;
  left: -26px;
  width: 22px;
  height: 30px;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 1px solid var(--line2);
  border-radius: 8px;
  background: var(--raise);
  color: var(--muted);
  cursor: pointer;
}
.split-show:hover {
  color: var(--text);
}
.split-show svg {
  transform: rotate(-90deg);
}
@media (prefers-reduced-motion: reduce) {
  .split-handle::after {
    transition: none;
  }
}
</style>
