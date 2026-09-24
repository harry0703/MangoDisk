<script setup lang="ts">
withDefaults(defineProps<{ size?: number }>(), {
  size: 24,
});
</script>

<template>
  <!-- A layered, Yaru-inspired folder stays recognizable without depending on the host icon theme. -->
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
    data-icon="linux-folder"
  >
    <path
      class="folder-back"
      d="M3.25 5.25A2.25 2.25 0 0 1 5.5 3h3.38c.7 0 1.36.32 1.78.87l1.22 1.58h6.62a2.25 2.25 0 0 1 2.25 2.25v2.05H3.25z"
    />
    <path
      class="folder-front"
      d="M2.25 8.25A2.25 2.25 0 0 1 4.5 6h15a2.25 2.25 0 0 1 2.25 2.25v9.5A3.25 3.25 0 0 1 18.5 21h-13a3.25 3.25 0 0 1-3.25-3.25z"
    />
    <path class="folder-highlight" d="M4.5 7.4h15c.7 0 1.32.32 1.73.82H2.77A2.24 2.24 0 0 1 4.5 7.4" />
  </svg>
</template>

<style scoped>
.folder-back {
  fill: var(--linux-folder-back);
}

.folder-front {
  fill: var(--linux-folder-front);
}

.folder-highlight {
  fill: var(--linux-folder-highlight);
  opacity: 0.22;
}
</style>
