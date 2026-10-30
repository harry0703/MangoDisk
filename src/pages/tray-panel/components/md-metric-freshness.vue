<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import type { MetricReading } from '@/lib/models/system-resources/system-resources';
import { cachedMetricValue } from '@/lib/utils/system-resources/metric-reading';

const props = defineProps<{ reading: MetricReading<unknown>; now: number; active: boolean; compact?: boolean }>();
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
const label = computed(() =>
  t(props.reading.status === 'failed' ? 'systemStatus.failed' : 'systemStatus.cachedReading')
);
</script>

<template>
  <MdTooltip v-if="cached" :text="compact ? `${label}\n${hint}` : hint" :active="active" multiline>
    <small
      :class="compact && reading.status === 'failed' ? 'text-warning-foreground' : 'text-muted-foreground'"
      role="status"
      tabindex="0"
    >
      <MdIcon
        v-if="compact"
        :name="reading.status === 'failed' ? ICON_NAMES.info : ICON_NAMES.refresh"
        :size="12"
        aria-hidden="true"
      />
      <span :class="{ 'sr-only': compact }">{{ label }}</span>
    </small>
  </MdTooltip>
</template>
