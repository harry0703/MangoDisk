<script setup lang="ts">
import { computed } from 'vue';
import MdResourceFacts from './md-resource-facts.vue';
import MdCpuTemperature from './md-cpu-temperature.vue';
import MdResourceTrend from './md-resource-trend.vue';
import { useI18n } from 'vue-i18n';
import { summarizeUtilizationHistory } from '@/lib/utils/utilization-history-summary';
import type { ResourceReadings } from '@/lib/models/system-resources';
const props = withDefaults(defineProps<{ reading: ResourceReadings; active?: boolean }>(), { active: true });
const { t } = useI18n({ useScope: 'global' });
const facts = computed(() => {
  const frequencyReading = props.reading.cpuFrequency;
  const frequency =
    frequencyReading.status === 'failed' || frequencyReading.status === 'unsupported' ? null : frequencyReading.value;
  const frequencyFacts = [
    { label: 'cpuDetails.average', mhz: frequency?.averageMhz },
    { label: 'cpuDetails.efficiency', mhz: frequency?.efficiencyMhz },
    { label: 'cpuDetails.performance', mhz: frequency?.performanceMhz },
  ]
    .filter(fact => fact.mhz != null && Number.isFinite(fact.mhz) && fact.mhz >= 1 && fact.mhz <= 20_000)
    .map(fact => ({
      label: fact.label,
      value: (fact.mhz! / 1000).toFixed(2),
      unit: 'GHz',
      cached: frequencyReading.status !== 'ready',
    }));
  const hasFrequencyClasses =
    frequencyFacts.some(fact => fact.label === 'cpuDetails.efficiency') &&
    frequencyFacts.some(fact => fact.label === 'cpuDetails.performance');
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
          cached: props.reading.cpu.status !== 'ready',
        }))
    : [];
  return [...selectedFrequencyFacts, ...usageFacts].map(fact => ({
    ...fact,
    key: fact.label,
    label: t(fact.label),
  }));
});
</script>
<template>
  <MdResourceFacts :facts="facts" class="cpu-details" :aria-label="t('cpuDetails.summary')" />
  <section class="temperature-details" :aria-label="t('cpuTemperature.label')">
    <MdCpuTemperature :reading="reading.cpuTemperature" :observed-at-ms="reading.observedAtMs" />
    <div v-if="reading.cpuTemperatureHistory.length" class="temperature-chart">
      <div class="temperature-axis" aria-hidden="true"><span>150°C</span><span>0°C</span></div>
      <MdResourceTrend
        :key="`${reading.cpuTemperature.value?.source}:${reading.cpuTemperature.value?.kind}:${reading.cpuTemperature.value?.sensorCount}`"
        metric="temperature"
        :history="reading.cpuTemperatureHistory"
        :observed-at-ms="reading.observedAtMs"
        :active="active"
        :label="t('cpuTemperature.trend')"
      />
    </div>
  </section>
</template>
<style scoped>
@reference "@assets/main.css";
.temperature-details {
  @apply rounded-lg bg-muted/40;
  padding: 8px;
  flex: none;
}
.temperature-axis {
  @apply text-muted-foreground;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  font-size: 9px;
  padding-top: 4px;
}
.temperature-chart {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: 6px;
}
</style>
