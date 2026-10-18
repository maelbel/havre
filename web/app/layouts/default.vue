<template>
  <div class="shell">
    <DockRail />
    <div class="content">
      <slot />
    </div>
    <ComposeWindow />
    <CommandPalette />
    <ToastHost />
    <TaskReminders />
  </div>
</template>

<style scoped>
.shell {
  display: flex;
  height: 100%;
}
.content {
  flex: 1;
  min-width: 0;
  display: flex;
  overflow: hidden;
}
@media (max-width: 720px) {
  .shell {
    flex-direction: column;
  }
  .content {
    flex: 1;
    min-height: 0;
  }
}
</style>
