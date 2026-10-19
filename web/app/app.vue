<script setup lang="ts">
// Havre Desktop extras (tray count, notifications, mailto:), once signed in.
const route = useRoute()
const desktop = isDesktop()
</script>

<template>
  <!-- UApp: Nuxt UI's root (toasts, tooltips, dialogs opened from code). -->
  <UApp :toaster="{ position: 'bottom-center', max: 3 }">
    <NuxtLayout>
      <NuxtPage />
    </NuxtLayout>
    <DesktopBridge v-if="desktop && !isLoginPath(route.path)" />
  </UApp>
</template>
