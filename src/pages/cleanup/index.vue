<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { computed, defineAsyncComponent, ref, watch } from 'vue';

import MdEmptyState from '@/components/custom/md-empty-state.vue';
import MdOperationWorkspace from '@/components/custom/md-operation-workspace.vue';
import MdPageShell from '@/components/custom/md-page-shell.vue';
import MdResultSummary from '@/components/custom/md-result-summary.vue';
import MdResultWorkspace from '@/components/custom/md-result-workspace.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import type {
  ApplicationLeftoverCandidate,
  ApplicationLeftoverResult,
  ApplicationLeftoverScanResult,
} from '@/lib/models/application';
import type { ApplicationCloseBatchResult, ApplicationCloseMode } from '@/lib/models/application-close';
import { CLEANUP_OPERATION_IDS, type CleanupSourceSelection } from '@/lib/models/cleanup';
import type { DiskInfo } from '@/lib/models/disk';
import { ICON_NAMES } from '@/lib/models/ui';
import type { CleanupOperationId, PresentedCleanupResult, PresentedCleanupScanResult } from '@/lib/models/cleanup';
import type { TraversalProgress } from '@/lib/models/progress';
import { CleanupRuleSelectionUtils } from '@/lib/utils/cleanup-rule-selection';
import type { CleanupSelectionMode } from '@/lib/utils/cleanup-rule-selection';
import { ByteSizeService } from '@/lib/services/byte-size-service';
import { AiAdvisorService } from '@/lib/services/ai-advisor-service';
import { FormatUtils } from '@/lib/utils/format';
import { useAppStore } from '@/stores/app-store';

import { groupApplicationLeftovers, recommendedApplicationLeftoverIds } from './application-leftover-groups';
import { selectedCleanupCloseRequirement } from './cleanup-close-requirement';
import { countSelectedCleanupGroups } from './cleanup-result-categories';
import MdCleanupPlanDialog from './components/md-cleanup-plan-dialog.vue';
import MdCleanupScanButton from './components/md-cleanup-scan-button.vue';

// Result browsing is not needed on the startup empty state. The confirmation
// dialog remains in the main chunk because an async placeholder can expose the
// modal overlay before its content is ready, leaving an apparently frozen UI.
const loadCleanupResultDialog = () => import('./components/md-cleanup-result-dialog.vue');
const loadCleanupRuleGroups = () => import('./components/md-cleanup-rule-groups.vue');
const loadCleanupSelectionMode = () => import('./components/md-cleanup-selection-mode.vue');
const loadOperationProgress = () => import('@/components/custom/md-operation-progress.vue');
const loadSelectionActionBar = () => import('@/components/custom/md-selection-action-bar.vue');
const MdCleanupResultDialog = defineAsyncComponent(loadCleanupResultDialog);
const MdCleanupRuleGroups = defineAsyncComponent(loadCleanupRuleGroups);
const MdCleanupSelectionMode = defineAsyncComponent(loadCleanupSelectionMode);
const MdOperationProgress = defineAsyncComponent(loadOperationProgress);
const MdSelectionActionBar = defineAsyncComponent(loadSelectionActionBar);

const { t } = useI18n({ useScope: 'global' });
const appStore = useAppStore();

const props = defineProps<{
  busy: boolean;
  disk: DiskInfo | null;
  leftovers: ApplicationLeftoverScanResult | null;
  leftoverResult: ApplicationLeftoverResult | null;
  scanningLeftovers: boolean;
  deletingLeftovers: boolean;
  loadingMessage: string;
  operation: CleanupOperationId;
  progress: TraversalProgress | null;
  result: PresentedCleanupResult | null;
  scan: PresentedCleanupScanResult | null;
  selectedBytes: number;
  selectedRuleIds: string[];
  sourceSelections: CleanupSourceSelection[];
  closingApplications: boolean;
  closeResult: ApplicationCloseBatchResult | null;
}>();
const emit = defineEmits<{
  cancel: [];
  closeApplications: [ruleIds: string[], mode: ApplicationCloseMode];
  execute: [leftovers: ApplicationLeftoverCandidate[]];
  open: [path: string];
  scan: [deepProjectDiscovery: boolean];
  selectAll: [ruleIds: string[], selected: boolean];
  toggleSource: [ruleId: string, path: string];
  error: [error: unknown];
}>();

const confirmOpen = ref(false);
const resultOpen = ref(false);
const awaitingResult = ref(false);
const awaitingCleanupResult = ref(false);
const awaitingLeftoverResult = ref(false);
const resultBeforeExecution = ref<PresentedCleanupResult | null>(null);
const leftoverResultBeforeExecution = ref<ApplicationLeftoverResult | null>(null);
const dialogCleanupResult = ref<PresentedCleanupResult | null>(null);
const dialogLeftoverResult = ref<ApplicationLeftoverResult | null>(null);
const currentScanIsDeep = ref(false);
const selectedLeftoverIds = ref<string[]>([]);
const aiAnalyzing = ref(false);
const aiRecommendedRuleIds = ref<string[]>([]);
let aiAbortController: AbortController | null = null;
let aiStaggerTimer: ReturnType<typeof setTimeout> | null = null;

function abortAiAnalysis() {
  if (aiStaggerTimer !== null) {
    clearTimeout(aiStaggerTimer);
    aiStaggerTimer = null;
  }
  if (aiAbortController) {
    aiAbortController.abort();
    aiAbortController = null;
  }
  aiAnalyzing.value = false;
}

const scanRules = computed(() => props.scan?.rules ?? []);
const selectableRuleIds = computed(() => CleanupRuleSelectionUtils.selectableRuleIds(scanRules.value));
const recommendedRuleIds = computed(() => CleanupRuleSelectionUtils.recommendedRuleIds(scanRules.value));
const foundCleanupBytes = computed(() => CleanupRuleSelectionUtils.foundBytes(scanRules.value));
const recommendedCleanupBytes = computed(() => CleanupRuleSelectionUtils.recommendedBytes(scanRules.value));
const selectedRules = computed(() =>
  CleanupRuleSelectionUtils.selectedRules(scanRules.value, props.selectedRuleIds).map(rule => {
    const closeRequirement = selectedCleanupCloseRequirement(rule, props.selectedRuleIds, props.sourceSelections);
    return {
      ...rule,
      ...closeRequirement,
      bytes: CleanupRuleSelectionUtils.selectedBytesForRule(rule, props.selectedRuleIds, props.sourceSelections),
    };
  })
);
const leftoverCandidates = computed(() => props.leftovers?.candidates ?? []);
const recommendedLeftoverIds = computed(() => recommendedApplicationLeftoverIds(leftoverCandidates.value));
const selectedLeftoverSet = computed(() => new Set(selectedLeftoverIds.value));
const selectedLeftovers = computed(() =>
  leftoverCandidates.value.filter(candidate => selectedLeftoverSet.value.has(candidate.candidateId))
);
const selectedLeftoverBytes = computed(() =>
  selectedLeftovers.value.reduce((total, candidate) => total + candidate.bytes, 0)
);
const selectedLeftoverApplicationCount = computed(() => groupApplicationLeftovers(selectedLeftovers.value).length);
const selectedCleanupItemCount = computed(() =>
  countSelectedCleanupGroups(scanRules.value, props.selectedRuleIds, props.sourceSelections)
);
const foundCleanupItemCount = computed(() => countSelectedCleanupGroups(scanRules.value, selectableRuleIds.value, []));
const foundLeftoverApplicationCount = computed(() => groupApplicationLeftovers(leftoverCandidates.value).length);
const totalFoundItemCount = computed(() => foundCleanupItemCount.value + foundLeftoverApplicationCount.value);
const selectedItemCount = computed(() => selectedCleanupItemCount.value + selectedLeftoverApplicationCount.value);
const totalSelectedBytes = computed(() => props.selectedBytes + selectedLeftoverBytes.value);
const totalFoundBytes = computed(() => foundCleanupBytes.value + (props.leftovers?.totalBytes ?? 0));
const selectionMode = computed<CleanupSelectionMode>(() => {
  if (aiAnalyzing.value) return 'smart';
  const effectiveRecommended =
    aiRecommendedRuleIds.value.length > 0 ? aiRecommendedRuleIds.value : recommendedRuleIds.value;
  const selected = new Set(props.selectedRuleIds);
  const selectable = selectableRuleIds.value;
  const leftoverCount = leftoverCandidates.value.length;
  const selectedLeftoverCount = selectedLeftoverIds.value.length;
  const selectedRecommendedLeftovers =
    selectedLeftoverCount === recommendedLeftoverIds.value.length &&
    recommendedLeftoverIds.value.every(candidateId => selectedLeftoverSet.value.has(candidateId));

  if (!selected.size && !selectedLeftoverCount) return 'none';
  if (props.sourceSelections.length) return 'manual';

  const matchesRecommended =
    selected.size === effectiveRecommended.length && effectiveRecommended.every(id => selected.has(id));
  const matchesAll = selected.size === selectable.length && selectable.every(id => selected.has(id));

  if (matchesRecommended && (!leftoverCount || selectedRecommendedLeftovers)) return 'smart';
  if (matchesAll && (!leftoverCount || selectedLeftoverCount === leftoverCount)) return 'all';
  return 'manual';
});
const selectedRunningProcesses = computed(() => [
  ...new Set(selectedRules.value.flatMap(rule => rule.runningProcesses)),
]);
const selectedRequiresAppClose = computed(() => selectedRules.value.some(rule => rule.requiresAppClose));
const selectionHint = computed(() => {
  if (selectedRunningProcesses.value.length) {
    return t(
      'cleanup.appsToCloseCount',
      { count: FormatUtils.integer(selectedRunningProcesses.value.length) },
      selectedRunningProcesses.value.length
    );
  }
  return selectedRequiresAppClose.value ? t('cleanup.requiresClose') : undefined;
});
const scanning = computed(
  () =>
    props.scanningLeftovers ||
    (props.busy && [CLEANUP_OPERATION_IDS.scanning, CLEANUP_OPERATION_IDS.cancelling].includes(props.operation))
);
const diskUsagePercent = computed(() =>
  props.disk ? FormatUtils.percent(props.disk.usedBytes, props.disk.totalBytes) : 0
);
const diskUsageSummary = computed(() =>
  props.disk
    ? t('cleanup.diskUsageSummary', {
        available: ByteSizeService.bytes(props.disk.availableBytes),
      })
    : ''
);
const diskUsageDetails = computed(() =>
  props.disk
    ? t('cleanup.diskUsageDetails', {
        name: props.disk.name || props.disk.mountPoint,
        used: ByteSizeService.bytes(props.disk.usedBytes),
        available: ByteSizeService.bytes(props.disk.availableBytes),
        total: ByteSizeService.bytes(props.disk.totalBytes),
      })
    : ''
);

function openConfirm() {
  if (selectedItemCount.value) confirmOpen.value = true;
}

function execute() {
  abortAiAnalysis();
  confirmOpen.value = false;
  resultOpen.value = false;
  awaitingCleanupResult.value = Boolean(selectedRules.value.length);
  awaitingLeftoverResult.value = Boolean(selectedLeftovers.value.length);
  awaitingResult.value = awaitingCleanupResult.value || awaitingLeftoverResult.value;
  resultBeforeExecution.value = props.result;
  leftoverResultBeforeExecution.value = props.leftoverResult;
  dialogCleanupResult.value = null;
  dialogLeftoverResult.value = null;
  emit('execute', selectedLeftovers.value);
}

function closeApplications(ruleIds: string[], mode: ApplicationCloseMode) {
  emit('closeApplications', ruleIds, mode);
}

function selectAll(ruleIds: string[], selected: boolean) {
  if (aiAnalyzing.value) abortAiAnalysis();
  emit('selectAll', ruleIds, selected);
}

function toggleSource(ruleId: string, path: string) {
  if (aiAnalyzing.value) abortAiAnalysis();
  emit('toggleSource', ruleId, path);
}

async function startScan(deep: boolean) {
  abortAiAnalysis();
  aiRecommendedRuleIds.value = [];
  // These components are not needed by the startup empty state. Load them
  // immediately before a scan so the initial bundle stays small without
  // allowing a blank async placeholder when progress or results first appear.
  await Promise.allSettled([
    loadCleanupResultDialog(),
    loadCleanupRuleGroups(),
    loadCleanupSelectionMode(),
    loadOperationProgress(),
    loadSelectionActionBar(),
  ]);
  currentScanIsDeep.value = deep;
  emit('scan', deep);
}

function toggleLeftover(candidate: ApplicationLeftoverCandidate) {
  if (props.busy) return;
  if (aiAnalyzing.value) abortAiAnalysis();
  selectedLeftoverIds.value = selectedLeftoverSet.value.has(candidate.candidateId)
    ? selectedLeftoverIds.value.filter(candidateId => candidateId !== candidate.candidateId)
    : [...selectedLeftoverIds.value, candidate.candidateId];
}

function setLeftoverGroupSelected(candidateIds: string[], selected: boolean) {
  if (props.busy) return;
  if (aiAnalyzing.value) abortAiAnalysis();
  const targetIds = new Set(candidateIds);
  selectedLeftoverIds.value = selected
    ? [...new Set([...selectedLeftoverIds.value, ...candidateIds])]
    : selectedLeftoverIds.value.filter(candidateId => !targetIds.has(candidateId));
}

async function setSelectionMode(value: unknown) {
  if (!['smart', 'all', 'none'].includes(String(value))) return;
  const mode = String(value) as Exclude<CleanupSelectionMode, 'manual'>;

  abortAiAnalysis();

  if (mode === 'none') {
    aiRecommendedRuleIds.value = [];
    emit('selectAll', selectableRuleIds.value, false);
    selectedLeftoverIds.value = [];
    return;
  }

  if (mode === 'all') {
    aiRecommendedRuleIds.value = [];
    emit('selectAll', selectableRuleIds.value, true);
    selectedLeftoverIds.value = leftoverCandidates.value.map(candidate => candidate.candidateId);
    return;
  }

  if (mode === 'smart') {
    aiAnalyzing.value = true;
    aiRecommendedRuleIds.value = [];
    emit('selectAll', selectableRuleIds.value, false);
    selectedLeftoverIds.value = recommendedLeftoverIds.value;

    const controller = new AbortController();
    aiAbortController = controller;

    try {
      const config = {
        apiKey: appStore.settings.aiApiKey,
        baseUrl: appStore.settings.aiApiBaseUrl,
        model: appStore.settings.aiModel,
      };
      const recommended = await AiAdvisorService.analyzeCleanupRules(scanRules.value, config, controller.signal);
      if (controller.signal.aborted) return;

      aiRecommendedRuleIds.value = recommended;
      if (!recommended.length) {
        aiAnalyzing.value = false;
        aiAbortController = null;
        return;
      }

      // Smooth staggered selection animation (50ms interval)
      const rulesToSelect = [...recommended];
      let index = 0;
      const batchSize = Math.max(1, Math.floor(rulesToSelect.length / 10));

      const processBatch = () => {
        if (controller.signal.aborted) return;
        const nextBatch = rulesToSelect.slice(index, index + batchSize);
        index += batchSize;
        emit('selectAll', nextBatch, true);

        if (index < rulesToSelect.length) {
          aiStaggerTimer = setTimeout(processBatch, 50);
        } else {
          aiAnalyzing.value = false;
          aiAbortController = null;
          aiStaggerTimer = null;
        }
      };

      processBatch();
    } catch (error: unknown) {
      if (controller.signal.aborted) return;
      aiAnalyzing.value = false;
      aiAbortController = null;
      emit('error', error);
    }
  }
}

watch(scanRules, rules => {
  const existingIds = new Set(rules.map(r => r.ruleId));
  aiRecommendedRuleIds.value = aiRecommendedRuleIds.value.filter(id => existingIds.has(id));
});

watch(
  () => [props.result, props.leftoverResult, props.busy] as const,
  ([result, leftoverResult, busy]) => {
    /*
     * Both cleanup domains can retain their previous result while execution is
     * pending. Build the dialog from only the new results produced by this
     * confirmation so a leftovers-only run never displays an older cache result.
     */
    if (!awaitingResult.value || busy) return;
    awaitingResult.value = false;
    dialogCleanupResult.value = awaitingCleanupResult.value && result !== resultBeforeExecution.value ? result : null;
    dialogLeftoverResult.value =
      awaitingLeftoverResult.value && leftoverResult !== leftoverResultBeforeExecution.value ? leftoverResult : null;
    if (dialogCleanupResult.value || dialogLeftoverResult.value) resultOpen.value = true;
  }
);

watch(
  () => props.leftovers?.candidates,
  candidates => {
    /*
     * Every scan result is a new review snapshot. Core marks leftovers as
     * recommended only when application identity and filesystem evidence are
     * complete, so the UI never infers safety from names or paths.
     */
    selectedLeftoverIds.value = recommendedApplicationLeftoverIds(candidates ?? []);
  }
);
</script>

<template>
  <MdPageShell class="@container/cleanup" content-mode="workspace" :title="t('cleanup.title')">
    <template #actions>
      <div class="scan-action">
        <div
          v-if="disk"
          class="system-disk-usage"
          :class="{ 'system-disk-usage--tight': diskUsagePercent >= 90 }"
          :aria-label="diskUsageDetails"
          :title="diskUsageDetails"
        >
          <MdIcon class="system-disk-icon" :name="ICON_NAMES.hardDrive" :size="16" />
          <span class="system-disk-content">
            <span class="system-disk-copy">
              <span class="system-disk-name">{{ disk.name || disk.mountPoint }}</span>
              <span class="system-disk-value">{{ diskUsageSummary }}</span>
            </span>
            <span class="system-disk-track" aria-hidden="true">
              <span class="system-disk-progress" :style="{ width: `${diskUsagePercent}%` }" />
            </span>
          </span>
        </div>
        <MdCleanupScanButton v-if="scan && !scanning" :busy="busy" action="rescan" @scan="startScan" />
      </div>
    </template>

    <template v-if="!scanning && scan" #footer>
      <MdSelectionActionBar
        class="cleanup-action-bar"
        :selected-label="t('cleanup.selectedSummary')"
        :selected-value="t('common.itemCount', { count: FormatUtils.integer(selectedItemCount) }, selectedItemCount)"
        :space-label="t('common.estimatedRelease')"
        :space-value="ByteSizeService.bytes(totalSelectedBytes)"
        :hint="selectionHint"
        :action-label="t('cleanup.clean')"
        :disabled="!selectedItemCount"
        :busy="busy || aiAnalyzing"
        @action="openConfirm"
      >
        <template #options>
          <MdCleanupSelectionMode
            :busy="busy || aiAnalyzing"
            :mode="selectionMode"
            :analyzing="aiAnalyzing"
            :recommended-bytes="recommendedCleanupBytes"
            :total-bytes="totalFoundBytes"
            @change="setSelectionMode"
          />
        </template>
      </MdSelectionActionBar>
    </template>

    <MdOperationWorkspace v-if="scanning">
      <MdOperationProgress
        :icon-name="scanningLeftovers ? ICON_NAMES.application : ICON_NAMES.deepCleanup"
        :title="scanningLeftovers ? t('applicationLeftovers.scanning') : loadingMessage"
        :progress="progress"
        :path-label="t('loading.currentDirectory')"
        :preparing-text="t('loading.preparingDirectory')"
        :show-step-progress="false"
        :hint="
          scanningLeftovers
            ? t('applicationLeftovers.scanHint')
            : currentScanIsDeep
              ? t('cleanup.deepDiscoveryProgressHint')
              : t('loading.cancelHint')
        "
        :cancelable="true"
        :cancel-disabled="scanningLeftovers || operation === CLEANUP_OPERATION_IDS.cancelling"
        @cancel="emit('cancel')"
      />
    </MdOperationWorkspace>

    <MdResultWorkspace v-else-if="scan">
      <template #summary>
        <MdResultSummary
          :title="t('cleanup.summaryCount', { count: FormatUtils.integer(totalFoundItemCount) }, totalFoundItemCount)"
          :metric-label="t('cleanup.summarySpace')"
          :metric-value="ByteSizeService.bytes(totalFoundBytes)"
        />
      </template>

      <MdCleanupRuleGroups
        class="embedded"
        :busy="busy"
        :leftovers="leftovers"
        :rules="scan.rules"
        :ai-recommended-rule-ids="aiRecommendedRuleIds"
        :selected-leftover-ids="selectedLeftoverIds"
        :selected-rule-ids="selectedRuleIds"
        :source-selections="sourceSelections"
        @toggle-source="toggleSource"
        @toggle-leftover="toggleLeftover"
        @select-leftover-group="setLeftoverGroupSelected"
        @select-all="selectAll"
        @open="emit('open', $event)"
      />
    </MdResultWorkspace>

    <MdResultWorkspace v-else>
      <MdEmptyState
        :icon-name="ICON_NAMES.deepCleanup"
        :title="t('cleanup.scanFirst')"
        :description="t('cleanup.emptyDescription')"
      >
        <MdCleanupScanButton :busy="busy" @scan="startScan" />
      </MdEmptyState>
    </MdResultWorkspace>

    <MdCleanupPlanDialog
      v-if="scan"
      v-model="confirmOpen"
      :busy="busy"
      :rules="selectedRules"
      :selected-bytes="totalSelectedBytes"
      :leftover-application-count="selectedLeftoverApplicationCount"
      :leftover-item-count="selectedLeftovers.length"
      :leftover-bytes="selectedLeftoverBytes"
      :selected-item-count="selectedItemCount"
      :closing-applications="closingApplications"
      :close-result="closeResult"
      :application-icons="scan.applicationIcons"
      @execute="execute"
      @close-applications="closeApplications"
    />
    <MdCleanupResultDialog
      v-if="scan"
      v-model="resultOpen"
      :result="dialogCleanupResult"
      :leftover-result="dialogLeftoverResult"
    />
  </MdPageShell>
</template>

<style scoped>
@reference "@assets/main.css";

.scan-action {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 10px;
}

.system-disk-usage {
  @apply border-border/70 bg-card/35 text-muted-foreground;
  display: grid;
  width: 270px;
  height: 40px;
  min-width: 0;
  grid-template-columns: auto minmax(0, 1fr);
  align-items: center;
  gap: 9px;
  border-width: 1px;
  border-radius: 8px;
  padding: 5px 10px;
  font-size: 13px;
}

.system-disk-icon {
  color: var(--muted-foreground);
}

.system-disk-content {
  display: flex;
  min-width: 0;
  flex-direction: column;
  justify-content: center;
  gap: 4px;
}

.system-disk-copy {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 7px;
  line-height: 1;
}

.system-disk-track {
  @apply bg-border/45;
  position: relative;
  display: block;
  height: 3px;
  overflow: hidden;
  border-radius: 999px;
}

.system-disk-progress {
  @apply bg-muted-foreground/45;
  position: absolute;
  top: 0;
  bottom: 0;
  left: 0;
  border-radius: inherit;
  pointer-events: none;
  transition:
    width 180ms ease,
    background-color 180ms ease;
}

.system-disk-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  color: var(--foreground);
  font-weight: 500;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.system-disk-value {
  white-space: nowrap;
}

.system-disk-usage--tight .system-disk-progress {
  @apply bg-warning/75;
}

.system-disk-usage--tight {
  @apply border-warning/30;
}

.cleanup-action-bar {
  border-color: var(--border);
  box-shadow: 0 5px 18px color-mix(in srgb, var(--foreground) 7%, transparent);
}

@container (max-width: 800px) {
  .scan-action {
    width: 100%;
    justify-content: space-between;
  }
}

@container (max-width: 620px) {
  .system-disk-usage {
    min-width: 0;
    max-width: 100%;
  }

  .system-disk-value {
    overflow: hidden;
    text-overflow: ellipsis;
  }
}
</style>
