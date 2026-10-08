<script setup lang="ts">
import { cpuIdentityLabel } from '@/lib/utils/cpu-identity-label';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/ui';
import { computed } from 'vue';
import MdResourceTrend from './md-resource-trend.vue';
import MdMemoryPressure from './md-memory-pressure.vue';
import { useI18n } from 'vue-i18n';
import {
  METRIC_LABEL_KEYS,
  METRIC_STATUS_KEYS,
  type MetricId,
  type ResourceReadings,
} from '@/lib/models/system-resources';
import { ByteSizeService } from '@/lib/services/byte-size-service';
import { OperatingSystemService } from '@/lib/services/operating-system-service';

const props = withDefaults(
  defineProps<{ metric: MetricId; reading: ResourceReadings; active?: boolean; interactive?: boolean }>(),
  {
    active: true,
    interactive: false,
  }
);
defineEmits<{ cleanup: []; memory: []; cpu: []; gpu: [] }>();
const { t } = useI18n({ useScope: 'global' });
const isMacOs = OperatingSystemService.isMacOs();
const cpuLabel = computed(() => cpuIdentityLabel(props.reading.cpuIdentity));
const current = computed(() => props.reading[props.metric]);
const ready = computed(() => current.value.status === 'ready' && current.value.value !== null);
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
  props.metric === 'disk' ? props.reading.diskIo.status === 'ready' && props.reading.diskIo.value !== null : ready.value
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
  <section class="resource-overview" :data-metric="metric" :aria-label="t(METRIC_LABEL_KEYS[metric])">
    <button
      v-if="interactive && (metric === 'cpu' || metric === 'gpu' || metric === 'memory')"
      type="button"
      class="card-navigation"
      :aria-label="
        t(metric === 'cpu' ? 'systemStatus.cpu' : metric === 'gpu' ? 'systemStatus.gpu' : 'systemStatus.memoryDetails')
      "
      @click="metric === 'cpu' ? $emit('cpu') : metric === 'gpu' ? $emit('gpu') : $emit('memory')"
    />
    <header>
      <span class="resource-label">
        {{ t(METRIC_LABEL_KEYS[metric]) }}
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
          <MdTooltip :text="cpuLabel">
            <span class="gpu-source">{{ cpuLabel }}</span>
          </MdTooltip>
          <span v-if="!ready" class="resource-status" role="status">{{ t(METRIC_STATUS_KEYS[current.status]) }}</span>
        </template>
        <span v-else-if="!ready" role="status">{{ t(METRIC_STATUS_KEYS[current.status]) }}</span>
        <MdTooltip v-else-if="metric === 'gpu'" :text="t('systemStatus.gpuUsageHint')">
          <span class="gpu-source">{{ reading.gpu.value?.adapterName }}</span>
        </MdTooltip>
        <span v-else>{{ t('systemStatus.lastMinute') }}</span>
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
        <MdTooltip v-if="ready && reading.disk.value" :text="isMacOs ? t('systemStatus.diskCapacityHint') : null">
          <span>
            {{ t('systemStatus.available') }} {{ ByteSizeService.diskCapacity(reading.disk.value.availableBytes) }} /
            {{ ByteSizeService.diskCapacity(reading.disk.value.totalBytes) }}
          </span>
        </MdTooltip>
        <span v-else role="status">{{ t(METRIC_STATUS_KEYS[current.status]) }}</span>
        <button type="button" class="cleanup-link" @click="$emit('cleanup')">
          {{ t('navigation.cleanup') }} <span aria-hidden="true">›</span>
        </button>
      </template>
      <template v-else>
        <MdTooltip :text="t('systemStatus.networkScope')">
          <span class="resource-source">{{ source || t('systemStatus.automatic') }}</span>
        </MdTooltip>
        <span v-if="!ready" role="status">{{ t(METRIC_STATUS_KEYS[current.status]) }}</span>
        <span v-else>{{ t('systemStatus.lastMinute') }}</span>
      </template>
    </div>
    <div v-if="metric === 'disk'" class="disk-activity">
      <span class="disk-scope">{{ t('systemStatus.allDisks') }}</span>
      <span v-for="rate in rates" :key="rate.direction" class="disk-rate">
        <span :class="rate.direction">{{
          t(rate.direction === 'upload' ? 'systemStatus.write' : 'systemStatus.read')
        }}</span>
        <strong>{{ rate.value }}</strong
        ><small>{{ rate.unit }}</small>
      </span>
      <span v-if="!activityReady" class="sr-only" role="status">{{
        t(METRIC_STATUS_KEYS[reading.diskIo.status])
      }}</span>
    </div>
    <slot name="details" />
  </section>
</template>

<style scoped>
@reference "@assets/main.css";
.resource-overview {
  @apply rounded-xl border border-border bg-card;
  position: relative;
  padding: 10px 12px;
  min-width: 0;
  flex: none;
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
  flex: none;
  font-size: 10px;
  min-height: 20px;
  padding: 0 4px;
  border-radius: 4px;
  cursor: pointer;
}
.resource-meta .cleanup-link:hover {
  @apply bg-accent;
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
  gap: 10px;
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
.disk-activity {
  display: flex;
  align-items: center;
  gap: 8px;
  justify-content: space-between;
  margin-top: 4px;
}
.disk-rate {
  display: grid;
  grid-template-columns: auto auto auto;
  gap: 3px;
  align-items: baseline;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}
.disk-scope {
  @apply text-muted-foreground;
  font-size: 10px;
  flex: none;
}
.disk-rate strong {
  text-align: right;
  font-weight: 500;
}
.disk-rate > span {
  font-size: 10px;
}
.capacity-track {
  @apply bg-muted rounded-full;
  height: 4px;
  overflow: hidden;
  margin-top: 4px;
}
.capacity-track i {
  background: var(--primary);
  opacity: 0.55;
  height: 100%;
  display: block;
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
