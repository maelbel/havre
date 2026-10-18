<script setup lang="ts">
import type { AgendaEvent, Calendar, Response } from '~/composables/useAgenda'
import type { NoteSummary } from '~/composables/useNotes'
import type { Related } from '~/composables/usePlatform'

// The panel beside the calendar for the selected event.
const props = defineProps<{ event: AgendaEvent; calendar?: Calendar }>()
const emit = defineEmits<{ edit: []; close: []; responded: [event: AgendaEvent] }>()

// Someone else's meeting: answer it here (Microsoft / Google tell the organizer).
const busy = ref<Response | null>(null)
async function answer(response: Response) {
  busy.value = response
  try {
    emit('responded', await respondToEvent(props.event.id, response))
  } catch (err) {
    useToasts().show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    busy.value = null
  }
}
const answers: { response: Response; label: string }[] = [
  { response: 'accepted', label: 'Accept' },
  { response: 'tentative', label: 'Maybe' },
  { response: 'declined', label: 'Decline' },
]
const organizerEmail = computed(() => emailIn(props.event.organizer))

const isLink = (text: string) => /^https?:\/\//.test(text.trim())
const why = computed(() => {
  if (!props.event.read_only) return null
  if (props.calendar?.read_only) return 'This calendar is read-only.'
  if (props.event.organizer && props.event.attendees.length) return 'Organized by someone else: changes go through them.'
  if (props.event.recurring) return 'Repeating CalDAV events are changed in your calendar app for now.'
  return 'Read-only.'
})

// People: the organizer first, then the guests, with how they answered.
const accounts = useAccounts()
const mine = computed(() => new Set(accounts.data.value.map((a) => a.email.toLowerCase())))
const words: Record<string, string> = { accepted: 'going', tentative: 'maybe', declined: 'declined', needs_action: 'awaiting' }
const people = computed(() => {
  const out: { email: string; name: string; status: string; you: boolean }[] = []
  const org = organizerEmail.value
  if (props.event.organizer && org) {
    out.push({ email: org, name: props.event.organizer.replace(/\s*<.*>$/, ''), status: 'organizer', you: mine.value.has(org) })
  }
  for (const a of props.event.attendees) {
    const email = a.email.toLowerCase()
    if (email === org) continue
    out.push({ email, name: a.name || a.email, status: words[a.status] ?? a.status, you: mine.value.has(email) })
  }
  return out
})

/** "Thu 15 Oct · 10:00 – 11:00". */
const when = computed(() => {
  const { start, end } = eventSpan(props.event)
  const day = start.toLocaleDateString([], { weekday: 'short', day: 'numeric', month: 'short' })
  if (props.event.all_day) return `${day} · all day`
  return `${day} · ${timeLabel(start)} – ${timeLabel(end)}`
})
const place = computed(() => {
  const loc = props.event.location.trim()
  if (loc && !isLink(loc)) return loc
  return props.event.join_url || isLink(loc) ? 'Video call' : ''
})

// What it's about: recent mail and files from the same people.
const related = ref<Related | null>(null)
watch(
  () => props.event.id,
  async (id) => {
    related.value = null
    try {
      const r = await fetchRelated(id)
      if (props.event.id === id) related.value = r
    } catch {
      /* nothing to show */
    }
  },
  { immediate: true },
)
// Your notes for this meeting (when the Notes app is in the dock).
const notesOn = useNotesEnabled()
const eventNotes = ref<NoteSummary[]>([])
watch(
  [() => props.event.id, () => props.event.start, notesOn],
  async () => {
    eventNotes.value = []
    if (!notesOn.value) return
    const id = props.event.id
    const found = await fetchEventNotes(props.event).catch(() => [])
    if (props.event.id === id) eventNotes.value = found
  },
  { immediate: true },
)
const making = ref(false)
async function takeNotes() {
  making.value = true
  try {
    const note = await noteFromEvent(props.event.id)
    await navigateTo({ path: '/notes', query: { note: note.id, edit: 1 } })
  } catch (err) {
    useToasts().show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    making.value = false
  }
}

function short(text: string | null | undefined) {
  return (text ?? '').replace(/\s*<.*>$/, '').split(/\s+/)[0] ?? ''
}
function fileKindLabel(name: string, mime: string) {
  if (mime === 'text/calendar') return 'ICS'
  const ext = name.includes('.') ? name.split('.').pop()! : mime.split('/')[1] ?? 'file'
  return ext.slice(0, 4).toUpperCase()
}
</script>

<template>
  <aside class="details" aria-label="Event details" :style="{ '--c': calendar?.color ?? 'var(--accent)' }">
    <div class="head">
      <div class="calname">
        <span class="sq" aria-hidden="true" />
        <span class="grow">{{ calendar?.name ?? 'Calendar' }}</span>
        <button type="button" class="icon-btn small" aria-label="Close" @click="emit('close')">
          <HIcon name="close" :size="15" />
        </button>
      </div>
      <h2 class="serif">{{ event.title }}</h2>
      <div class="facts">
        <span><HIcon name="clock" :size="15" /> {{ when }}<template v-if="event.recurring"> · repeats</template></span>
        <span v-if="place"><HIcon name="pin" :size="15" /> {{ place }}</span>
      </div>
      <div v-if="event.join_url || !event.read_only" class="acts">
        <a v-if="event.join_url" class="act primary" :href="event.join_url" target="_blank" rel="noopener">Join call</a>
        <button v-if="!event.read_only" type="button" class="act" @click="emit('edit')">Edit</button>
      </div>
      <p v-if="event.read_only && why" class="faint small">{{ why }}</p>
    </div>

    <div v-if="event.my_response" class="rsvp" role="group" aria-label="Your answer">
      <span class="q">{{ event.my_response === 'needs_action' ? 'Going?' : `You answered: ${RESPONSE_LABELS[event.my_response]}` }}</span>
      <button
        v-for="a in answers"
        :key="a.response"
        type="button"
        class="act"
        :class="{ chosen: event.my_response === a.response }"
        :aria-pressed="event.my_response === a.response"
        :disabled="!!busy"
        @click="answer(a.response)"
      >
        {{ a.label }}
      </button>
    </div>

    <section v-if="people.length" class="block">
      <div class="eyebrow">People · {{ people.length }}</div>
      <NuxtLink
        v-for="p in people"
        :key="p.email"
        :to="`/people/${encodeURIComponent(p.email)}`"
        class="person"
        :title="p.email"
      >
        <span class="av" :class="{ me: p.you }" aria-hidden="true" :style="{ '--pc': personColor(p.name) }">{{ initials(p.name) }}</span>
        <span class="grow">{{ p.you ? 'You' : p.name }}</span>
        <span class="st" :class="p.status">{{ p.status }}</span>
      </NuxtLink>
    </section>

    <section v-if="notesOn" class="block" aria-label="Your notes">
      <div class="eyebrow ic"><HIcon name="note" :size="13" /> Your notes</div>
      <NuxtLink v-for="n in eventNotes" :key="n.id" :to="{ path: '/notes', query: { note: n.id } }" class="mail note">
        <span class="mrow">
          <span class="mt">{{ noteTitle(n) }}</span>
          <span class="accent-l tiny">Open</span>
        </span>
        <span v-if="n.snippet || n.open_todos" class="ms">
          {{ n.snippet }}<template v-if="n.open_todos"> · {{ n.open_todos }} to-do{{ n.open_todos === 1 ? '' : 's' }}</template>
        </span>
      </NuxtLink>
      <button type="button" class="act take" :class="{ dashed: eventNotes.length }" :disabled="making" @click="takeNotes">
        <HIcon name="plus" :size="13" :stroke-width="1.8" />{{ eventNotes.length ? 'Another note' : 'Take notes' }}
      </button>
    </section>

    <section v-if="related?.messages.length" class="block">
      <div class="eyebrow ic"><HIcon name="mail" :size="13" /> Related emails</div>
      <NuxtLink v-for="m in related.messages.slice(0, 4)" :key="`${m.account}-${m.folder}-${m.uid}`" :to="messageLink(m)" class="mail">
        <span class="mrow">
          <span class="mt">{{ m.subject || '(no subject)' }}</span>
          <span class="mono faint tiny">{{ shortDate(m.date) }}</span>
        </span>
        <span v-if="m.from || m.preview" class="ms">{{ short(m.from) }}<template v-if="m.from && m.preview"> · </template>{{ m.preview }}</span>
      </NuxtLink>
    </section>

    <section v-if="related?.files.length" class="block">
      <div class="eyebrow">Files</div>
      <a v-for="f in related.files.slice(0, 4)" :key="`${f.account}-${f.uid}-${f.index}`" class="file" :href="fileLink(f)" :download="f.name">
        <span class="kind mono" aria-hidden="true">{{ fileKindLabel(f.name, f.mime) }}</span>
        <span class="fname">{{ f.name }}</span>
      </a>
    </section>

    <section v-if="event.description.trim()" class="block">
      <div class="eyebrow">Description</div>
      <p class="notes">{{ event.description.trim() }}</p>
    </section>
  </aside>
</template>

<style scoped>
.details {
  flex: 0 1 300px;
  min-width: 260px;
  display: flex;
  flex-direction: column;
  gap: 22px;
  padding: 24px 22px;
  border-left: 1px solid var(--line);
  background: var(--bg);
  overflow-y: auto;
}
.head {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.calname {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--muted);
}
.sq {
  width: 8px;
  height: 8px;
  border-radius: 3px;
  background: var(--c);
}
.grow {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.icon-btn.small {
  width: 28px;
  height: 28px;
}
h2 {
  margin: 0;
  font-size: 26px;
  line-height: 1.15;
  letter-spacing: -0.01em;
  overflow-wrap: anywhere;
}
.facts {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 4px;
  font-size: 13px;
  color: var(--muted);
}
.facts span {
  display: flex;
  align-items: center;
  gap: 8px;
}
.acts {
  display: flex;
  gap: 8px;
  margin-top: 8px;
}
.act {
  display: inline-flex;
  align-items: center;
  height: 34px;
  padding: 0 14px;
  border: 1px solid var(--line2);
  border-radius: 9px;
  background: transparent;
  color: var(--text);
  font: inherit;
  font-size: 13.5px;
  text-decoration: none;
  cursor: pointer;
}
.act:hover:not(:disabled) {
  background: var(--hover);
}
.act.primary {
  border: 0;
  background: var(--accent);
  color: var(--on-accent);
  font-weight: 500;
}
.act.primary:hover {
  background: var(--accent);
  filter: brightness(1.08);
}
.act.chosen {
  border-color: var(--accent);
  background: color-mix(in oklab, var(--accent) 16%, transparent);
}
.faint {
  color: var(--faint);
}
.small {
  margin: 0;
  font-size: 12.5px;
}
.tiny {
  flex: 0 0 auto;
  font-size: 11.5px;
}
.rsvp {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  padding: 12px;
  border-radius: 12px;
  border: 1px solid var(--line2);
  background: var(--raise);
}
.rsvp .q {
  flex: 1 0 100%;
  font-size: 12.5px;
  color: var(--muted);
}
.rsvp .act {
  height: 30px;
  padding: 0 10px;
  font-size: 12.5px;
}
.block {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.ic {
  display: flex;
  align-items: center;
  gap: 6px;
}
.person {
  display: flex;
  align-items: center;
  gap: 10px;
  color: var(--text);
  text-decoration: none;
  font-size: 13.5px;
}
.person:hover .grow {
  text-decoration: underline;
}
.av {
  flex: 0 0 28px;
  height: 28px;
  border-radius: 50%;
  display: grid;
  place-items: center;
  font-size: 10.5px;
  font-weight: 600;
  background: color-mix(in oklab, var(--pc, var(--faint)) 20%, transparent);
  color: var(--pc, var(--faint));
}
.av.me {
  background: var(--raise);
  color: var(--text);
  border: 1px solid var(--line2);
}
.st {
  font-size: 12px;
  color: var(--faint);
}
.st.going {
  color: var(--ok);
}
.st.declined {
  color: var(--danger);
}
.mail {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 10px;
  margin: 0 -10px;
  border-radius: 10px;
  color: var(--text);
  text-decoration: none;
}
.mail:hover,
.file:hover {
  background: var(--hover);
}
.mrow {
  display: flex;
  justify-content: space-between;
  gap: 8px;
}
.mt {
  min-width: 0;
  font-size: 13.5px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ms {
  font-size: 12.5px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.mail.note {
  border: 1px solid var(--line);
  margin: 0;
  background: var(--panel);
}
.accent-l {
  color: var(--accent);
  font-size: 12px;
}
.act.take {
  align-self: flex-start;
  gap: 6px;
  height: 32px;
  padding: 0 12px;
  font-size: 13px;
}
.act.take.dashed {
  border-style: dashed;
  color: var(--muted);
}
.file {
  display: inline-flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  border: 1px solid var(--line2);
  border-radius: 10px;
  color: var(--text);
  text-decoration: none;
  font-size: 13px;
}
.kind {
  font-size: 10px;
  padding: 3px 5px;
  border-radius: 4px;
  background: color-mix(in oklab, var(--accent) 16%, transparent);
  color: var(--accent);
}
.fname {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.notes {
  margin: 0;
  font-size: 13.5px;
  line-height: 1.55;
  color: var(--muted);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
@media (max-width: 960px) {
  .details {
    position: fixed;
    z-index: 30;
    right: 0;
    top: 0;
    bottom: 0;
    width: min(360px, 100vw);
    box-shadow: -20px 0 60px -20px rgb(0 0 0 / 0.5);
  }
}
</style>
