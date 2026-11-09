<script setup lang="ts">
import type { PersonDetail, PersonEntry } from '~/composables/usePlatform'

// "Merge with…": pick the same person's other address. Their mail, meetings
// and details then show on one page.
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
  if (mine === seq) found.value = list.filter((p) => !p.emails.some((e) => props.person.emails.includes(e))).slice(0, 8)
}
watch(query, () => search())
onMounted(() => {
  // Start with people of the same name, if any.
  query.value = props.person.name.split(' ')[0] ?? ''
  search()
})

async function pick(p: PersonEntry) {
  busy.value = p.email
  try {
    const merged = await mergePerson(props.person.email, p.email)
    toast.show(`${personName(p)} is now part of ${personName(props.person)}`)
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
    :title="`Merge with ${personName(person)}`"
    description="Pick their other address: one page, one entry in the list, their mail and meetings together. You can separate them again."
    @update:open="(open) => !open && emit('cancel')"
  >
    <template #body>
      <UInput v-model="query" type="search" icon="i-havre-search" placeholder="Name or address" aria-label="Find their other address" class="w-full" autofocus />
      <ul class="found">
        <li v-for="p in found" :key="p.email">
          <button type="button" class="row" :disabled="!!busy" @click="pick(p)">
            <span class="avatar" :style="{ background: personColor(p.email) }">{{ initials(personName(p)) }}</span>
            <span class="who">
              <span class="name">{{ personName(p) }}</span>
              <span class="sub">{{ p.emails.join(', ') }}</span>
            </span>
            <HIcon :name="busy === p.email ? 'loading' : 'merge'" :size="15" class="go" />
          </button>
        </li>
      </ul>
      <p v-if="!found.length" class="none">Nobody matches. Try their name or another part of the address.</p>
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
.avatar {
  flex: none;
  display: grid;
  place-items: center;
  width: 30px;
  height: 30px;
  border-radius: 50%;
  color: #121210;
  font-size: 11.5px;
  font-weight: 600;
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
