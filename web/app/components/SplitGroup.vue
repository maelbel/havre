<script setup lang="ts">
import { SplitterGroup } from 'reka-ui'
import { SPLIT_KEY, splitStorage, useWideScreen, type SplitPaneControl } from '~/composables/useSplit'

// A row of panes the user can resize by dragging the handles between them
// (Reka UI's Splitter, styled with Havre's tokens). Wide screens only: on
// smaller ones the slot is rendered as it is, so each page keeps its layout.
//
//   <SplitGroup id="mail">
//     <SplitPane id="folders" :size="216" :min="180" :max="320" collapsible>…</SplitPane>
//     <SplitHandle label="Resize the folders" fold="folders" show-label="Show folders" />
//     <SplitPane id="reader" :min="420">…</SplitPane>
//   </SplitGroup>
//
// The widths are remembered in this browser, per `id`.
const props = defineProps<{ id: string }>()

const wide = useWideScreen()
// shallowReactive: a handle re-renders when the pane beside it registers.
const panes = shallowReactive(new Map<string, SplitPaneControl>())
provide(SPLIT_KEY, {
  wide,
  panes,
  // Double-click on a handle: every pane back to its default width.
  reset: () => panes.forEach((p) => p.reset()),
})
</script>

<template>
  <SplitterGroup
    v-if="wide"
    :id="`split-${props.id}`"
    :auto-save-id="`havre.split.${props.id}`"
    :storage="splitStorage"
    direction="horizontal"
    :keyboard-resize-by="2"
    class="split-group"
  >
    <slot />
  </SplitterGroup>
  <slot v-else />
</template>

<style>
/* Not scoped: these rules reach the pages' own panes inside the split. */
.split-group {
  flex: 1 1 0;
  min-width: 0;
}
</style>
