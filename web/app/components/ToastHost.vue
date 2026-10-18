<script setup lang="ts">
const { toasts, dismiss } = useToasts()
</script>

<template>
  <div class="toasts" role="status" aria-live="polite">
    <TransitionGroup name="toast">
      <div v-for="t in toasts" :key="t.id" class="toast" :class="t.tone">
        <HIcon v-if="t.tone === 'error'" name="alert" :size="16" />
        <span class="msg">{{ t.message }}</span>
        <button
          v-if="t.action"
          type="button"
          class="act"
          @click="
            () => {
              t.action!.run()
              dismiss(t.id)
            }
          "
        >
          {{ t.action.label }}
        </button>
        <button type="button" class="close" aria-label="Dismiss" @click="dismiss(t.id)">×</button>
      </div>
    </TransitionGroup>
  </div>
</template>

<style scoped>
.toasts {
  position: fixed;
  left: 50%;
  bottom: 24px;
  z-index: 50;
  transform: translateX(-50%);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  pointer-events: none;
}
.toast {
  pointer-events: auto;
  display: flex;
  align-items: center;
  gap: 12px;
  max-width: min(520px, calc(100vw - 32px));
  padding: 10px 10px 10px 16px;
  border-radius: 12px;
  border: 1px solid var(--line2);
  background: var(--raise);
  color: var(--text);
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.35);
  font-size: 13.5px;
}
.toast.error {
  color: var(--danger);
}
.toast.error .msg {
  color: var(--text);
}
.msg {
  flex: 1;
}
.act {
  border: 0;
  background: none;
  color: var(--accent);
  font-weight: 500;
  cursor: pointer;
  padding: 4px 6px;
}
.close {
  width: 28px;
  height: 28px;
  border: 0;
  border-radius: 7px;
  background: transparent;
  color: var(--faint);
  font-size: 18px;
  line-height: 1;
  cursor: pointer;
}
.close:hover {
  background: var(--hover);
  color: var(--text);
}
.toast-enter-active,
.toast-leave-active {
  transition: all 0.2s ease;
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateY(8px);
}
@media (max-width: 720px) {
  .toasts {
    bottom: 76px;
  }
}
</style>
