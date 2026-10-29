<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import type { MetricReading } from '@/lib/models/system-resources/system-resources';
import { cachedMetricValue } from '@/lib/utils/system-resources/metric-reading';

const props = defineProps<{ reading: MetricReading<unknown>; now: number; active: boolean }>();
const { t, locale } = useI18n();
const cached = computed(() => props.reading.status !== 'ready' && cachedMetricValue(props.reading, props.now) !== null);
const hint = computed(() =>
  cached.value
    ? t('systemStatus.lastUpdatedAt', {
        time: new Intl.DateTimeFormat(locale.value, { hour: '2-digit', minute: '2-digit', second: '2-digit' }).format(
          props.reading.sampledAtMs!
        ),
      })
    : ''
);
</script>

<template>
  <MdTooltip v-if="cached" :text="hint" :active="active">
    <small class="text-muted-foreground" role="status" tabindex="0">{{
      t(reading.status === 'failed' ? 'systemStatus.failed' : 'systemStatus.cachedReading')
    }}</small>
  </MdTooltip>
</template>
