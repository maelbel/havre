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
</script>

<template>
  <div class="frame">
    <nav class="nav" aria-label="Settings">
      <h1 class="serif">Settings</h1>
      <NuxtLink
        v-for="s in sections"
        :key="s.to"
        :to="s.to"
        class="link"
        :class="{ on: current(s) }"
        :aria-current="current(s) ? 'page' : undefined"
      >
        {{ s.label }}
      </NuxtLink>
      <div class="group eyebrow">Your apps</div>
      <NuxtLink
        v-for="s in appSections"
        :key="s.to"
        :to="s.to"
        class="link"
        :class="{ on: current(s) }"
        :aria-current="current(s) ? 'page' : undefined"
      >
        <HIcon v-if="s.icon" :name="s.icon" :size="15" class="app-ic" />
        {{ s.label }}
      </NuxtLink>
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
.link {
  height: 34px;
  padding: 0 10px;
  display: flex;
  align-items: center;
  border-radius: 8px;
  color: var(--muted);
  text-decoration: none;
  font-size: 14px;
}
.app-ic {
  margin-right: 10px;
  flex: none;
}
.group {
  padding: 18px 10px 6px;
}
.link:hover {
  background: var(--hover);
  color: var(--text);
}
.link.on {
  background: var(--sel);
  color: var(--text);
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
    flex-direction: row;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    padding: 16px;
    border-right: 0;
    border-bottom: 1px solid var(--line);
  }
  h1 {
    flex: 1 1 100%;
    margin: 0 4px 8px;
    font-size: 26px;
  }
  .inner {
    padding: 24px 16px 40px;
  }
}
</style>
