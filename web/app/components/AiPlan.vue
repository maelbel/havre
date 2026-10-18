<script setup lang="ts">
import type { AiProposal } from '~/composables/useAi'

// What the assistant proposes to do, as numbered steps to review: the
// event (and whether that time is free), the move, the email, the task, the
// note. Nothing here
// runs anything — the confirm buttons are the caller's (Ctrl+K's bar, the
// AI app's), so the same steps look the same everywhere.
const props = defineProps<{
  proposals: AiProposal[]
  /** Positions of the proposals already done. */
  done: number[]
  /** Dismissed: shown dimmed. */
  off?: boolean
}>()

// Is that time free? Checked once per proposal, when it shows up.
const conflicts = ref(new Map<AiProposal, string[] | null>())
watch(
  () => props.proposals,
  async (list) => {
    for (const p of list) {
      if ((p.kind !== 'event' && p.kind !== 'move') || conflicts.value.has(p)) continue
      conflicts.value.set(p, null)
      conflicts.value.set(p, await aiConflicts(p))
    }
  },
  { immediate: true },
)
// Each kind's icon and colour (the colour its app uses).
const LOOK: Record<AiProposal['kind'], { icon: 'mail' | 'calendar' | 'task' | 'note'; tint: string }> = {
  email: { icon: 'mail', tint: 'var(--c-work)' },
  event: { icon: 'calendar', tint: 'var(--c-lyon)' },
  move: { icon: 'calendar', tint: 'var(--c-lyon)' },
  task: { icon: 'task', tint: 'var(--c-hol)' },
  note: { icon: 'note', tint: 'var(--c-personal)' },
}
const who = (p: { name: string | null; email: string }) => p.name || p.email
</script>

<template>
  <section class="plan" :class="{ off }" aria-label="Proposed actions">
    <div class="plan-head">
      <span class="eyebrow">{{ proposals.length }} {{ proposals.length === 1 ? 'action' : 'actions' }} · review before running</span>
      <span class="faint small">{{ aiProposalApps(proposals) }}</span>
    </div>
    <div v-for="(p, k) in proposals" :key="k" class="step" :class="{ done: done.includes(k) }">
      <span class="sicon" :style="{ '--t': LOOK[p.kind].tint }" aria-hidden="true">
        <HIcon :name="LOOK[p.kind].icon" :size="17" />
      </span>
      <div class="sbody">
        <div class="stitle">{{ aiProposalTitle(p) }}</div>
        <template v-if="p.kind === 'move'">
          <div class="times mono">
            <span class="old">{{ aiDayTime(p.from_start) }}</span>
            <span class="faint" aria-hidden="true">→</span>
            <span class="sr-only">to</span>
            <span>{{ aiDayTime(p.start) }}</span>
          </div>
        </template>
        <template v-else-if="p.kind === 'event'">
          <div class="times mono">{{ aiProposalSpan(p) }}<template v-if="p.location"> · {{ p.location }}</template></div>
          <div v-if="p.guests.length" class="chips">
            <span v-for="g in p.guests" :key="g" class="chip">{{ g }}</span>
          </div>
        </template>
        <template v-else-if="p.kind === 'task'">
          <div class="times mono">{{ aiTaskWhen(p) }}</div>
          <div v-if="p.person || p.event || p.from_note" class="chips">
            <span v-if="p.person" class="chip"><HIcon name="people" :size="11" /> {{ who(p.person) }}</span>
            <span v-if="p.event" class="chip"><HIcon name="calendar" :size="11" /> {{ p.event.title }}</span>
            <span v-if="p.from_note" class="chip"><HIcon name="note" :size="11" /> Ticks the line in “{{ p.from_note.title }}”</span>
          </div>
          <div v-if="p.notes" class="mailbox"><div class="mbody">{{ p.notes }}</div></div>
        </template>
        <template v-else-if="p.kind === 'note'">
          <div v-if="p.people.length || p.event" class="chips">
            <span v-for="x in p.people" :key="x.email" class="chip" :title="x.email"><HIcon name="people" :size="11" /> {{ who(x) }}</span>
            <span v-if="p.event" class="chip"><HIcon name="calendar" :size="11" /> {{ p.event.title }}</span>
          </div>
          <!-- The Markdown as text: shown, never run as HTML. -->
          <div class="mailbox"><div class="mbody">{{ p.body }}</div></div>
        </template>
        <template v-if="p.kind === 'event' || p.kind === 'move'">
          <div v-if="conflicts.get(p) && conflicts.get(p)!.length" class="conflict">
            <HIcon name="alert" :size="13" /> Overlaps {{ conflicts.get(p)!.map((t) => `“${t}”`).join(', ') }}
          </div>
          <div v-else-if="conflicts.get(p)" class="free"><HIcon name="check" :size="13" /> No conflict at that time</div>
        </template>
        <template v-else-if="p.kind === 'email'">
          <div class="chips">
            <span v-for="r in aiRecipients(p)" :key="r" class="chip" :title="r">{{ nameIn(r) }}</span>
          </div>
          <div class="mailbox">
            <div class="msub">{{ p.subject }}</div>
            <div class="mbody">{{ p.body }}</div>
          </div>
        </template>
      </div>
      <span v-if="done.includes(k)" class="ok"><HIcon name="check" :size="14" /> Done</span>
    </div>
  </section>
</template>

<style scoped>
.plan {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin-top: 10px;
}
.plan.off {
  opacity: 0.55;
}
.plan-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
}
.small {
  font-size: 12px;
}
.faint {
  color: var(--faint);
}
.step {
  display: flex;
  gap: 14px;
  padding: 14px;
  border-radius: 12px;
  border: 1px solid var(--line);
  background: var(--panel);
}
.step.done {
  opacity: 0.7;
}
.sicon {
  flex: 0 0 32px;
  height: 32px;
  border-radius: 9px;
  display: flex;
  align-items: center;
  justify-content: center;
}
.sicon {
  background: color-mix(in oklab, var(--t) 18%, transparent);
  color: var(--t);
}
.sbody {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 14px;
}
.stitle {
  font-weight: 500;
}
.times {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  font-size: 12.5px;
}
.old {
  color: var(--faint);
  text-decoration: line-through;
}
.free,
.conflict {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12.5px;
  color: var(--ok);
}
.conflict {
  color: var(--danger);
}
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  font-size: 12.5px;
}
.chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
  border-radius: 999px;
  border: 1px solid var(--line2);
}
.mailbox {
  padding: 10px 12px;
  border-radius: 9px;
  border: 1px dashed var(--line2);
  font-size: 13.5px;
  line-height: 1.55;
  white-space: pre-wrap;
}
.msub {
  font-weight: 500;
  margin-bottom: 4px;
}
.mbody {
  max-height: 10em;
  overflow: auto;
}
.ok {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  color: var(--ok);
  font-size: 13px;
}
</style>
