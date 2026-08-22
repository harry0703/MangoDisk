<script setup lang="ts">
import { useI18n } from 'vue-i18n';

import MdIcon from '@/components/icons/md-icon.vue';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { ICON_NAMES } from '@/lib/models/ui';

defineProps<{
  busy: boolean;
  mode: 'smart' | 'all' | 'none' | 'manual';
  analyzing?: boolean;
}>();

const emit = defineEmits<{
  change: [value: unknown];
}>();

const { t } = useI18n({ useScope: 'global' });
</script>

<template>
  <div class="selection-mode">
    <span>{{ t('largeFiles.selectionMode.label') }}</span>
    <Select :model-value="mode" :disabled="busy" @update:model-value="emit('change', $event)">
      <SelectTrigger :aria-label="t('largeFiles.selectionMode.label')">
        <SelectValue>
          <span v-if="analyzing && mode === 'smart'" class="selection-mode-analyzing">
            <MdIcon class="icon-spin" :name="ICON_NAMES.refresh" :size="13" />
            <span>{{ t('largeFiles.selectionMode.analyzing') }}</span>
          </span>
          <template v-else>
            {{ t(`largeFiles.selectionMode.${mode}`) }}
          </template>
        </SelectValue>
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="smart">
          <span class="selection-mode-item-smart">
            <MdIcon :name="ICON_NAMES.smartSelect" :size="14" class="text-primary" />
            <span>{{ t('largeFiles.selectionMode.smart') }}</span>
          </span>
        </SelectItem>
        <SelectItem value="all">
          {{ t('largeFiles.selectionMode.all') }}
        </SelectItem>
        <SelectItem value="none">{{ t('largeFiles.selectionMode.none') }}</SelectItem>
        <SelectItem v-if="mode === 'manual'" value="manual" disabled>
          {{ t('largeFiles.selectionMode.manual') }}
        </SelectItem>
      </SelectContent>
    </Select>
  </div>
</template>

<style scoped>
@reference "@assets/main.css";

.selection-mode {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 8px;
}

.selection-mode > span {
  @apply text-muted-foreground;
  flex: none;
  font-size: 11px;
}

.selection-mode-analyzing,
.selection-mode-item-smart {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.selection-mode :deep([data-slot='select-trigger']) {
  width: 202px;
  min-width: 202px;
  height: 38px;
}

.selection-mode :deep([data-slot='select-value']) {
  overflow: hidden;
  text-overflow: ellipsis;
}

@container (max-width: 760px) {
  .selection-mode > span {
    display: none;
  }

  .selection-mode :deep([data-slot='select-trigger']) {
    width: 148px;
    min-width: 148px;
  }
}
</style>
