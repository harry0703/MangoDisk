<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/ui';
import { currentCpuTemperature } from '@/lib/utils/cpu-temperature';
import { METRIC_STATUS_KEYS, type CpuTemperature, type MetricReading } from '@/lib/models/system-resources';

const props = withDefaults(
  defineProps<{
    reading: MetricReading<CpuTemperature>;
    observedAtMs: number;
    compact?: boolean;
    interactive?: boolean;
  }>(),
  { compact: false, interactive: false }
);
defineEmits<{ activate: [] }>();
const { t } = useI18n({ useScope: 'global' });
const hintKeys: Record<CpuTemperature['kind'], string> = {
  coreAverage: 'cpuTemperature.coreAverageHint',
  package: 'cpuTemperature.packageHint',
  coreMaximum: 'cpuTemperature.coreMaximumHint',
};
const value = computed(() => currentCpuTemperature(props.reading, props.observedAtMs));
const statusText = computed(() =>
  t(METRIC_STATUS_KEYS[props.reading.status === 'ready' ? 'stale' : props.reading.status])
);
const hint = computed(() =>
  value.value
    ? t(hintKeys[value.value.kind], { count: value.value.sensorCount })
    : props.reading.status === 'unsupported'
      ? t('cpuTemperature.unsupportedHint')
      : statusText.value
);
</script>

<template>
  <div v-if="!compact || reading.status !== 'unsupported'" class="cpu-temperature" :class="{ compact }">
    <MdTooltip v-if="compact" :text="hint">
      <component
        :is="interactive ? 'button' : 'span'"
        :type="interactive ? 'button' : undefined"
        class="temperature-value"
        tabindex="0"
        :aria-label="`${t('cpuTemperature.label')} ${value ? `${value.celsius.toFixed(0)}°C` : hint}`"
        @click="interactive && $emit('activate')"
      >
        <strong>{{ value ? value.celsius.toFixed(0) : '—' }}<small v-if="value">°C</small></strong>
      </component>
    </MdTooltip>
    <template v-else>
      <div class="temperature-heading">
        <div class="temperature-label">
          <span>{{ t('cpuTemperature.label') }}</span>
          <MdTooltip :text="hint">
            <button type="button" class="temperature-help" :aria-label="t('cpuTemperature.label')">
              <MdIcon :name="ICON_NAMES.info" :size="12" />
            </button>
          </MdTooltip>
        </div>
        <strong>{{ value ? value.celsius.toFixed(0) : '—' }}<small v-if="value">°C</small></strong>
      </div>
      <span v-if="!value" class="temperature-status" role="status">{{ statusText }}</span>
    </template>
  </div>
</template>

<style scoped>
@reference "@assets/main.css";
.cpu-temperature {
  min-width: 0;
}
.temperature-value,
.temperature-heading {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
  font-size: 11px;
  border-radius: 3px;
  padding: 0;
  border: 0;
  background: transparent;
  color: inherit;
}
.temperature-value:focus-visible,
.temperature-help:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}
strong {
  @apply text-foreground;
  font-size: 18px;
  font-variant-numeric: tabular-nums;
}
small {
  margin-left: 2px;
  font-size: 10px;
  font-weight: normal;
}
.temperature-label {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}
.temperature-help {
  @apply text-muted-foreground;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: none;
  padding: 2px;
  border-radius: 3px;
  cursor: help;
}
.temperature-help:hover {
  @apply text-primary-text;
}
.temperature-status {
  @apply text-muted-foreground;
  display: block;
  margin-top: 4px;
  font-size: 10px;
}
.compact {
  flex: none;
  position: relative;
  z-index: 1;
}
.compact .temperature-value,
.compact strong {
  font-size: 10px;
  font-weight: normal;
  white-space: nowrap;
}
</style>
