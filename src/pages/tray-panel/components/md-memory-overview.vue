<script setup lang="ts">
import MdTooltip from '@/components/custom/md-tooltip.vue';
import { computed, ref, watch } from 'vue';
import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuPortal,
  DropdownMenuRoot,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from 'reka-ui';
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/ui';
import { useI18n } from 'vue-i18n';
import type { MemoryReleaseResult } from '@/lib/models/resident';
import type { MemoryOverview, MetricStatus } from '@/lib/models/system-resources';
import { ByteSizeService } from '@/lib/services/byte-size-service';
import MdMemoryPressure from './md-memory-pressure.vue';

const props = withDefaults(
  defineProps<{
    memory: MemoryOverview;
    status?: MetricStatus;
    releasing?: boolean;
    releaseResult?: MemoryReleaseResult | null;
    releaseAvailable?: boolean;
    active?: boolean;
    automaticRelease?: boolean | null;
    releaseSettingsFailed?: boolean;
    automaticReleaseRule?: string;
  }>(),
  {
    status: 'ready',
    releaseAvailable: true,
    releaseResult: null,
    releasing: false,
    active: true,
    automaticRelease: null,
    releaseSettingsFailed: false,
    automaticReleaseRule: '',
  }
);
defineEmits<{ release: []; settings: []; reloadSettings: [] }>();
const optionsOpen = ref(false);
watch(
  () => props.active,
  active => {
    if (!active) optionsOpen.value = false;
  }
);
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
    <header class="memory-heading">
      <div class="memory-title">
        <span>{{ t('monitoring.memory') }}</span>
        <MdMemoryPressure :pressure="memory.pressure" :status="status" :active="active" />
      </div>
      <strong class="memory-percent">{{ memory.usedPercent }}<small>%</small></strong>
    </header>
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
    <div class="memory-footer">
      <span class="memory-capacity">{{
        t('monitoring.usedCapacity', {
          used: ByteSizeService.memory(memory.usedBytes),
          total: ByteSizeService.memory(memory.totalBytes),
        })
      }}</span>
      <div class="release-actions" :class="{ 'release-supported': releaseAvailable }">
        <MdTooltip v-if="releaseAvailable" :text="resultMessage || undefined">
          <button
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
                    : ICON_NAMES.zap
              "
              :class="{ 'animate-spin motion-reduce:animate-none': releasing }"
              :size="12"
            />
            <span class="release-label" role="status" aria-live="polite" aria-atomic="true">{{ shortLabel }}</span>
          </button>
        </MdTooltip>
        <DropdownMenuRoot v-model:open="optionsOpen">
          <DropdownMenuTrigger as-child>
            <button class="release-menu-button" :aria-label="t('monitoring.memoryOptions')">
              <MdIcon :name="ICON_NAMES.chevronDown" :size="12" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuPortal>
            <DropdownMenuContent align="end" :side-offset="6" class="md-memory-options">
              <template v-if="releaseAvailable">
                <DropdownMenuLabel class="md-memory-options-label">
                  {{
                    releaseSettingsFailed
                      ? t('memoryRelease.failed')
                      : automaticRelease === null
                        ? t('systemStatus.loading')
                        : t(automaticRelease ? 'memoryRelease.autoOn' : 'memoryRelease.autoOff')
                  }}
                </DropdownMenuLabel>
                <p v-if="automaticReleaseRule && !releaseSettingsFailed" class="md-memory-options-note">
                  {{ automaticReleaseRule }}
                </p>
                <DropdownMenuItem
                  v-if="releaseSettingsFailed"
                  class="md-memory-options-item"
                  @select="$emit('reloadSettings')"
                >
                  {{ t('memoryRelease.reload') }}
                </DropdownMenuItem>
                <DropdownMenuItem class="md-memory-options-item" @select="$emit('settings')">
                  <MdIcon :name="ICON_NAMES.settings" :size="14" />{{ t('memoryRelease.entry') }}
                </DropdownMenuItem>
                <DropdownMenuSeparator class="md-memory-options-separator" />
              </template>
              <div class="md-memory-options-details">
                <span>{{ t('monitoring.free') }}</span
                ><strong>{{ ByteSizeService.memory(memory.freeBytes) }}</strong> <span>{{ t('monitoring.swap') }}</span
                ><strong>{{ ByteSizeService.memory(memory.swapUsedBytes) }}</strong>
              </div>
            </DropdownMenuContent>
          </DropdownMenuPortal>
        </DropdownMenuRoot>
      </div>
    </div>
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
.memory-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  min-width: 0;
}
.memory-heading {
  min-height: 30px;
  font-size: 12px;
}
.memory-title {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  line-height: 20px;
}
.memory-title > :first-child {
  flex: none;
  white-space: nowrap;
}
.memory-percent {
  font-size: 24px;
  line-height: 30px;
  font-variant-numeric: tabular-nums;
}
.memory-percent small {
  margin-left: 2px;
  font-size: 11px;
  font-weight: normal;
}
.memory-capacity {
  @apply text-muted-foreground;
  min-width: 0;
  font-size: 10px;
  font-variant-numeric: tabular-nums;
}
.release-actions {
  display: inline-flex;
  align-items: stretch;
  flex: none;
  max-width: 48%;
  min-width: 0;
  border-radius: 6px;
}
.release-supported {
  @apply bg-primary text-primary-foreground;
}
.release-button,
.release-menu-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-height: 26px;
  cursor: pointer;
  border-radius: 6px;
}
.release-button {
  min-width: 0;
  gap: 4px;
  padding: 3px 8px;
  font-size: 11px;
  font-weight: 550;
  border-top-right-radius: 0;
  border-bottom-right-radius: 0;
}
.release-menu-button {
  flex: none;
  width: 24px;
}
.release-supported .release-menu-button {
  border-left: 1px solid color-mix(in srgb, var(--primary-foreground) 25%, transparent);
  border-top-left-radius: 0;
  border-bottom-left-radius: 0;
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
.release-button:hover:not(:disabled),
.release-supported .release-menu-button:hover {
  background: color-mix(in srgb, var(--primary-foreground) 12%, transparent);
}
.release-menu-button:hover {
  @apply bg-accent;
}
.release-button:disabled {
  opacity: 0.55;
  cursor: default;
}
.release-button:focus-visible,
.release-menu-button:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}
.memory-meter {
  @apply bg-muted;
  overflow: hidden;
  height: 4px;
  border-radius: 4px;
}
.memory-meter > span {
  @apply bg-primary;
  display: block;
  height: 100%;
  border-radius: inherit;
}
</style>

<style>
@reference "@assets/main.css";
/* Portal content does not inherit the overview component scope. */
.md-memory-options {
  @apply rounded-lg border border-border bg-popover text-popover-foreground shadow-lg;
  z-index: 50;
  width: 240px;
  max-width: calc(100vw - 24px);
  max-height: var(--reka-dropdown-menu-content-available-height);
  overflow-y: auto;
  padding: 4px;
  font-size: 11px;
}
.md-memory-options-label {
  padding: 6px 8px;
  font-weight: 550;
}
.md-memory-options-note {
  @apply text-muted-foreground;
  padding: 0 8px 6px;
  font-size: 10px;
  line-height: 1.5;
}
.md-memory-options-item {
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 28px;
  padding: 4px 8px;
  border-radius: 4px;
  cursor: pointer;
  outline: none;
}
.md-memory-options-item[data-highlighted] {
  @apply bg-accent text-accent-foreground;
}
.md-memory-options-separator {
  @apply bg-border;
  height: 1px;
  margin: 4px;
}
.md-memory-options-details {
  @apply text-muted-foreground;
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 8px;
  padding: 8px;
  font-size: 10px;
}
.md-memory-options-details strong {
  @apply text-foreground;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
}
</style>
