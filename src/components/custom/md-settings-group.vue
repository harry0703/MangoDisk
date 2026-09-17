<script setup lang="ts">
import { Card } from '@/components/ui/card';

defineProps<{ title?: string; plain?: boolean }>();
</script>

<template>
  <section class="settings-section" :class="{ 'settings-section--plain': plain }">
    <h2 v-if="title">{{ title }}</h2>
    <component :is="plain ? 'div' : Card" class="settings-list"><slot /></component>
  </section>
</template>

<style scoped>
@reference "@assets/main.css";
.settings-section > h2 {
  margin: 1px 0 6px 2px;
  font-size: var(--font-content-body);
  font-weight: 600;
  @apply text-muted-foreground;
}
.settings-list {
  gap: 0;
  overflow: hidden;
  border-radius: 10px;
  @apply border-border/70 bg-card shadow-none;
}
/* Inset dividers belong to the group, including feature-owned row wrappers.
   Paint them without borders so row sizing and the full hover surface stay
   consistent. Opacity also keeps the line subtle on older WebKit versions. */
.settings-section:not(.settings-section--plain) > .settings-list > :deep(* + *) {
  position: relative;
}
.settings-section:not(.settings-section--plain) > .settings-list > :deep(* + *)::before {
  content: '';
  position: absolute;
  z-index: 1;
  top: 0;
  right: 14px;
  left: 14px;
  height: 1px;
  background: var(--border);
  opacity: 0.4;
  pointer-events: none;
}
/* Dialog sections share the compact heading and unframed field layout. */
.settings-section--plain > h2 {
  margin: 0 0 8px;
  font-size: var(--font-content-secondary);
  font-weight: 500;
}
.settings-section--plain > .settings-list {
  display: grid;
  gap: 8px;
  overflow: visible;
  border: 0;
  border-radius: 0;
  background: transparent;
}
</style>
