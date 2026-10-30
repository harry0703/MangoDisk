<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import MdMetricFreshness from './md-metric-freshness.vue';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import { observedCpuTemperature } from '@/lib/utils/system-resources/cpu-temperature';
import { observedGpuTemperature } from '@/lib/utils/system-resources/gpu-temperature';
import {
  METRIC_STATUS_KEYS,
  type CpuTemperature,
  type GpuTemperature,
  type MetricReading,
} from '@/lib/models/system-resources/system-resources';

const props = withDefaults(
  defineProps<{
    reading: MetricReading<CpuTemperature> | MetricReading<GpuTemperature>;
    metric: 'cpu' | 'gpu';
    observedAtMs: number;
    active?: boolean;
    compact?: boolean;
    interactive?: boolean;
    heading?: string;
    showStatus?: boolean;
  }>(),
  { active: true, compact: false, interactive: false, heading: undefined, showStatus: true }
);
defineEmits<{ activate: [] }>();
const { t } = useI18n({ useScope: 'global' });
const hintKeys: Record<CpuTemperature['kind'], string> = {
  coreAverage: 'cpuTemperature.coreAverageHint',
  package: 'cpuTemperature.packageHint',
  coreMaximum: 'cpuTemperature.coreMaximumHint',
  diode: 'cpuTemperature.diodeHint',
  proximity: 'cpuTemperature.proximityHint',
};
const value = computed(() =>
  props.metric === 'cpu'
    ? observedCpuTemperature(props.reading as MetricReading<CpuTemperature>, props.observedAtMs)
    : observedGpuTemperature(props.reading as MetricReading<GpuTemperature>, props.observedAtMs)
);
const label = computed(() => t(props.metric === 'cpu' ? 'cpuTemperature.label' : 'gpuTemperature.label'));
const statusText = computed(() =>
  t(METRIC_STATUS_KEYS[props.reading.status === 'ready' ? 'stale' : props.reading.status])
);
const hint = computed(() =>
  value.value
    ? (props.reading.status === 'failed' ? `${statusText.value} · ` : '') +
      t(
        value.value.source === 'amdZenPawnIo'
          ? 'cpuTemperature.amdZenHint'
          : value.value.kind === 'sensorAverage'
            ? 'gpuTemperature.sensorAverageHint'
            : value.value.kind === 'device'
              ? 'gpuTemperature.deviceHint'
              : hintKeys[value.value.kind],
        { count: value.value.sensorCount }
      )
    : props.reading.status === 'unsupported'
      ? t(props.metric === 'cpu' ? 'cpuTemperature.unsupportedHint' : 'gpuTemperature.unsupportedHint')
      : statusText.value
);
</script>

<template>
  <div v-if="!compact || reading.status !== 'unsupported'" class="resource-temperature" :class="{ compact }">
    <MdTooltip v-if="compact" :active="active" :text="hint">
      <component
        :is="interactive ? 'button' : 'span'"
        :type="interactive ? 'button' : undefined"
        class="temperature-value"
        tabindex="0"
        :aria-label="`${label} ${value ? `${Math.round(value.celsius)}°C` : hint}`"
        @click="interactive && $emit('activate')"
      >
        <strong>{{ value ? Math.round(value.celsius) : '—' }}<small v-if="value">°C</small></strong>
      </component>
    </MdTooltip>
    <template v-else>
      <div class="temperature-heading">
        <div class="temperature-label">
          <span>{{ heading ?? label }}</span>
          <MdTooltip :active="active" :text="hint">
            <button type="button" class="temperature-help" :aria-label="label">
              <MdIcon :name="ICON_NAMES.info" :size="12" />
            </button>
          </MdTooltip>
        </div>
        <strong>{{ value ? Math.round(value.celsius) : '—' }}<small v-if="value">°C</small></strong>
      </div>
      <MdMetricFreshness v-if="value && showStatus" :reading="reading" :now="observedAtMs" :active="active" />
      <span v-if="!value && showStatus" class="temperature-status" role="status">{{ statusText }}</span>
    </template>
  </div>
</template>

<style scoped>
@reference "@assets/main.css";
.resource-temperature {
  min-width: 0;
}
.temperature-value,
.temperature-heading {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
  border-radius: 3px;
  padding: 0;
  border: 0;
  background: transparent;
  color: inherit;
}
.temperature-value {
  font-size: 11px;
}
.temperature-heading {
  min-height: 30px;
  font-size: 12px;
}
.temperature-heading strong {
  font-size: 24px;
  line-height: 30px;
}
.temperature-heading small {
  font-size: 11px;
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
