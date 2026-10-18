<script setup lang="ts">
import type { Account } from '~/composables/useAccounts'
import type { Folder } from '~/composables/useMail'

// One account in the sidebar: a header that opens and closes, and its folders.
const props = defineProps<{
  account: Account
  expanded: boolean
  /** The account and folder open in the message list. */
  currentAccount: number | undefined
  currentFolder: string
  /** Bumped by the page when this account's folders change (live events). */
  version: number
}>()
const emit = defineEmits<{ toggle: []; select: [path: string] }>()

const folders = ref<Folder[]>([])
const loading = ref(false)
const isCurrent = computed(() => props.account.id === props.currentAccount)

async function load() {
  loading.value = true
  try {
    folders.value = await $fetch<Folder[]>('/api/mail/folders', { query: { account: props.account.id } })
  } catch {
    /* the header shows the sync status */
  } finally {
    loading.value = false
  }
}

// Folders are loaded the first time the section opens, then kept fresh.
watch(
  () => [props.expanded, props.version] as const,
  ([expanded], old) => {
    if (expanded && (!folders.value.length || props.version !== old?.[1])) load()
  },
  { immediate: true },
)

const bodyId = `account-${props.account.id}-folders`
</script>

<template>
  <section class="account" :class="{ current: isCurrent }">
    <button
      type="button"
      class="head"
      :aria-expanded="expanded"
      :aria-controls="bodyId"
      :title="account.email"
      @click="emit('toggle')"
    >
      <HIcon name="chevron" :size="13" class="chevron" />
      <span class="dot" :style="{ background: `var(--c-${account.color})` }" aria-hidden="true" />
      <span class="name">{{ account.name }}</span>
      <span v-if="account.status.state === 'error'" class="warn" :title="account.status.error ?? 'Sync problem'">!</span>
      <span v-else-if="!expanded && account.unread" class="count mono" :aria-label="`${account.unread} unread`">
        {{ account.unread }}
      </span>
    </button>

    <div v-show="expanded" :id="bodyId" class="body">
      <FolderList
        :folders="folders"
        :current="isCurrent ? currentFolder : null"
        :loading="loading"
        :nav-label="`${account.name} folders`"
        @select="(path) => emit('select', path)"
      />
    </div>
  </section>
</template>

<style scoped>
.account + .account {
  margin-top: 6px;
}
.head {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 8px;
  height: 32px;
  padding: 0 8px 0 4px;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--muted);
  text-align: left;
  cursor: pointer;
}
.head:hover {
  background: var(--hover);
  color: var(--text);
}
.chevron {
  flex: 0 0 auto;
  color: var(--faint);
  transform: rotate(-90deg);
  transition: transform 0.15s ease;
}
.head[aria-expanded='true'] .chevron {
  transform: none;
}
.dot {
  flex: 0 0 8px;
  height: 8px;
  border-radius: 50%;
}
.name {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
  font-size: 11px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.current .name {
  color: var(--text);
}
.count {
  font-size: 11.5px;
  color: var(--accent);
}
.warn {
  width: 18px;
  height: 18px;
  border-radius: 50%;
  display: grid;
  place-items: center;
  font-size: 11px;
  font-weight: 700;
  background: color-mix(in oklab, var(--danger) 20%, transparent);
  color: var(--danger);
}
.body {
  padding: 2px 0 4px 6px;
}
@media (prefers-reduced-motion: reduce) {
  .chevron {
    transition: none;
  }
}
</style>
