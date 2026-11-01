<script setup lang="ts">
import type { ScanNameExclusion } from '@/lib/models/storage/storage-scan';
import { FolderSelectionService } from '@/lib/services/storage/folder-selection-service';
import { useI18n } from 'vue-i18n';
import {
  computed,
  nextTick,
  onActivated,
  onBeforeUnmount,
  onDeactivated,
  onMounted,
  ref,
  shallowRef,
  watch,
} from 'vue';

import MdDelayedOperationWorkspace from '@/components/custom/md-delayed-operation-workspace.vue';
import MdStorageScopeSelect from '@/components/custom/md-storage-scope-select.vue';
import MdEmptyState from '@/components/custom/md-empty-state.vue';
import MdOperationProgress from '@/components/custom/md-operation-progress.vue';
import MdPageShell from '@/components/custom/md-page-shell.vue';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import { Button } from '@/components/ui/button';
import MdIcon from '@/components/icons/md-icon.vue';
import MdDestructiveActionDialog from '@/components/custom/md-destructive-action-dialog.vue';
import { ANALYSIS_SCAN_MODES, ANALYSIS_VIEW_IDS } from '@/lib/models/storage/analysis';
import { STORAGE_SCOPE_IDS } from '@/lib/models/storage/storage-scope';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import type {
  AnalysisResult,
  AnalysisDeleteBatchResult,
  AnalysisScanMode,
  AnalysisViewId,
  DirectoryEntryInfo,
} from '@/lib/models/storage/analysis';
import type { VolumeInfo } from '@/lib/models/storage/volume';
import type { TraversalProgress } from '@/lib/models/presentation/progress';
import * as AnalysisBreadcrumbUtils from '@/lib/utils/storage/analysis-breadcrumb';
import * as VolumeUtils from '@/lib/utils/storage/volume';
import { ByteSizeService } from '@/lib/services/settings/byte-size-service';
import { OperatingSystemService } from '@/lib/services/settings/operating-system-service';
import * as PathUtils from '@/lib/utils/filesystem/path';
import * as StorageScanPreferenceUtils from '@/lib/utils/storage/storage-scan-preference';
import { useStorageScanPreferencesStore } from '@/stores/storage-scan-preferences-store';
import { useStorageScopeStore } from '@/stores/storage-scope-store';
import { useAnalysisStore } from '@/stores/analysis-store';

import { FormatService } from '@/lib/services/presentation/format-service';
import MdAnalysisDeleteResultDialog from './components/md-analysis-delete-result-dialog.vue';
import MdAnalysisExportDialog from './components/md-analysis-export-dialog.vue';
import MdAnalysisCoverage from './components/md-analysis-coverage.vue';
import MdAnalysisScanButton from './components/md-analysis-scan-button.vue';
import MdAnalysisBrowserToolbar from './components/md-analysis-browser-toolbar.vue';
import MdAnalysisFolderPane from './components/md-analysis-folder-pane.vue';
import MdAnalysisVisualPane from './components/md-analysis-visual-pane.vue';

const { t } = useI18n({ useScope: 'global' });

const props = defineProps<{
  result: AnalysisResult | null;
  cachedResults?: Readonly<Record<string, AnalysisResult>>;
  excludedFolders: string[];
  excludedNames?: ScanNameExclusion[];
  homePath: string;
  disk: VolumeInfo | null;
  disks: VolumeInfo[];
  progress: TraversalProgress | null;
  busy: boolean;
  cancelling: boolean;
  deleting: boolean;
  deletingPath?: string | null;
  recovering?: boolean;
  recoveryRequired?: boolean;
  deleteBatchResult?: AnalysisDeleteBatchResult | null;
  administratorRestartBusy?: boolean;
  administratorRestartDisabled?: boolean;
}>();

let scopeRequest = 0;
const resolvingScope = ref(false);
function discardScopeResolution() {
  if (resolvingScope.value) primaryAnalysisPending.value = false;
  scopeRequest += 1;
  resolvingScope.value = false;
}
onBeforeUnmount(discardScopeResolution);
onDeactivated(discardScopeResolution);
function cancelScan() {
  if (resolvingScope.value) discardScopeResolution();
  else emit('cancel');
}
const scanBusy = computed(() => props.busy || resolvingScope.value);
const exportOpen = ref(false);
const exportSnapshot = shallowRef<AnalysisResult | null>(null);
function openExport() {
  if (!props.result || exportOpen.value || scanBusy.value || props.deleting || props.recoveryRequired) return;
  exportSnapshot.value = props.result;
  exportOpen.value = true;
}

watch(
  [() => props.result?.scanId, scanBusy, () => props.deleting, () => props.recoveryRequired],
  ([scanId, busy, deleting, recoveryRequired]) => {
    if (exportOpen.value && (busy || deleting || recoveryRequired || scanId !== exportSnapshot.value?.scanId)) {
      exportOpen.value = false;
    }
  }
);

const emit = defineEmits<{
  analyze: [path?: string, refresh?: boolean, setHome?: boolean, scanMode?: AnalysisScanMode];
  cancel: [];
  error: [error: unknown];
  openExclusions: [];
  openEntry: [scanId: number, path: string];
  reveal: [path: string];
  delete: [entry: DirectoryEntryInfo];
  deleteMany: [entries: DirectoryEntryInfo[]];
  restartAsAdministrator: [scanId: number];
}>();

const storageScopeStore = useStorageScopeStore();
const scanPreferencesStore = useStorageScanPreferencesStore();
const scopeId = STORAGE_SCOPE_IDS.analysis;
const selectedScopePath = ref(
  PathUtils.display(storageScopeStore.selectedPath(scopeId) || props.result?.root || props.disk?.mountPoint || '')
);
const navigationHistory = ref<string[]>([]);
const navigationIndex = ref(-1);
interface PendingHistoryNavigation {
  index: number;
  target: string;
}

const pendingHistoryNavigation = ref<PendingHistoryNavigation | null>(null);
const primaryAnalysisPending = ref(false);
const confirmOpen = ref(false);
const pendingDelete = ref<DirectoryEntryInfo[]>([]);
const pendingDeleteScanId = ref<number | null>(null);
const pendingBatch = ref(false);
const selectedPaths = ref<string[]>([]);
const selectionMode = ref(false);
const deleteResultOpen = ref(false);
const pendingBytes = computed(() => pendingDelete.value.reduce((sum, entry) => sum + entry.bytes, 0));
const analysisStore = useAnalysisStore();
const scanHint = computed(() => {
  const hint = t('loading.cancelHint');
  if (!OperatingSystemService.isWindows()) return hint;
  const mode = t(
    analysisStore.scanMode === ANALYSIS_SCAN_MODES.fast ? 'analysis.scanMode.fast' : 'analysis.scanMode.standard'
  );
  return `${mode} · ${hint}`;
});
const viewMode = computed<AnalysisViewId>({
  get: () => analysisStore.viewPreferences.viewMode,
  set: value => analysisStore.setViewMode(value),
});
const hoveredEntryPath = ref<string | null>(null);
const listCollapsed = ref(false);
const treemapDepth = computed({
  get: () => analysisStore.viewPreferences.treemapDepth,
  set: value => analysisStore.setChartDepth(ANALYSIS_VIEW_IDS.treemap, value),
});
const sunburstDepth = computed({
  get: () => analysisStore.viewPreferences.sunburstDepth,
  set: value => analysisStore.setChartDepth(ANALYSIS_VIEW_IDS.sunburst, value),
});

function hoverEntry(path: string | null) {
  hoveredEntryPath.value = props.busy || props.deleting ? null : path;
}

// A hover belongs to one visible result, never to a later scan or view.
watch([() => props.result, () => props.busy, () => props.deleting, viewMode, listCollapsed], () => {
  hoveredEntryPath.value = null;
});
onMounted(() => window.addEventListener('keydown', handleBodyEscape));
onActivated(() => window.addEventListener('keydown', handleBodyEscape));
onBeforeUnmount(() => window.removeEventListener('keydown', handleBodyEscape));
onDeactivated(() => {
  exportOpen.value = false;
  hoveredEntryPath.value = null;
  window.removeEventListener('keydown', handleBodyEscape);
});

const entries = computed(() => [...(props.result?.entries ?? [])].sort((left, right) => right.bytes - left.bytes));
const selectedEntries = computed(() => {
  const paths = new Set(selectedPaths.value);
  return entries.value.filter(entry => paths.has(entry.path) && !entry.deletionProtection);
});
function setSelectionMode(enabled: boolean) {
  if (props.deleting) return;
  if (enabled && (props.busy || props.recoveryRequired || !resultMatchesExclusions.value)) return;
  selectionMode.value = enabled;
  if (!enabled) selectedPaths.value = [];
}
// WebKit may keep mouse actions focused on body, outside the page's bubbling path.
function handleBodyEscape(event: KeyboardEvent) {
  if (event.key !== 'Escape' || event.defaultPrevented) return;
  if (event.target === document.body) exitSelection(event);
}
function exitSelection(event: KeyboardEvent) {
  if (!selectionMode.value || event.defaultPrevented || confirmOpen.value || deleteResultOpen.value || props.deleting)
    return;
  event.preventDefault();
  event.stopPropagation();
  setSelectionMode(false);
}
function updateSelection(paths: string[]) {
  if (!selectionMode.value) return;
  if (props.busy || props.deleting || props.recoveryRequired || !resultMatchesExclusions.value) return;
  const allowed = new Set(entries.value.filter(entry => !entry.deletionProtection).map(entry => entry.path));
  selectedPaths.value = [...new Set(paths)].filter(path => allowed.has(path));
}
const folderCount = computed(() => entries.value.filter(entry => entry.isDirectory).length);
const fileCount = computed(() => entries.value.reduce((total, entry) => total + entry.fileCount, 0));
const hasRelevantExclusions = computed(() => {
  if (!props.result) return false;
  const rootKey = PathUtils.comparisonKey(props.result.root);
  return (
    !!props.excludedNames?.length ||
    props.excludedFolders.some(folder => {
      const folderKey = PathUtils.comparisonKey(folder);
      return PathUtils.isSameOrChildKey(folderKey, rootKey) || PathUtils.isSameOrChildKey(rootKey, folderKey);
    })
  );
});
const resultMatchesExclusions = computed(
  () =>
    StorageScanPreferenceUtils.sameExcludedFolders(
      props.excludedFolders,
      scanPreferencesStore.pathsForScope('analysis')
    ) &&
    StorageScanPreferenceUtils.sameExcludedNames(
      props.excludedNames ?? [],
      scanPreferencesStore.namesForScope('analysis')
    )
);
watch(
  [
    () => props.result?.scanId,
    () => props.result?.root,
    () => props.busy,
    () => props.recoveryRequired,
    resultMatchesExclusions,
  ],
  () => {
    selectedPaths.value = [];
    selectionMode.value = false;
    if (!props.deleting) {
      confirmOpen.value = false;
      pendingDelete.value = [];
    }
  }
);
watch(
  () => props.deleteBatchResult,
  result => {
    if (!result) return;
    const removed = new Set(result.removedPaths);
    selectedPaths.value = selectedPaths.value.filter(path => !removed.has(path));
    deleteResultOpen.value = result.failed.length > 0 || result.notAttemptedPaths.length > 0;
  }
);
const activeDisk = computed(() =>
  VolumeUtils.findForPath(
    props.disks,
    props.result?.root || selectedScopePath.value || props.homePath || props.disk?.mountPoint || '',
    props.disk
  )
);
// The primary action belongs to the selected analysis scope, while
// `result.root` follows whichever child folder the user is browsing. Compare
// against the stable analysis origin so folder navigation cannot incorrectly
// turn "Analyze Again" back into "Start Analysis".
const scopeIsAnalysisRoot = computed(
  () =>
    Boolean(props.homePath && selectedScopePath.value) &&
    PathUtils.comparisonKey(props.homePath) === PathUtils.comparisonKey(selectedScopePath.value)
);
const breadcrumbs = computed(() =>
  AnalysisBreadcrumbUtils.withSiblingFolders(
    AnalysisBreadcrumbUtils.create(
      props.result?.root ?? selectedScopePath.value,
      activeDisk.value,
      t('analysis.localDisk')
    ),
    [...(props.result ? [props.result] : []), ...Object.values(props.cachedResults ?? {})]
  )
);
const canGoBack = computed(() => navigationIndex.value > 0);
const canGoForward = computed(
  () => navigationIndex.value >= 0 && navigationIndex.value < navigationHistory.value.length - 1
);
// Folder navigation and an explicit rescan share the same backend operation.
// Track the initiating interaction so ordinary browsing does not replace the
// page-level action label and make the right-aligned header change width.
const showPrimaryAnalysisProgress = computed(() => scanBusy.value && primaryAnalysisPending.value);
const folderNavigationPending = computed(() => props.busy && !primaryAnalysisPending.value);
// Starting the native request does not imply a scan: Core can still reuse its
// index for an unvisited child. Only real scan progress replaces stale browser
// data; cache navigation keeps the mounted list and its sorting state. Cache
// projection and transport can exceed the UI delay without scanning anything.
const showFullAnalysisProgress = computed(
  () => scanBusy.value && !props.recovering && (primaryAnalysisPending.value || props.progress !== null)
);

watch(
  () => props.disk?.mountPoint,
  value => {
    if (!value || selectedScopePath.value) return;
    selectedScopePath.value = PathUtils.display(value);
  },
  { immediate: true }
);

watch(
  () => props.result?.root,
  value => {
    if (!value) return;
    const normalized = PathUtils.display(value);
    if (pendingHistoryNavigation.value) {
      const pending = pendingHistoryNavigation.value;
      pendingHistoryNavigation.value = null;
      // Advance history only when the result still belongs to this navigation.
      if (PathUtils.comparisonKey(pending.target) === PathUtils.comparisonKey(normalized)) {
        navigationIndex.value = pending.index;
        return;
      }
    }
    if (navigationHistory.value[navigationIndex.value] === normalized) return;
    navigationHistory.value = [...navigationHistory.value.slice(0, navigationIndex.value + 1), normalized];
    navigationIndex.value = navigationHistory.value.length - 1;
  },
  { immediate: true }
);

watch(
  () => props.busy,
  busy => {
    if (busy) return;
    primaryAnalysisPending.value = false;
    // Let the result watcher settle before discarding a failed request so one
    // Store update cannot create a duplicate history entry.
    void nextTick(() => {
      if (!props.busy) pendingHistoryNavigation.value = null;
    });
  }
);

function analyze(path?: string, refresh = false, setHome = false) {
  const target = path?.trim() || selectedScopePath.value;
  if (!target || props.busy || props.deleting) return;
  pendingHistoryNavigation.value = null;
  emit('analyze', target, refresh, setHome);
}

function selectScope(value: unknown) {
  if (props.busy || props.deleting) return;
  const target = typeof value === 'string' ? value : '';
  if (!target) return;

  // Selection changes only configure the next explicit analysis.
  selectedScopePath.value = PathUtils.display(target);
  storageScopeStore.select(scopeId, selectedScopePath.value, props.disks);
  pendingHistoryNavigation.value = null;
}

function removeScopeFolder(path: string) {
  const removingCurrent = PathUtils.comparisonKey(path) === PathUtils.comparisonKey(selectedScopePath.value);
  storageScopeStore.removeFolder(path);
  if (!removingCurrent) return;

  const fallback = PathUtils.display(props.disk?.mountPoint || props.disks[0]?.mountPoint || '');
  selectedScopePath.value = fallback;
  if (fallback) storageScopeStore.select(scopeId, fallback, props.disks);
  pendingHistoryNavigation.value = null;
}

async function startPrimaryAnalysis(scanMode: AnalysisScanMode = analysisStore.scanMode) {
  if (scanBusy.value || props.deleting || !selectedScopePath.value) return;
  const requested = selectedScopePath.value;
  primaryAnalysisPending.value = true;
  const request = ++scopeRequest;
  resolvingScope.value = true;
  try {
    const [target] = await FolderSelectionService.filterExistingDirectories(
      [requested],
      () => request !== scopeRequest
    );
    if (
      request !== scopeRequest ||
      !target ||
      PathUtils.comparisonKey(requested) !== PathUtils.comparisonKey(selectedScopePath.value)
    )
      return;
    selectScope(target);
    pendingHistoryNavigation.value = null;
    emit('analyze', target, scopeIsAnalysisRoot.value, !scopeIsAnalysisRoot.value, scanMode);
  } catch (error) {
    if (request === scopeRequest) emit('error', error);
  } finally {
    await nextTick();
    if (request === scopeRequest) resolvingScope.value = false;
    if (request === scopeRequest && !props.busy) primaryAnalysisPending.value = false;
  }
}

function activateEntry(entry: DirectoryEntryInfo) {
  if (!props.deleting && entry.isDirectory) analyze(entry.path);
}

function openEntry(entry: DirectoryEntryInfo) {
  if (!props.result || props.busy || props.deleting) return;
  emit('openEntry', props.result.scanId, entry.path);
}

function requestDelete(entry: DirectoryEntryInfo) {
  requestDeletion([entry], false);
}
function requestDeletion(targets: DirectoryEntryInfo[], batch: boolean) {
  if (batch && !selectionMode.value) return;
  if (props.busy || props.deleting || props.recoveryRequired || !props.result || !resultMatchesExclusions.value) return;
  const paths = new Set(targets.map(entry => entry.path));
  const current = entries.value.filter(entry => paths.has(entry.path) && !entry.deletionProtection);
  if (!current.length || current.length !== paths.size) return;
  pendingDelete.value = current;
  pendingDeleteScanId.value = props.result.scanId;
  pendingBatch.value = batch;
  confirmOpen.value = true;
}
function confirmDelete() {
  if (
    !pendingDelete.value.length ||
    pendingDeleteScanId.value !== props.result?.scanId ||
    props.busy ||
    props.deleting ||
    props.recoveryRequired ||
    !resultMatchesExclusions.value
  )
    return;
  const paths = new Set(pendingDelete.value.map(entry => entry.path));
  const current = entries.value.filter(entry => paths.has(entry.path) && !entry.deletionProtection);
  if (current.length !== paths.size) return;
  if (pendingBatch.value) emit('deleteMany', current);
  else emit('delete', current[0]!);
  confirmOpen.value = false;
  pendingDelete.value = [];
}

function navigateHistory(index: number) {
  const target = navigationHistory.value[index];
  if (!target || props.busy || props.deleting) return;
  pendingHistoryNavigation.value = { index, target };
  emit('analyze', target);
}
</script>

<template>
  <MdPageShell
    class="analysis-shell @container/analysis"
    content-mode="workspace"
    :title="t('analysis.title')"
    @keydown.esc="exitSelection"
  >
    <template #actions>
      <div class="header-actions" :class="{ 'folder-navigation-pending': folderNavigationPending }">
        <MdStorageScopeSelect
          :model-value="selectedScopePath || activeDisk?.mountPoint || ''"
          :disks="disks"
          :recent-folders="storageScopeStore.recentFolders"
          :standard-folders="storageScopeStore.standardFolders"
          :disabled="scanBusy || deleting"
          @error="emit('error', $event)"
          @remove-folder="removeScopeFolder"
          @update:model-value="selectScope"
        />
        <MdAnalysisScanButton
          v-if="result"
          class="analysis-scan-action"
          :mode="analysisStore.scanMode"
          :rescan="scopeIsAnalysisRoot"
          :scanning="showPrimaryAnalysisProgress"
          :disabled="scanBusy || deleting || !selectedScopePath"
          @scan="startPrimaryAnalysis"
        />
        <MdTooltip
          v-if="result"
          :text="
            scanBusy
              ? t('analysis.export.waitForScan')
              : recoveryRequired
                ? t('analysis.export.unavailable')
                : t('analysis.export.action')
          "
        >
          <span
            ><Button
              variant="outline"
              class="analysis-export-button"
              :aria-label="t('analysis.export.action')"
              :disabled="scanBusy || deleting || recoveryRequired"
              @click="openExport"
              ><MdIcon :name="ICON_NAMES.download" :size="17" /><span class="analysis-export-label">{{
                t('analysis.export.action')
              }}</span></Button
            ></span
          >
        </MdTooltip>
      </div>
    </template>

    <article class="browser-card">
      <MdAnalysisBrowserToolbar
        v-if="result && !showFullAnalysisProgress"
        :breadcrumbs="breadcrumbs"
        :busy="scanBusy || deleting"
        :preserve-busy-appearance="folderNavigationPending"
        :can-go-back="canGoBack"
        :can-go-forward="canGoForward"
        :home-disabled="!homePath"
        :list-collapsed="listCollapsed"
        @toggle-list="listCollapsed = !listCollapsed"
        @back="navigateHistory(navigationIndex - 1)"
        @forward="navigateHistory(navigationIndex + 1)"
        @home="analyze(homePath)"
        @navigate="analyze"
      />
      <div v-if="recoveryRequired" class="recovery-status" role="status" aria-live="polite">
        <span>{{ t(recovering ? 'analysis.recovering' : 'analysis.recoveryRequired') }}</span>
        <button v-if="recovering" type="button" :disabled="cancelling" @click="emit('cancel')">
          {{ t(cancelling ? 'loading.cancelling' : 'common.cancel') }}
        </button>
      </div>
      <!-- A cache hit keeps the browser; a real scan replaces stale navigation and totals. -->
      <MdDelayedOperationWorkspace
        :key="showFullAnalysisProgress ? 'full' : 'navigation'"
        class="analysis-overlay"
        :class="{ 'analysis-overlay--full': !result || showFullAnalysisProgress }"
        :active="scanBusy && !recovering && (!result || showFullAnalysisProgress)"
        :delay="showFullAnalysisProgress ? 0 : undefined"
        mode="overlay"
        role="status"
        aria-live="polite"
      >
        <MdOperationProgress
          :icon-name="ICON_NAMES.analysis"
          :title="cancelling ? t('loading.cancelling') : t('analysis.analyzing')"
          :progress="progress"
          :path-label="t('loading.currentAnalysisDirectory')"
          :preparing-text="t('loading.preparingAnalysisDirectory')"
          :hint="scanHint"
          :cancelable="true"
          :cancel-disabled="cancelling"
          @cancel="cancelScan"
        />
      </MdDelayedOperationWorkspace>

      <MdEmptyState
        v-if="!result"
        :icon-name="ICON_NAMES.analysis"
        :title="t('analysis.emptyTitle')"
        :description="t('analysis.emptyDescription')"
      >
        <div class="empty-primary-actions">
          <MdAnalysisScanButton
            :mode="analysisStore.scanMode"
            :disabled="scanBusy || deleting || !selectedScopePath"
            large
            @scan="startPrimaryAnalysis"
          />
        </div>
      </MdEmptyState>

      <div
        v-else-if="!showFullAnalysisProgress"
        class="browser-content"
        :class="{ 'browser-content--list-collapsed': listCollapsed }"
        :inert="(scanBusy && !recovering) || undefined"
        :aria-busy="scanBusy"
      >
        <MdAnalysisFolderPane
          v-show="!listCollapsed"
          id="analysis-file-list"
          :entries="entries"
          :selected-paths="selectedPaths"
          :selection-mode="selectionMode"
          :deleting="deleting"
          :total-bytes="result.totalBytes"
          :folder-count="folderCount"
          :file-count="fileCount"
          :truncated="result.truncated"
          :open-disabled="scanBusy || deleting || recoveryRequired"
          :delete-disabled="scanBusy || deleting || recoveryRequired || !resultMatchesExclusions"
          :deleting-path="deletingPath"
          :hovered-entry-path="hoveredEntryPath"
          @update:selected-paths="updateSelection"
          @update:selection-mode="setSelectionMode"
          @delete-selected="requestDeletion(selectedEntries, true)"
          @hover-entry="hoverEntry"
          @activate="activateEntry"
          @open-entry="openEntry"
          @reveal="emit('reveal', $event)"
          @delete="requestDelete"
        />
        <MdAnalysisVisualPane
          v-model:treemap-depth="treemapDepth"
          v-model:sunburst-depth="sunburstDepth"
          :result="result"
          :exclusions-active="hasRelevantExclusions"
          :entries="entries"
          :folder-count="folderCount"
          :view-mode="viewMode"
          :open-disabled="scanBusy || deleting || recoveryRequired"
          :delete-disabled="scanBusy || deleting || recoveryRequired || !resultMatchesExclusions"
          :deleting-path="deletingPath"
          :hovered-entry-path="hoveredEntryPath"
          @hover-entry="hoverEntry"
          @update:view-mode="viewMode = $event"
          @navigate="analyze"
          @refresh-directory="analyze($event, true)"
          @open-exclusions="emit('openExclusions')"
          @activate="activateEntry"
          @open-entry="openEntry"
          @reveal="emit('reveal', $event)"
          @delete="requestDelete"
        >
          <template #coverage>
            <MdAnalysisCoverage
              :result="result"
              :disabled="scanBusy || deleting || recoveryRequired || administratorRestartDisabled"
              :restarting="administratorRestartBusy"
              @restart-as-administrator="emit('restartAsAdministrator', $event)"
            />
          </template>
        </MdAnalysisVisualPane>
      </div>
    </article>

    <MdDestructiveActionDialog
      v-model:open="confirmOpen"
      :title="pendingBatch ? t('analysis.batchDeleteTitle') : t('analysis.deleteTitle')"
      :description="pendingBatch ? t('analysis.batchDeleteDescription') : t('analysis.deleteDescription')"
      :summary-label="
        pendingBatch
          ? t('analysis.selectedCount', { count: FormatService.integer(pendingDelete.length) })
          : pendingDelete[0]?.name
      "
      :summary-value="ByteSizeService.bytes(pendingBytes)"
      :summary-value-hint="t('analysis.deleteSizeHint')"
      :cancel-label="t('common.cancel')"
      :confirm-label="t('analysis.deleteAction')"
      :busy="deleting"
      @confirm="confirmDelete"
    >
      <template v-if="pendingBatch" #details>
        <ul
          class="max-h-48 overflow-y-auto scrollbar-stable"
          :aria-label="t('analysis.selectedCount', { count: FormatService.integer(pendingDelete.length) })"
        >
          <li
            v-for="entry in pendingDelete"
            :key="entry.path"
            class="flex min-w-0 items-center justify-between gap-3 py-1.5"
          >
            <MdTooltip :text="`${entry.name}\n${entry.path}`" multiline :active="confirmOpen">
              <span
                tabindex="0"
                class="min-w-0 truncate rounded-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >{{ entry.name }}</span
              >
            </MdTooltip>
            <span class="shrink-0 text-muted-foreground">{{ ByteSizeService.bytes(entry.bytes) }}</span>
          </li>
        </ul>
      </template>
    </MdDestructiveActionDialog>
    <MdAnalysisDeleteResultDialog v-model:open="deleteResultOpen" :result="deleteBatchResult ?? null" />
    <MdAnalysisExportDialog
      v-model:open="exportOpen"
      :result="exportSnapshot"
      @rescan="emit('analyze', $event, true, true)"
    />
  </MdPageShell>
</template>

<style scoped>
@reference "@assets/main.css";

.recovery-status {
  display: flex;
  flex: none;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 8px 16px;
  color: var(--color-muted-foreground);
  font-size: 12px;
}

.recovery-status button {
  color: var(--color-primary-text);
  cursor: pointer;
}

.analysis-shell {
  min-height: 0;
  overflow: hidden;
}

.header-actions {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 12px;
}

/* Keep a readable title while long translations yield space before desktop controls. */
.analysis-shell :deep(.md-page-header) {
  grid-template-columns: minmax(8rem, 1fr) minmax(0, auto);
}
.header-actions :deep(.analysis-scan-action),
.header-actions :deep(.md-split-action__primary),
.header-actions :deep(.labels) {
  min-width: 0;
  flex-shrink: 1;
}
.header-actions :deep(.labels > span) {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

@container analysis (max-width: 1100px) {
  .header-actions {
    gap: 8px;
  }
  .header-actions :deep(.scope-select) {
    width: 9rem;
  }
  .analysis-export-button {
    width: 40px;
    padding-inline: 8px;
  }
  .analysis-export-label {
    display: none;
  }
}

/*
 * Native disabled controls still block concurrent analysis. During folder
 * navigation, keep their resting appearance so a short request cannot flash
 * the entire page header between full and half opacity.
 */
.header-actions.folder-navigation-pending :deep(button:disabled) {
  opacity: 1;
  transition-duration: 0s;
}

.browser-card {
  position: relative;
  display: flex;
  min-height: 0;
  flex: 1;
  flex-direction: column;
  overflow: hidden;
  overflow-anchor: none;
  border-width: 1px;
  border-radius: var(--radius);
  @apply border-border/70 bg-workspace text-foreground;
}

.analysis-overlay {
  --operation-workspace-overlay-top: calc(var(--layout-workspace-toolbar-height) + 36px + 2px);
}

.analysis-overlay--full {
  --operation-workspace-overlay-top: 0;
}

.empty-primary-actions {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
}

.browser-content {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  min-height: 0;
  flex: 1;
  overflow: hidden;
  overflow-anchor: none;
  contain: layout paint;
}

/*
 * Keep the mounted rank pane and chart state when the user expands the chart.
 */
@container analysis (min-width: 672px) {
  .browser-content:not(.browser-content--list-collapsed) {
    grid-template-columns: minmax(300px, 38%) minmax(0, 1fr);
  }
}

@container analysis (max-width: 671px) {
  /* Keep one primary view on small windows instead of splitting scarce height. */
  .browser-content :deep(.folder-pane) {
    display: none;
  }
}

@container analysis (min-width: 1024px) {
  .browser-content:not(.browser-content--list-collapsed) {
    grid-template-columns: minmax(330px, 32%) minmax(0, 1fr);
  }
}

/* Monterey's WKWebView has no container queries. At MangoDisk's supported
 * minimum desktop width, the available page area matches the two-pane mode.
 */
@supports not (container-type: inline-size) {
  @media (min-width: 900px) {
    .browser-content:not(.browser-content--list-collapsed) {
      grid-template-columns: minmax(300px, 38%) minmax(0, 1fr);
    }
  }
}
</style>
