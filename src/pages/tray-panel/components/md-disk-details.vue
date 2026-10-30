<script setup lang="ts">
import { cachedMetricValue } from '@/lib/utils/system-resources/metric-reading';
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { METRIC_STATUS_KEYS, type ResourceReadings } from '@/lib/models/system-resources/system-resources';
import { ByteSizeService } from '@/lib/services/settings/byte-size-service';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdResourceOverview from './md-resource-overview.vue';
import MdResourceFacts from './md-resource-facts.vue';
import MdResourceTrend from './md-resource-trend.vue';
import MdMetricFreshness from './md-metric-freshness.vue';

const HEALTH_LABEL_KEYS = {
  healthy: 'diskHealth.healthy',
  warning: 'diskHealth.warning',
  unhealthy: 'diskHealth.unhealthy',
  unknown: 'diskHealth.unknown',
} as const;
const props = defineProps<{ reading: ResourceReadings; active: boolean }>();
defineEmits<{ cleanup: [] }>();
const { t, n } = useI18n({ useScope: 'global' });
const health = computed(() => {
  const current = props.reading.diskHealth;
  const value = cachedMetricValue(current, props.reading.observedAtMs);
  return props.reading.disk.status !== 'disconnected' &&
    props.reading.disk.status !== 'unsupported' &&
    value?.volumeId === props.reading.disk.value?.volume.id
    ? value
    : null;
});
const facts = computed(() => {
  const value = health.value;
  if (!value) return [];
  const rows = [];
  if (value.percentageUsed !== null)
    rows.push({
      key: 'health',
      label: t('diskHealth.healthPercent'),
      hint: t('diskHealth.wearHint'),
      value: n(Math.max(0, 100 - value.percentageUsed)),
      unit: '%',
    });
  if (value.totalWrittenBytes !== null) {
    const bytes = Number(value.totalWrittenBytes);
    if (Number.isFinite(bytes) && bytes >= 0)
      rows.push({ key: 'written', label: t('diskHealth.written'), value: ByteSizeService.bytes(bytes), unit: '' });
  }
  if (value.powerOnHours !== null)
    rows.push({
      key: 'hours',
      label: t('diskHealth.powerOnHours'),
      value: n(value.powerOnHours),
      unit: t('diskHealth.hoursUnit'),
    });
  return rows;
});
const detailFacts = computed(() => {
  const value = health.value;
  if (!value) return [];
  const rows = [];
  if (value.totalReadBytes !== null)
    rows.push({
      key: 'read',
      label: t('diskHealth.read'),
      value: ByteSizeService.bytes(Number(value.totalReadBytes)),
      unit: '',
    });
  const temperature = props.reading.diskDetailTemperature;
  rows.push({
    key: 'temperature',
    label: t('diskHealth.temperature'),
    value:
      cachedMetricValue(temperature, props.reading.observedAtMs)?.volumeId === value.volumeId
        ? n(temperature.value!.celsius, { maximumFractionDigits: 0 })
        : '—',
    unit: cachedMetricValue(temperature, props.reading.observedAtMs)?.volumeId === value.volumeId ? '°C' : '',
  });
  if (value.powerCycles !== null)
    rows.push({ key: 'cycles', label: t('diskHealth.powerCycles'), value: n(value.powerCycles), unit: '' });
  if (value.availableSpare !== null)
    rows.push({ key: 'spare', label: t('diskHealth.spare'), value: n(value.availableSpare), unit: '%' });
  if (value.criticalWarning !== null)
    rows.push({
      key: 'warnings',
      label: t('diskHealth.criticalWarning'),
      value: t(value.criticalWarning === 0 ? 'diskHealth.none' : 'diskHealth.warning'),
      unit: '',
    });
  if (value.mediaErrors !== null)
    rows.push({ key: 'errors', label: t('diskHealth.mediaErrors'), value: value.mediaErrors, unit: '' });
  return rows;
});
const activities = computed(() => {
  const value = cachedMetricValue(props.reading.diskIo, props.reading.observedAtMs);
  return [
    {
      key: 'read',
      label: t('systemStatus.read'),
      direction: 'download',
      rate: value ? `${ByteSizeService.bytes(value.readBytesPerSecond)}/s` : '—',
      totalLabel: t('diskDetails.systemRead'),
      value: value ? ByteSizeService.bytes(Number(value.totalReadBytes)) : '—',
    },
    {
      key: 'written',
      label: t('systemStatus.write'),
      direction: 'upload',
      rate: value ? `${ByteSizeService.bytes(value.writtenBytesPerSecond)}/s` : '—',
      totalLabel: t('diskDetails.systemWritten'),
      value: value ? ByteSizeService.bytes(Number(value.totalWrittenBytes)) : '—',
    },
  ];
});
</script>

<template>
  <MdResourceOverview
    metric="disk"
    cleanup-variant="button"
    :reading="reading"
    :active="active"
    class="detail-summary"
    @cleanup="$emit('cleanup')"
  />
  <section class="disk-activity" :aria-label="t('diskDetails.activity')">
    <header>
      <MdTooltip :active="active" :text="t('diskDetails.systemHint')" multiline
        ><h2 tabindex="0">{{ t('diskDetails.activity') }}</h2></MdTooltip
      >
      <span class="health-unavailable"
        >{{ t('systemStatus.allDisks') }} · {{ t('systemStatus.lastMinute')
        }}<template v-if="reading.diskIo.status === 'failed'"> · {{ t('systemStatus.failed') }}</template></span
      >
    </header>
    <div class="activity-columns">
      <dl v-for="activity in activities" :key="activity.key" class="activity-column">
        <div class="activity-rate">
          <dt :class="activity.direction">{{ activity.label }}</dt>
          <dd>{{ activity.rate }}</dd>
        </div>
        <div class="activity-total">
          <dt>{{ activity.totalLabel }}</dt>
          <dd>{{ activity.value }}</dd>
        </div>
      </dl>
    </div>
    <MdResourceTrend
      metric="disk"
      :history="reading.diskIoHistory"
      :observed-at-ms="reading.observedAtMs"
      :label="t('diskDetails.activity')"
      :active="active"
    />
  </section>
  <section
    class="disk-health"
    :aria-label="t('diskHealth.title')"
    :aria-busy="!health && reading.diskHealth.status === 'loading'"
  >
    <header>
      <h2>{{ t('diskHealth.title') }}</h2>
      <div class="health-state">
        <span class="health-freshness">
          <MdMetricFreshness
            v-if="health"
            :reading="reading.diskHealth"
            :now="reading.observedAtMs"
            :active="active"
            compact
          />
        </span>
        <MdTooltip
          :active="active"
          multiline
          :text="
            !health
              ? t('diskHealth.unavailableHint')
              : health.source === 'windowsStorage'
                ? t('diskHealth.systemHint')
                : t('diskHealth.smartHint')
          "
        >
          <!-- Keep one DOM anchor while asynchronous health readings change state. -->
          <span
            :class="health ? 'health-status' : 'health-unavailable'"
            :data-status="health?.status"
            :data-current="reading.diskHealth.status === 'ready'"
            :role="health ? undefined : 'status'"
            tabindex="0"
          >
            <template v-if="health">{{ t(HEALTH_LABEL_KEYS[health.status]) }}</template>
            <template v-if="!health">{{
              t(
                reading.diskHealth.status === 'ready'
                  ? 'diskHealth.unavailable'
                  : METRIC_STATUS_KEYS[reading.diskHealth.status]
              )
            }}</template>
          </span>
        </MdTooltip>
      </div>
    </header>
    <div v-if="reading.disk.value" class="volume-identity">
      {{ reading.disk.value.volume.name
      }}<span v-if="reading.disk.value.volume.fileSystem"> · {{ reading.disk.value.volume.fileSystem }}</span>
    </div>
    <MdResourceFacts :active="active" :facts="facts" />
    <MdResourceFacts :active="active" :facts="detailFacts" layout="rows" class="health-metadata" />
  </section>
</template>

<style scoped>
@reference "@assets/main.css";
.disk-health,
.disk-activity {
  padding: 12px;
  display: flex;
  flex-direction: column;
  flex: none;
  gap: 6px;
}
.disk-health {
  @apply border-t border-border;
  padding-inline: 0;
}
header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
h2 {
  font-size: 12px;
  font-weight: 600;
}
.health-status,
.health-unavailable {
  @apply text-muted-foreground;
  font-size: 11px;
}
.health-state {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  min-height: 20px;
}
.health-freshness {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  flex: none;
}
.health-status {
  border-radius: 4px;
  padding: 2px 6px;
  line-height: 16px;
}
.health-state > :last-child {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
.disk-health h2 {
  flex: none;
}
.health-status[data-current='true'][data-status='healthy'] {
  @apply bg-success/12 text-success-foreground;
}
.health-status[data-current='true'][data-status='warning'] {
  @apply bg-warning/12 text-warning-foreground;
}
.health-status[data-current='true'][data-status='unhealthy'] {
  @apply bg-destructive/10 text-destructive-text;
}
.health-status[data-status='warning'] {
  @apply text-warning-foreground;
}
.health-status[data-status='unhealthy'] {
  @apply text-destructive-text;
}
.disk-activity :deep(.resource-trend) {
  height: 28px;
}
.activity-columns {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 20px;
}
.activity-column {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.activity-rate {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
}
.activity-rate dt {
  font-size: 11px;
  line-height: 16px;
}
.activity-rate dd {
  font-size: 16px;
  line-height: 22px;
  font-weight: 600;
}
.activity-total {
  @apply text-muted-foreground;
  display: flex;
  align-items: baseline;
  gap: 6px;
  font-size: 10px;
  line-height: 15px;
}
.activity-total dt {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.activity-column dd {
  flex: none;
  font-variant-numeric: tabular-nums;
}
.volume-identity {
  @apply text-muted-foreground;
  font-size: 11px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.disk-activity {
  @apply rounded-lg bg-muted/40;
  gap: 8px;
}
.disk-health .health-metadata {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 0 16px;
}
.disk-health .health-metadata.compact {
  grid-template-columns: minmax(0, 1fr);
}
.upload {
  color: var(--status-upload);
}
.download {
  color: var(--status-download);
}
.health-metadata :deep(dt) {
  min-width: 0;
  flex: 1;
}
</style>
