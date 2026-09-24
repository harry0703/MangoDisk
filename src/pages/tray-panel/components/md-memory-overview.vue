<script setup lang="ts">
import MdTooltip from '@/components/custom/md-tooltip.vue';
import { computed } from 'vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/ui';
import { useI18n } from 'vue-i18n';
import type { MemoryReleaseResult } from '@/lib/models/resident';
import type { MemoryOverview } from '@/lib/models/system-resources';
import { ByteSizeService } from '@/lib/services/byte-size-service';

const props = withDefaults(
  defineProps<{
    memory: MemoryOverview;
    releasing?: boolean;
    releaseResult?: MemoryReleaseResult | null;
    releaseAvailable?: boolean;
  }>(),
  { releaseAvailable: true, releaseResult: null, releasing: false }
);
defineEmits<{ release: [] }>();
const { t } = useI18n({ useScope: 'global' });
const resultMessage = computed(() => {
  const result = props.releaseResult;
  if (!result) return '';
  switch (result.status) {
    case 'completed':
      if (result.observedReductionBytes === null) return t('monitoring.releaseUnmeasured');
      return result.observedReductionBytes > 0
        ? t('monitoring.releaseObserved', { size: ByteSizeService.memory(result.observedReductionBytes) })
        : t('monitoring.releaseNoChange');
    case 'cancelled':
      return t('monitoring.releaseCancelled');
    case 'unsupported':
      return t('monitoring.releaseUnsupported');
    case 'busy':
      return t('monitoring.releaseBusy');
    default:
      return t('monitoring.releaseFailed');
  }
});
const shortLabel = computed(() => {
  if (props.releasing) return t('monitoring.releasing');
  const result = props.releaseResult;
  if (!result) return t('monitoring.release');
  if (result.status === 'completed') {
    if (result.observedReductionBytes === null) return t('monitoring.releaseButtonUnmeasured');
    if (result.observedReductionBytes <= 0 || props.memory.totalBytes <= 0)
      return t('monitoring.releaseButtonNoChange');
    // Show the observed change against total RAM, never the difference of rounded counters.
    const percent = Math.min(100, (result.observedReductionBytes / props.memory.totalBytes) * 100);
    return t('monitoring.releaseButtonReduced', {
      percent: percent < 0.1 ? '<0.1' : String(Math.floor(percent * 10) / 10),
    });
  }
  return t(
    result.status === 'failed'
      ? 'monitoring.releaseButtonFailed'
      : result.status === 'busy'
        ? 'monitoring.releaseButtonBusy'
        : result.status === 'cancelled'
          ? 'monitoring.releaseButtonCancelled'
          : 'monitoring.releaseButtonUnsupported'
  );
});
</script>

<template>
  <section class="memory-overview" :aria-label="t('monitoring.memory')">
    <div class="memory-heading">
      <div class="memory-total">
        <span>{{ t('monitoring.memory') }}</span>
        <strong>{{ ByteSizeService.memory(memory.usedBytes) }}</strong>
        <span>/ {{ ByteSizeService.memory(memory.totalBytes) }}</span>
        <span class="memory-percent">{{ memory.usedPercent }}%</span>
      </div>
      <MdTooltip v-if="releaseAvailable" :text="resultMessage || undefined"
        ><button
          class="release-button"
          :disabled="releasing"
          :aria-busy="releasing"
          :aria-label="t('monitoring.release')"
          @click="$emit('release')"
        >
          <MdIcon
            :name="
              releasing
                ? ICON_NAMES.refresh
                : releaseResult?.status === 'completed' && (releaseResult.observedReductionBytes ?? 0) > 0
                  ? ICON_NAMES.check
                  : ICON_NAMES.startup
            "
            :class="{ 'animate-spin motion-reduce:animate-none': releasing }"
            :size="12"
          />
          <span class="release-label" role="status" aria-live="polite" aria-atomic="true">{{ shortLabel }}</span>
        </button></MdTooltip
      >
    </div>
    <div
      class="memory-meter"
      role="progressbar"
      :aria-label="t('monitoring.memoryUsage')"
      :aria-valuenow="memory.usedPercent"
      :aria-valuemin="0"
      :aria-valuemax="100"
    >
      <span :style="{ width: `${memory.usedPercent}%` }" />
    </div>
    <div class="memory-details">
      <span>{{ t('monitoring.free') }} {{ ByteSizeService.memory(memory.freeBytes) }}</span>
      <span>{{ t('monitoring.swap') }} {{ ByteSizeService.memory(memory.swapUsedBytes) }}</span>
    </div>
    <div v-if="$slots.settings" class="memory-settings"><slot name="settings" /></div>
    <span v-if="releaseAvailable && resultMessage" class="sr-only" role="status" aria-live="polite">{{
      resultMessage
    }}</span>
  </section>
</template>

<style scoped>
@reference "@assets/main.css";
.memory-overview {
  @apply rounded-xl border border-border bg-card;
  padding: 10px 12px;
  flex: none;
}
.memory-heading,
.memory-total {
  display: flex;
  align-items: center;
  gap: 8px;
}
.memory-heading {
  justify-content: space-between;
  font-size: 12px;
}
.release-button {
  @apply bg-primary text-primary-foreground;
  flex: none;
  max-width: 44%;
  min-width: 0;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border-radius: 6px;
  min-height: 26px;
  padding: 3px 10px;
  font-size: 11px;
  font-weight: 550;
  cursor: pointer;
}
.release-button > :first-child {
  flex: none;
}
.release-label {
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.release-button:hover:not(:disabled) {
  @apply bg-primary/90;
}
.release-button:disabled {
  opacity: 0.55;
  cursor: default;
}
.release-button:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}
.memory-total {
  align-items: baseline;
  flex-wrap: wrap;
  min-width: 0;
  gap: 2px 6px;
  font-variant-numeric: tabular-nums;
}
.memory-total strong {
  font-size: 17px;
  font-weight: 650;
  letter-spacing: -0.03em;
}
.memory-total > span {
  @apply text-muted-foreground;
  font-size: 11px;
}
.memory-total > .memory-percent {
  @apply text-primary;
  font-weight: 600;
}
.memory-meter {
  @apply bg-muted;
  overflow: hidden;
  height: 4px;
  border-radius: 4px;
  margin-top: 7px;
}
.memory-meter > span {
  @apply bg-primary;
  display: block;
  height: 100%;
  border-radius: inherit;
}
.memory-settings {
  border-top: 1px solid var(--border);
  margin-top: 7px;
  padding-top: 3px;
}
.memory-details {
  @apply text-muted-foreground;
  display: flex;
  flex-wrap: wrap;
  gap: 4px 12px;
  margin-top: 7px;
  font-size: 10px;
}
</style>
