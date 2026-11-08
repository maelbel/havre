<script setup lang="ts">
// Havre Desktop extras (tray count, notifications, mailto:), once signed in.
const route = useRoute()
const desktop = isDesktop()
// Toasts in the bottom right corner, out of the way of the reader and the
// lists; on phones (where they take the width) above the bottom dock.
const phone = useIsPhone()
const toaster = computed(() => ({
  position: 'bottom-right' as const,
  max: 3,
  ui: phone.value ? { viewport: 'bottom-[calc(76px+env(safe-area-inset-bottom))]' } : undefined,
}))
</script>

<template>
  <!-- UApp: Nuxt UI's root (toasts, tooltips, dialogs opened from code). -->
  <UApp :toaster="toaster">
    <NuxtLayout>
      <NuxtPage />
    </NuxtLayout>
    <DesktopBridge v-if="desktop && !isLoginPath(route.path)" />
  </UApp>
</template>
