<script setup lang="ts">
// The top of a screen on a phone: an optional Back, a big title, and
// buttons on the right (Search by default, since search left the dock).
//
// It renders nothing on wider screens, so a page can just drop it in: the
// desktop keeps its own header. Slots: `actions` (more buttons, before
// Search) and the default slot (a line under the title, e.g. tabs).
const props = withDefaults(
  defineProps<{
    title?: string
    /** Where Back goes; no Back button without it. */
    back?: string | Record<string, unknown>
    backLabel?: string
    /** Show the Search button (opens the full-screen search). */
    search?: boolean
    /** A small line above the title (e.g. today's date). */
    eyebrow?: string
  }>(),
  { title: undefined, back: undefined, backLabel: 'Back', search: true, eyebrow: undefined },
)
const phone = useIsPhone()
const palette = usePalette()
</script>

<template>
  <header v-if="phone" class="phone-head">
    <div class="bar">
      <UButton
        v-if="props.back"
        :to="props.back"
        color="neutral"
        variant="ghost"
        size="lg"
        square
        icon="i-havre-back"
        :aria-label="props.backLabel"
        class="-ml-2"
      />
      <span v-if="props.eyebrow" class="mono eyebrow-line">{{ props.eyebrow }}</span>
      <span class="grow" />
      <slot name="actions" />
      <UButton
        v-if="props.search"
        color="neutral"
        variant="ghost"
        size="lg"
        square
        icon="i-havre-search"
        aria-label="Search"
        @click="palette = true"
      />
    </div>
    <h1 v-if="props.title" class="serif title">{{ props.title }}</h1>
    <slot />
  </header>
</template>

<style scoped>
.phone-head {
  flex: 0 0 auto;
  /* Clear of the notch / status bar when the page fills the screen. */
  padding: calc(4px + env(safe-area-inset-top)) 12px 0 16px;
}
.bar {
  display: flex;
  align-items: center;
  gap: 2px;
  min-height: 44px;
}
.grow {
  flex: 1;
}
.eyebrow-line {
  font-size: 12px;
  color: var(--faint);
}
.title {
  margin: 0 0 10px;
  font-size: 30px;
  line-height: 1.15;
  font-weight: 400;
}
</style>
