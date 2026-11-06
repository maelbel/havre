<script setup lang="ts">
import type { IconName } from './HIcon.vue'

// Settings, as in the design: a list of sections on the left, one section
// at a time on the right.
const auth = useAuthStatus()
const sections = computed(() => [
  { to: '/settings', label: 'Accounts', exact: true, icon: 'mail' as IconName },
  { to: '/settings/calendars', label: 'Calendars', icon: 'calendar' as IconName },
  ...(auth.value?.enabled ? [{ to: '/settings/security', label: 'Security', icon: 'shield' as IconName }] : []),
  { to: '/settings/backups', label: 'Backups', icon: 'archive' as IconName },
  { to: '/settings/connections', label: 'Connections (MCP)', icon: 'command' as IconName },
])
// Every app in the dock has a page: its settings, or what it is.
const { apps } = useApps()
const appSections = computed(() => [
  ...apps.value.filter((a) => a.enabled).map((a) => ({ to: appSettingsPath(a), label: a.name, icon: a.icon })),
  { to: '/apps', label: 'App library →', icon: undefined },
])
const route = useRoute()
// The server may add a slash (`/settings/` when the page is opened directly).
const path = computed(() => route.path.replace(/(.)\/$/, '$1'))
const current = (s: { to: string; exact?: boolean }) => (s.exact ? path.value === s.to : path.value.startsWith(s.to))

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
// ---- Phones ----------------------------------------------------------------
// `/settings` is a menu (grouped like a phone's own settings) and each
// section opens on its own screen with Back. The Accounts section is at
// `/settings` on wider screens, so on a phone it's `/settings?page=accounts`
// (and `?add=1`, "Add an account", opens it too).
const phone = useIsPhone()
const showMenu = computed(() => path.value === '/settings' && !route.query.page && !route.query.add)

const accounts = useAccounts()
const { theme, toggle } = useTheme()
const stateLabel = (state: string) => (state === 'error' ? 'Needs attention' : state === 'syncing' ? 'Updating' : 'Working')
</script>

<template>
  <!-- Phones: the menu… -->
  <div v-if="phone && showMenu" class="phone-menu">
    <PhoneHeader title="Settings" />
    <div class="groups">
      <section aria-labelledby="pm-accounts">
        <h2 id="pm-accounts" class="eyebrow">Accounts</h2>
        <div class="group">
          <NuxtLink v-for="a in accounts.data.value" :key="a.id" :to="{ path: '/settings', query: { page: 'accounts' } }" class="row">
            <HIcon name="mail" />
            <span class="what"><span class="ellipsis">{{ a.email }}</span><span class="sub ellipsis">{{ a.name }}</span></span>
            <span class="state" :class="a.status.state" :aria-label="stateLabel(a.status.state)" />
            <HIcon name="chevron" :size="16" class="chev" />
          </NuxtLink>
          <NuxtLink :to="{ path: '/settings', query: { add: 1 } }" class="row">
            <HIcon name="plus" /><span class="what">Add an account</span><HIcon name="chevron" :size="16" class="chev" />
          </NuxtLink>
        </div>
      </section>
      <section aria-labelledby="pm-havre">
        <h2 id="pm-havre" class="eyebrow">Your apps</h2>
        <div class="group">
          <NuxtLink v-for="s in appSections" :key="s.to" :to="s.to" class="row">
            <HIcon :name="s.icon ?? 'apps'" /><span class="what">{{ s.label.replace(' →', '') }}</span><HIcon name="chevron" :size="16" class="chev" />
          </NuxtLink>
        </div>
      </section>
      <section aria-labelledby="pm-more">
        <h2 id="pm-more" class="eyebrow">Havre</h2>
        <div class="group">
          <NuxtLink v-for="s in sections.filter((x) => !x.exact)" :key="s.to" :to="s.to" class="row">
            <HIcon :name="s.icon" /><span class="what">{{ s.label }}</span><HIcon name="chevron" :size="16" class="chev" />
          </NuxtLink>
          <button type="button" class="row" @click="toggle">
            <HIcon :name="theme === 'dark' ? 'moon' : 'sun'" />
            <span class="what">Appearance<span class="sub">{{ theme === 'dark' ? 'Dark' : 'Light' }} · tap to switch</span></span>
          </button>
        </div>
      </section>
    </div>
  </div>

  <!-- …and each section on its own screen. -->
  <div v-else-if="phone" class="phone-page">
    <!-- The section's own heading is the title, so here only "‹ Settings". -->
    <PhoneHeader back="/settings" back-label="Back to Settings" eyebrow="Settings" :search="false" />
    <div class="inner">
      <slot />
    </div>
  </div>

  <div v-else class="frame">
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
/* ---- Phones ---- */
.phone-menu,
.phone-page {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  background: var(--panel);
}
.phone-page .inner {
  padding: 8px 16px 40px;
}
.groups {
  display: flex;
  flex-direction: column;
  gap: 18px;
  padding: 0 12px 32px;
}
.groups h2 {
  margin: 0 6px 6px;
  font-weight: 400;
}
.group {
  border: 1px solid var(--line);
  border-radius: 4px;
  background: var(--raise);
  overflow: hidden;
}
.row {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: 52px;
  padding: 6px 14px;
  border: 0;
  border-bottom: 1px solid var(--line);
  background: transparent;
  color: var(--text);
  font-size: 15px;
  text-align: left;
  text-decoration: none;
  cursor: pointer;
}
.row:last-child {
  border-bottom: 0;
}
.row:active {
  background: var(--sel);
}
.row > :deep(svg:first-child) {
  color: var(--muted);
}
.what {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.sub {
  font-size: 12.5px;
  color: var(--faint);
}
.ellipsis {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.chev {
  color: var(--faint);
  /* The icon points down; a menu row points right. */
  transform: rotate(-90deg);
}
.state {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--ok);
}
.state.error {
  background: var(--danger);
}
</style>
