<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { toast } from 'vue-sonner';

import MdCategoryFilter from '@/components/custom/md-category-filter.vue';
import MdEmptyState from '@/components/custom/md-empty-state.vue';
import MdOperationProgress from '@/components/custom/md-operation-progress.vue';
import MdOperationWorkspace from '@/components/custom/md-operation-workspace.vue';
import MdPageShell from '@/components/custom/md-page-shell.vue';
import MdResultCheckbox from '@/components/custom/md-result-checkbox.vue';
import MdResultFilterToolbar from '@/components/custom/md-result-filter-toolbar.vue';
import MdResultSearch from '@/components/custom/md-result-search.vue';
import MdResultSummary from '@/components/custom/md-result-summary.vue';
import MdResultTable from '@/components/custom/md-result-table.vue';
import MdResultWorkspace from '@/components/custom/md-result-workspace.vue';
import MdSelectionActionBar from '@/components/custom/md-selection-action-bar.vue';
import MdDestructiveActionDialog from '@/components/custom/md-destructive-action-dialog.vue';
import MdApplicationClosePanel from '@/components/custom/md-application-close-panel.vue';
import MdConfirmDialog from '@/components/custom/md-confirm-dialog.vue';
import MdDialogContent from '@/components/custom/md-dialog-content.vue';
import MdDialogFooter from '@/components/custom/md-dialog-footer.vue';
import MdDialogHeader from '@/components/custom/md-dialog-header.vue';
import MdInlineNotice from '@/components/custom/md-inline-notice.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { Button } from '@/components/ui/button';
import { Dialog, DialogDescription, DialogTitle } from '@/components/ui/dialog';
import type {
  ApplicationUninstallBatchPlan,
  ApplicationUninstallBatchResult,
  ApplicationUninstallBatchSelection,
  ApplicationUninstallCandidate,
  ApplicationUninstallComponentSummary,
  ApplicationUninstallExecutionProgress,
  ApplicationUninstallScanResult,
} from '@/lib/models/application';
import type {
  ApplicationCloseBatchResult,
  ApplicationCloseItem,
  ApplicationCloseMode,
} from '@/lib/models/application-close';
import type { TraversalProgress } from '@/lib/models/progress';
import { ICON_NAMES } from '@/lib/models/ui';
import { ApplicationService } from '@/lib/services/application-service';
import { ApplicationIconService } from '@/lib/services/application-icon-service';
import { LoggerService } from '@/lib/services/logger-service';
import { ByteSizeService } from '@/lib/services/byte-size-service';
import { OperatingSystemService } from '@/lib/services/operating-system-service';
import { parseCommandError } from '@/lib/utils/error';
import * as FormatUtils from '@/lib/utils/format';

import {
  applicationCatalogFilters,
  applicationCatalogSortAscending,
  applicationCatalogSortKey,
  applicationCanStartUninstall,
  applicationMatchesCatalogFilter,
  filterAndSortApplications,
  nextApplicationCatalogSort,
  type ApplicationCatalogFilter,
  type ApplicationCatalogSort,
  type ApplicationCatalogSortKey,
} from './application-uninstall-catalog';
import {
  shouldNotifyUninstallCancellation,
  UNINSTALL_CANCELLATION_TOAST_ID,
} from './application-uninstall-confirmation';
import {
  applicationCloseRequestIds,
  applyApplicationCloseResult,
  beginApplicationCloseWorkflow,
  createApplicationUninstallCloseWorkflow,
  finishApplicationCloseWorkflow,
} from './application-uninstall-close-workflow';
import {
  selectedApplicationBytes,
  selectionIncludesUserData,
  setVisibleApplicationSelection,
  toggleApplicationComponent,
  toggleApplicationSelection,
} from './application-uninstall-selection';
import MdApplicationUninstallRow from './components/md-application-uninstall-row.vue';

const { t } = useI18n({ useScope: 'global' });
const props = defineProps<{
  catalog: ApplicationUninstallScanResult | null;
  scanning: boolean;
  cancelling: boolean;
  progress: TraversalProgress | null;
  executionProgress: ApplicationUninstallExecutionProgress | null;
  plan: ApplicationUninstallBatchPlan | null;
  preview: ApplicationUninstallBatchResult | null;
  lastResult: ApplicationUninstallBatchResult | null;
  preparing: boolean;
  executing: boolean;
  cancellingExecution: boolean;
  cancellationRevision: number;
  closingApplications: boolean;
  closeResult: ApplicationCloseBatchResult | null;
}>();
const emit = defineEmits<{
  scan: [];
  cancelScan: [];
  prepare: [selections: ApplicationUninstallBatchSelection[]];
  cancelPlan: [];
  execute: [];
  cancelExecution: [];
  closeApplications: [applicationIds: string[], mode: ApplicationCloseMode];
  open: [path: string];
}>();

function toggleDetails(candidate: ApplicationUninstallCandidate): void {
  expandedId.value = expandedId.value === candidate.applicationId ? null : candidate.applicationId;
  const revision = props.catalog?.catalogRevision;
  if (expandedId.value && revision) {
    void ApplicationService.recordUninstallDetailsOpened(candidate.applicationId, revision).catch(() => {
      LoggerService.warn('application-uninstall', 'details_log_failed');
    });
  }
}

async function openWindowsInstalledApps(): Promise<void> {
  try {
    await ApplicationService.openWindowsInstalledApps();
  } catch {
    toast.error(t('applicationUninstall.openWindowsInstalledAppsFailed'));
  }
}

const query = ref('');
const filter = ref<ApplicationCatalogFilter>('all');
const sort = ref<ApplicationCatalogSort>('sizeDescending');
const expandedId = ref<string | null>(null);
const selectedIds = ref<string[]>([]);
const selectedComponentIds = ref<Record<string, string[]>>({});
const confirmOpen = ref(false);
const recordToRemove = ref<ApplicationUninstallCandidate | null>(null);
const removingRecord = ref(false);

async function removeRecord(): Promise<void> {
  const candidate = recordToRemove.value;
  if (!candidate || removingRecord.value) return;
  removingRecord.value = true;
  try {
    await ApplicationService.removeRecord(candidate.applicationId);
    recordToRemove.value = null;
    toast.success(t('applicationUninstall.removeRecordSuccess'));
    emit('scan');
  } catch (error) {
    const failure = parseCommandError(error);
    if (failure?.code === 'operationCancelled') {
      recordToRemove.value = null;
    } else if (failure?.details.mutationState === 'mayHaveChanged' || failure?.details.reason === 'itemChanged') {
      // The registration may already be gone. Refresh instead of inviting another stale removal.
      recordToRemove.value = null;
      toast.info(t('applicationUninstall.removeRecordRefreshing'));
      emit('scan');
    } else {
      toast.error(
        t(
          failure?.code === 'permissionDenied'
            ? 'applicationUninstall.removeRecordPermissionDenied'
            : 'applicationUninstall.removeRecordFailed'
        )
      );
    }
  } finally {
    removingRecord.value = false;
  }
}

const closeWorkflow = ref(createApplicationUninstallCloseWorkflow());
const cancellationConfirmOpen = ref(false);
const executionList = ref<HTMLElement | null>(null);
const applicationList = ref<InstanceType<typeof MdResultTable> | null>(null);
const iconUrls = ref<ReadonlyMap<string, string>>(new Map());
const busy = computed(
  () => props.scanning || props.preparing || props.executing || props.closingApplications || removingRecord.value
);
const confirmationLoading = computed(() => props.preparing || (confirmOpen.value && !props.plan && !props.preview));
const candidates = computed(() => props.catalog?.candidates ?? []);
const windowsCatalog = OperatingSystemService.isWindows();
const catalogFilters = applicationCatalogFilters(windowsCatalog);
const catalogBytes = computed(() => candidates.value.reduce((total, candidate) => total + candidate.totalBytes, 0));
const actionableCandidates = computed(() => candidates.value.filter(applicationCanStartUninstall));
const filteredCandidates = computed(() =>
  filterAndSortApplications(candidates.value, query.value, filter.value, sort.value)
);
const filteredReadyIds = computed(() =>
  filteredCandidates.value.filter(applicationCanStartUninstall).map(candidate => candidate.applicationId)
);
const selectedSet = computed(() => new Set(selectedIds.value));
const selectedCandidates = computed(() =>
  actionableCandidates.value.filter(candidate => selectedSet.value.has(candidate.applicationId))
);
const selection = computed(() => ({
  applicationIds: selectedIds.value,
  componentIds: selectedComponentIds.value,
}));
const selectedBytes = computed(() => selectedApplicationBytes(actionableCandidates.value, selection.value));
const includesUserData = computed(() => selectionIncludesUserData(actionableCandidates.value, selection.value));
const runningSelectedCandidates = computed(() =>
  selectedCandidates.value.filter(candidate => candidate.capability === 'applicationRunning')
);
const closeItems = computed<ApplicationCloseItem[]>(() =>
  runningSelectedCandidates.value.map(candidate => ({
    id: candidate.applicationId,
    iconPath: candidate.iconPath ?? undefined,
    name: candidate.name,
    processes: candidate.runningProcesses,
  }))
);
const remainingCloseItems = computed(() => {
  const remaining = new Set(closeWorkflow.value.remainingApplicationIds);
  return closeItems.value.filter(item => remaining.has(item.id));
});
const allFilteredSelected = computed(
  () =>
    filteredReadyIds.value.length > 0 &&
    filteredReadyIds.value.every(applicationId => selectedSet.value.has(applicationId))
);
const someFilteredSelected = computed(() =>
  filteredReadyIds.value.some(applicationId => selectedSet.value.has(applicationId))
);
const filterCounts = computed(() => ({
  all: candidates.value.length,
  ready: candidates.value.filter(candidate => applicationMatchesCatalogFilter(candidate, 'ready')).length,
  requiresElevation: candidates.value.filter(candidate =>
    applicationMatchesCatalogFilter(candidate, 'requiresElevation')
  ).length,
  running: candidates.value.filter(candidate => applicationMatchesCatalogFilter(candidate, 'running')).length,
  unavailable: candidates.value.filter(candidate => applicationMatchesCatalogFilter(candidate, 'unavailable')).length,
}));
const filterOptions = computed(() =>
  catalogFilters.map(value => ({
    value,
    label: t(`applicationUninstall.${value}`),
    count: filterCounts.value[value],
  }))
);
const nativeBatch = computed(
  () =>
    (Boolean(props.plan?.plans.length) &&
      props.plan?.plans.every(applicationPlan =>
        applicationPlan.items.some(item => item.kind === 'nativeInstaller')
      )) ||
    (selectedCandidates.value.length > 0 &&
      selectedCandidates.value.every(candidate =>
        candidate.components.some(component => component.kind === 'nativeInstaller')
      ))
);

function updateCatalogFilter(value: string) {
  const nextFilter = catalogFilters.find(option => option === value);
  if (nextFilter) filter.value = nextFilter;
}
const scanStage = computed(() => props.progress?.currentStage ?? 'discoveringApplications');
const scanStageText = computed(() => {
  if (scanStage.value === 'checkingProcesses') return t('applicationUninstall.checkingProcesses');
  if (scanStage.value === 'validatingApplications') return t('applicationUninstall.validatingApplications');
  if (scanStage.value === 'inspectingApplications') return t('applicationUninstall.inspectingApplication');
  return t('applicationUninstall.readingApplicationCatalog');
});
const scanPathLabel = computed(() =>
  scanStage.value === 'inspectingApplications'
    ? t('applicationUninstall.currentApplication')
    : scanStage.value === 'discoveringApplications' || scanStage.value === 'validatingApplications'
      ? t('applicationUninstall.currentApplicationSource')
      : t('applicationUninstall.currentScanStage')
);
const executionProgressValue = computed(() => {
  const completed = props.executionProgress?.completedApplicationCount ?? 0;
  const total = props.executionProgress?.totalApplicationCount ?? props.plan?.plans.length ?? 0;
  return t('applicationUninstall.executionProgressValue', {
    completed: FormatUtils.integer(Math.min(completed, total)),
    total: FormatUtils.integer(total),
  });
});
const executionItems = computed(() => {
  const completed = new Map(
    (props.executionProgress?.completedApplications ?? []).map(result => [result.applicationId, result])
  );
  return (props.plan?.plans ?? []).map(applicationPlan => {
    const candidate = candidates.value.find(item => item.applicationId === applicationPlan.applicationId);
    const previewResult = props.preview?.results.find(item => item.applicationId === applicationPlan.applicationId);
    const result = completed.get(applicationPlan.applicationId);
    const active =
      !result &&
      props.executionProgress?.stage === 'uninstalling' &&
      props.executionProgress.currentApplicationId === applicationPlan.applicationId;
    const state = result?.status ?? (active ? 'active' : 'pending');
    const detail =
      state === 'completed'
        ? t('applicationUninstall.uninstallItemCompleted')
        : state === 'cancelled'
          ? t('applicationUninstall.uninstallItemCancelled')
          : state === 'failed'
            ? t('applicationUninstall.uninstallItemFailed')
            : state === 'active'
              ? t(
                  candidate?.executionMode === 'interactive'
                    ? 'applicationUninstall.uninstallItemInteractive'
                    : 'applicationUninstall.uninstallItemActive'
                )
              : t('applicationUninstall.uninstallItemPending');
    const statusLabel =
      state === 'completed'
        ? t('applicationUninstall.uninstallItemStatus.completed')
        : state === 'cancelled'
          ? t('applicationUninstall.uninstallItemStatus.cancelled')
          : state === 'failed'
            ? t('applicationUninstall.uninstallItemStatus.failed')
            : state === 'active'
              ? t('applicationUninstall.uninstallItemStatus.active')
              : t('applicationUninstall.uninstallItemStatus.pending');
    return {
      applicationId: applicationPlan.applicationId,
      detail,
      name: candidate?.name ?? previewResult?.applicationName ?? t('applicationUninstall.unknownApplication'),
      state,
      statusLabel,
    };
  });
});
const activeExecutionItem = computed(() => executionItems.value.find(item => item.state === 'active') ?? null);
const executionTitle = computed(() => {
  if (props.cancellingExecution) return t('applicationUninstall.cancellingBatch');
  if (props.executionProgress?.stage === 'finalizing') {
    return t('applicationUninstall.finalizingBatch');
  }
  if (activeExecutionItem.value) {
    return t('applicationUninstall.uninstallingApplicationTitle', { name: activeExecutionItem.value.name });
  }
  return t('applicationUninstall.validatingBatch');
});
const executionDescription = computed(() =>
  props.cancellingExecution
    ? t('applicationUninstall.cancellingBatchDescription')
    : t('applicationUninstall.executionProgressDescription', {
        completed: FormatUtils.integer(props.executionProgress?.completedApplicationCount ?? 0),
        total: FormatUtils.integer(props.executionProgress?.totalApplicationCount ?? props.plan?.plans.length ?? 0),
      })
);
const executionPercent = computed(() => {
  const total = props.executionProgress?.totalApplicationCount ?? props.plan?.plans.length ?? 0;
  if (!total) return 0;
  return Math.min(100, ((props.executionProgress?.completedApplicationCount ?? 0) / total) * 100);
});
let iconRequestVersion = 0;
const UNINSTALL_RESULT_TOAST_ID = 'application-uninstall-result';

watch(
  () => filteredCandidates.value.map(candidate => candidate.iconPath).filter((path): path is string => Boolean(path)),
  paths => {
    const requestVersion = ++iconRequestVersion;
    if (!paths.length) {
      if (!candidates.value.length) iconUrls.value = new Map();
      return;
    }

    void ApplicationIconService.resolveIncrementally(paths, icons => {
      if (requestVersion !== iconRequestVersion) return;
      iconUrls.value = new Map([...iconUrls.value, ...icons]);
    });
  },
  { immediate: true }
);

onBeforeUnmount(() => {
  iconRequestVersion += 1;
});

function handleApplicationIconError(iconPath: string | null) {
  if (!iconPath) return;
  const icons = new Map(iconUrls.value);
  icons.delete(iconPath);
  iconUrls.value = icons;
}

watch([query, filter, sort], async () => {
  // A filter or sort can shrink a long result. Reset the domain-owned scroll
  // position after rendering so an old offset never leaves a blank list.
  await nextTick();
  applicationList.value?.scrollTo({ top: 0 });
});

watch(
  () => actionableCandidates.value.map(candidate => candidate.applicationId),
  readyIds => {
    const readySet = new Set(readyIds);
    selectedIds.value = selectedIds.value.filter(applicationId => readySet.has(applicationId));
    selectedComponentIds.value = Object.fromEntries(
      Object.entries(selectedComponentIds.value).filter(([applicationId]) => readySet.has(applicationId))
    );
  }
);

watch(
  () => props.plan?.batchId,
  batchId => {
    confirmOpen.value = Boolean(batchId);
  }
);

watch(
  () => props.preparing,
  (preparing, wasPreparing) => {
    if (wasPreparing && !preparing && !props.plan) confirmOpen.value = false;
  }
);

watch(
  () => props.executionProgress?.currentApplicationId,
  async applicationId => {
    if (!applicationId) return;
    await nextTick();
    // Keep the current application visible in long batches without changing
    // the fixed execution order or stealing focus from a native uninstaller.
    executionList.value
      ?.querySelector<HTMLElement>('.uninstall-execution-item.is-active')
      ?.scrollIntoView({ block: 'nearest' });
  }
);

watch(
  () => props.cancellationRevision,
  (revision, previousRevision) => {
    if (!revision || revision === previousRevision) return;
    toast.info(t('applicationUninstall.executionCancelledTitle'), {
      description: t('applicationUninstall.cancelledDescription'),
      id: UNINSTALL_RESULT_TOAST_ID,
    });
  }
);

watch(
  () => props.lastResult,
  result => {
    if (!result) return;

    // Clear the completed selection immediately so the action bar cannot
    // submit the same batch again while the result toast is being published.
    selectedIds.value = [];
    selectedComponentIds.value = {};
    expandedId.value = null;

    const cancelledApplications = result.results.filter(application =>
      application.actions.some(action => action.status === 'cancelled')
    ).length;
    const title = cancelledApplications
      ? t('applicationUninstall.executionCancelledTitle')
      : result.failedItemCount
        ? t('applicationUninstall.completedWithWarnings')
        : t('applicationUninstall.completed');
    if (cancelledApplications) {
      toast.info(title, {
        description: t('applicationUninstall.executionCancelledDescription', {
          completed: FormatUtils.integer(result.affectedApplicationCount),
          cancelled: FormatUtils.integer(cancelledApplications),
        }),
        id: UNINSTALL_RESULT_TOAST_ID,
      });
      return;
    }
    const summary = t(
      result.releasedBytesIsEstimate
        ? 'applicationUninstall.batchExecutionEstimateSummary'
        : 'applicationUninstall.batchExecutionSummary',
      {
        count: FormatUtils.integer(result.affectedApplicationCount),
        size: ByteSizeService.bytes(result.releasedBytes),
        failed: FormatUtils.integer(result.failedApplicationCount),
      }
    );
    const description = result.restartRequired ? `${summary} ${t('applicationUninstall.restartRequired')}` : summary;
    const options = { description, id: UNINSTALL_RESULT_TOAST_ID };
    if (result.failedItemCount) toast.warning(title, options);
    else toast.success(title, options);
  }
);

function toggleSelection(candidate: ApplicationUninstallCandidate) {
  if (busy.value) return;
  const next = toggleApplicationSelection(selection.value, candidate);
  selectedIds.value = next.applicationIds;
  selectedComponentIds.value = next.componentIds;
}

function toggleComponent(candidate: ApplicationUninstallCandidate, component: ApplicationUninstallComponentSummary) {
  if (busy.value) return;
  const next = toggleApplicationComponent(selection.value, candidate, component);
  selectedIds.value = next.applicationIds;
  selectedComponentIds.value = next.componentIds;
}

function toggleFilteredSelection(checked: boolean) {
  if (busy.value || !filteredReadyIds.value.length) return;
  const next = setVisibleApplicationSelection(
    selection.value,
    filteredCandidates.value.filter(applicationCanStartUninstall),
    checked
  );
  selectedIds.value = next.applicationIds;
  selectedComponentIds.value = next.componentIds;
}

function clearSelection() {
  if (busy.value) return;
  selectedIds.value = [];
  selectedComponentIds.value = {};
}

function prepareSelection() {
  if (busy.value || !selectedIds.value.length) return;
  if (runningSelectedCandidates.value.length) {
    closeWorkflow.value = beginApplicationCloseWorkflow(
      runningSelectedCandidates.value.map(candidate => candidate.applicationId)
    );
    return;
  }
  prepareCurrentSelection();
}

function prepareCurrentSelection() {
  if (!selectedIds.value.length) return;
  confirmOpen.value = true;
  emit(
    'prepare',
    selectedIds.value.map(applicationId => ({
      applicationId,
      componentIds: selectedComponentIds.value[applicationId] ?? [],
    }))
  );
}

function prepareApplication(candidate: ApplicationUninstallCandidate) {
  if (busy.value || !applicationCanStartUninstall(candidate)) return;
  /*
   * Row uninstall is an explicit single-application shortcut, so it reuses
   * the existing selection policy from an empty selection. Required/default
   * components stay consistent without carrying earlier choices into review.
   */
  const next = toggleApplicationSelection({ applicationIds: [], componentIds: {} }, candidate);
  if (!next.applicationIds.length) return;
  selectedIds.value = next.applicationIds;
  selectedComponentIds.value = next.componentIds;
  prepareSelection();
}

function requestApplicationClose(mode: ApplicationCloseMode) {
  if (props.closingApplications) return;
  const applicationIds = applicationCloseRequestIds(closeWorkflow.value, mode);
  if (!applicationIds.length) {
    finishApplicationClose([]);
    return;
  }
  emit('closeApplications', applicationIds, mode);
}

function updateCloseDialog(open: boolean) {
  // A close request owns the process snapshot until it completes. Keeping the
  // dialog mounted prevents a late result from being applied to a new choice.
  if (!open && props.closingApplications) return;
  closeWorkflow.value.open = open;
  if (open) return;
  closeWorkflow.value = createApplicationUninstallCloseWorkflow();
}

function finishApplicationClose(skippedApplicationIds: string[]) {
  const nextSelection = finishApplicationCloseWorkflow(selection.value, closeWorkflow.value, skippedApplicationIds);
  selectedIds.value = nextSelection.applicationIds;
  selectedComponentIds.value = nextSelection.componentIds;
  closeWorkflow.value = createApplicationUninstallCloseWorkflow();
  if (selectedIds.value.length) prepareCurrentSelection();
}

watch(
  () => props.closeResult,
  result => {
    if (!closeWorkflow.value.open || !result) return;
    const transition = applyApplicationCloseResult(closeWorkflow.value, result);
    if (!transition.completed) {
      closeWorkflow.value = transition.workflow;
      return;
    }
    finishApplicationClose([]);
  }
);

function changeSort(key: ApplicationCatalogSortKey) {
  sort.value = nextApplicationCatalogSort(sort.value, key);
}

function sortIcon(key: ApplicationCatalogSortKey) {
  if (applicationCatalogSortKey(sort.value) !== key) return ICON_NAMES.arrowUpDown;
  return applicationCatalogSortAscending(sort.value) ? ICON_NAMES.arrowUp : ICON_NAMES.arrowDown;
}

function updateConfirmation(open: boolean) {
  if (props.executing) return;
  const notifyCancellation = shouldNotifyUninstallCancellation(confirmOpen.value, open);
  confirmOpen.value = open;
  if (!open) {
    emit('cancelPlan');
    if (notifyCancellation) {
      toast.info(t('applicationUninstall.cancelledTitle'), {
        description: t('applicationUninstall.cancelledDescription'),
        id: UNINSTALL_CANCELLATION_TOAST_ID,
      });
    }
  }
}

function requestCancelExecution() {
  if (!props.executing || props.cancellingExecution) return;
  cancellationConfirmOpen.value = true;
}

function confirmCancelExecution() {
  if (!props.executing || props.cancellingExecution) return;
  cancellationConfirmOpen.value = false;
  emit('cancelExecution');
}
</script>

<template>
  <MdPageShell
    class="@container/application-uninstall"
    content-mode="workspace"
    :title="t('applicationUninstall.title')"
  >
    <template v-if="catalog" #actions>
      <Button variant="outline" type="button" :disabled="busy" @click="emit('scan')">
        <MdIcon :class="{ 'icon-spin': scanning }" :name="ICON_NAMES.refresh" :size="17" />
        {{ scanning ? t('loading.currentStage') : t('applicationUninstall.rescan') }}
      </Button>
    </template>

    <MdOperationWorkspace v-if="scanning">
      <MdOperationProgress
        :icon-name="ICON_NAMES.uninstall"
        :title="cancelling ? t('loading.cancelling') : t('applicationUninstall.scanning')"
        :progress="progress"
        :path-label="scanPathLabel"
        :preparing-text="scanStageText"
        :hint="t('applicationUninstall.scanHint')"
        :items-label="t('applicationUninstall.checkedApplications')"
        :bytes-label="t('applicationUninstall.scannedApplicationData')"
        :cancelable="true"
        :cancel-disabled="cancelling"
        @cancel="emit('cancelScan')"
      />
    </MdOperationWorkspace>

    <MdResultWorkspace v-else>
      <template v-if="catalog?.catalogActionable" #summary>
        <MdResultSummary
          :title="
            t('applicationUninstall.summary', { count: FormatUtils.integer(candidates.length) }, candidates.length)
          "
          :metric-label="t('applicationUninstall.summarySpace')"
          :metric-value="ByteSizeService.bytes(catalogBytes)"
        />
      </template>

      <template v-if="catalog?.catalogActionable" #header>
        <MdResultFilterToolbar>
          <MdCategoryFilter
            :model-value="filter"
            :options="filterOptions"
            :accessibility-label="t('applicationUninstall.filterLabel')"
            :disabled="busy"
            @update:model-value="updateCatalogFilter"
          />
          <template #aside>
            <MdResultSearch v-model="query" :placeholder="t('applicationUninstall.searchPlaceholder')" />
          </template>
        </MdResultFilterToolbar>
      </template>

      <template v-if="catalog">
        <MdInlineNotice
          v-if="preview?.failedItemCount && !plan"
          class="catalog-notice"
          :icon-name="ICON_NAMES.info"
          :title="t('applicationUninstall.preflightFailed')"
          tone="warning"
          role="alert"
        >
          {{ t('applicationUninstall.batchPreflightFailedSummary') }}
        </MdInlineNotice>

        <MdEmptyState
          v-if="!catalog.catalogActionable"
          :icon-name="ICON_NAMES.info"
          :title="t('applicationUninstall.incompleteTitle')"
          :description="t('applicationUninstall.incompleteDescription')"
        />
        <template v-else>
          <MdInlineNotice
            v-if="!catalog.executionSupported"
            class="catalog-notice"
            :icon-name="ICON_NAMES.info"
            :title="t('applicationUninstall.viewOnlyTitle')"
          >
            {{ t('applicationUninstall.viewOnlyDescription') }}
          </MdInlineNotice>

          <section class="catalog">
            <MdEmptyState
              v-if="!filteredCandidates.length"
              class="catalog-empty"
              compact
              :icon-name="ICON_NAMES.search"
              :title="t('applicationUninstall.emptyTitle')"
              :description="t('applicationUninstall.emptyDescription')"
            />

            <MdResultTable v-else ref="applicationList" class="application-list">
              <template #header>
                <div class="application-list-header">
                  <label>
                    <MdResultCheckbox
                      :checked="allFilteredSelected"
                      :indeterminate="someFilteredSelected && !allFilteredSelected"
                      :disabled="busy || !filteredReadyIds.length"
                      :aria-label="t('applicationUninstall.selectVisible')"
                      @update:checked="toggleFilteredSelection"
                    />
                  </label>
                  <button
                    class="name-header md-result-sort"
                    type="button"
                    :data-active="applicationCatalogSortKey(sort) === 'name'"
                    :aria-label="t('applicationUninstall.sortByName')"
                    @click="changeSort('name')"
                  >
                    {{ t('applicationUninstall.applicationName') }}
                    <MdIcon :name="sortIcon('name')" :size="14" />
                  </button>
                  <button
                    class="status-header md-result-sort"
                    type="button"
                    :data-active="applicationCatalogSortKey(sort) === 'status'"
                    :aria-label="t('applicationUninstall.sortByStatus')"
                    @click="changeSort('status')"
                  >
                    {{ t('applicationUninstall.status') }}
                    <MdIcon :name="sortIcon('status')" :size="14" />
                  </button>
                  <button
                    class="size-header md-result-sort"
                    type="button"
                    :data-active="applicationCatalogSortKey(sort) === 'size'"
                    :aria-label="t('applicationUninstall.sortBySize')"
                    @click="changeSort('size')"
                  >
                    {{ t('applicationUninstall.applicationSize') }}
                    <MdIcon :name="sortIcon('size')" :size="14" />
                  </button>
                  <button
                    class="application-date-header md-result-sort"
                    type="button"
                    :data-active="applicationCatalogSortKey(sort) === 'date'"
                    :aria-label="t('applicationUninstall.sortByDate')"
                    @click="changeSort('date')"
                  >
                    {{
                      t(windowsCatalog ? 'applicationUninstall.installedOrUpdated' : 'applicationUninstall.lastUsed')
                    }}
                    <MdIcon :name="sortIcon('date')" :size="14" />
                  </button>
                  <span />
                </div>
              </template>

              <MdApplicationUninstallRow
                v-for="candidate in filteredCandidates"
                :key="candidate.applicationId"
                :candidate="candidate"
                :icon-src="candidate.iconPath ? iconUrls.get(candidate.iconPath) : undefined"
                :selected="selectedSet.has(candidate.applicationId)"
                :selected-component-ids="selectedComponentIds[candidate.applicationId] ?? []"
                :expanded="expandedId === candidate.applicationId"
                :busy="busy"
                :uninstall-enabled="catalog.executionSupported"
                @toggle-selection="toggleSelection(candidate)"
                @toggle-component="toggleComponent(candidate, $event)"
                @toggle-expanded="toggleDetails(candidate)"
                @open="emit('open', $event)"
                @open-windows-settings="openWindowsInstalledApps"
                @remove-record="recordToRemove = candidate"
                @uninstall="prepareApplication(candidate)"
                @icon-error="handleApplicationIconError(candidate.iconPath)"
              />
            </MdResultTable>
          </section>
        </template>
      </template>

      <MdEmptyState
        v-else
        :icon-name="ICON_NAMES.uninstall"
        :title="t('applicationUninstall.initialTitle')"
        :description="t('applicationUninstall.initialDescription')"
      >
        <Button size="lg" type="button" :disabled="busy" @click="emit('scan')">
          <MdIcon :name="ICON_NAMES.scan" :size="17" />
          {{ t('applicationUninstall.startScan') }}
        </Button>
      </MdEmptyState>
    </MdResultWorkspace>

    <template v-if="!scanning && catalog?.catalogActionable && catalog.executionSupported" #footer>
      <MdSelectionActionBar
        :selected-label="t('applicationUninstall.selectedApplicationsLabel')"
        :selected-value="FormatUtils.integer(selectedCandidates.length)"
        :space-label="t('applicationUninstall.selectedSizeLabel')"
        :space-value="ByteSizeService.bytes(selectedBytes)"
        :clear-label="selectedCandidates.length ? t('applicationUninstall.clearSelection') : undefined"
        :action-label="t('applicationUninstall.uninstallSelected')"
        :disabled="!selectedCandidates.length"
        :busy="busy"
        @clear="clearSelection"
        @action="prepareSelection"
      >
        <template #action-icon>
          <span v-if="preparing || executing" class="button-spinner" />
          <MdIcon v-else :name="ICON_NAMES.uninstall" :size="17" />
        </template>
      </MdSelectionActionBar>
    </template>

    <MdDestructiveActionDialog
      :open="recordToRemove !== null"
      :title="t('applicationUninstall.removeRecordTitle')"
      :summary-label="recordToRemove?.name ?? ''"
      :description="t('applicationUninstall.removeRecordDescription')"
      :cancel-label="t('common.cancel')"
      :confirm-label="t('applicationUninstall.removeRecord')"
      :busy="removingRecord"
      @update:open="
        open => {
          if (!open) recordToRemove = null;
        }
      "
      @confirm="removeRecord"
    />
    <MdDestructiveActionDialog
      :open="confirmOpen"
      :title="
        executing
          ? executionTitle
          : t(nativeBatch ? 'applicationUninstall.confirmNativeBatchTitle' : 'applicationUninstall.confirmBatchTitle', {
              count: FormatUtils.integer(preview?.selectedApplicationCount ?? selectedCandidates.length),
            })
      "
      :description="
        executing
          ? executionDescription
          : t(
              nativeBatch
                ? 'applicationUninstall.confirmNativeBatchDescription'
                : includesUserData
                  ? 'applicationUninstall.confirmBatchWithDataDescription'
                  : 'applicationUninstall.confirmBatchDescription'
            )
      "
      :summary-label="
        executing
          ? ''
          : preparing
            ? t('applicationUninstall.preparingBatchSummary')
            : t('applicationUninstall.confirmBatchSummary', {
                count: FormatUtils.integer(preview?.previewedApplicationCount ?? 0),
              })
      "
      :summary-value="
        executing ? '' : ByteSizeService.bytes(preview?.previewedBytes ?? plan?.expectedBytes ?? selectedBytes)
      "
      :cancel-label="t('common.cancel')"
      :confirm-label="t('applicationUninstall.confirmBatchAction')"
      :busy="executing"
      :loading="confirmationLoading"
      :loading-label="t('applicationUninstall.preparingBatchSummary')"
      :show-details="executing"
      @update:open="updateConfirmation"
      @confirm="emit('execute')"
    >
      <div
        ref="executionList"
        class="uninstall-execution-list"
        :aria-label="t('applicationUninstall.uninstallItemList')"
      >
        <div
          v-for="item in executionItems"
          :key="item.applicationId"
          class="uninstall-execution-item"
          :class="`is-${item.state}`"
        >
          <span class="uninstall-execution-status" aria-hidden="true">
            <MdIcon v-if="item.state === 'completed'" :name="ICON_NAMES.check" :size="14" />
            <MdIcon v-else-if="item.state === 'cancelled'" :name="ICON_NAMES.minus" :size="13" />
            <b v-else-if="item.state === 'failed'">!</b>
            <i v-else-if="item.state === 'active'" class="md-operational-motion" />
            <i v-else />
          </span>
          <span class="uninstall-execution-copy">
            <strong :title="item.name">{{ item.name }}</strong>
            <small>{{ item.detail }}</small>
          </span>
          <small class="uninstall-execution-item-label">{{ item.statusLabel }}</small>
        </div>
      </div>
      <div
        class="uninstall-execution-progress"
        role="progressbar"
        :aria-label="executionProgressValue"
        aria-valuemin="0"
        aria-valuemax="100"
        :aria-valuenow="Math.round(executionPercent)"
      >
        <span :style="{ width: `${executionPercent}%` }" />
      </div>
      <div class="uninstall-execution-actions">
        <Button
          class="uninstall-execution-cancel"
          variant="ghost"
          size="sm"
          type="button"
          :disabled="cancellingExecution"
          @click="requestCancelExecution"
        >
          {{
            cancellingExecution
              ? t('applicationUninstall.cancellingAction')
              : t('applicationUninstall.cancelExecutionAction')
          }}
        </Button>
      </div>
    </MdDestructiveActionDialog>

    <Dialog :open="closeWorkflow.open" @update:open="updateCloseDialog">
      <MdDialogContent class="flex min-h-0 flex-col" size="large" :show-close="!closingApplications">
        <MdDialogHeader>
          <DialogTitle>{{ t('applicationUninstall.closeBeforeUninstallTitle') }}</DialogTitle>
          <DialogDescription>
            {{
              closeWorkflow.phase === 'selection'
                ? t('applicationUninstall.closeBeforeUninstallDescription')
                : t('applicationClose.normalCloseFailed')
            }}
          </DialogDescription>
        </MdDialogHeader>

        <div class="min-h-0 overflow-auto px-6 pb-4">
          <p v-if="closeWorkflow.phase === 'force'" class="uninstall-force-close-warning">
            {{ t('applicationClose.forceWarning') }}
          </p>
          <MdApplicationClosePanel
            v-if="closeWorkflow.phase === 'selection'"
            v-model:selected-ids="closeWorkflow.selectedApplicationIds"
            :items="closeItems"
            :disabled="closingApplications"
          />
          <MdApplicationClosePanel v-else :items="remainingCloseItems" :selectable="false" />
        </div>

        <MdDialogFooter>
          <Button
            v-if="closeWorkflow.phase === 'selection'"
            variant="outline"
            type="button"
            :disabled="closingApplications"
            @click="updateCloseDialog(false)"
          >
            {{ t('common.cancel') }}
          </Button>
          <Button
            v-else
            variant="outline"
            type="button"
            :disabled="closingApplications"
            @click="finishApplicationClose(closeWorkflow.remainingApplicationIds)"
          >
            {{ t('applicationClose.skipAndContinue') }}
          </Button>
          <Button
            :variant="closeWorkflow.phase === 'force' ? 'destructive' : 'default'"
            type="button"
            :disabled="closingApplications"
            @click="requestApplicationClose(closeWorkflow.phase === 'force' ? 'force' : 'graceful')"
          >
            {{
              closingApplications
                ? t('applicationClose.closing')
                : closeWorkflow.phase === 'force'
                  ? t('applicationClose.forceAndContinue')
                  : closeWorkflow.selectedApplicationIds.length
                    ? t(
                        'applicationClose.closeSelectedAndContinue',
                        { count: FormatUtils.integer(closeWorkflow.selectedApplicationIds.length) },
                        closeWorkflow.selectedApplicationIds.length
                      )
                    : t('applicationClose.skipAndContinue')
            }}
          </Button>
        </MdDialogFooter>
      </MdDialogContent>
    </Dialog>

    <MdConfirmDialog
      v-model:open="cancellationConfirmOpen"
      :title="t('applicationUninstall.cancelExecutionConfirmTitle')"
      :description="t('applicationUninstall.cancelExecutionConfirmDescription')"
      :cancel-label="t('common.cancel')"
      :confirm-label="t('applicationUninstall.stopExecutionAction')"
      confirm-variant="destructive"
      @confirm="confirmCancelExecution"
    />
  </MdPageShell>
</template>

<style scoped src="./application-uninstall.css"></style>
