<script setup lang="ts">
import type { NoteBlock } from '~/composables/useNotes'

// A note, read: each Markdown block drawn as itself. To-dos can be ticked
// here, and turned into a task ("Make a task") when the Tasks app is on.
const props = defineProps<{
  blocks: NoteBlock[]
  /** Show "Make a task" / "In Tasks". */
  tasks: boolean
  /** The line being saved (its controls wait). */
  busy: number | null
}>()
const emit = defineEmits<{ tick: [line: number, done: boolean]; task: [line: number] }>()

// Consecutive bullets / numbered lines go in one list (`<ul>` / `<ol>`),
// so screen readers announce "list, 3 items".
type Group =
  | { kind: 'ul'; items: Extract<NoteBlock, { kind: 'bullet' }>[] }
  | { kind: 'ol'; items: Extract<NoteBlock, { kind: 'numbered' }>[] }
  | { kind: 'todos'; items: Extract<NoteBlock, { kind: 'todo' }>[] }
  | { kind: 'one'; block: NoteBlock }
const groups = computed(() => {
  const out: Group[] = []
  for (const b of props.blocks) {
    const last = out[out.length - 1]
    // A bullet not written yet (`- ` in the meeting template) shows nothing.
    if (b.kind === 'bullet' && !b.spans.length) continue
    if (b.kind === 'bullet') {
      if (last?.kind === 'ul') last.items.push(b)
      else out.push({ kind: 'ul', items: [b] })
    } else if (b.kind === 'numbered') {
      if (last?.kind === 'ol') last.items.push(b)
      else out.push({ kind: 'ol', items: [b] })
    } else if (b.kind === 'todo') {
      if (last?.kind === 'todos') last.items.push(b)
      else out.push({ kind: 'todos', items: [b] })
    } else out.push({ kind: 'one', block: b })
  }
  return out
})
</script>

<template>
  <div class="note-body">
    <template v-for="(g, i) in groups" :key="i">
      <ul v-if="g.kind === 'ul'" class="bullets">
        <li v-for="b in g.items" :key="b.line" :style="{ marginLeft: `${b.depth * 20}px` }">
          <NoteSpans :spans="b.spans" />
        </li>
      </ul>
      <ol v-else-if="g.kind === 'ol'" class="numbers" :start="g.items[0]?.number">
        <li v-for="b in g.items" :key="b.line"><NoteSpans :spans="b.spans" /></li>
      </ol>
      <ul v-else-if="g.kind === 'todos'" class="todos">
        <li v-for="b in g.items" :key="b.line" class="todo" :class="{ done: b.done }">
          <button
            type="button"
            class="box"
            :aria-pressed="b.done"
            :aria-label="`${b.done ? 'Mark not done' : 'Mark done'}: ${b.text || 'empty to-do'}`"
            :disabled="busy === b.line"
            @click="emit('tick', b.line, !b.done)"
          >
            <HIcon v-if="b.done" name="check" :size="11" :stroke-width="3" />
          </button>
          <span class="t"><NoteSpans :spans="b.spans" /></span>
          <template v-if="props.tasks && b.text">
            <NuxtLink v-if="b.task" :to="{ path: '/tasks', query: { task: b.task } }" class="in-tasks">
              <HIcon name="task" :size="12" />In Tasks
            </NuxtLink>
            <button
              v-else-if="!b.done"
              type="button"
              class="make"
              :aria-label="`Make a task: ${b.text}`"
              :disabled="busy === b.line"
              @click="emit('task', b.line)"
            >
              Make a task
            </button>
          </template>
        </li>
      </ul>
      <template v-else>
        <h2 v-if="g.block.kind === 'heading' && g.block.level === 1" class="h1"><NoteSpans :spans="g.block.spans" /></h2>
        <h3 v-else-if="g.block.kind === 'heading'" class="h" :class="`l${g.block.level}`">
          <NoteSpans :spans="g.block.spans" />
        </h3>
        <p v-else-if="g.block.kind === 'paragraph'"><NoteSpans :spans="g.block.spans" /></p>
        <blockquote v-else-if="g.block.kind === 'quote'"><NoteSpans :spans="g.block.spans" /></blockquote>
        <pre v-else-if="g.block.kind === 'code'" class="mono">{{ g.block.text }}</pre>
        <hr v-else-if="g.block.kind === 'rule'" />
      </template>
    </template>
  </div>
</template>

<style scoped>
.note-body {
  display: flex;
  flex-direction: column;
  gap: 8px;
  font-size: 15px;
  line-height: 1.6;
}
p {
  margin: 0;
}
.h1 {
  margin: 14px 0 0;
  font-size: 18px;
  font-weight: 600;
}
.h {
  margin: 14px 0 0;
  font-size: 15px;
  font-weight: 600;
}
.h.l3 {
  font-size: 14px;
  color: var(--muted);
}
ul,
ol {
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.bullets {
  list-style: none;
}
.bullets li {
  position: relative;
  padding-left: 19px;
}
.bullets li::before {
  content: '';
  position: absolute;
  left: 4px;
  top: 10px;
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--faint);
}
.numbers {
  padding-left: 22px;
}
.numbers li::marker {
  color: var(--faint);
  font-family: var(--font-mono);
  font-size: 13px;
}
.todos {
  list-style: none;
  gap: 2px;
}
.todo {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 3px 6px;
  margin: 0 -6px;
  border-radius: 8px;
}
.todo:hover {
  background: var(--hover);
}
.box {
  flex: 0 0 18px;
  height: 18px;
  margin-top: 3px;
  padding: 0;
  border-radius: 5px;
  border: 1.5px solid var(--line2);
  background: transparent;
  color: var(--on-accent);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}
.done .box {
  background: var(--accent);
  border-color: var(--accent);
}
.t {
  flex: 1;
  min-width: 0;
}
.done .t {
  color: var(--faint);
  text-decoration: line-through;
}
.in-tasks {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  margin-top: 2px;
  font-size: 12px;
  color: var(--faint);
  text-decoration: none;
}
.in-tasks:hover {
  color: var(--text);
}
.make {
  flex: none;
  height: 26px;
  padding: 0 8px;
  border: 1px solid var(--line2);
  border-radius: 7px;
  background: transparent;
  color: var(--muted);
  font-size: 12px;
  cursor: pointer;
}
.make:hover {
  background: var(--hover);
  color: var(--text);
}
blockquote {
  margin: 0;
  padding: 2px 0 2px 14px;
  border-left: 2px solid var(--line2);
  color: var(--muted);
}
pre {
  margin: 0;
  padding: 12px 14px;
  border-radius: 10px;
  border: 1px solid var(--line);
  background: var(--panel);
  font-size: 13px;
  white-space: pre-wrap;
  overflow-x: auto;
}
hr {
  width: 100%;
  border: 0;
  border-top: 1px solid var(--line);
  margin: 8px 0;
}
@media (max-width: 720px) {
  /* Bigger targets on a phone. */
  .box {
    flex-basis: 22px;
    height: 22px;
    margin-top: 1px;
  }
  .make {
    height: 32px;
  }
}
</style>
