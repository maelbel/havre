<script setup lang="ts">
import type { Phone, PersonDetail } from '~/composables/usePlatform'

// Edit what you know about someone: name, company, role, phones, birthday
// (the year is optional), a note. Everything is optional; Save sends it all
// (`updatePerson`), the server keeps the rest.
const props = defineProps<{ person: PersonDetail }>()
const emit = defineEmits<{ saved: [person: PersonDetail]; cancel: [] }>()
const toast = useToasts()

const p = props.person
// The birthday in three fields: a day and month, and a year you may not know.
const bday = p.birthday?.match(/^(\d{4}|-)-(\d{2})-(\d{2})$/)
const form = reactive({
  name: p.name,
  company: p.company,
  role: p.role,
  phones: p.phones.length ? p.phones.map((x) => ({ ...x })) : [{ label: 'mobile', number: '' }],
  day: bday ? Number(bday[3]) : 0,
  month: bday ? Number(bday[2]) : 0,
  year: (bday && bday[1] !== '-' ? bday[1] : '') as string,
  note: p.note,
})
const busy = ref(false)

const months = [{ label: 'Month', value: 0 }].concat(
  Array.from({ length: 12 }, (_, i) => ({
    label: new Date(2000, i, 1).toLocaleDateString([], { month: 'long' }),
    value: i + 1,
  })),
)
const days = [{ label: 'Day', value: 0 }].concat(Array.from({ length: 31 }, (_, i) => ({ label: String(i + 1), value: i + 1 })))
// Havre's labels, plus any an address book gave ("main", "fax"…).
const labels = computed(() =>
  [...new Set(['mobile', 'work', 'home', 'other', ...form.phones.map((p) => p.label).filter(Boolean)])].map((l) => ({
    label: l,
    value: l,
  })),
)

/** `YYYY-MM-DD`, `--MM-DD`, null (no birthday), or an error to show. */
const birthday = computed<{ value: string | null } | { error: string }>(() => {
  if (!form.day && !form.month && !form.year.trim()) return { value: null }
  if (!form.day || !form.month) return { error: 'Pick a day and a month for the birthday.' }
  const pad = (n: number) => String(n).padStart(2, '0')
  const year = form.year.trim()
  if (year && !/^\d{4}$/.test(year)) return { error: 'The year is four digits, or empty.' }
  // Checked against a leap year when there's no year (29 February is fine).
  const check = new Date(Number(year || 2000), form.month - 1, form.day)
  if (check.getMonth() !== form.month - 1) return { error: 'That day doesn’t exist in that month.' }
  return { value: `${year || '-'}-${pad(form.month)}-${pad(form.day)}` }
})

function addPhone() {
  form.phones.push({ label: 'mobile', number: '' })
}

async function save() {
  const b = birthday.value
  if ('error' in b) {
    toast.show(b.error, { tone: 'error' })
    return
  }
  busy.value = true
  try {
    const phones: Phone[] = form.phones.filter((x) => x.number.trim())
    const saved = await updatePerson(p.email, {
      name: form.name,
      company: form.company,
      role: form.role,
      phones,
      birthday: b.value,
      note: form.note,
    })
    emit('saved', saved)
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <UModal
    :open="true"
    :title="`About ${personName(person)}`"
    :description="
      person.books.length
        ? `Changes also go to ${person.books.join(' and ')}. Leave anything empty.`
        : 'Only you see this. Leave anything empty.'
    "
    @update:open="(open) => !open && emit('cancel')"
  >
    <template #body>
      <form id="person-form" class="form" @submit.prevent="save">
        <UFormField label="Name" :help="person.name ? undefined : 'Instead of their address'">
          <UInput v-model="form.name" autocomplete="off" class="w-full" autofocus />
        </UFormField>
        <div class="two">
          <UFormField label="Company">
            <UInput v-model="form.company" autocomplete="off" class="w-full" />
          </UFormField>
          <UFormField label="Role">
            <UInput v-model="form.role" autocomplete="off" class="w-full" />
          </UFormField>
        </div>
        <UFormField label="Phones">
          <div class="phones">
            <div v-for="(ph, i) in form.phones" :key="i" class="phone">
              <USelect v-model="ph.label" :items="labels" aria-label="Kind of number" class="w-28" />
              <UInput v-model="ph.number" type="tel" autocomplete="off" placeholder="+33 6 12 34 56 78" aria-label="Phone number" class="flex-1" />
              <UButton
                color="neutral"
                variant="ghost"
                icon="i-havre-close"
                square
                :aria-label="`Remove this number`"
                @click="form.phones.splice(i, 1)"
              />
            </div>
            <UButton color="neutral" variant="link" icon="i-havre-plus" size="sm" class="self-start" @click="addPhone">
              Add a number
            </UButton>
          </div>
        </UFormField>
        <UFormField label="Birthday" help="The year is optional.">
          <div class="bday">
            <USelect v-model="form.day" :items="days" aria-label="Birthday: day" class="w-20" />
            <USelect v-model="form.month" :items="months" aria-label="Birthday: month" class="w-36" />
            <UInput v-model="form.year" inputmode="numeric" maxlength="4" placeholder="Year" aria-label="Birthday: year" class="w-24" />
          </div>
        </UFormField>
        <UFormField label="Note">
          <UTextarea v-model="form.note" :rows="3" autoresize placeholder="Met at the Lyon kick-off, prefers calls…" class="w-full" />
        </UFormField>
      </form>
    </template>
    <template #footer>
      <div class="actions">
        <UButton color="neutral" variant="outline" @click="emit('cancel')">Cancel</UButton>
        <UButton type="submit" form="person-form" :loading="busy">Save</UButton>
      </div>
    </template>
  </UModal>
</template>

<style scoped>
.form {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.two {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
}
.phones {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.phone,
.bday {
  display: flex;
  gap: 6px;
  align-items: center;
}
.actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  width: 100%;
}
@media (max-width: 520px) {
  .two {
    grid-template-columns: 1fr;
  }
}
</style>
