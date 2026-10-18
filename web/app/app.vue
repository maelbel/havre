<script setup lang="ts">
// Havre Desktop extras (tray count, notifications, mailto:), once signed in.
const route = useRoute()
const desktop = isDesktop()
</script>

<template>
  <NuxtLayout>
    <NuxtPage />
  </NuxtLayout>
  <DesktopBridge v-if="desktop && route.path !== '/login'" />
</template>
