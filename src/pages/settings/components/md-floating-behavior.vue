<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import MdSettingsGroup from '@/components/custom/md-settings-group.vue';
import MdSettingsRow from '@/components/custom/md-settings-row.vue';
import MdSwitch from '@/components/custom/md-switch.vue';
import type { ResidentPreferences } from '@/lib/models/resident/resident';

withDefaults(defineProps<{ preferences: ResidentPreferences; showTitle?: boolean }>(), { showTitle: true });
const emit = defineEmits<{ change: [patch: Partial<ResidentPreferences>] }>();
const { t } = useI18n();
</script>
<template>
  <MdSettingsGroup plain :title="showTitle ? t('floatingMonitor.floating') : undefined">
    <MdSettingsRow compact :title="t('floatingMonitor.alwaysOnTop')" description="" label-for="floating-top">
      <MdSwitch
        id="floating-top"
        :model-value="preferences.floating.alwaysOnTop"
        @update:model-value="emit('change', { floating: { ...preferences.floating, alwaysOnTop: $event } })"
      />
    </MdSettingsRow>
    <MdSettingsRow compact :title="t('floatingMonitor.snapToEdges')" description="" label-for="floating-snap">
      <MdSwitch
        id="floating-snap"
        :model-value="preferences.floating.snapToEdges"
        @update:model-value="emit('change', { floating: { ...preferences.floating, snapToEdges: $event } })"
      />
    </MdSettingsRow>
    <MdSettingsRow compact :title="t('floatingMonitor.locked')" description="" label-for="floating-lock">
      <MdSwitch
        id="floating-lock"
        :model-value="preferences.floating.locked"
        @update:model-value="emit('change', { floating: { ...preferences.floating, locked: $event } })"
      />
    </MdSettingsRow>
    <MdSettingsRow
      compact
      :title="t('floatingMonitor.clickThrough')"
      :description="t('floatingMonitor.clickThroughHint')"
      label-for="floating-click-through"
    >
      <MdSwitch
        id="floating-click-through"
        :model-value="preferences.floating.clickThrough"
        @update:model-value="emit('change', { floating: { ...preferences.floating, clickThrough: $event } })"
      />
    </MdSettingsRow>
    <MdSettingsRow
      compact
      :title="t('floatingMonitor.hideOnFullscreen')"
      description=""
      label-for="floating-fullscreen"
    >
      <MdSwitch
        id="floating-fullscreen"
        :model-value="preferences.floating.hideOnFullscreen"
        @update:model-value="emit('change', { floating: { ...preferences.floating, hideOnFullscreen: $event } })"
      />
    </MdSettingsRow>
  </MdSettingsGroup>
</template>
