<script setup lang="ts">
import MdComponentPower from './md-component-power.vue';
import { summarizeUtilizationHistory } from '@/lib/utils/system-resources/utilization-history-summary';
import MdGpuEngineHistory from './md-gpu-engine-history.vue';
import MdResourceFacts from './md-resource-facts.vue';
import { customGpuActivities, summarizeGpuActivities } from '@/lib/utils/system-resources/gpu-activity';
import { gpuMemoryRows } from '@/lib/utils/system-resources/gpu-memory';
import { computed, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { Select, SelectContent, SelectItem } from '@/components/ui/select';
import { SelectTrigger } from 'reka-ui';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import { GPU_ACTIVITY_LABEL_KEYS, GPU_FACT_LABEL_KEYS } from '@/lib/models/system-resources/gpu-details';
import type { GpuActivity } from '@/lib/models/system-resources/gpu-details';
import { METRIC_STATUS_KEYS, type ResourceReadings } from '@/lib/models/system-resources/system-resources';
import { ByteSizeService } from '@/lib/services/settings/byte-size-service';
import { useResidentSettingsStore } from '@/stores/resident-settings-store';
import MdResourceTrend from './md-resource-trend.vue';
import MdResourceCard from './md-resource-card.vue';
import MdTemperatureDetails from './md-temperature-details.vue';
import MdResourceTemperature from './md-resource-temperature.vue';
const props = defineProps<{ reading: ResourceReadings; active: boolean }>();
const { t } = useI18n({ useScope: 'global' });
const settings = useResidentSettingsStore();
const deviceMenuOpen = ref(false);
const temperatureDevice = computed(() =>
  props.reading.gpuDevices.find(device => device.adapterId === (selectedId.value ?? value.value?.adapterId))
);
const temperatureMatches = computed(
  () =>
    !selectedId.value ||
    !props.reading.gpuTemperature.value ||
    props.reading.gpuTemperature.value.adapterId === selectedId.value
);
const temperatureReading = computed(
  () =>
    temperatureDevice.value?.temperature ??
    (temperatureMatches.value
      ? props.reading.gpuTemperature
      : { status: 'loading' as const, sampledAtMs: null, value: null })
);
const temperatureHistory = computed(
  () =>
    temperatureDevice.value?.temperatureHistory ?? (temperatureMatches.value ? props.reading.gpuTemperatureHistory : [])
);
const selectedId = computed(() => settings.draft?.gpuAdapter ?? null);
const missing = computed(
  () => selectedId.value && !props.reading.gpuAdapters.some(adapter => adapter.id === selectedId.value)
);
const canSelectDevice = computed(() => props.reading.gpuAdapters.length > 1 || missing.value);
watch(canSelectDevice, visible => {
  if (!visible) deviceMenuOpen.value = false;
});
const selectedLabel = computed(() =>
  !selectedId.value
    ? t('systemStatus.automatic')
    : (props.reading.gpuAdapters.find(adapter => adapter.id === selectedId.value)?.name ??
      t('systemStatus.savedDisconnected'))
);
// Reopening refreshes preferences in the background. Retain named native samples
// while they age, but never attribute a previous adapter's details to a new one.
const summaryReading = computed(() => (props.reading.gpu.value ? props.reading.gpu : props.reading.gpuDetails));
const value = computed(() => {
  const sample = summaryReading.value;
  return sample.status !== 'disconnected' &&
    sample.status !== 'unsupported' &&
    (!selectedId.value || selectedId.value === sample.value?.adapterId)
    ? sample.value
    : null;
});
const powerMatches = computed(() => {
  const adapterId =
    selectedId.value ??
    value.value?.adapterId ??
    (props.reading.gpuAdapters.length === 1 ? props.reading.gpuAdapters[0]?.id : null);
  return (
    !!adapterId &&
    props.reading.gpuAdapters.some(adapter => adapter.id === adapterId) &&
    props.reading.gpuPower.value?.adapterId === adapterId
  );
});
const deviceIdentity = computed(() => JSON.stringify([value.value?.adapterId, value.value?.adapterName]));
const temperatureIdentity = computed(() => {
  const temperature = temperatureReading.value.value;
  return JSON.stringify([
    temperature?.adapterId,
    temperature?.adapterName,
    temperature?.source,
    temperature?.kind,
    temperature?.sensorCount,
  ]);
});
const details = computed(() => {
  const sample = props.reading.gpuDetails;
  return value.value &&
    sample.status !== 'disconnected' &&
    sample.status !== 'unsupported' &&
    sample.value?.adapterId === value.value.adapterId
    ? sample.value.details
    : null;
});
const history = computed(() =>
  value.value?.adapterId === props.reading.gpuDetailAdapterId ? props.reading.gpuDetailHistory : []
);
const historySummary = computed(() => summarizeUtilizationHistory(history.value, props.reading.observedAtMs));
const deviceTooltip = computed(() => {
  if (!value.value) return '';
  const device = !canSelectDevice.value
    ? value.value.adapterName
    : selectedId.value
      ? selectedLabel.value
      : `${selectedLabel.value} · ${value.value.adapterName}`;
  return readingFailed.value ? `${device} · ${t(METRIC_STATUS_KEYS.failed)}` : device;
});
const facts = computed(() => {
  const telemetry = details.value?.telemetry;
  // Prefer live clock measurements; temperature has its own shared history.
  const selected = [
    { key: 'engineClock', value: telemetry?.engineClockMhz, unit: 'MHz' },
    { key: 'coreClock', value: telemetry?.coreClockMhz, unit: 'MHz' },
    { key: 'memoryClock', value: telemetry?.memoryClockMhz, unit: 'MHz' },
    { key: 'coreCount', value: telemetry?.coreCount, unit: '' },
    { key: 'average', value: historySummary.value?.average, unit: '%' },
    { key: 'peak', value: historySummary.value?.peak, unit: '%' },
    { key: 'fanSpeed', value: telemetry?.fanPercent, unit: '%' },
  ]
    .filter(fact => fact.value != null && Number.isFinite(fact.value))
    .slice(0, 3);
  return selected.map(fact => ({
    key: fact.key,
    label: t(GPU_FACT_LABEL_KEYS[fact.key as keyof typeof GPU_FACT_LABEL_KEYS]),
    value: fact.value!.toFixed(0),
    unit: fact.unit,
    liveHint: fact.key === 'average' || fact.key === 'peak',
    hint:
      (fact.key === 'average' || fact.key === 'peak') && historySummary.value
        ? t('cpuDetails.coverage', { seconds: (historySummary.value.coveredMs / 1000).toFixed(1) })
        : undefined,
  }));
});
function engineHistory(kind: GpuActivity['kind']) {
  if (value.value?.adapterId !== props.reading.gpuDetailAdapterId) return [];
  return kind === 'renderer' ? props.reading.gpuRendererHistory : kind === 'tiler' ? props.reading.gpuTilerHistory : [];
}
const standard = computed(() => summarizeGpuActivities(details.value?.activities ?? []));
const custom = computed(() => customGpuActivities(details.value?.activities ?? []));
const readingFailed = computed(() => {
  const detail = props.reading.gpuDetails;
  const detailAdapterId = detail.value?.adapterId ?? props.reading.gpuDetailAdapterId;
  return (
    summaryReading.value.status === 'failed' ||
    (detail.status === 'failed' && detailAdapterId === value.value?.adapterId)
  );
});
const memory = computed(() => (details.value?.memoryStatus === 'ready' ? details.value.memory : null));
const memoryRows = computed(() => gpuMemoryRows(memory.value));
const allocatableCapacity = computed(
  () =>
    memory.value?.dedicatedTotalSource === 'allocatable' &&
    memoryRows.value.some(row => row.key === 'dedicated' && row.total !== null)
);
function activityLabel(activity: GpuActivity) {
  return activity.name || t(GPU_ACTIVITY_LABEL_KEYS[activity.kind]);
}
// The prewarmed panel has its own store; settings changed in the main window
// must be reloaded when native visibility returns.
watch(
  () => props.active,
  active => {
    if (active) void settings.load();
    else deviceMenuOpen.value = false;
  }
);
onMounted(() => {
  void settings.load();
});
</script>
<template>
  <div class="gpu-details">
    <MdResourceCard class="gpu-summary">
      <div class="gpu-usage">
        <header>
          <Select
            v-if="canSelectDevice"
            v-model:open="deviceMenuOpen"
            :model-value="selectedId ?? 'automatic'"
            :disabled="!settings.draft || settings.loading || settings.saving"
            @update:model-value="settings.change({ gpuAdapter: $event === 'automatic' ? null : String($event) })"
          >
            <SelectTrigger class="device-trigger" :aria-label="`${t('systemStatus.gpu')} · ${selectedLabel}`">
              <span>{{ t('systemStatus.gpuUsage') }}</span>
              <MdIcon :name="ICON_NAMES.chevronDown" :size="12" />
            </SelectTrigger>
            <SelectContent class="z-60 max-w-[calc(100vw-2rem)]" align="start">
              <SelectItem value="automatic">{{ t('systemStatus.automatic') }}</SelectItem>
              <SelectItem
                v-for="adapter in reading.gpuAdapters"
                :key="adapter.id"
                :value="adapter.id"
                class="break-all"
                >{{ adapter.name }}</SelectItem
              >
              <SelectItem v-if="missing" :value="selectedId!">{{ t('systemStatus.savedDisconnected') }}</SelectItem>
            </SelectContent>
          </Select>
          <span v-else class="device-label">{{ t('systemStatus.gpuUsage') }}</span>
          <strong>{{ value ? value.usedPercent.toFixed(0) : '—' }}<small v-if="value">%</small></strong>
        </header>
        <MdResourceTrend
          :key="deviceIdentity"
          metric="gpu"
          :history="history"
          :observed-at-ms="reading.observedAtMs"
          :active="active"
          :label="t('systemStatus.lastMinute')"
        />
        <p class="detail-note device-caption">
          <template v-if="value">
            <MdTooltip :active="active" :text="deviceTooltip">
              <span class="device-name">{{ value.adapterName }}</span>
            </MdTooltip>
            <span v-if="readingFailed" class="reading-error" role="status" :aria-label="t(METRIC_STATUS_KEYS.failed)">{{
              t(METRIC_STATUS_KEYS.failed)
            }}</span>
          </template>
          <span v-else role="status">{{
            t(METRIC_STATUS_KEYS[reading.gpuDetails.status === 'ready' ? 'loading' : reading.gpuDetails.status])
          }}</span>
          <MdComponentPower
            v-if="powerMatches"
            metric="gpu"
            :reading="reading.gpuPower"
            :observed-at-ms="reading.observedAtMs"
            :active="active"
          />
        </p>
      </div>
    </MdResourceCard>
    <MdTemperatureDetails
      v-if="reading.gpuAdapters.length > 0 || temperatureReading.status !== 'unsupported'"
      :history="temperatureHistory"
      :observed-at-ms="reading.observedAtMs"
      :active="active"
      :label="t('gpuTemperature.trend')"
      :aria-label="t('gpuTemperature.label')"
      :identity="temperatureIdentity"
    >
      <MdResourceTemperature metric="gpu" :reading="temperatureReading" :observed-at-ms="reading.observedAtMs" />
    </MdTemperatureDetails>
    <MdResourceFacts v-if="value" :active="active" :facts="facts" class="gpu-facts" />
    <div class="gpu-details-body scrollbar-stable-end">
      <div v-if="settings.error" class="detail-note" role="alert">
        {{ t('monitoring.unavailable') }} <button @click="settings.load()">{{ t('monitoring.refresh') }}</button>
      </div>
      <template v-if="details">
        <section v-if="memoryRows.length || details.memoryStatus === 'failed'" class="memory-section">
          <h3>
            <span>{{ t('systemStatus.memory') }}</span>
            <MdTooltip
              v-if="allocatableCapacity"
              :active="active"
              :text="t('gpuDetails.allocatableMemoryHint')"
              multiline
            >
              <button type="button" class="activity-help memory-help" :aria-label="t('systemStatus.memory')">
                <MdIcon :name="ICON_NAMES.info" :size="12" />
              </button>
            </MdTooltip>
          </h3>
          <template v-if="memoryRows.length">
            <div v-for="row in memoryRows" :key="row.key" class="memory-item">
              <div class="memory-row">
                <span>{{ t(row.label) }}</span
                ><b
                  >{{ ByteSizeService.memory(row.used)
                  }}<template v-if="row.total !== null"> / {{ ByteSizeService.memory(row.total) }}</template></b
                >
              </div>
              <div
                v-if="row.percent !== null"
                class="activity-track"
                role="meter"
                :aria-label="t(row.label)"
                :aria-valuenow="row.percent"
                :aria-valuemin="0"
                :aria-valuemax="100"
              >
                <i :style="{ width: `${row.percent}%` }" />
              </div>
            </div>
          </template>
          <p v-else-if="details.memoryArchitecture !== 'unified'" class="detail-note" role="status">
            {{ t(details.memoryStatus === 'failed' ? 'systemStatus.failed' : 'systemStatus.unsupported') }}
          </p>
        </section>
        <section v-if="standard.length" class="activity-section">
          <div class="activity-heading">
            <h3>
              <span>{{ t('gpuDetails.activities') }}</span>
              <MdTooltip :active="active" :text="t('gpuDetails.activitiesHint')">
                <button type="button" class="activity-help" :aria-label="t('gpuDetails.activities')">
                  <MdIcon :name="ICON_NAMES.info" :size="12" />
                </button>
              </MdTooltip>
            </h3>
            <span v-if="standard.some(activity => engineHistory(activity.kind).length)" class="history-caption">
              {{ t('systemStatus.lastMinute') }}
            </span>
          </div>
          <div v-for="activity in standard" :key="activity.id" class="activity-row">
            <div>
              <span>{{ activityLabel(activity) }}</span
              ><b>{{ activity.usedPercent.toFixed(0) }}%</b>
            </div>
            <MdGpuEngineHistory
              v-if="engineHistory(activity.kind).length"
              :key="`${deviceIdentity}:${activity.kind}`"
              :history="engineHistory(activity.kind)"
              :observed-at-ms="reading.observedAtMs"
              :active="active"
              :label="`${activityLabel(activity)} · ${t('systemStatus.lastMinute')}`"
            />
            <div
              v-else
              class="activity-track"
              role="meter"
              :aria-label="activityLabel(activity)"
              :aria-valuenow="activity.usedPercent"
              :aria-valuemin="0"
              :aria-valuemax="100"
            >
              <i :style="{ width: `${activity.usedPercent}%` }" />
            </div>
          </div>
        </section>
        <details v-if="custom.length" class="custom-activities">
          <MdTooltip :active="active" :text="t('gpuDetails.customHint')">
            <summary>
              {{ t('gpuDetails.customActivities') }} <span>{{ custom.length }}</span>
            </summary>
          </MdTooltip>
          <div v-for="activity in custom" :key="activity.id" class="activity-row">
            <div>
              <span>{{ activityLabel(activity) }}</span
              ><b>{{ activity.usedPercent.toFixed(0) }}%</b>
            </div>
            <div
              class="activity-track"
              role="meter"
              :aria-label="activityLabel(activity)"
              :aria-valuenow="activity.usedPercent"
              :aria-valuemin="0"
              :aria-valuemax="100"
            >
              <i :style="{ width: `${activity.usedPercent}%` }" />
            </div>
          </div>
        </details>
      </template>
      <p v-else-if="value" class="detail-note" role="status">
        {{ t(METRIC_STATUS_KEYS[reading.gpuDetails.status === 'ready' ? 'loading' : reading.gpuDetails.status]) }}
      </p>
    </div>
  </div>
</template>
<style scoped>
@reference "@assets/main.css";
.gpu-details {
  display: flex;
  flex-direction: column;
  gap: var(--resource-card-gap, 8px);
  min-height: 0;
  flex: 1;
}
.gpu-details-body {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin-right: -12px;
  padding-right: 12px;
  min-height: 0;
  flex: 1;
}
.device-trigger {
  @apply text-foreground rounded-md;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  min-height: 30px;
  min-width: 0;
  max-width: 100%;
  text-align: left;
  overflow-wrap: anywhere;
  font-size: 12px;
  cursor: pointer;
  background: transparent;
}
.device-trigger:hover:not(:disabled),
.device-trigger[data-state='open'] {
  @apply bg-accent text-accent-foreground;
}
.device-trigger:disabled {
  cursor: default;
  opacity: 0.55;
}
.gpu-summary {
  min-height: var(--resource-summary-height, 104px);
  display: flex;
  flex-direction: column;
}
.gpu-usage {
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  flex: 1;
  min-height: 0;
}
header {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 8px;
  align-items: center;
  justify-content: space-between;
  min-height: 30px;
  font-size: 12px;
}
header strong {
  margin-left: auto;
  flex: none;
  font-size: 24px;
  line-height: 30px;
  font-variant-numeric: tabular-nums;
}
header small {
  font-size: 11px;
  font-weight: normal;
  margin-left: 2px;
}
.resource-trend {
  height: 28px;
}
h3 {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 12px;
  font-weight: 600;
  margin-bottom: 8px;
}
.activity-row {
  margin: 8px 0;
  font-size: 11px;
}
.activity-row > div:first-child,
.memory-row {
  display: flex;
  justify-content: space-between;
  gap: 10px;
}
.activity-row span {
  overflow-wrap: anywhere;
}
b {
  font-weight: 500;
  font-variant-numeric: tabular-nums;
  flex: none;
}
.activity-track {
  @apply rounded-full bg-muted;
  height: 4px;
  overflow: hidden;
  margin-top: 5px;
}
.activity-track i {
  display: block;
  height: 100%;
  background: var(--primary);
  border-radius: inherit;
}
.detail-note {
  @apply text-muted-foreground;
  font-size: 10px;
  line-height: 1.6;
  margin-top: 6px;
}
.device-caption {
  display: flex;
  align-items: center;
  gap: 6px;
}
.reading-error {
  @apply rounded bg-muted text-muted-foreground;
  flex: none;
  padding: 0 4px;
  font-size: 9px;
}
.device-name {
  min-width: 0;

  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.gpu-summary .detail-note {
  min-height: 18px;
  line-height: 16px;
  margin-top: 4px;
}
.history-caption {
  @apply text-muted-foreground;
  font-size: 10px;
  line-height: 1.5;
  overflow-wrap: anywhere;
}
.history-caption {
  flex: none;
}
.activity-heading {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 8px;
}
.activity-heading h3 {
  margin-bottom: 0;
  min-width: 0;
}
.activity-help {
  @apply text-muted-foreground;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: none;
  padding: 2px;
  border-radius: 3px;
  cursor: help;
}
.activity-help:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}
.activity-help:hover {
  @apply text-primary-text;
}
summary {
  width: fit-content;
  max-width: 100%;
  cursor: pointer;
  font-size: 11px;
}
summary span {
  @apply text-muted-foreground;
  margin-left: 4px;
}
summary:focus-visible,
button:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}
.memory-row {
  font-size: 11px;
  margin: 8px 0;
  flex-wrap: wrap;
}
.memory-item + .memory-item {
  margin-top: 10px;
}
.detail-note button {
  @apply text-primary-text;
  cursor: pointer;
  text-decoration: underline;
}
</style>
