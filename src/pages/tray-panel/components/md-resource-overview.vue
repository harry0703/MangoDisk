<script setup lang="ts">
import { cpuIdentityLabel } from '@/lib/utils/system-resources/cpu-identity-label';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import { cachedMetricValue } from '@/lib/utils/system-resources/metric-reading';
import { computed } from 'vue';
import MdResourceTrend from './md-resource-trend.vue';
import MdResourceCard from './md-resource-card.vue';
import MdMemoryPressure from './md-memory-pressure.vue';
import MdCpuTemperature from './md-cpu-temperature.vue';
import MdResourceTemperature from './md-resource-temperature.vue';
import { useI18n } from 'vue-i18n';
import {
  METRIC_LABEL_KEYS,
  METRIC_STATUS_KEYS,
  type MetricId,
  type ResourceReadings,
} from '@/lib/models/system-resources/system-resources';
import { ByteSizeService } from '@/lib/services/settings/byte-size-service';
import { OperatingSystemService } from '@/lib/services/settings/operating-system-service';

const props = withDefaults(
  defineProps<{
    metric: MetricId;
    reading: ResourceReadings;
    active?: boolean;
    interactive?: boolean;
    cleanupVariant?: 'link' | 'button';
    showCpuTemperature?: boolean;
    heading?: string;
  }>(),
  {
    active: true,
    interactive: false,
    cleanupVariant: 'link',
    showCpuTemperature: true,
    heading: undefined,
  }
);
defineEmits<{ cleanup: []; memory: []; cpu: []; gpu: []; disk: []; network: [] }>();
const { t } = useI18n({ useScope: 'global' });
const isMacOs = OperatingSystemService.isMacOs();
const cpuLabel = computed(() => cpuIdentityLabel(props.reading.cpuIdentity));
const current = computed(() => props.reading[props.metric]);
const ready = computed(() => cachedMetricValue<unknown>(current.value, props.reading.observedAtMs) !== null);
const memory = computed(() => props.reading.memory.value?.memory);
const percentage = computed(() => {
  if (!ready.value) return null;
  switch (props.metric) {
    case 'cpu':
      return props.reading.cpu.value?.usedPercent ?? null;
    case 'gpu':
      return props.reading.gpu.value?.usedPercent ?? null;
    case 'memory':
      return memory.value?.usedPercent ?? null;
    case 'disk':
      return props.reading.disk.value?.usedPercent ?? null;
    default:
      return null;
  }
});
const activityReady = computed(() =>
  props.metric === 'disk' ? cachedMetricValue(props.reading.diskIo, props.reading.observedAtMs) !== null : ready.value
);
const history = computed(() =>
  props.metric === 'cpu'
    ? props.reading.cpuHistory
    : props.metric === 'gpu'
      ? props.reading.gpuHistory
      : props.reading.networkHistory
);
const source = computed(() =>
  props.metric === 'network'
    ? props.reading.network.value?.interface.name
    : props.reading.disk.value?.volume.system
      ? t('systemStatus.systemDisk')
      : props.reading.disk.value?.volume.name
);
const rates = computed(() =>
  [
    {
      direction: 'upload',
      arrow: '↑',
      bytes:
        props.metric === 'disk'
          ? props.reading.diskIo.value?.writtenBytesPerSecond
          : props.reading.network.value?.transmittedBytesPerSecond,
    },
    {
      direction: 'download',
      arrow: '↓',
      bytes:
        props.metric === 'disk'
          ? props.reading.diskIo.value?.readBytesPerSecond
          : props.reading.network.value?.receivedBytesPerSecond,
    },
  ].map(rate => {
    // Split only the app-owned byte formatter, keeping unit positions independent of digits.
    const formatted = activityReady.value && rate.bytes !== undefined ? ByteSizeService.bytes(rate.bytes) : '—';
    const separator = formatted.lastIndexOf(' ');
    return {
      ...rate,
      value: separator < 0 ? formatted : formatted.slice(0, separator),
      unit: separator < 0 ? '' : `${formatted.slice(separator + 1)}/s`,
    };
  })
);
</script>

<template>
  <MdResourceCard class="resource-overview" :data-metric="metric" :aria-label="heading ?? t(METRIC_LABEL_KEYS[metric])">
    <button
      v-if="interactive"
      type="button"
      class="card-navigation"
      :aria-label="t(METRIC_LABEL_KEYS[metric])"
      @click="
        metric === 'cpu'
          ? $emit('cpu')
          : metric === 'gpu'
            ? $emit('gpu')
            : metric === 'disk'
              ? $emit('disk')
              : metric === 'network'
                ? $emit('network')
                : $emit('memory')
      "
    />
    <header>
      <span class="resource-label">
        {{ heading ?? t(METRIC_LABEL_KEYS[metric]) }}
        <MdIcon v-if="interactive" :name="ICON_NAMES.chevronRight" :size="12" aria-hidden="true" />
        <span v-if="metric === 'disk'" class="resource-source">{{ source }}</span>
      </span>
      <div v-if="metric === 'network'" class="network-values">
        <span
          v-for="rate in rates"
          :key="rate.direction"
          class="network-rate"
          :aria-label="t(rate.direction === 'upload' ? 'systemStatus.upload' : 'systemStatus.download')"
        >
          <b :class="rate.direction" aria-hidden="true">{{ rate.arrow }}</b>
          <strong>{{ rate.value }}</strong
          ><small>{{ rate.unit }}</small>
        </span>
      </div>
      <strong v-else class="resource-value">
        {{ percentage === null ? '—' : percentage.toFixed(0) }}<small v-if="percentage !== null">%</small>
      </strong>
    </header>

    <div
      v-if="metric === 'disk' || metric === 'memory'"
      class="capacity-track"
      :role="ready ? 'meter' : undefined"
      :aria-label="t(METRIC_LABEL_KEYS[metric])"
      :aria-valuenow="percentage ?? undefined"
      :aria-valuemin="0"
      :aria-valuemax="100"
    >
      <i v-if="percentage !== null" :style="{ width: `${Math.max(0, Math.min(100, percentage))}%` }" />
    </div>
    <MdResourceTrend
      v-else
      :key="metric === 'network' ? reading.network.value?.interface.id : metric"
      :metric="metric"
      :active="active"
      :history="history"
      :observed-at-ms="reading.observedAtMs"
      :label="t('systemStatus.lastMinute')"
    />

    <div class="resource-meta">
      <template v-if="metric === 'cpu' || metric === 'gpu'">
        <template v-if="metric === 'cpu' && cpuLabel">
          <MdTooltip :active="active" :text="cpuLabel">
            <span class="gpu-source">{{ cpuLabel }}</span>
          </MdTooltip>
          <span v-if="!ready" class="resource-status" role="status">{{ t(METRIC_STATUS_KEYS[current.status]) }}</span>
        </template>
        <span v-else-if="!ready" role="status">{{ t(METRIC_STATUS_KEYS[current.status]) }}</span>
        <MdTooltip v-else-if="metric === 'gpu'" :active="active" :text="t('systemStatus.gpuUsageHint')" multiline>
          <span class="gpu-source">{{ reading.gpu.value?.adapterName }}</span>
        </MdTooltip>
        <span v-else>{{ t('systemStatus.lastMinute') }}</span>
        <MdCpuTemperature
          v-if="metric === 'cpu' && showCpuTemperature"
          :active="active"
          compact
          :interactive="interactive"
          :reading="reading.cpuTemperature"
          :observed-at-ms="reading.observedAtMs"
          @activate="$emit('cpu')"
        />
        <MdResourceTemperature
          v-if="
            metric === 'gpu' &&
            reading.gpuTemperature.status !== 'unsupported' &&
            (!reading.gpu.value || reading.gpu.value.adapterId === reading.gpuTemperature.value?.adapterId)
          "
          :active="active"
          metric="gpu"
          compact
          :interactive="interactive"
          :reading="reading.gpuTemperature"
          :observed-at-ms="reading.observedAtMs"
          @activate="$emit('gpu')"
        />
        <slot name="meta-end" />
      </template>
      <template v-else-if="metric === 'memory'">
        <span v-if="ready && memory">
          {{ ByteSizeService.memory(memory.usedBytes) }} / {{ ByteSizeService.memory(memory.totalBytes) }}
        </span>
        <span v-else role="status">{{ t(METRIC_STATUS_KEYS[current.status]) }}</span>
        <span v-if="memory && memory.pressure !== 'unsupported'" class="overview-pressure">
          <MdMemoryPressure
            :pressure="memory.pressure"
            :status="current.status"
            :active="active"
            @click="interactive && $emit('memory')"
          />
        </span>
      </template>
      <template v-else-if="metric === 'disk'">
        <MdTooltip
          v-if="ready && reading.disk.value"
          :active="active"
          :text="isMacOs ? t('systemStatus.diskCapacityHint') : null"
          multiline
        >
          <span>
            {{ t('systemStatus.available') }} {{ ByteSizeService.diskCapacity(reading.disk.value.availableBytes) }} /
            {{ ByteSizeService.diskCapacity(reading.disk.value.totalBytes) }}
          </span>
        </MdTooltip>
        <span v-else role="status">{{ t(METRIC_STATUS_KEYS[current.status]) }}</span>
        <button
          type="button"
          :class="['cleanup-link', { 'cleanup-button': cleanupVariant === 'button' }]"
          @click="$emit('cleanup')"
        >
          <MdIcon v-if="cleanupVariant === 'button'" :name="ICON_NAMES.deepCleanup" :size="12" aria-hidden="true" />
          {{ t('navigation.cleanup') }}<span v-if="cleanupVariant === 'link'" aria-hidden="true">›</span>
        </button>
      </template>
      <template v-else>
        <MdTooltip :active="active" :text="t('systemStatus.networkScope')">
          <span class="resource-source">{{ source || t('systemStatus.automatic') }}</span>
        </MdTooltip>
        <span v-if="!ready" role="status">{{ t(METRIC_STATUS_KEYS[current.status]) }}</span>
        <span v-else>{{ t('systemStatus.lastMinute') }}</span>
      </template>
      <MdTooltip
        v-if="ready && current.status === 'failed'"
        :active="active"
        :text="t(METRIC_STATUS_KEYS[current.status])"
      >
        <span class="reading-error" role="status" tabindex="0" :aria-label="t(METRIC_STATUS_KEYS[current.status])">{{
          t(METRIC_STATUS_KEYS[current.status])
        }}</span>
      </MdTooltip>
    </div>
  </MdResourceCard>
</template>

<style scoped>
@reference "@assets/main.css";
.resource-meta > .reading-error {
  @apply text-muted-foreground;
  font-size: 10px;
  min-width: 24px;
  max-width: 45%;
  position: relative;
  z-index: 2;
}
.resource-overview {
  /* Detail components can be fragment roots, outside the page's scoped sizing rules. */
  height: auto;
  min-height: var(--resource-summary-height, 104px);
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  position: relative;
}
header,
.resource-meta {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
}
header {
  /* Reserve the percentage baseline even while the value is an em dash. */
  min-height: 30px;
  flex: none;
  flex-wrap: wrap;
}
.resource-label {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
  font-size: 12px;
  flex: 1;
}
.resource-value {
  font-size: 24px;
  line-height: 30px;
  font-variant-numeric: tabular-nums;
}
small {
  font-size: 11px;
  font-weight: normal;
}
.resource-value small {
  margin-left: 2px;
}
.resource-meta,
.resource-source {
  @apply text-muted-foreground;
  font-size: 10px;
  line-height: 16px;
  min-height: 16px;
}
.resource-meta {
  flex: none;
  margin-top: 4px;
  min-height: 18px;
}
.resource-meta > span,
.resource-source {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.resource-meta > span {
  min-width: 0;
}
.resource-meta > .resource-status {
  flex: none;
}
.resource-meta .cleanup-link {
  @apply text-primary-text;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex: none;
  position: relative;
  z-index: 2;
  font-size: 10px;
  min-height: 20px;
  padding: 0 4px;
  border-radius: 4px;
  cursor: pointer;
}
.resource-meta .cleanup-link:hover {
  @apply bg-accent;
}
.resource-meta .cleanup-button {
  @apply bg-primary text-primary-foreground;
  min-height: 26px;
  padding: 3px 8px;
  border-radius: 6px;
}
.resource-meta .cleanup-button:hover {
  background: color-mix(in srgb, var(--primary) 88%, var(--primary-foreground));
}
.resource-meta .cleanup-link:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}
.resource-meta > .overview-pressure {
  display: inline-flex;
  flex: none;
  max-width: 55%;
  overflow: visible;
  position: relative;
  z-index: 2;
}
.gpu-source {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
}
.network-values {
  display: flex;
  flex: none;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 10px;
  max-width: 100%;
}
.network-rate {
  display: grid;
  grid-template-columns: 10px auto auto;
  align-items: baseline;
  column-gap: 4px;
  font-size: 14px;
  font-variant-numeric: tabular-nums;
}
.network-rate strong {
  text-align: right;
  font-weight: 600;
}
.network-rate small {
  white-space: nowrap;
}
.network-rate b {
  font-weight: 600;
}
.upload {
  color: var(--status-upload);
}
.download {
  color: var(--status-download);
}
.capacity-track {
  @apply bg-muted rounded-full;
  height: 4px;
  flex: none;
  overflow: hidden;
  margin-top: 4px;
}
.capacity-track i {
  background: var(--primary);
  border-radius: inherit;
  height: 100%;
  display: block;
}
.resource-overview :deep(.resource-trend) {
  flex: none;
}
.card-navigation {
  position: absolute;
  inset: 0;
  z-index: 1;
  border-radius: inherit;
  background: transparent;
  cursor: pointer;
}
.resource-overview:has(.card-navigation:hover) {
  @apply bg-accent/40;
}
.card-navigation:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: -2px;
}
</style>
