<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import type { ComponentPower, MetricReading } from '@/lib/models/system-resources/system-resources';
import { hasFreshValue } from '@/lib/utils/system-resources/metric-reading';
import MdTooltip from '@/components/custom/md-tooltip.vue';

const props = defineProps<{
  metric: 'cpu' | 'gpu';
  reading: MetricReading<ComponentPower>;
  observedAtMs: number;
  active: boolean;
}>();
const { t } = useI18n({ useScope: 'global' });
const watts = computed(() => {
  const value = props.reading.value;
  return hasFreshValue(props.reading, props.observedAtMs, 5000) &&
    value &&
    Number.isFinite(value.watts) &&
    value.watts >= 0 &&
    value.watts <= 1000
    ? value.watts.toFixed(1)
    : null;
});
const label = computed(() => t(props.metric === 'cpu' ? 'systemStatus.cpuPower' : 'systemStatus.gpuPower'));
</script>
<template>
  <MdTooltip
    v-if="watts !== null"
    :active="active"
    :text="`${label} · ${t('systemStatus.componentPowerHint')}`"
    multiline
  >
    <span class="component-power" :data-component-power="metric" :aria-label="`${label} ${watts} W`" tabindex="0"
      >{{ watts }}<small>W</small></span
    >
  </MdTooltip>
</template>
<style scoped>
@reference "@assets/main.css";
.component-power {
  @apply text-foreground;
  margin-left: auto;
  flex: none;
  white-space: nowrap;
  font-size: 11px;
  font-weight: 500;
  line-height: 16px;
  font-variant-numeric: tabular-nums;
}
small {
  @apply text-muted-foreground;
  margin-left: 3px;
  font-size: 10px;
  font-weight: normal;
}
</style>
