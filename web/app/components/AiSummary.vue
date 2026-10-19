<script setup lang="ts">
// A summary of the open message, written by the AI as you watch.
const props = defineProps<{ account: number; folder: string; uid: number }>()
const emit = defineEmits<{ close: []; draft: [] }>()
const { status: ai } = useAiStatus()

const text = ref('')
const warning = ref<string | null>(null)
const error = ref<string | null>(null)
const busy = ref(false)
let abort: AbortController | null = null

async function run(refresh = false) {
  abort?.abort()
  abort = new AbortController()
  busy.value = true
  error.value = null
  warning.value = null
  text.value = ''
  try {
    await streamAi(
      '/api/ai/summarize',
      { account: props.account, folder: props.folder, uid: props.uid, refresh },
      (t) => {
        const split = splitWarning(t)
        text.value = split.text
        warning.value = split.warning
      },
      abort.signal,
    )
  } catch (err) {
    if ((err as Error).name !== 'AbortError') error.value = (err as Error).message
  } finally {
    busy.value = false
  }
}
watch(() => [props.account, props.folder, props.uid], () => run(), { immediate: true })
onBeforeUnmount(() => abort?.abort())

/** "- a" lines as a list; "→ do this" as the action line. */
const lines = computed(() =>
  text.value
    .split('\n')
    .map((l) => l.trim())
    .filter(Boolean)
    .map((l) => ({ action: l.startsWith('→'), text: l.replace(/^[-•*]\s*|^→\s*/, '') })),
)
</script>

<template>
  <section class="summary" aria-label="Summary" :aria-busy="busy">
    <header>
      <HIcon name="sparkle" :size="14" class="spark" />
      <span class="title">Summary</span>
      <span class="model mono">{{ ai.local ? 'local model' : ai.model }}</span>
      <span class="spacer" />
      <UButton color="neutral" variant="ghost" size="sm" type="button" @click="emit('draft')">
        Draft a reply
      </UButton>
      <UButton
        icon="i-havre-refresh"
        color="neutral"
        variant="ghost"
        size="sm"
        square
        type="button"
        title="Write it again"
        aria-label="Summarize again"
        :disabled="busy"
        @click="run(true)"
      />
      <UButton
        icon="i-havre-close"
        color="neutral"
        variant="ghost"
        size="sm"
        square
        type="button"
        aria-label="Hide summary"
        @click="emit('close')"
      />
    </header>
    <p v-if="error" class="err">{{ error }}</p>
    <ul v-else-if="lines.length">
      <li v-for="(l, i) in lines" :key="i" :class="{ action: l.action }">{{ l.text }}</li>
    </ul>
    <p v-else class="wait"><span class="dot" /><span class="dot" /><span class="dot" /></p>
    <p v-if="warning" class="err">{{ warning }}</p>
  </section>
</template>

<style scoped>
.summary {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px 18px 16px;
  border-radius: 14px;
  border: 1px solid var(--line);
  background: var(--bg);
}
.spark {
  color: var(--accent);
}
.title {
  font-size: 13px;
  font-weight: 600;
}
.model {
  font-size: 11px;
  color: var(--faint);
}
header {
  display: flex;
  align-items: center;
  gap: 8px;
}
.spacer {
  flex: 1;
}
ul {
  margin: 0;
  padding-left: 18px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 14px;
  line-height: 1.55;
}
li.action {
  list-style: '→ ';
  font-weight: 500;
}
.err {
  margin: 8px 0 0;
  font-size: 13px;
  color: var(--danger);
}
.wait {
  display: flex;
  gap: 4px;
  margin: 10px 0 2px;
}
.dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent);
  animation: blink 1s infinite;
}
.dot:nth-child(2) {
  animation-delay: 0.2s;
}
.dot:nth-child(3) {
  animation-delay: 0.4s;
}
@keyframes blink {
  50% {
    opacity: 0.25;
  }
}
</style>
