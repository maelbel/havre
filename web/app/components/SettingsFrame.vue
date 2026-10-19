<script setup lang="ts">
// Settings, as in the design: a list of sections on the left, one section
// at a time on the right.
const auth = useAuthStatus()
const sections = computed(() => [
  { to: '/settings', label: 'Accounts', exact: true },
  { to: '/settings/calendars', label: 'Calendars' },
  ...(auth.value?.enabled ? [{ to: '/settings/security', label: 'Security' }] : []),
  { to: '/settings/backups', label: 'Backups' },
  { to: '/settings/connections', label: 'Connections (MCP)' },
])
// Every app in the dock has a page: its settings, or what it is.
const { apps } = useApps()
const appSections = computed(() => [
  ...apps.value.filter((a) => a.enabled).map((a) => ({ to: appSettingsPath(a), label: a.name, icon: a.icon })),
  { to: '/apps', label: 'App library →', icon: undefined },
])
const route = useRoute()
const current = (s: { to: string; exact?: boolean }) => (s.exact ? route.path === s.to : route.path.startsWith(s.to))

// One list for Nuxt UI's UNavigationMenu: the sections, then "Your apps".
const navItems = computed(() => [
  sections.value.map((s) => ({ label: s.label, to: s.to, active: current(s) })),
  [
    { label: 'Your apps', type: 'label' as const },
    ...appSections.value.map((s) => ({
      label: s.label,
      to: s.to,
      icon: s.icon ? havreIcon(s.icon) : undefined,
      active: current(s),
    })),
  ],
])
// Phones: the menu runs across, under the title.
const phone = ref(false)
onMounted(() => {
  const q = window.matchMedia('(max-width: 720px)')
  phone.value = q.matches
  q.addEventListener('change', (e) => (phone.value = e.matches))
})
</script>

<template>
  <div class="frame">
    <nav class="nav" aria-label="Settings">
      <h1 class="serif">Settings</h1>
      <UNavigationMenu
        :items="navItems"
        :orientation="phone ? 'horizontal' : 'vertical'"
        color="neutral"
        :ui="{ root: phone ? 'overflow-x-auto' : '', label: 'eyebrow pt-4' }"
      />
    </nav>
    <main class="main">
      <div class="inner">
        <slot />
      </div>
    </main>
  </div>
</template>

<style scoped>
.frame {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-wrap: wrap;
  background: var(--panel);
  overflow-y: auto;
}
.nav {
  flex: 0 0 216px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 20px 12px;
  border-right: 1px solid var(--line);
  background: var(--bg);
  box-sizing: border-box;
}
h1 {
  margin: 0 8px 18px;
  font-weight: 400;
  font-size: 28px;
  letter-spacing: -0.01em;
}
.main {
  flex: 999 1 480px;
  min-width: 0;
  background: var(--panel);
}
.inner {
  max-width: 720px;
  padding: 40px 48px 64px;
  display: flex;
  flex-direction: column;
  gap: 36px;
}
@media (max-width: 720px) {
  .nav {
    flex: 1 1 100%;
    padding: 16px 16px 8px;
    border-right: 0;
    border-bottom: 1px solid var(--line);
  }
  h1 {
    margin: 0 4px 8px;
    font-size: 26px;
  }
  .inner {
    padding: 24px 16px 40px;
  }
}
</style>
