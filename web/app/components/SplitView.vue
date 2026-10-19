<script setup lang="ts">
import type { SplitterPanel } from 'reka-ui'
import { splitStorage, useWideScreen, type SplitPaneSpec } from '~/composables/useSplit'

// Panes the user can resize by dragging the handles between them: Nuxt UI's
// USplitter, styled with Havre's tokens. Wide screens only — on smaller ones
// each pane's slot is rendered as it is, so the page keeps its own layout.
//
//   <SplitView id="mail" :panes="[
//     { id: 'folders', size: 216, min: 180, max: 320, collapsible: true,
//       handle: { label: 'Resize the folders', fold: true, showLabel: 'Show folders' } },
//     { id: 'list', size: 380, min: 300, max: 640, handle: { label: 'Resize the message list' } },
//     { id: 'reader', min: 420 },
//   ]">
//     <template #folders>…</template>  <template #list>…</template>  <template #reader>…</template>
//   </SplitView>
//
// A pane's `handle` is the one on its right. Widths are pixels, remembered in
// this browser per `id`. Double-click a handle: default widths again. Enter on
// a handle with `fold`: that pane folds to a thin strip (its tab brings it back).
const props = defineProps<{ id: string; panes: SplitPaneSpec[] }>()

// A folded pane keeps a thin strip with the tab that brings it back.
const FOLDED_WIDTH = 32

const wide = useWideScreen()
// Folded panes, by id. Read from the panels after each layout change and
// kept here, rather than read from the panels while rendering: USplitter
// collects them again on every render, which would loop.
const folded = reactive(new Set<string>())

// What USplitter wants for each pane. `order` keeps panes that come and go
// (`v-if` on the page → left out of `panes`) in the right place.
const items = computed(() =>
  props.panes.map((p, i) => ({
    id: `pane-${p.id}`,
    slot: p.id,
    order: p.order ?? i + 1,
    defaultSize: p.size,
    minSize: p.min ?? 200,
    maxSize: p.max ?? 4000,
    sizeUnit: 'px' as const,
    collapsible: p.collapsible ?? false,
    collapsedSize: FOLDED_WIDTH,
    class: ['split-pane', { folded: folded.has(p.id) }],
  })),
)

// The Reka panels USplitter made, to resize, fold and unfold them.
const splitter = ref<{ panelsRef: InstanceType<typeof SplitterPanel>[] } | null>(null)
const panel = (i: number) => splitter.value?.panelsRef[i]
const isFolded = (i: number) => folded.has(props.panes[i]?.id ?? '')
function readFolded() {
  folded.clear()
  props.panes.forEach((p, i) => panel(i)?.isCollapsed && folded.add(p.id))
}

function reset() {
  props.panes.forEach((p, i) => {
    if (p.size == null) return
    panel(i)?.expand()
    panel(i)?.resize(p.size)
  })
}

function toggleFold(i: number) {
  const pane = props.panes[i]
  if (!pane?.handle?.fold) return
  if (isFolded(i)) panel(i)?.expand()
  else panel(i)?.collapse()
}

// USplitter doesn't take attributes for its handles: name each one (it's a
// `separator`, and screen readers and tests find it by that name) once
// they're drawn, and catch double-click and Enter on the root instead.
const root = ref<HTMLElement | null>(null)
const handles = () => [...(root.value?.querySelectorAll<HTMLElement>(':scope > [data-slot=root] > [data-slot=handle]') ?? [])]
function nameHandles() {
  handles().forEach((el, i) => {
    const label = props.panes[i]?.handle?.label
    if (label) el.setAttribute('aria-label', label)
  })
}
function drawn() {
  nameHandles()
  readFolded()
}
watch([wide, () => props.panes.map((p) => p.id).join()], () => nextTick(drawn), { flush: 'post' })
onMounted(() => nextTick(drawn))

const handleIndex = (target: EventTarget | null) =>
  handles().findIndex((el) => el === (target as HTMLElement | null)?.closest('[data-slot=handle]'))
function onDblclick(e: MouseEvent) {
  if (handleIndex(e.target) >= 0) reset()
}
function onKeydown(e: KeyboardEvent) {
  if (e.key !== 'Enter') return
  const i = handleIndex(e.target)
  if (i < 0 || !props.panes[i]?.handle?.fold) return
  e.preventDefault()
  toggleFold(i)
}
</script>

<template>
  <div v-if="wide" ref="root" class="split-view" @dblclick="onDblclick" @keydown="onKeydown">
    <USplitter
      :id="`split-${props.id}`"
      ref="splitter"
      :items="items"
      :auto-save-id="`havre.split.${props.id}`"
      :storage="splitStorage"
      :keyboard-resize-by="2"
      :ui="{ root: 'split-group', handle: 'split-handle' }"
      @layout="nextTick(readFolded)"
    >
      <template v-for="p in props.panes" :key="p.id" #[p.id]>
        <slot :name="p.id" />
      </template>
      <template #resize-handle="{ index }">
        <!-- mousedown.stop: Reka listens on <body> to start a drag; a click on the tab mustn't. -->
        <button
          v-if="props.panes[index]?.handle?.fold && isFolded(index)"
          type="button"
          class="split-show"
          :aria-label="props.panes[index]?.handle?.showLabel ?? 'Show the pane'"
          :title="props.panes[index]?.handle?.showLabel"
          @mousedown.stop
          @click.stop="toggleFold(index)"
        >
          <HIcon name="chevron" :size="14" />
        </button>
      </template>
    </USplitter>
  </div>
  <template v-else>
    <template v-for="p in props.panes" :key="p.id">
      <slot :name="p.id" />
    </template>
  </template>
</template>

<style>
/* Not scoped: these rules reach the pages' own panes inside the split. */
.split-view {
  display: flex;
  flex: 1 1 0;
  min-width: 0;
}
.split-group {
  flex: 1 1 0;
  min-width: 0;
}

/* The page's pane fills the split pane, whatever width its own CSS asked for:
   here the handles decide. Only inside a split, so smaller screens and phones
   keep the page's own rules. */
.split-pane {
  display: flex;
  min-width: 0;
}
.split-pane > * {
  flex: 1 1 auto !important;
  width: auto !important;
  min-width: 0 !important;
  max-width: none !important;
}
.split-pane.folded {
  background: var(--bg);
  border-right: 1px solid var(--line);
}
.split-pane.folded > * {
  visibility: hidden;
}

/* The handle takes no room (10 px wide, pulled back 5 px on each side): the
   panes' borders stay the visible line. Those 10 px are where you grab it,
   and being a real box there is what lets it get the double-click. */
.split-handle {
  position: relative;
  flex: 0 0 10px;
  width: 10px;
  margin: 0 -5px;
  z-index: 2;
}
/* Not a ring around the 10 px box: the line below shows focus. */
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
