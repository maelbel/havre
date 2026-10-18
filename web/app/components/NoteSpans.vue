<script setup lang="ts">
import type { NoteSpan } from '~/composables/useNotes'

// The inside of one line of a note: text, **bold**, *italic*, `code`,
// links. Drawn as Vue elements (never `v-html`), so a note's text can't
// become markup. Links only ever point to http(s) or mailto (the server
// checks) and open in a new tab.
defineProps<{ spans: NoteSpan[] }>()
</script>

<template>
  <template v-for="(s, i) in spans" :key="i">
    <strong v-if="s.kind === 'bold'">{{ s.text }}</strong>
    <em v-else-if="s.kind === 'italic'">{{ s.text }}</em>
    <code v-else-if="s.kind === 'code'" class="mono">{{ s.text }}</code>
    <a v-else-if="s.kind === 'link'" :href="s.href" target="_blank" rel="noopener noreferrer">{{ s.text }}</a>
    <template v-else>{{ s.text }}</template>
  </template>
</template>

<style scoped>
code {
  padding: 1px 5px;
  border-radius: 5px;
  background: var(--raise);
  border: 1px solid var(--line);
  font-size: 0.88em;
}
</style>
