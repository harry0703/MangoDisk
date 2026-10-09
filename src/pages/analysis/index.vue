<script setup lang="ts">
import type { ScanNameExclusion } from '@/lib/models/storage-scan';
import { useI18n } from 'vue-i18n';
import { computed, nextTick, onDeactivated, ref, watch } from 'vue';

import MdDelayedOperationWorkspace from '@/components/custom/md-delayed-operation-workspace.vue';
import MdStorageScopeSelect from '@/components/custom/md-storage-scope-select.vue';
import MdEmptyState from '@/components/custom/md-empty-state.vue';
import MdOperationProgress from '@/components/custom/md-operation-progress.vue';
import MdPageShell from '@/components/custom/md-page-shell.vue';
import MdDestructiveActionDialog from '@/components/custom/md-destructive-action-dialog.vue';
import { ANALYSIS_SCAN_MODES, ANALYSIS_VIEW_IDS } from '@/lib/models/analysis';
import { STORAGE_SCOPE_IDS } from '@/lib/models/storage-scope';
import { ICON_NAMES } from '@/lib/models/ui';
import type { AnalysisResult, AnalysisScanMode, AnalysisViewId, DirectoryEntryInfo } from '@/lib/models/analysis';
import type { DiskInfo } from '@/lib/models/disk';
import type { TraversalProgress } from '@/lib/models/progress';
import * as AnalysisBreadcrumbUtils from '@/lib/utils/analysis-breadcrumb';
import * as DiskUtils from '@/lib/utils/disk';
import { ByteSizeService } from '@/lib/services/byte-size-service';
import { OperatingSystemService } from '@/lib/services/operating-system-service';
import * as PathUtils from '@/lib/utils/path';
import * as StorageScanPreferenceUtils from '@/lib/utils/storage-scan-preference';
import { useStorageScanPreferencesStore } from '@/stores/storage-scan-preferences-store';
import { useStorageScopeStore } from '@/stores/storage-scope-store';
import { useAnalysisStore } from '@/stores/analysis-store';

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
  disk: DiskInfo | null;
  disks: DiskInfo[];
  progress: TraversalProgress | null;
  busy: boolean;
  cancelling: boolean;
  deleting: boolean;
  deletingPath?: string | null;
}>();

const emit = defineEmits<{
  analyze: [path?: string, refresh?: boolean, setHome?: boolean, scanMode?: AnalysisScanMode];
  cancel: [];
  error: [error: unknown];
  openExclusions: [];
  openEntry: [scanId: number, path: string];
  reveal: [path: string];
  delete: [entry: DirectoryEntryInfo];
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
const pendingDelete = ref<DirectoryEntryInfo | null>(null);
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
onDeactivated(() => {
  hoveredEntryPath.value = null;
});

const entries = computed(() => [...(props.result?.entries ?? [])].sort((left, right) => right.bytes - left.bytes));
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
const activeDisk = computed(() =>
  DiskUtils.findForPath(
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
const showPrimaryAnalysisProgress = computed(() => props.busy && primaryAnalysisPending.value);
const folderNavigationPending = computed(() => props.busy && !primaryAnalysisPending.value);
// Starting the native request does not imply a scan: Core can still reuse its
// index for an unvisited child. Only real scan progress replaces stale browser
// data; cache navigation keeps the mounted list and its sorting state. Cache
// projection and transport can exceed the UI delay without scanning anything.
const showFullAnalysisProgress = computed(
  () => props.busy && (primaryAnalysisPending.value || props.progress !== null)
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

function startPrimaryAnalysis(scanMode: AnalysisScanMode = analysisStore.scanMode) {
  primaryAnalysisPending.value = true;
  if (props.busy || props.deleting || !selectedScopePath.value) return;
  pendingHistoryNavigation.value = null;
  emit('analyze', selectedScopePath.value, scopeIsAnalysisRoot.value, !scopeIsAnalysisRoot.value, scanMode);
  // Cache hits and rejected operations may finish without toggling `busy`.
  // Clear the local interaction state after Vue has applied any synchronous
  // Store updates so it cannot leak into a later folder navigation.
  void nextTick(() => {
    if (!props.busy) primaryAnalysisPending.value = false;
  });
}

function activateEntry(entry: DirectoryEntryInfo) {
  if (!props.deleting && entry.isDirectory) analyze(entry.path);
}

function openEntry(entry: DirectoryEntryInfo) {
  if (!props.result || props.busy || props.deleting) return;
  emit('openEntry', props.result.scanId, entry.path);
}

function requestDelete(entry: DirectoryEntryInfo) {
  if (props.busy || props.deleting || !resultMatchesExclusions.value) return;
  pendingDelete.value = entry;
  confirmOpen.value = true;
}

function confirmDelete() {
  if (!pendingDelete.value || props.busy || props.deleting || !resultMatchesExclusions.value) return;
  emit('delete', pendingDelete.value);
  confirmOpen.value = false;
  pendingDelete.value = null;
}

function navigateHistory(index: number) {
  const target = navigationHistory.value[index];
  if (!target || props.busy || props.deleting) return;
  pendingHistoryNavigation.value = { index, target };
  emit('analyze', target);
}
</script>

<template>
  <MdPageShell class="analysis-shell @container/analysis" content-mode="workspace" :title="t('analysis.title')">
    <template #actions>
      <div class="header-actions" :class="{ 'folder-navigation-pending': folderNavigationPending }">
        <MdStorageScopeSelect
          :model-value="selectedScopePath || activeDisk?.mountPoint || ''"
          :disks="disks"
          :recent-folders="storageScopeStore.recentFolders"
          :standard-folders="storageScopeStore.standardFolders"
          :disabled="busy || deleting"
          @error="emit('error', $event)"
          @remove-folder="removeScopeFolder"
          @update:model-value="selectScope"
        />
        <MdAnalysisScanButton
          v-if="result"
          :mode="analysisStore.scanMode"
          :rescan="scopeIsAnalysisRoot"
          :scanning="showPrimaryAnalysisProgress"
          :disabled="busy || deleting || !selectedScopePath"
          @scan="startPrimaryAnalysis"
        />
      </div>
    </template>

    <article class="browser-card">
      <MdAnalysisBrowserToolbar
        v-if="result && !showFullAnalysisProgress"
        :breadcrumbs="breadcrumbs"
        :busy="busy || deleting"
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
      <!-- A cache hit keeps the browser; a real scan replaces stale navigation and totals. -->
      <MdDelayedOperationWorkspace
        :key="showFullAnalysisProgress ? 'full' : 'navigation'"
        class="analysis-overlay"
        :class="{ 'analysis-overlay--full': !result || showFullAnalysisProgress }"
        :active="busy && (!result || showFullAnalysisProgress)"
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
          @cancel="emit('cancel')"
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
            :disabled="busy || deleting || !selectedScopePath"
            large
            @scan="startPrimaryAnalysis"
          />
        </div>
      </MdEmptyState>

      <div
        v-else-if="!showFullAnalysisProgress"
        class="browser-content"
        :class="{ 'browser-content--list-collapsed': listCollapsed }"
        :inert="busy || undefined"
        :aria-busy="busy"
      >
        <MdAnalysisFolderPane
          v-show="!listCollapsed"
          id="analysis-file-list"
          :entries="entries"
          :total-bytes="result.totalBytes"
          :folder-count="folderCount"
          :file-count="fileCount"
          :truncated="result.truncated"
          :open-disabled="busy || deleting"
          :delete-disabled="busy || deleting || !resultMatchesExclusions"
          :deleting-path="deletingPath"
          :hovered-entry-path="hoveredEntryPath"
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
          :open-disabled="busy || deleting"
          :delete-disabled="busy || deleting || !resultMatchesExclusions"
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
        />
      </div>
    </article>

    <MdDestructiveActionDialog
      v-model:open="confirmOpen"
      :title="t('analysis.deleteTitle')"
      :description="t('analysis.deleteDescription')"
      :summary-label="pendingDelete?.name"
      :summary-value="pendingDelete ? ByteSizeService.bytes(pendingDelete.bytes) : ''"
      :cancel-label="t('common.cancel')"
      :confirm-label="t('analysis.deleteAction')"
      :busy="deleting"
      @confirm="confirmDelete"
    />
  </MdPageShell>
</template>

<style scoped>
@reference "@assets/main.css";

.analysis-shell {
  min-height: 0;
  overflow: hidden;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 12px;
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
