<script setup lang="ts">
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/ui';

withDefaults(
  defineProps<{
    title: string;
    value: string;
    description?: string;
    valueDetail?: string;
    badge?: string;
    badgeTone?: 'accent' | 'neutral' | 'positive' | 'warning';
    expandable?: boolean;
    expanded?: boolean;
    valueTone?: 'default' | 'warning';
    disclosureLabel?: string;
    disclosureDisabled?: boolean;
  }>(),
  {
    description: undefined,
    valueDetail: undefined,
    badge: undefined,
    badgeTone: 'neutral',
    expandable: false,
    expanded: false,
    valueTone: 'default',
    disclosureLabel: undefined,
    disclosureDisabled: false,
  }
);
const emit = defineEmits<{ toggle: [] }>();
</script>

<template>
  <span
    class="result-item-content"
    :class="{ expandable, 'has-actions': $slots.actions, 'has-disclosure': disclosureLabel }"
  >
    <!-- A sibling hit target keeps inline actions out of a nested button. -->
    <button
      v-if="disclosureLabel"
      type="button"
      class="result-item-disclosure"
      :aria-label="disclosureLabel"
      :aria-expanded="expandable ? expanded : undefined"
      :disabled="disclosureDisabled"
      @click="emit('toggle')"
    />
    <span class="result-item-icon"><slot name="icon" /></span>
    <span class="result-item-copy">
      <span class="result-item-title">
        <strong class="md-result-primary">{{ title }}</strong>
        <em v-if="badge" class="result-item-badge" :class="badgeTone">{{ badge }}</em>
      </span>
      <small v-if="description" class="result-item-description">{{ description }}</small>
    </span>
    <span v-if="$slots.actions" class="result-item-actions"><slot name="actions" /></span>
    <span class="result-item-value" :class="valueTone">
      <strong class="md-result-primary">{{ value }}</strong>
      <small v-if="valueDetail">{{ valueDetail }}</small>
    </span>
    <span v-if="expandable" class="result-item-expand">
      <MdIcon :name="ICON_NAMES.chevronDown" :size="17" :class="{ expanded }" />
    </span>
  </span>
</template>

<style scoped>
@reference "@assets/main.css";

.result-item-content {
  display: grid;
  min-width: 0;
  grid-template-columns: 30px minmax(0, 1fr) auto;
  align-items: center;
  gap: 9px;
  padding: 5px 6px;
  text-align: left;
}

.result-item-content.expandable {
  grid-template-columns: 30px minmax(0, 1fr) auto 22px;
}

.result-item-content.has-actions {
  grid-template-columns: 30px minmax(0, 1fr) auto auto;
}

.result-item-content.has-actions.expandable {
  grid-template-columns: 30px minmax(0, 1fr) auto auto 22px;
}

.result-item-content.has-disclosure {
  position: relative;
}

.has-disclosure .result-item-disclosure {
  position: absolute;
  inset: 0;
  z-index: 1;
  border-radius: var(--radius);
  cursor: pointer;
}

.result-item-disclosure:disabled {
  cursor: default;
}

.result-item-disclosure:focus-visible {
  @apply outline-none ring-2 ring-ring/35;
}

.result-item-actions {
  position: relative;
  z-index: 2;
  display: flex;
  align-items: center;
}

.result-item-icon {
  @apply text-muted-foreground;
  display: grid;
  width: 30px;
  height: 36px;
  flex: none;
  place-items: center;
  contain: layout;
}

.result-item-icon :deep(svg) {
  display: block;
}

.result-item-copy,
.result-item-value {
  display: flex;
  min-width: 0;
  flex-direction: column;
}

.result-item-copy {
  gap: 2px;
}

.result-item-title {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 7px;
}

.result-item-title strong,
.result-item-description {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.result-item-title strong {
  font-size: 13px;
}

.result-item-description,
.result-item-value small {
  @apply text-muted-foreground;
  font-size: 10px;
}

.result-item-badge {
  display: inline-flex;
  flex: none;
  align-items: center;
  border-radius: 999px;
  padding: 2px 6px;
  font-size: 9px;
  font-style: normal;
  font-weight: 500;
  white-space: nowrap;
}

.result-item-badge.positive {
  @apply text-success-foreground;
  background: var(--surface-success-subtle);
}

.result-item-badge.accent {
  background: color-mix(in srgb, var(--primary) 13%, transparent);
  color: var(--primary);
}

.result-item-badge.warning {
  @apply text-warning-foreground;
  background: var(--surface-warning-subtle);
}

.result-item-badge.neutral {
  @apply bg-muted text-muted-foreground;
}

.result-item-value {
  min-width: 72px;
  align-items: flex-end;
  font-size: 12px;
  white-space: nowrap;
}

.result-item-value.warning strong {
  @apply text-warning-foreground;
}

.result-item-expand {
  @apply text-muted-foreground;
  display: grid;
  place-items: center;
}

.result-item-expand svg {
  transition: transform 0.16s ease;
}

.result-item-expand svg.expanded {
  transform: rotate(180deg);
}

@container (max-width: 760px) {
  .result-item-badge {
    display: none;
  }
}
</style>
