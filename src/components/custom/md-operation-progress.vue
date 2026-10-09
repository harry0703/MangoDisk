<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';

import MdIcon from '@/components/icons/md-icon.vue';
import MdMiddleEllipsis from '@/components/custom/md-middle-ellipsis.vue';
import { Button } from '@/components/ui/button';
import { Card } from '@/components/ui/card';
import { ICON_NAMES, OPERATION_PROGRESS_CLOCK_INTERVAL_MS } from '@/lib/models/ui';
import type { TraversalProgress } from '@/lib/models/progress';
import { ByteSizeService } from '@/lib/services/byte-size-service';
import * as FormatUtils from '@/lib/utils/format';
import * as PathUtils from '@/lib/utils/path';

const { t } = useI18n({ useScope: 'global' });

type FeatureIconName = (typeof ICON_NAMES)[keyof typeof ICON_NAMES];

const props = withDefaults(
  defineProps<{
    title: string;
    progress: TraversalProgress | null;
    pathLabel: string;
    preparingText: string;
    hint: string;
    cancelable: boolean;
    cancelDisabled: boolean;
    iconName: FeatureIconName;
    showTraversalDetails?: boolean;
    showStepProgress?: boolean;
    itemsLabel?: string;
    bytesLabel?: string;
  }>(),
  {
    bytesLabel: undefined,
    itemsLabel: undefined,
    showTraversalDetails: true,
    showStepProgress: true,
  }
);

const hasDeterminateProgress = computed(() => props.showStepProgress !== false && Boolean(props.progress?.totalSteps));
const checkPercent = computed(() => {
  if (!hasDeterminateProgress.value || !props.progress?.totalSteps) return 0;
  return Math.min(100, (props.progress.completedSteps / props.progress.totalSteps) * 100);
});
// Wall-clock corrections, including VM time synchronization, must not count
// as operation time. Backend elapsed values use the same monotonic contract.
const clockMs = ref(performance.now());
const elapsedAnchorMs = ref(0);
const elapsedAnchorAtMs = ref(clockMs.value);
let clockTimer: ReturnType<typeof setInterval> | undefined;
const elapsedMs = computed(() => elapsedAnchorMs.value + Math.max(0, clockMs.value - elapsedAnchorAtMs.value));
const elapsedSeconds = computed(() => Math.floor(elapsedMs.value / 1000));
const elapsedText = computed(() =>
  t('loading.elapsedSeconds', { count: FormatUtils.integer(elapsedSeconds.value) }, elapsedSeconds.value)
);
const currentPath = computed(() =>
  props.progress?.currentPath ? PathUtils.display(props.progress.currentPath) : props.preparingText
);
const accessibleProgressSummary = computed(() => {
  const progress = props.progress;
  if (!hasDeterminateProgress.value || !progress?.totalSteps) return props.title;
  return t('loading.accessibleRuleProgress', {
    completed: FormatUtils.integer(progress.completedSteps),
    total: FormatUtils.integer(progress.totalSteps),
    checked: FormatUtils.integer(progress.itemsScanned),
    bytes: ByteSizeService.bytes(progress.bytesScanned),
  });
});

watch(
  [() => props.progress?.operationId, () => props.progress?.elapsedMs],
  ([operationId, backendElapsedMs], [previousOperationId]) => {
    const elapsed = backendElapsedMs ?? 0;
    const now = performance.now();
    // Transport can deliver a progress sample after the local clock has passed
    // it. Preserve the displayed duration until a different operation starts.
    elapsedAnchorMs.value =
      operationId !== previousOperationId
        ? elapsed
        : Math.max(elapsed, elapsedAnchorMs.value + Math.max(0, now - elapsedAnchorAtMs.value));
    elapsedAnchorAtMs.value = now;
    clockMs.value = now;
  },
  { immediate: true }
);

onMounted(() => {
  clockTimer = setInterval(() => {
    clockMs.value = performance.now();
  }, OPERATION_PROGRESS_CLOCK_INTERVAL_MS);
});

onUnmounted(() => {
  if (clockTimer) clearInterval(clockTimer);
});

defineEmits<{ cancel: [] }>();
</script>

<template>
  <Card class="operation-progress-card" :aria-label="title" aria-busy="true">
    <div class="progress-heading">
      <span class="progress-icon"
        ><MdIcon :name="iconName" :size="30" /><i class="spinner md-operational-motion"
      /></span>
      <div>
        <span class="current-stage">{{ t('loading.currentStage') }}</span>
        <h2>{{ title }}</h2>
      </div>
    </div>
    <p class="sr-only" role="status" aria-live="polite">{{ accessibleProgressSummary }}</p>

    <div v-if="showTraversalDetails !== false" class="path-card">
      <span class="folder-icon" aria-hidden="true"><MdIcon :name="ICON_NAMES.folder" :size="18" /></span>
      <span class="path-content">
        <small class="path-meta">
          <span>{{ pathLabel }}</span>
          <span v-if="progress">{{ elapsedText }}</span>
        </small>
        <strong><MdMiddleEllipsis :text="currentPath" :tail-length="24" /></strong>
      </span>
    </div>
    <div class="activity-track" :class="{ determinate: hasDeterminateProgress }">
      <span class="md-operational-motion" :style="hasDeterminateProgress ? { width: `${checkPercent}%` } : undefined" />
    </div>
    <div
      v-if="showTraversalDetails !== false && hasDeterminateProgress && progress"
      class="progress-stats rule-progress-stats"
    >
      <span>
        <small>{{ t('loading.checkedChecks') }}</small>
        <strong>{{ progress.completedSteps }} / {{ progress.totalSteps }}</strong>
      </span>
      <span>
        <small>{{ itemsLabel || t('loading.checkedItems') }}</small>
        <strong>{{ FormatUtils.integer(progress.itemsScanned) }}</strong>
      </span>
      <span>
        <small>{{ bytesLabel || t('loading.traversedData') }}</small>
        <strong>{{ ByteSizeService.bytes(progress.bytesScanned) }}</strong>
      </span>
      <span>
        <small>{{ t('loading.elapsed') }}</small>
        <strong>{{ elapsedText }}</strong>
      </span>
    </div>
    <div v-else-if="showTraversalDetails !== false" class="progress-stats">
      <span>
        <small>{{ itemsLabel || t('loading.checkedItems') }}</small>
        <strong>{{ FormatUtils.integer(progress?.itemsScanned ?? 0) }}</strong>
      </span>
      <span>
        <small>{{ bytesLabel || t('loading.traversedData') }}</small>
        <strong>{{ ByteSizeService.bytes(progress?.bytesScanned ?? 0) }}</strong>
      </span>
      <span>
        <small>{{ t('loading.elapsed') }}</small>
        <strong>
          {{ t('loading.elapsedSeconds', { count: FormatUtils.integer(elapsedSeconds) }, elapsedSeconds) }}
        </strong>
      </span>
    </div>
    <p v-else class="activity-summary">{{ elapsedText }}</p>
    <p class="progress-hint">{{ hint }}</p>
    <Button
      v-if="cancelable"
      class="cancel-button"
      variant="outline"
      type="button"
      :disabled="cancelDisabled"
      @click="$emit('cancel')"
      >{{ t('common.cancel') }}</Button
    >
  </Card>
</template>

<style scoped>
@reference "@assets/main.css";
.operation-progress-card {
  width: 100%;
  max-width: 610px;
  @apply gap-0 border-border bg-card p-7 text-card-foreground;
  box-shadow: 0 25px 50px -12px var(--shadow-dialog);
}
.progress-heading {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 18px;
}
.progress-heading > div {
  min-width: 0;
}
.progress-heading h2 {
  overflow: hidden;
  min-height: 26px;
  margin: 3px 0 0;
  @apply text-card-foreground;
  font-size: 20px;
  line-height: 1.3;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.current-stage {
  display: block;
  overflow: hidden;
  @apply text-primary-text;
  font-size: 12px;
  font-weight: 650;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.progress-icon {
  position: relative;
  display: grid;
  width: 66px;
  height: 66px;
  flex: none;
  place-items: center;
  border-radius: 50%;
  @apply text-primary-text;
  background: var(--surface-primary-subtle);
}
.spinner {
  position: absolute;
  inset: 0;
  box-sizing: border-box;
  @apply border-primary/20 border-t-primary;
  border-width: 3px;
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}
.path-card {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 13px;
  margin-top: 24px;
  @apply border bg-muted/55;
  border-color: var(--border-subtle);
  border-radius: 11px;
  padding: 13px 15px;
}
.folder-icon {
  display: grid;
  width: 24px;
  height: 34px;
  flex: none;
  place-items: center;
  @apply text-muted-foreground;
}
.path-content {
  display: flex;
  min-width: 0;
  flex: 1;
  flex-direction: column;
  gap: 3px;
}
.path-content small {
  @apply text-muted-foreground;
  font-size: 11px;
}
.path-meta {
  display: flex;
  min-height: 14px;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  line-height: 14px;
  font-variant-numeric: tabular-nums;
}
.path-content strong {
  display: block;
  height: 16px;
  min-width: 0;
  @apply text-card-foreground;
  font-family: 'Cascadia Mono', 'Segoe UI Mono', monospace;
  font-size: 12px;
  font-weight: 550;
  line-height: 16px;
  direction: ltr;
}
.activity-track {
  height: 4px;
  margin-top: 16px;
  overflow: hidden;
  border-radius: 999px;
  background: var(--surface-primary-subtle);
}
.activity-track span {
  display: block;
  width: 34%;
  height: 100%;
  border-radius: inherit;
  @apply bg-primary;
  animation: scan-activity 1.35s linear infinite;
}
.activity-track.determinate span {
  position: relative;
  overflow: hidden;
  transition: width 0.2s ease;
  animation: none;
}
.activity-track.determinate span::after {
  position: absolute;
  inset: 0;
  background: linear-gradient(90deg, transparent, var(--progress-shimmer), transparent);
  content: '';
  transform: translateX(-100%);
  animation: progress-activity 1.4s ease-in-out infinite;
}
.activity-summary {
  min-height: 18px;
  margin: 13px 0 0;
  @apply text-center text-muted-foreground;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}
.progress-stats {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 10px;
  margin-top: 15px;
}
.rule-progress-stats {
  grid-template-columns: repeat(4, minmax(0, 1fr));
}
.progress-stats > span {
  display: flex;
  min-height: 55px;
  min-width: 0;
  flex-direction: column;
  gap: 3px;
  border-radius: 9px;
  padding: 10px 12px;
  @apply bg-muted/65 transition-colors duration-200 hover:bg-muted;
}
.progress-stats small {
  display: block;
  height: 14px;
  min-width: 0;
  overflow: hidden;
  @apply text-muted-foreground;
  font-size: 10px;
  line-height: 14px;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.progress-stats strong {
  overflow: hidden;
  @apply text-card-foreground;
  font-size: 13px;
  font-variant-numeric: tabular-nums;
  line-height: 17px;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.progress-hint {
  margin: 13px 0 0;
  @apply text-muted-foreground;
  font-size: 11px;
  text-align: center;
}
.cancel-button {
  min-width: 100px;
  margin: 16px auto 0;
}
.cancel-button:disabled {
  cursor: wait;
  opacity: 0.55;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
@keyframes scan-activity {
  0% {
    transform: translateX(-110%);
  }
  100% {
    transform: translateX(310%);
  }
}
@keyframes progress-activity {
  to {
    transform: translateX(100%);
  }
}
</style>
