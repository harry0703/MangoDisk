<script setup lang="ts">
import MdResultTableRow from '@/components/custom/md-result-table-row.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import type { IconName } from '@/lib/models/ui';

defineProps<{
  icon: IconName;
  title: string;
  description?: string;
  descriptionTitle?: string;
}>();
</script>

<template>
  <!-- Preserve component alignment without suggesting that informational rows can be selected. -->
  <MdResultTableRow class="readonly-detail-row">
    <span aria-hidden="true" />
    <span class="detail-icon"><MdIcon :name="icon" :size="17" /></span>
    <span class="detail-copy">
      <strong class="md-result-primary">{{ title }}</strong>
      <small v-if="description" :title="descriptionTitle ?? description">{{ description }}</small>
    </span>
    <span class="detail-actions"><slot name="actions" /></span>
  </MdResultTableRow>
</template>

<style scoped>
@reference "@assets/main.css";

.readonly-detail-row {
  display: grid;
  min-width: 0;
  min-height: var(--layout-result-child-row-height);
  grid-template-columns: 17px 34px minmax(0, 1fr) auto;
  align-items: center;
  gap: 10px;
  border-radius: calc(var(--radius) - 5px);
  padding-block: 2px;
}

.detail-icon {
  display: grid;
  width: 34px;
  height: 34px;
  place-items: center;
  @apply text-muted-foreground;
}

.detail-copy {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 2px;
  @apply text-muted-foreground;
}

.detail-copy strong,
.detail-copy small {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.detail-copy strong {
  font-size: 12px;
}

.detail-copy small {
  font-size: 10px;
}

.detail-actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
}
</style>
