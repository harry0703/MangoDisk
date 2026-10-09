<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { computed, defineAsyncComponent, ref, watch } from 'vue';
import { Button } from '@/components/ui/button';

import MdEmptyState from '@/components/custom/md-empty-state.vue';
import MdOperationWorkspace from '@/components/custom/md-operation-workspace.vue';
import MdPageShell from '@/components/custom/md-page-shell.vue';
import MdAiWorkspace from '@/layouts/components/md-ai-workspace.vue';
import MdPermissionGuidance from '@/components/custom/md-permission-guidance.vue';
import MdResultSummary from '@/components/custom/md-result-summary.vue';
import MdScanExclusionLink from '@/components/custom/md-scan-exclusion-link.vue';
import MdResultWorkspace from '@/components/custom/md-result-workspace.vue';
import type {
  ApplicationLeftoverCandidate,
  ApplicationLeftoverResult,
  ApplicationLeftoverScanResult,
} from '@/lib/models/application';
import type { ApplicationCloseBatchResult, ApplicationCloseMode } from '@/lib/models/application-close';
import {
  CLEANUP_OPERATION_IDS,
  CLEANUP_SCAN_SCOPE_MODES,
  STANDARD_CLEANUP_SCAN_SCOPE,
  type CleanupScanScope,
  type CleanupSourceSelection,
} from '@/lib/models/cleanup';
import type { DiskInfo } from '@/lib/models/disk';
import { LOG_DOMAINS, LOG_EVENTS } from '@/lib/models/telemetry';
import { ICON_NAMES } from '@/lib/models/ui';
import { MACOS_PRIVACY_DESTINATION_IDS } from '@/lib/models/macos-permissions';
import type { CleanupOperationId, PresentedCleanupResult, PresentedCleanupScanResult } from '@/lib/models/cleanup';
import type { TraversalProgress } from '@/lib/models/progress';
import * as CleanupRuleSelectionUtils from '@/lib/utils/cleanup-rule-selection';
import type { CleanupSelectionMode } from '@/lib/utils/cleanup-rule-selection';
import { ByteSizeService } from '@/lib/services/byte-size-service';
import { DiskService } from '@/lib/services/disk-service';
import { FileManagerService } from '@/lib/services/file-manager-service';
import { LoggerService } from '@/lib/services/logger-service';
import { MacOsPermissionService } from '@/lib/services/macos-permission-service';
import * as FormatUtils from '@/lib/utils/format';
import * as PathUtils from '@/lib/utils/path';
import { useCustomCleanupStore } from '@/stores/custom-cleanup-store';
import { useCleanupStore } from '@/stores/cleanup-store';
import { useAppStore } from '@/stores/app-store';
import { useAiStore } from '@/stores/ai-store';

import { groupApplicationLeftovers, recommendedApplicationLeftoverIds } from './application-leftover-groups';
import { selectedCleanupCloseRequirement } from './cleanup-close-requirement';
import { countSelectedCleanupGroups } from './cleanup-result-categories';
import MdCleanupPlanDialog from './components/md-cleanup-plan-dialog.vue';
import MdCleanupScanButton from './components/md-cleanup-scan-button.vue';
import MdCleanupVolumeDialog from './components/md-cleanup-volume-dialog.vue';
import MdCustomCleanupDialog from './components/md-custom-cleanup-dialog.vue';
import MdIncompleteScanGuidance from './components/md-incomplete-scan-guidance.vue';
import MdSystemDiskUsage from './components/md-system-disk-usage.vue';

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
const customCleanupStore = useCustomCleanupStore();
const cleanupStore = useCleanupStore();
const appStore = useAppStore();
const aiStore = useAiStore();

// Start the small preference read with the page instead of making the first
// dialog interaction wait for storage initialization.
void customCleanupStore.initialize();

const props = defineProps<{
  busy: boolean;
  disk: DiskInfo | null;
  disks: DiskInfo[];
  leftovers: ApplicationLeftoverScanResult | null;
  leftoverResult: ApplicationLeftoverResult | null;
  scanningLeftovers: boolean;
  deletingLeftovers: boolean;
  loadingMessage: string;
  operation: CleanupOperationId;
  progress: TraversalProgress | null;
  result: PresentedCleanupResult | null;
  scan: PresentedCleanupScanResult | null;
  scanScope: CleanupScanScope;
  selectedBytes: number;
  selectedRuleIds: string[];
  sourceSelections: CleanupSourceSelection[];
  closingApplications: boolean;
  closeResult: ApplicationCloseBatchResult | null;
  privilegedScanRuleId: string | null;
}>();
const emit = defineEmits<{
  cancel: [];
  closeApplications: [ruleIds: string[], mode: ApplicationCloseMode];
  execute: [leftovers: ApplicationLeftoverCandidate[]];
  open: [path: string];
  scan: [scope: CleanupScanScope];
  selectAll: [ruleIds: string[], selected: boolean];
  toggleSource: [ruleId: string, path: string];
  privilegedScan: [];
  openExclusions: [];
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
const volumeDialogOpen = ref(false);
const customDialogOpen = ref(false);
const permissionPromptOpen = ref(false);
const permissionPromptShown = ref(false);
const incompleteScanPromptOpen = ref(false);
const selectableDisks = ref<DiskInfo[]>([]);
const selectedLeftoverIds = ref<string[]>([]);
const scanRules = computed(() => props.scan?.rules ?? []);
const isMacOs = computed(() => MacOsPermissionService.isMacOs());
// Read failures alone do not imply missing privacy permission. The backend
// distinguishes possible privacy restrictions from ordinary access and I/O errors.
const showPermissionGuidance = computed(
  () => Boolean(props.scan?.accessLimited) && isMacOs.value && !appStore.settings.hideCleanupReadFailureAlerts
);
const hideReadFailureAlerts = computed(() => appStore.settings.hideCleanupReadFailureAlerts);

function setHideReadFailureAlerts(value: boolean) {
  appStore.saveSettings({ ...appStore.settings, hideCleanupReadFailureAlerts: value });
}

watch(
  () => props.scan,
  () => {
    incompleteScanPromptOpen.value = false;
  }
);

watch(
  showPermissionGuidance,
  needsPermission => {
    if (!needsPermission) {
      permissionPromptOpen.value = false;
      return;
    }
    if (permissionPromptShown.value) return;
    permissionPromptShown.value = true;
    permissionPromptOpen.value = true;
  },
  { immediate: true }
);

async function openPrivacySettings(): Promise<boolean> {
  try {
    await MacOsPermissionService.openPrivacySettings(MACOS_PRIVACY_DESTINATION_IDS.fullDiskAccess);
    return true;
  } catch (error) {
    useAppStore().reportError(error);
    return false;
  }
}

async function openApplicationLogs() {
  try {
    await FileManagerService.openApplicationLogs();
  } catch (error) {
    useAppStore().reportError(error);
  }
}

const selectableRuleIds = computed(() => CleanupRuleSelectionUtils.selectableRuleIds(scanRules.value));
const bulkSelectableRuleIds = computed(() => CleanupRuleSelectionUtils.bulkSelectableRuleIds(scanRules.value));
const recommendedRuleIds = computed(() => CleanupRuleSelectionUtils.recommendedRuleIds(scanRules.value));
const foundCleanupBytes = computed(() => CleanupRuleSelectionUtils.foundBytes(scanRules.value));
const selectableCleanupBytes = computed(() => CleanupRuleSelectionUtils.selectableBytes(scanRules.value));
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
const totalSelectableBytes = computed(() => selectableCleanupBytes.value + (props.leftovers?.totalBytes ?? 0));
const selectionMode = computed<CleanupSelectionMode>(() => {
  const cleanupMode = CleanupRuleSelectionUtils.selectionMode(
    scanRules.value,
    props.selectedRuleIds,
    props.sourceSelections
  );
  const leftoverCount = leftoverCandidates.value.length;
  const selectedLeftoverCount = selectedLeftoverIds.value.length;
  const selectedRecommendedLeftovers =
    selectedLeftoverCount === recommendedLeftoverIds.value.length &&
    recommendedLeftoverIds.value.every(candidateId => selectedLeftoverSet.value.has(candidateId));
  if (!selectedLeftoverCount && ['smart', 'none'].includes(cleanupMode)) return cleanupMode;
  if (!leftoverCount) return cleanupMode;
  if (cleanupMode === 'smart' && selectedRecommendedLeftovers) return 'smart';
  if (cleanupMode === 'all' && selectedLeftoverCount === leftoverCount) return 'all';
  return 'manual';
});
const scanning = computed(
  () =>
    props.scanningLeftovers ||
    (props.busy &&
      (props.operation === CLEANUP_OPERATION_IDS.scanning || props.operation === CLEANUP_OPERATION_IDS.cancelling))
);
function openConfirm() {
  if (selectedItemCount.value) confirmOpen.value = true;
}

function execute() {
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
  emit('selectAll', ruleIds, selected);
}

function toggleSource(ruleId: string, path: string) {
  emit('toggleSource', ruleId, path);
}

async function startScan(scope: CleanupScanScope) {
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
  emit('scan', scope);
}

function startStandardScan() {
  void startScan(STANDARD_CLEANUP_SCAN_SCOPE);
}

async function refreshSelectableDisks() {
  try {
    selectableDisks.value = await DiskService.listDisks();
  } catch (error) {
    selectableDisks.value = [...props.disks];
    LoggerService.warn(LOG_DOMAINS.cleanup, LOG_EVENTS.volumeSelectionRefreshFailed, {
      errorType: error instanceof Error ? error.name : typeof error,
      fallbackDiskCount: selectableDisks.value.length,
    });
  }
}

async function repeatScan() {
  if (props.scanScope.mode !== CLEANUP_SCAN_SCOPE_MODES.selectedVolumes) {
    await startScan(props.scanScope);
    return;
  }
  await refreshSelectableDisks();
  const availableKeys = new Set(selectableDisks.value.map(disk => PathUtils.comparisonKey(disk.mountPoint)));
  const selectionIsAvailable = props.scanScope.volumeMountPoints.every(mountPoint =>
    availableKeys.has(PathUtils.comparisonKey(mountPoint))
  );
  if (!selectionIsAvailable) {
    volumeDialogOpen.value = true;
    return;
  }
  await startScan(props.scanScope);
}

function startCustomScan(rules: CleanupScanScope & { mode: typeof CLEANUP_SCAN_SCOPE_MODES.custom }) {
  void startScan(rules);
}

function handleCustomRules(rules: Parameters<typeof startCustomScan>[0]['rules'], includeStandardRules: boolean) {
  startCustomScan({ mode: CLEANUP_SCAN_SCOPE_MODES.custom, includeStandardRules, rules });
}

async function openVolumeDialog() {
  await refreshSelectableDisks();
  volumeDialogOpen.value = true;
}

function startSelectedVolumeScan(mountPoints: string[]) {
  if (!mountPoints.length) {
    startStandardScan();
    return;
  }
  void startScan({
    mode: CLEANUP_SCAN_SCOPE_MODES.selectedVolumes,
    volumeMountPoints: mountPoints,
  });
}

function toggleLeftover(candidate: ApplicationLeftoverCandidate) {
  if (props.busy) return;
  selectedLeftoverIds.value = selectedLeftoverSet.value.has(candidate.candidateId)
    ? selectedLeftoverIds.value.filter(candidateId => candidateId !== candidate.candidateId)
    : [...selectedLeftoverIds.value, candidate.candidateId];
}

function setLeftoverGroupSelected(candidateIds: string[], selected: boolean) {
  if (props.busy) return;
  const targetIds = new Set(candidateIds);
  selectedLeftoverIds.value = selected
    ? [...new Set([...selectedLeftoverIds.value, ...candidateIds])]
    : selectedLeftoverIds.value.filter(candidateId => !targetIds.has(candidateId));
}

function setSelectionMode(value: unknown) {
  if (!['smart', 'all', 'none'].includes(String(value))) return;
  const mode = String(value) as Exclude<CleanupSelectionMode, 'manual'>;
  emit('selectAll', selectableRuleIds.value, false);
  selectedLeftoverIds.value = [];
  if (mode === 'smart') {
    emit('selectAll', recommendedRuleIds.value, true);
    selectedLeftoverIds.value = recommendedLeftoverIds.value;
  } else if (mode === 'all') {
    emit('selectAll', bulkSelectableRuleIds.value, true);
    selectedLeftoverIds.value = leftoverCandidates.value.map(candidate => candidate.candidateId);
  }
}

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
// A reply describes one scan snapshot. Native work can invalidate its sizes,
// paths or capabilities; navigation and selection-only edits must preserve it.
watch(
  () => [props.scan, props.result, props.busy, scanning.value, props.closingApplications, props.privilegedScanRuleId],
  () => aiStore.dismissModule('cleanup')
);
</script>

<template>
  <MdPageShell class="@container/cleanup" content-mode="workspace" :title="t('cleanup.title')">
    <template #overlay><MdAiWorkspace module="cleanup" /></template>
    <template #actions>
      <div class="scan-action">
        <MdSystemDiskUsage v-if="disk" :disk="disk" />
        <MdCleanupScanButton
          v-if="scan && !scanning"
          :busy="busy"
          action="rescan"
          @primary="repeatScan"
          @standard="startStandardScan"
          @select-volumes="openVolumeDialog"
          @custom="customDialogOpen = true"
        />
      </div>
    </template>

    <template v-if="!scanning && scan" #footer>
      <MdSelectionActionBar
        :selected-label="t('cleanup.selectedSummary')"
        :selected-value="t('common.itemCount', { count: FormatUtils.integer(selectedItemCount) }, selectedItemCount)"
        :space-label="t('common.estimatedRelease')"
        :space-value="ByteSizeService.bytes(totalSelectedBytes)"
        :action-label="t('cleanup.clean')"
        :disabled="!selectedItemCount"
        :busy="busy"
        @action="openConfirm"
      >
        <template #options>
          <MdCleanupSelectionMode
            :busy="busy"
            :mode="selectionMode"
            :recommended-bytes="recommendedCleanupBytes"
            :total-bytes="totalSelectableBytes"
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
            : isMacOs && scanScope.mode === CLEANUP_SCAN_SCOPE_MODES.standard
              ? t('cleanup.permission.protectedScanHint')
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
        >
          <template v-if="cleanupStore.scanExcludedFolders.length || cleanupStore.scanExcludedNames.length" #status>
            <MdScanExclusionLink
              :hint="
                t(
                  cleanupStore.scanExcludedNames.length
                    ? 'storageScanExclusions.cleanupNamesResultHint'
                    : 'storageScanExclusions.cleanupResultHint'
                )
              "
              @open="emit('openExclusions')"
            />
          </template>
          <template
            v-if="
              showPermissionGuidance ||
              (scan.readFailureCount && (!hideReadFailureAlerts || incompleteScanPromptOpen)) ||
              scan.missingCustomRootCount
            "
            #actions
          >
            <div class="flex max-w-full flex-wrap items-center justify-end gap-x-3 gap-y-1 text-right">
              <MdPermissionGuidance
                v-if="showPermissionGuidance"
                v-model="permissionPromptOpen"
                :summary="t('fullDiskAccessGuidance.summary')"
                :title="t('fullDiskAccessGuidance.title')"
                :description="t('cleanup.permission.description')"
                :instructions="t('fullDiskAccessGuidance.instructions')"
                :skip-label="t('fullDiskAccessGuidance.skip')"
                :open-settings-label="t('fullDiskAccessGuidance.openSettings')"
                :open-settings="openPrivacySettings"
              />
              <MdIncompleteScanGuidance
                v-else-if="scan.readFailureCount && (!hideReadFailureAlerts || incompleteScanPromptOpen)"
                v-model="incompleteScanPromptOpen"
                :failure-count="scan.readFailureCount"
                :failure-details="scan.readFailureDetails ?? []"
                :retry-disabled="busy"
                :hide-read-failure-alerts="appStore.settings.hideCleanupReadFailureAlerts"
                @update:hide-read-failure-alerts="setHideReadFailureAlerts"
                @open-logs="openApplicationLogs"
                @error="useAppStore().reportError($event)"
                @retry="repeatScan"
              />
              <span
                v-if="scan.missingCustomRootCount"
                class="text-content-secondary text-muted-foreground"
                role="status"
              >
                {{
                  t(
                    'cleanup.customCleanup.missingDirectoriesSkipped',
                    { count: scan.missingCustomRootCount },
                    scan.missingCustomRootCount
                  )
                }}
              </span>
            </div>
          </template>
        </MdResultSummary>
      </template>

      <MdEmptyState
        v-if="scan.missingCustomRootCount && !scan.rules.length"
        :icon-name="ICON_NAMES.folderPlus"
        :title="t('cleanup.customCleanup.noAvailableDirectories')"
        :description="t('cleanup.customCleanup.restoreDirectoriesHint')"
      >
        <Button type="button" variant="outline" @click="customDialogOpen = true">
          {{ t('cleanup.customCleanup.editRules') }}
        </Button>
      </MdEmptyState>

      <MdCleanupRuleGroups
        v-else
        :embedded="true"
        :busy="busy"
        :leftovers="leftovers"
        :rules="scan.rules"
        :selected-leftover-ids="selectedLeftoverIds"
        :selected-rule-ids="selectedRuleIds"
        :source-selections="sourceSelections"
        :privileged-scan-rule-id="privilegedScanRuleId"
        @toggle-source="toggleSource"
        @toggle-leftover="toggleLeftover"
        @select-leftover-group="setLeftoverGroupSelected"
        @select-all="selectAll"
        @open="emit('open', $event)"
        @privileged-scan="emit('privilegedScan')"
      />
    </MdResultWorkspace>

    <MdResultWorkspace v-else>
      <MdEmptyState
        :icon-name="ICON_NAMES.deepCleanup"
        :title="t('cleanup.scanFirst')"
        :description="t('cleanup.emptyDescription')"
      >
        <MdCleanupScanButton
          :busy="busy"
          @primary="startStandardScan"
          @standard="startStandardScan"
          @select-volumes="openVolumeDialog"
          @custom="customDialogOpen = true"
        />
      </MdEmptyState>
    </MdResultWorkspace>

    <MdCleanupVolumeDialog
      v-model="volumeDialogOpen"
      :disks="selectableDisks"
      :system-disk="disk"
      :initial-mount-points="
        scanScope.mode === CLEANUP_SCAN_SCOPE_MODES.selectedVolumes ? scanScope.volumeMountPoints : []
      "
      @confirm="startSelectedVolumeScan"
    />

    <MdCustomCleanupDialog v-model="customDialogOpen" @scan="handleCustomRules" />

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
      :application-close-identities="scan.applicationCloseIdentities"
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

@container (max-width: 800px) {
  .scan-action {
    width: 100%;
    justify-content: space-between;
  }
}
</style>
