<script setup lang="ts">
// An icon from Havre's set (`web/icons.ts`), drawn inline so the size and
// stroke can change per use.
import { ICON_PATHS as paths, type IconName } from '~~/icons'

export type { IconName }

withDefaults(defineProps<{ name: IconName; size?: number; strokeWidth?: number }>(), {
  size: 20,
  strokeWidth: 1.6,
})
</script>

<template>
  <svg
    aria-hidden="true"
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    :stroke-width="strokeWidth"
    stroke-linecap="round"
    stroke-linejoin="round"
    v-html="paths[name]"
  />
</template>
