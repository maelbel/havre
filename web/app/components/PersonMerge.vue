<script setup lang="ts">
import type { PersonDetail, PersonEntry } from '~/composables/usePlatform'

// "Merge with…": pick the same person's other address (or type a new one).
// Their mail, meetings, details and address books then show on one page.
const props = defineProps<{ person: PersonDetail }>()
const emit = defineEmits<{ merged: [person: PersonDetail]; cancel: [] }>()
const toast = useToasts()

const query = ref('')
const found = ref<PersonEntry[]>([])
const busy = ref<string | null>(null)
let seq = 0
async function search() {
  const mine = ++seq
  const list = await searchPeople(query.value.trim(), 30).catch(() => [])
  // Not this person (any of their addresses).
  if (mine === seq) {
    found.value = list
      .filter((p) => p.email !== props.person.email && !p.emails.some((e) => props.person.emails.includes(e)))
      .slice(0, 8)
  }
}
watch(query, () => search())
onMounted(() => {
  // Start with people of the same name, if any.
  query.value = props.person.name.split(' ')[0] ?? ''
  search()
})

/** A typed address nobody has: offered as a new one for them. */
const typed = computed(() => {
  const q = query.value.trim().toLowerCase()
  if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(q)) return null
  return found.value.some((p) => p.emails.includes(q)) || props.person.emails.includes(q) ? null : q
})
const bare = computed(() => noAddress(props.person))

async function pick(p: PersonEntry | string) {
  const other = typeof p === 'string' ? p : p.email
  busy.value = other
  try {
    const merged = await mergePerson(props.person.email, other)
    toast.show(
      typeof p === 'string'
        ? `${p} added to ${personName(merged)}`
        : `${personName(p)} is now part of ${personName(props.person)}`,
    )
    emit('merged', merged)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    busy.value = null
  }
}
</script>

<template>
  <UModal
    :open="true"
    :title="bare ? `An address for ${personName(person)}` : `Merge with ${personName(person)}`"
    :description="
      bare
        ? 'Type their address, or pick them if they’re already in People: one page, their mail and meetings with the rest.'
        : 'Pick their other address (or type a new one): one page, one entry in the list, their mail and meetings together. You can separate them again.'
    "
    @update:open="(open) => !open && emit('cancel')"
  >
    <template #body>
      <UInput v-model="query" type="search" icon="i-havre-search" placeholder="Name or address" aria-label="Find their other address" class="w-full" autofocus />
      <ul class="found">
        <li v-if="typed">
          <button type="button" class="row" :disabled="!!busy" @click="pick(typed)">
            <span class="plus" aria-hidden="true"><HIcon name="plus" :size="15" /></span>
            <span class="who">
              <span class="name">Add {{ typed }}</span>
              <span class="sub">A new address for {{ personName(person) }}</span>
            </span>
            <HIcon :name="busy === typed ? 'loading' : 'merge'" :size="15" class="go" />
          </button>
        </li>
        <li v-for="p in found" :key="p.email">
          <button type="button" class="row" :disabled="!!busy" @click="pick(p)">
            <PersonAvatar :person="p" :size="30" />
            <span class="who">
              <span class="name">{{ personName(p) }}</span>
              <span class="sub">{{ p.emails.length ? p.emails.join(', ') : (p.phones[0]?.number ?? 'No address') }}</span>
            </span>
            <HIcon :name="busy === p.email ? 'loading' : 'merge'" :size="15" class="go" />
          </button>
        </li>
      </ul>
      <p v-if="!found.length && !typed" class="none">Nobody matches. Try their name or another part of the address.</p>
    </template>
  </UModal>
</template>

<style scoped>
.found {
  list-style: none;
  margin: 10px 0 0;
  padding: 0;
}
.row {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 8px 6px;
  border: 0;
  border-radius: var(--radius);
  background: transparent;
  color: var(--text);
  text-align: left;
  cursor: pointer;
}
.row:hover,
.row:focus-visible {
  background: var(--raise);
}
.plus {
  flex: none;
  display: grid;
  place-items: center;
  width: 30px;
  height: 30px;
  border-radius: var(--radius);
  border: 1px dashed var(--line);
  color: var(--muted);
}
.who {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.name {
  font-size: 13.5px;
}
.sub {
  color: var(--faint);
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.go {
  color: var(--muted);
}
.none {
  margin: 12px 0 0;
  color: var(--faint);
  font-size: 13px;
}
</style>
