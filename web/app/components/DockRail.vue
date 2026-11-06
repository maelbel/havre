<script setup lang="ts">
import type { IconName } from './HIcon.vue'

// The dock, as in the design: the Havre mark, the App library, Today, the
// apps you chose (in your order, from `/api/apps`), "Add an app", then
// search (Ctrl+K), Settings and your account (theme, sign out).
const fallback: { to: string; label: string; icon: IconName; id: string }[] = [
  { id: 'mail', to: '/mail', label: 'Mail', icon: 'mail' },
  { id: 'agenda', to: '/agenda', label: 'Agenda', icon: 'calendar' },
  { id: 'people', to: '/people', label: 'People', icon: 'people' },
]
const { apps: installed, loaded } = useApps()
const apps = computed(() =>
  loaded.value
    ? installed.value.filter((a) => a.enabled).map((a) => ({ id: a.id, to: a.route, label: a.name, icon: a.icon }))
    : fallback,
)

// Unread mail in every inbox, on the Mail icon.
const accounts = useAccounts()
const unread = computed(() => accounts.data.value.reduce((n, a) => n + (a.unread ?? 0), 0))
// Tasks due today (or late), on the Tasks icon (kept by TaskReminders).
const due = useState('tasks-due', () => 0)
const label = (app: { id: string; label: string }) =>
  app.id === 'mail' && unread.value
    ? `${app.label}, ${unread.value} unread`
    : app.id === 'tasks' && due.value
      ? `${app.label}, ${due.value} due today`
      : app.label

const { theme, toggle } = useTheme()
const palette = usePalette()
const shortcut = import.meta.client && /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘K' : 'Ctrl+K'

// The account button: initials of your name, and a small menu.
const me = computed(() => {
  const a = accounts.data.value.find((x) => x.sender_name) ?? accounts.data.value[0]
  return a ? a.sender_name || a.email : ''
})
const auth = useAuthStatus()
const menuItems = computed(() => [
  ...(me.value ? [[{ label: me.value, type: 'label' as const }]] : []),
  [
    {
      label: theme.value === 'dark' ? 'Light theme' : 'Dark theme',
      icon: theme.value === 'dark' ? 'i-havre-sun' : 'i-havre-moon',
      onSelect: toggle,
    },
    { label: 'Settings', icon: 'i-havre-settings', to: '/settings' },
    ...(auth.value?.enabled ? [{ label: 'Sign out', icon: 'i-havre-back', onSelect: () => signOut() }] : []),
  ],
])
// ---- Phones ------------------------------------------------------------
const phone = useIsPhone()
// Room for five icons and the avatar: Today and four apps. With more apps
// than that, three stay and the fourth slot becomes "More" (the rest).
const BAR_APPS = 4
const barApps = computed(() => (apps.value.length > BAR_APPS ? apps.value.slice(0, BAR_APPS - 1) : apps.value))
const moreApps = computed(() => apps.value.slice(barApps.value.length))
const route = useRoute()
const inMore = computed(() => moreApps.value.some((a) => route.path.startsWith(a.to)))
// Which sheet is open, if any (one at a time).
const sheet = ref<'more' | 'account' | null>(null)
const close = (open: boolean) => {
  if (!open) sheet.value = null
}
// Close a sheet when the page changes (a link inside it was followed).
watch(() => route.fullPath, () => (sheet.value = null))
function openSearch() {
  sheet.value = null
  palette.value = true
}
</script>

<template>
  <nav v-if="!phone" class="rail" aria-label="Apps">
    <div class="logo" role="img" aria-label="Havre">
      <HIcon name="logo" :size="22" :stroke-width="1.9" />
    </div>

    <NuxtLink to="/apps" class="item" aria-label="App library" title="App library" active-class="active">
      <HIcon name="apps" />
    </NuxtLink>
    <NuxtLink to="/" class="item" aria-label="Today" title="Today" exact-active-class="active">
      <HIcon name="sun" />
    </NuxtLink>
    <NuxtLink
      v-for="app in apps"
      :key="app.to"
      :to="app.to"
      class="item"
      :aria-label="label(app)"
      :title="app.label"
      active-class="active"
    >
      <HIcon :name="app.icon" />
      <span v-if="app.id === 'mail' && unread" class="badge mono" aria-hidden="true">{{ unread > 99 ? '99+' : unread }}</span>
      <span v-else-if="app.id === 'tasks' && due" class="badge quiet mono" aria-hidden="true">{{ due > 99 ? '99+' : due }}</span>
    </NuxtLink>

    <span class="divider" aria-hidden="true" />
    <NuxtLink to="/apps#available" class="item add" aria-label="Add an app" title="Add an app">
      <HIcon name="plus" :size="18" />
    </NuxtLink>

    <span class="spacer" />

    <button
      type="button"
      class="item"
      aria-label="Search and ask"
      :title="`Search, go to, ask (${shortcut})`"
      @click="palette = true"
    >
      <HIcon name="search" />
    </button>
    <NuxtLink to="/settings" class="item" aria-label="Settings" title="Settings" active-class="active">
      <HIcon name="settings" />
    </NuxtLink>
    <div class="account">
      <UDropdownMenu :items="menuItems" :content="{ side: 'right', align: 'end', sideOffset: 10 }">
        <button type="button" class="avatar" :aria-label="me ? `Account: ${me}` : 'Account'">
          {{ me ? initials(me) : '·' }}
        </button>
      </UDropdownMenu>
    </div>
  </nav>
  <!-- Phones: a bottom bar for the thumb. Today, the first apps of your
       dock, "More" for the rest, and your account. Search sits at the top
       of each screen (and in the account sheet). -->
  <nav v-else class="bar" aria-label="Apps">
    <NuxtLink to="/" class="item" aria-label="Today" exact-active-class="active">
      <HIcon name="sun" />
    </NuxtLink>
    <NuxtLink
      v-for="app in barApps"
      :key="app.to"
      :to="app.to"
      class="item"
      :aria-label="label(app)"
      active-class="active"
    >
      <HIcon :name="app.icon" />
      <span v-if="app.id === 'mail' && unread" class="badge mono" aria-hidden="true">{{ unread > 99 ? '99+' : unread }}</span>
      <span v-else-if="app.id === 'tasks' && due" class="badge quiet mono" aria-hidden="true">{{ due > 99 ? '99+' : due }}</span>
    </NuxtLink>
    <button
      v-if="moreApps.length"
      type="button"
      class="item"
      :class="{ active: inMore }"
      :aria-label="`More apps: ${moreApps.map((a) => a.label).join(', ')}`"
      @click="sheet = 'more'"
    >
      <HIcon name="apps" />
    </button>
    <button type="button" class="avatar" :aria-label="me ? `Account and settings: ${me}` : 'Account and settings'" @click="sheet = 'account'">
      {{ me ? initials(me) : '·' }}
    </button>
  </nav>

  <UDrawer v-if="phone" :open="sheet === 'more'" title="More apps" @update:open="close">
    <template #body>
      <nav class="sheet" aria-label="More apps">
        <NuxtLink v-for="app in moreApps" :key="app.to" :to="app.to" class="row" active-class="active" @click="sheet = null">
          <HIcon :name="app.icon" /><span>{{ app.label }}</span>
        </NuxtLink>
        <span class="sep" aria-hidden="true" />
        <NuxtLink to="/apps" class="row" @click="sheet = null"><HIcon name="apps" /><span>App library</span></NuxtLink>
        <NuxtLink to="/apps#available" class="row" @click="sheet = null"><HIcon name="plus" /><span>Add an app</span></NuxtLink>
      </nav>
    </template>
  </UDrawer>

  <UDrawer v-if="phone" :open="sheet === 'account'" :title="me || 'Havre'" @update:open="close">
    <template #body>
      <nav class="sheet" aria-label="Account">
        <button type="button" class="row" @click="openSearch"><HIcon name="search" /><span>Search, go to, ask</span></button>
        <NuxtLink to="/settings" class="row" @click="sheet = null"><HIcon name="settings" /><span>Settings</span></NuxtLink>
        <NuxtLink v-if="!moreApps.length" to="/apps" class="row" @click="sheet = null"><HIcon name="apps" /><span>App library</span></NuxtLink>
        <button type="button" class="row" @click="toggle"><HIcon :name="theme === 'dark' ? 'sun' : 'moon'" /><span>{{ theme === 'dark' ? 'Light theme' : 'Dark theme' }}</span></button>
        <button v-if="auth?.enabled" type="button" class="row" @click="signOut()"><HIcon name="back" /><span>Sign out</span></button>
      </nav>
    </template>
  </UDrawer>
</template>

<style scoped>
.rail {
  flex: 0 0 64px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 16px 0;
  border-right: 1px solid var(--line);
  background: var(--bg);
}
.item {
  position: relative;
}
.badge {
  position: absolute;
  top: 5px;
  right: 4px;
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  border-radius: 8px;
  background: var(--accent);
  color: var(--on-accent);
  font-size: 10px;
  font-weight: 500;
  display: flex;
  align-items: center;
  justify-content: center;
}
/* Tasks due: quieter than unread mail. */
.badge.quiet {
  background: var(--raise);
  border: 1px solid var(--line2);
  color: var(--text);
}
.account {
  position: relative;
  margin-top: 8px;
}
.avatar {
  width: 34px;
  height: 34px;
  border-radius: 50%;
  border: 1px solid var(--line2);
  background: var(--raise);
  color: var(--text);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
}
.logo {
  width: 34px;
  height: 34px;
  margin-bottom: 14px;
  border-radius: 10px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--text);
  color: var(--bg);
}
.item {
  width: 44px;
  height: 44px;
  border: 0;
  border-radius: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  color: var(--muted);
  cursor: pointer;
}
/* Only with a real pointer: on a touch screen the hover look would stay
   on the last button tapped. */
@media (hover: hover) {
  .item:hover {
    background: var(--hover);
    color: var(--text);
  }
}
.item.active {
  background: var(--sel);
  color: var(--text);
}
.add {
  border: 1px dashed var(--line2);
  color: var(--faint);
}
.divider {
  width: 20px;
  height: 1px;
  margin: 6px 0;
  background: var(--line2);
}
.spacer {
  flex: 1;
}

/* ---- Phones: the bottom bar and its sheets ---- */
.bar {
  order: 2;
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  justify-content: space-around;
  /* Clear of the iPhone home bar / Android gesture bar. */
  padding: 6px 8px calc(6px + env(safe-area-inset-bottom));
  border-top: 1px solid var(--line);
  background: var(--bg);
}
.bar .item.active {
  color: var(--accent);
}
.sheet {
  display: flex;
  flex-direction: column;
  padding-bottom: env(safe-area-inset-bottom);
}
.row {
  display: flex;
  align-items: center;
  gap: 14px;
  min-height: 48px;
  padding: 0 6px;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  font-size: 15px;
  text-align: left;
  text-decoration: none;
  cursor: pointer;
}
.row :deep(svg) {
  color: var(--muted);
}
.row:active,
.row.active {
  background: var(--sel);
}
.sep {
  height: 1px;
  margin: 6px 0;
  background: var(--line);
}
</style>
