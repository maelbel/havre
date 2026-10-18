<script setup lang="ts">
import { SplitterPanel } from 'reka-ui'
import { SPLIT_KEY } from '~/composables/useSplit'

// One pane of a SplitGroup. Its content (the page's own <aside>, <section>…)
// fills it; widths are in pixels. Without `size` the pane takes what's left.
// Outside a wide screen it renders its content as it is.
const props = withDefaults(
  defineProps<{
    id: string
    /** Default width (px). */
    size?: number
    min?: number
    max?: number
    /** Can be folded away (dragged past `min`, or Enter on its handle). */
    collapsible?: boolean
    /** Needed when the pane appears and disappears (v-if), so the order stays right. */
    order?: number
  }>(),
  { size: undefined, min: 200, max: 4000, collapsible: false, order: undefined },
)

// A folded pane keeps a thin strip with the tab that brings it back.
const FOLDED_WIDTH = 32

const ctx = inject(SPLIT_KEY, null)
const wide = computed(() => !!ctx?.wide.value)

// The Reka panel, to resize, fold and unfold it from the handles.
const panel = ref<InstanceType<typeof SplitterPanel> | null>(null)
const collapsed = computed(() => !!panel.value?.isCollapsed)
const control = {
  reset: () => {
    if (props.size != null) {
      panel.value?.expand()
      panel.value?.resize(props.size)
    }
  },
  collapse: () => panel.value?.collapse(),
  expand: () => panel.value?.expand(),
  collapsed,
}
ctx?.panes.set(props.id, control)
onBeforeUnmount(() => ctx?.panes.delete(props.id))
</script>

<template>
  <SplitterPanel
    v-if="wide"
    :id="`pane-${props.id}`"
    ref="panel"
    :order="props.order"
    :default-size="props.size"
    :min-size="props.min"
    :max-size="props.max"
    size-unit="px"
    :collapsible="props.collapsible"
    :collapsed-size="FOLDED_WIDTH"
    class="split-pane"
    :class="{ folded: collapsed }"
  >
    <slot />
  </SplitterPanel>
  <slot v-else />
</template>

<style>
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
</style>
