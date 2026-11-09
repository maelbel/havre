<script setup lang="ts">
// A person's picture: their photo from an address book when there is one,
// else their initials on a colour that stays the same for them.
const props = defineProps<{
  person: { email: string; name: string; photo?: boolean; phones?: { number: string }[] }
  size?: number
}>()
const failed = ref(false)
// Another person in the same place (the list scrolls, the page changes): try again.
watch(
  () => props.person.email,
  () => (failed.value = false),
)
const px = computed(() => props.size ?? 34)
</script>

<template>
  <span
    class="person-avatar"
    :style="{ width: `${px}px`, height: `${px}px`, flexBasis: `${px}px`, fontSize: `${Math.round(px * 0.36)}px`, background: personColor(person.email) }"
    aria-hidden="true"
  >
    <img v-if="person.photo && !failed" :src="personPhoto(person)" alt="" loading="lazy" @error="failed = true" />
    <template v-else>{{ initials(personName(person)) }}</template>
  </span>
</template>

<style scoped>
.person-avatar {
  flex-grow: 0;
  flex-shrink: 0;
  border-radius: var(--radius);
  display: grid;
  place-items: center;
  overflow: hidden;
  font-weight: 600;
  color: #1b1b1b;
}
img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
</style>
