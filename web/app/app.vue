<script setup lang="ts">
// Havre Desktop extras (tray count, notifications, mailto:), once signed in.
const route = useRoute()
const desktop = isDesktop()
// Phones: toasts above the bottom dock, not over it.
const phone = useIsPhone()
const toaster = computed(() => ({
  position: 'bottom-center' as const,
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
