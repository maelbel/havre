<script setup lang="ts">
useHead({ title: 'Backups · Settings · Havre' })
// The admin's only: anyone else goes back to Settings.
if (!useIsAdmin().value) await navigateTo('/settings', { replace: true })
</script>

<template>
  <SettingsFrame>
    <BackupSettings />
  </SettingsFrame>
</template>
