<script setup lang="ts">
import MdComponentPower from './md-component-power.vue';
import { computed } from 'vue';
import MdResourceFacts from './md-resource-facts.vue';
import MdCpuTemperature from './md-cpu-temperature.vue';
import MdTemperatureDetails from './md-temperature-details.vue';
import MdResourceOverview from './md-resource-overview.vue';
import { useI18n } from 'vue-i18n';
import { summarizeUtilizationHistory } from '@/lib/utils/system-resources/utilization-history-summary';
import type { ResourceReadings } from '@/lib/models/system-resources/system-resources';
import { METRIC_STATUS_KEYS } from '@/lib/models/system-resources/system-resources';
import { observedCpuTemperature } from '@/lib/utils/system-resources/cpu-temperature';
const props = withDefaults(
  defineProps<{
    reading: ResourceReadings;
    active?: boolean;
    temperatureEnabled?: boolean;
    temperatureSupported?: boolean;
  }>(),
  { active: true, temperatureEnabled: true, temperatureSupported: true }
);
defineEmits<{ configureTemperature: [] }>();
const { t } = useI18n({ useScope: 'global' });
const temperatureAvailable = computed(
  () =>
    props.temperatureEnabled &&
    observedCpuTemperature(props.reading.cpuTemperature, props.reading.observedAtMs) !== null
);
const facts = computed(() => {
  const frequencyReading = props.reading.cpuFrequency;
  const frequency =
    frequencyReading.status === 'failed' || frequencyReading.status === 'unsupported' ? null : frequencyReading.value;
  const frequencyFacts = [
    { label: 'cpuDetails.average', mhz: frequency?.averageMhz },
    { label: 'cpuDetails.efficiency', mhz: frequency?.efficiencyMhz },
    { label: 'cpuDetails.performance', mhz: frequency?.performanceMhz },
    { label: 'cpuDetails.super', mhz: frequency?.superMhz },
  ]
    .filter(fact => fact.mhz != null && Number.isFinite(fact.mhz) && fact.mhz >= 1 && fact.mhz <= 20_000)
    .map(fact => ({
      label: fact.label,
      value: (fact.mhz! / 1000).toFixed(1),
      unit: 'GHz',
    }));
  const hasFrequencyClasses = frequencyFacts.filter(fact => fact.label !== 'cpuDetails.average').length >= 2;
  const selectedFrequencyFacts = hasFrequencyClasses
    ? frequencyFacts.filter(fact => fact.label !== 'cpuDetails.average')
    : frequencyFacts;
  const history = summarizeUtilizationHistory(props.reading.cpuHistory, props.reading.observedAtMs);
  const usageFacts = history
    ? [
        { label: 'cpuDetails.minuteAverage', value: history.average },
        { label: 'cpuDetails.minutePeak', value: history.peak },
      ]
        .filter(fact => !hasFrequencyClasses || fact.label !== 'cpuDetails.minutePeak')
        .map(fact => ({
          label: fact.label,
          value: Math.round(fact.value).toString(),
          unit: '%',
          hint: t('cpuDetails.coverage', { seconds: (history.coveredMs / 1000).toFixed(1) }),
          liveHint: true,
        }))
    : [];
  return [...selectedFrequencyFacts, ...usageFacts].slice(0, 3).map(fact => ({
    ...fact,
    key: fact.label,
    label: t(fact.label),
  }));
});
</script>
<template>
  <MdResourceOverview
    class="cpu-summary"
    metric="cpu"
    :heading="t('systemStatus.cpuUsage')"
    :show-cpu-temperature="false"
    :reading="reading"
    :active="active"
  >
    <template #meta-end>
      <MdComponentPower
        metric="cpu"
        :reading="reading.cpuPower"
        :observed-at-ms="reading.observedAtMs"
        :active="active"
      />
    </template>
  </MdResourceOverview>
  <MdTemperatureDetails
    v-if="temperatureSupported && reading.cpuTemperature.status !== 'unsupported'"
    :history="temperatureEnabled ? reading.cpuTemperatureHistory : []"
    :observed-at-ms="reading.observedAtMs"
    :active="active"
    :label="t('cpuTemperature.trend')"
    :identity="`${reading.cpuTemperature.value?.source}:${reading.cpuTemperature.value?.kind}:${reading.cpuTemperature.value?.sensorCount}`"
    :aria-label="t('cpuTemperature.label')"
  >
    <div v-if="!temperatureEnabled" class="temperature-disabled-heading">
      <span>{{ t('cpuTemperature.label') }}</span
      ><strong>—</strong>
    </div>
    <MdCpuTemperature
      v-else
      :active="active"
      :reading="reading.cpuTemperature"
      :observed-at-ms="reading.observedAtMs"
      :show-status="false"
    />
    <template v-if="!temperatureAvailable" #placeholder>
      <div class="temperature-placeholder" role="status">
        <span>{{
          t(
            !temperatureEnabled
              ? 'windowsMonitoring.collectionOff'
              : METRIC_STATUS_KEYS[reading.cpuTemperature.status === 'ready' ? 'stale' : reading.cpuTemperature.status]
          )
        }}</span>
        <button v-if="!temperatureEnabled" type="button" @click="$emit('configureTemperature')">
          {{ t('windowsMonitoring.goToSettings') }}
        </button>
      </div>
    </template>
  </MdTemperatureDetails>
  <MdResourceFacts :active="active" :facts="facts" class="cpu-details" :aria-label="t('cpuDetails.summary')" />
</template>
<style scoped>
@reference "@assets/main.css";
.temperature-disabled-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  min-height: 30px;
  font-size: 12px;
}
.temperature-disabled-heading strong {
  font-size: 24px;
  line-height: 30px;
}
.temperature-placeholder {
  @apply text-muted-foreground;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: center;
  gap: 2px 8px;
  font-size: 11px;
  text-align: center;
}
.temperature-placeholder button {
  @apply text-primary-text;
  cursor: pointer;
  text-decoration: underline;
  text-underline-offset: 3px;
  border-radius: 3px;
  padding: 2px;
}
.temperature-placeholder button:hover {
  @apply bg-accent;
}
.temperature-placeholder button:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}
.cpu-summary :deep(.resource-trend.cpu) {
  height: 28px;
}
.cpu-details {
  grid-template-columns: none;
  grid-auto-flow: column;
  grid-auto-columns: minmax(0, 1fr);
}
</style>
