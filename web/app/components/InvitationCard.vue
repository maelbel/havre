<script setup lang="ts">
import type { AgendaEvent, InvitationRef, InvitationView, Response } from '~/composables/useAgenda'

// A meeting invitation in Mail: what, when, what else you have then, and
// Accept / Maybe / Decline. The answer goes through Microsoft or Google
// when the meeting is already in that calendar, otherwise Havre emails the
// organizer (and adds the meeting to your calendar if you accept).
const props = defineProps<{ invitation: InvitationRef }>()

const view = ref<InvitationView | null>(null)
const error = ref<string | null>(null)
const busy = ref<Response | null>(null)

async function load() {
  error.value = null
  try {
    view.value = await fetchInvitation(props.invitation)
  } catch (err) {
    view.value = null
    error.value = apiErrorMessage(err)
  }
}
watch(() => props.invitation, load, { immediate: true, deep: true })

async function answer(response: Response) {
  busy.value = response
  try {
    view.value = await respondToInvitation(props.invitation, response)
    const verb = { accepted: 'Accepted', tentative: 'Answered maybe to', declined: 'Declined' }[response]
    useToasts().show(`${verb} “${view.value.title}”`)
  } catch (err) {
    useToasts().show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    busy.value = null
  }
}

/** No answer to give (no organizer, your own meeting): just add it. */
const adding = ref(false)
async function addToAgenda() {
  adding.value = true
  try {
    const events = await addInvitation(props.invitation)
    useToasts().show(`Added to the Agenda: ${events.map((e) => e.title).join(', ')}`)
    await load()
  } catch (err) {
    useToasts().show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    adding.value = false
  }
}

/** The card's times, shaped like an event for the date helpers. */
const asEvent = computed(() => (view.value ? ({ ...view.value, attendees: [] } as unknown as AgendaEvent) : null))
const day = computed(() => (asEvent.value ? eventSpan(asEvent.value).start : null))
const cancelled = computed(() => view.value?.method === 'CANCEL')
const agendaLink = computed(() => (day.value ? { path: '/agenda', query: { date: isoDate(day.value) } } : '/agenda'))
const buttons: { response: Response; label: string }[] = [
  { response: 'accepted', label: 'Accept & add to Agenda' },
  { response: 'tentative', label: 'Maybe' },
  { response: 'declined', label: 'Decline' },
]

// "Invitation · Work calendar" once it's in a calendar, else who sent it.
const calendars = useCalendars()
const source = computed(() => {
  const id = view.value?.event?.calendar
  const cal = id ? calendars.data.value.find((c) => c.id === id) : null
  if (cal) return /calendar|agenda|calendrier/i.test(cal.name) ? cal.name : `${cal.name} calendar`
  return view.value?.organizer ? `from ${view.value.organizer}` : ''
})
const time = computed(() => {
  const e = asEvent.value
  if (!e) return ''
  if (e.all_day) return 'All day'
  return `${timeLabel(new Date(e.start * 1000))} – ${timeLabel(new Date(e.end * 1000))}`
})
/** "Thursday morning" — the part of the day you're free. */
const partOfDay = computed(() => {
  if (!day.value || !asEvent.value) return ''
  const h = new Date(asEvent.value.start * 1000).getHours()
  const part = asEvent.value.all_day ? '' : h < 12 ? ' morning' : h < 18 ? ' afternoon' : ' evening'
  return `${day.value.toLocaleDateString([], { weekday: 'long' })}${part}`
})
</script>

<template>
  <section v-if="view" class="card" :class="{ cancelled }" aria-label="Invitation in this email">
    <div class="top">
      <div v-if="day" class="date" aria-hidden="true">
        <span class="mon mono">{{ day.toLocaleDateString([], { month: 'short' }).toUpperCase().replace('.', '') }}</span>
        <span class="num">{{ day.getDate() }}</span>
        <span class="wd">{{ day.toLocaleDateString([], { weekday: 'short' }) }}</span>
      </div>
      <div class="body">
        <div class="src">{{ cancelled ? 'Cancelled meeting' : 'Invitation' }}<template v-if="source"> · {{ source }}</template></div>
        <h3>{{ view.title }}</h3>
        <div class="meta">
          <span><HIcon name="clock" :size="14" /> {{ time }}<template v-if="view.occurrences > 1"> · repeats</template></span>
          <span v-if="view.location"><HIcon name="pin" :size="14" /> {{ view.location }}</span>
        </div>
        <p v-if="view.organizer && source !== `from ${view.organizer}`" class="faint small">Organized by {{ view.organizer }}</p>
        <p v-if="view.conflicts.length" class="conflict">
          <HIcon name="alert" :size="14" />
          <span>
            At the same time:
            <template v-for="(c, i) in view.conflicts" :key="c.id">
              <strong>{{ c.title }}</strong>
              <template v-if="i < view.conflicts.length - 1">, </template>
            </template>
          </span>
        </p>
        <p v-else-if="!cancelled" class="free">
          <HIcon name="check" :size="14" />
          You’re free — nothing else {{ partOfDay }}
        </p>
      </div>
    </div>

    <div class="actions">
      <template v-if="view.can_respond">
        <UButton
          v-for="(b, i) in buttons"
          :key="b.response"
          v-bind="responseButton(b.response, view.my_response === b.response, i === 0)"
          size="sm"
          :loading="busy === b.response"
          type="button"
          :aria-pressed="view.my_response === b.response"
          :disabled="!!busy"
          @click="answer(b.response)"
        >
          {{ i === 0 && view.my_response === 'accepted' ? 'Accepted' : b.label }}
        </UButton>
      </template>
      <span v-if="view.my_response && !view.can_respond" class="status" :class="view.my_response">
        {{ RESPONSE_LABELS[view.my_response] }}
      </span>
      <UButton
        v-if="!view.can_respond && !view.event && !cancelled"
        color="primary"
        variant="solid"
        size="sm"
        type="button"
        :disabled="adding"
        @click="addToAgenda"
      >
        {{ adding ? 'Adding…' : 'Add to Agenda' }}
      </UButton>
      <span class="spacer" />
      <UButton color="primary" variant="link" class="p-0" :to="agendaLink">Open in Agenda →</UButton>
    </div>
  </section>
  <p v-else-if="error" class="err">
    <HIcon name="alert" :size="14" />
    This invitation can't be read: {{ error }}
  </p>
</template>

<style scoped>
.card {
  border: 1px solid var(--line2);
  border-radius: 16px;
  background: var(--raise);
  overflow: hidden;
}
.card.cancelled h3 {
  text-decoration: line-through;
  color: var(--muted);
}
.top {
  display: flex;
  gap: 18px;
  padding: 18px 20px;
}
.date {
  flex: 0 0 60px;
  height: 68px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  border-radius: 12px;
  border: 1px solid var(--line2);
  background: var(--panel);
}
.mon {
  font-size: 10.5px;
  letter-spacing: 0.08em;
  color: var(--accent);
}
.num {
  font-family: var(--font-serif);
  font-size: 28px;
  line-height: 1;
}
.wd {
  font-size: 11px;
  color: var(--muted);
}
.body {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.src {
  font-size: 12px;
  color: var(--faint);
}
h3 {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  overflow-wrap: anywhere;
}
.meta {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 16px;
  font-size: 13px;
  color: var(--muted);
}
.meta span {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.faint {
  color: var(--faint);
}
.small {
  margin: 0;
  font-size: 12.5px;
}
.conflict,
.free {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  margin: 6px 0 0;
  font-size: 13px;
}
.conflict {
  color: var(--danger);
}
.conflict span {
  color: var(--text);
}
.free {
  color: var(--ok);
}
.actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  padding: 12px 20px;
  border-top: 1px solid var(--line);
  background: var(--panel);
}
.status {
  font-size: 12.5px;
  color: var(--faint);
}
.status.accepted {
  color: var(--ok);
}
.status.declined {
  color: var(--danger);
}
.spacer {
  flex: 1;
}
.err {
  display: flex;
  gap: 8px;
  align-items: center;
  margin: 0;
  font-size: 13px;
  color: var(--faint);
}
.spinner {
  width: 12px;
  height: 12px;
  border-radius: 50%;
  border: 2px solid var(--line2);
  border-top-color: var(--text);
  animation: spin 0.7s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
@media (max-width: 520px) {
  .date {
    display: none;
  }
}
</style>
