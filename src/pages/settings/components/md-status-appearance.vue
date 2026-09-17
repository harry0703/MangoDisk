<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import MdSettingsGroup from '@/components/custom/md-settings-group.vue';
import MdSettingsRow from '@/components/custom/md-settings-row.vue';
import MdSwitch from '@/components/custom/md-switch.vue';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { ICON_NAMES } from '@/lib/models/ui';
import type { ResidentPreferences } from '@/lib/models/resident';

const props = defineProps<{ preferences: ResidentPreferences; isMacOs: boolean }>();
const emit = defineEmits<{ change: [patch: Partial<ResidentPreferences>] }>();
const { t } = useI18n({ useScope: 'global' });
const thresholds = computed(() =>
  [
    ...new Set([
      1,
      ...Array.from({ length: 20 }, (_, index) => (index + 1) * 5),
      props.preferences.usageWarningPercent,
      props.preferences.usageCriticalPercent,
    ]),
  ].sort((a, b) => a - b)
);
</script>

<template>
  <MdSettingsGroup plain :title="t('systemStatus.appearanceTitle')">
    <MdSettingsRow
      v-if="isMacOs || preferences.windowsDisplayMode === 'taskbar'"
      compact
      :title="t('systemStatus.taskbarCompact')"
      description=""
      :label-for="isMacOs ? 'menu-bar-compact' : 'taskbar-compact'"
    >
      <template #help>
        <MdTooltip :text="t('systemStatus.menuBarCompactHint')">
          <button type="button" class="appearance-help" :aria-label="t('systemStatus.menuBarCompactHint')">
            <MdIcon :name="ICON_NAMES.help" :size="14" />
          </button>
        </MdTooltip>
      </template>
      <MdSwitch
        :id="isMacOs ? 'menu-bar-compact' : 'taskbar-compact'"
        :model-value="isMacOs ? preferences.menuBarCompact : preferences.taskbarCompact"
        @update:model-value="emit('change', isMacOs ? { menuBarCompact: $event } : { taskbarCompact: $event })"
      />
    </MdSettingsRow>
    <div>
      <MdSettingsRow compact :title="t('systemStatus.usageColors')" description="" label-for="usage-colors">
        <template #help>
          <MdTooltip :text="t('systemStatus.usageColorsHelp')">
            <button type="button" class="appearance-help" :aria-label="t('systemStatus.usageColorsHelp')">
              <MdIcon :name="ICON_NAMES.help" :size="14" />
            </button>
          </MdTooltip>
        </template>
        <MdSwitch
          id="usage-colors"
          :model-value="preferences.usageColors"
          @update:model-value="emit('change', { usageColors: $event })"
        />
      </MdSettingsRow>
      <div v-if="preferences.usageColors" class="grid grid-cols-2 gap-3 pt-1">
        <div class="flex min-w-0 items-center gap-2">
          <label for="usage-warning" class="flex shrink-0 items-center gap-1 whitespace-nowrap text-content-secondary">
            <span class="size-2 shrink-0 rounded-full bg-usage-warning" aria-hidden="true" />
            {{ t('systemStatus.usageWarning') }}
          </label>
          <Select
            :model-value="String(preferences.usageWarningPercent)"
            @update:model-value="emit('change', { usageWarningPercent: Number($event) })"
          >
            <SelectTrigger id="usage-warning" size="sm" class="min-w-0 flex-1 gap-1 px-2"
              ><SelectValue
            /></SelectTrigger>
            <SelectContent>
              <SelectItem
                v-for="value in thresholds.filter(value => value < preferences.usageCriticalPercent)"
                :key="value"
                :value="String(value)"
                >{{ value }}%</SelectItem
              >
            </SelectContent>
          </Select>
        </div>
        <div class="flex min-w-0 items-center gap-2">
          <label for="usage-critical" class="flex shrink-0 items-center gap-1 whitespace-nowrap text-content-secondary">
            <span class="size-2 shrink-0 rounded-full bg-usage-critical" aria-hidden="true" />
            {{ t('systemStatus.usageCritical') }}
          </label>
          <Select
            :model-value="String(preferences.usageCriticalPercent)"
            @update:model-value="emit('change', { usageCriticalPercent: Number($event) })"
          >
            <SelectTrigger id="usage-critical" size="sm" class="min-w-0 flex-1 gap-1 px-2"
              ><SelectValue
            /></SelectTrigger>
            <SelectContent>
              <SelectItem
                v-for="value in thresholds.filter(value => value > preferences.usageWarningPercent)"
                :key="value"
                :value="String(value)"
                >{{ value }}%</SelectItem
              >
            </SelectContent>
          </Select>
        </div>
      </div>
    </div>
  </MdSettingsGroup>
</template>

<style scoped>
@reference "@assets/main.css";
.appearance-help {
  @apply grid size-6 shrink-0 place-items-center rounded bg-transparent text-muted-foreground focus-visible:outline-2 focus-visible:outline-ring;
}
</style>
