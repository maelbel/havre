<script setup lang="ts">
import type { Task, TaskList } from '~/composables/useTasks'

// One task in a list: the checkbox, the title, and what it's linked to
// (due day, email, meeting, person) — the row of the design.
const props = defineProps<{
  task: Task
  list?: TaskList
  selected?: boolean
  /** Smaller, for Today, People and the Agenda. */
  compact?: boolean
  /** Hide the list name (when showing one list). */
  hideList?: boolean
}>()
const emit = defineEmits<{ toggle: []; open: [] }>()

const done = computed(() => !!props.task.done_at)
const late = computed(() => isLate(props.task))
const dueClass = computed(() => {
  if (late.value) return 'late'
  if (props.task.due != null && props.task.due === localDay() && !done.value) return 'soon'
  return ''
})
const who = computed(() => props.task.person?.name || props.task.person?.email || '')
</script>

<template>
  <div class="task" :class="{ selected, compact, done }">
    <button
      type="button"
      class="check"
      :aria-label="`${done ? 'Mark not done' : 'Mark done'}: ${task.title}`"
      :aria-pressed="done"
      :disabled="task.read_only"
      @click="emit('toggle')"
    >
      <HIcon v-if="done" name="check" :size="compact ? 10 : 12" :stroke-width="3" />
    </button>
    <button type="button" class="body" @click="emit('open')">
      <span class="title">{{ task.title }}</span>
      <span class="meta">
        <span v-if="task.due != null || !compact" class="due" :class="dueClass">{{ dueLabel(task) }}</span>
        <span v-if="task.mail" class="chip">
          <HIcon name="mail" :size="12" class="c-work" />
          {{ task.mail.subject || '(no subject)' }}
        </span>
        <span v-if="task.event" class="chip">
          <HIcon name="calendar" :size="12" class="c-lyon" />
          {{ task.event.title }}
        </span>
        <span v-if="task.person && !compact" class="chip">
          <span class="mini-avatar" :style="{ background: personColor(task.person.email) }">{{ initials(who) }}</span>
          {{ who }}
        </span>
        <span v-if="list && !hideList" class="m-list">
          <span class="dot" :style="{ background: list.color }" aria-hidden="true" />
          {{ list.name }}
        </span>
      </span>
    </button>
    <span v-if="list && !hideList" class="list">
      <span class="dot" :style="{ background: list.color }" aria-hidden="true" />
      {{ list.name }}
    </span>
  </div>
</template>

<style scoped>
.task {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  padding: 9px 8px;
  border-radius: 10px;
}
.task:hover {
  background: var(--hover);
}
.task.selected {
  background: var(--sel);
}
.check {
  flex: 0 0 20px;
  height: 20px;
  margin-top: 1px;
  padding: 0;
  border-radius: 6px;
  border: 1.5px solid var(--line2);
  background: transparent;
  color: var(--on-accent);
  display: grid;
  place-items: center;
  cursor: pointer;
}
.check:hover {
  border-color: var(--accent);
}
.done .check {
  background: var(--accent);
  border-color: var(--accent);
}
.check:disabled {
  cursor: default;
  opacity: 0.6;
}
.body {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 0;
  border: 0;
  background: transparent;
  text-align: left;
  cursor: pointer;
}
.title {
  font-size: 14px;
  color: var(--text);
  overflow-wrap: anywhere;
}
.done .title {
  color: var(--faint);
  text-decoration: line-through;
}
.meta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px 10px;
  font-size: 12px;
  color: var(--faint);
}
.meta:empty {
  display: none;
}
.due.soon {
  color: var(--accent);
}
.due.late {
  color: var(--danger);
}
.chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
  max-width: 260px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.c-work {
  color: var(--c-work);
  flex: none;
}
.c-lyon {
  color: var(--c-lyon);
  flex: none;
}
.mini-avatar {
  width: 14px;
  height: 14px;
  border-radius: 50%;
  display: inline-grid;
  place-items: center;
  font-size: 7.5px;
  font-weight: 600;
  color: #1b1b1b;
}
.list {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  margin-top: 2px;
  font-size: 12px;
  color: var(--faint);
  white-space: nowrap;
}
.dot {
  width: 7px;
  height: 7px;
  border-radius: 2px;
}
.m-list {
  display: none;
  align-items: center;
  gap: 5px;
}
.compact {
  gap: 10px;
  padding: 7px 8px;
}
.compact .check {
  flex-basis: 16px;
  height: 16px;
  border-radius: 5px;
}
.compact .title {
  font-size: 13.5px;
}
@media (max-width: 720px) {
  .task {
    padding: 6px 4px;
  }
  /* A 44px touch target around the box. */
  .check {
    flex-basis: 22px;
    height: 22px;
    margin: 0 0 0 2px;
    border-radius: 7px;
    position: relative;
  }
  .check::before {
    content: '';
    position: absolute;
    inset: -11px;
  }
  .list {
    display: none;
  }
  .m-list {
    display: inline-flex;
  }
}
</style>
