<script setup lang="ts">
import { FormatService } from '@/lib/services/presentation/format-service';
import { FolderSelectionService } from '@/lib/services/storage/folder-selection-service';
import { useI18n } from 'vue-i18n';
import MdAiWorkspace from '@/layouts/components/md-ai-workspace.vue';
import { useAiStore } from '@/stores/ai-store';
import { duplicateFileAiContext } from './duplicate-file-ai-context';
import { computed, nextTick, onBeforeUnmount, onDeactivated, onMounted, ref, watch } from 'vue';

import MdDelayedOperationWorkspace from '@/components/custom/md-delayed-operation-workspace.vue';
import MdStorageScopeSelect from '@/components/custom/md-storage-scope-select.vue';
import MdEmptyState from '@/components/custom/md-empty-state.vue';
import MdFileCategoryFilter from '@/components/custom/md-file-category-filter.vue';
import MdOperationProgress from '@/components/custom/md-operation-progress.vue';
import MdPageShell from '@/components/custom/md-page-shell.vue';
import MdResultFilterToolbar from '@/components/custom/md-result-filter-toolbar.vue';
import MdResultSummary from '@/components/custom/md-result-summary.vue';
import MdScanExclusionLink from '@/components/custom/md-scan-exclusion-link.vue';
import MdResultWorkspace from '@/components/custom/md-result-workspace.vue';
import MdSelectionActionBar from '@/components/custom/md-selection-action-bar.vue';
import MdDestructiveActionDialog from '@/components/custom/md-destructive-action-dialog.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { Button } from '@/components/ui/button';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { FILE_CATEGORY_FILTER_ORDER, FILE_CATEGORY_IDS } from '@/lib/models/filesystem/file-category';
import {
  DUPLICATE_ENTRY_DELETE_POLICIES,
  DUPLICATE_FILE_MINIMUM_PRESETS,
  DUPLICATE_KEEPER_RULE_IDS,
  DUPLICATE_SCAN_LOCATION_MODES,
} from '@/lib/models/storage/duplicate-file';
import { STORAGE_SCOPE_IDS } from '@/lib/models/storage/storage-scope';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import type {
  DuplicateFileEntry,
  DuplicateGroup,
  DuplicateFilesResult,
  DuplicateKeeperRuleId,
  DuplicateScanLocation,
} from '@/lib/models/storage/duplicate-file';
import type { VolumeInfo } from '@/lib/models/storage/volume';
import type { TraversalProgress } from '@/lib/models/presentation/progress';
import type { FileCategoryId } from '@/lib/models/filesystem/file-category';
import * as DuplicateFileSelectionUtils from '@/lib/utils/storage/duplicate-file-selection';
import * as DuplicateFileGroupUtils from '@/lib/utils/storage/duplicate-file-group';
import { OperatingSystemService } from '@/lib/services/settings/operating-system-service';
import { ByteSizeService } from '@/lib/services/settings/byte-size-service';
import * as PathUtils from '@/lib/utils/filesystem/path';
import * as StorageScanPreferenceUtils from '@/lib/utils/storage/storage-scan-preference';
import { useDuplicateFilesStore } from '@/stores/duplicate-files-store';
import { useStorageScanPreferencesStore } from '@/stores/storage-scan-preferences-store';
import { useStorageScopeStore } from '@/stores/storage-scope-store';

import MdDuplicateFileGroups from './components/md-duplicate-file-groups.vue';
import MdDuplicateSmartSelectButton from './components/md-duplicate-smart-select-button.vue';
import { duplicateProgressBytesLabelKey } from './duplicate-file-progress-presentation';

const { locale, t } = useI18n({ useScope: 'global' });

const props = defineProps<{
  disk: VolumeInfo | null;
  disks: VolumeInfo[];
  result: DuplicateFilesResult | null;
  resultComplete: boolean;
  hasMore: boolean;
  loadingMore: boolean;
  progress: TraversalProgress | null;
  busy: boolean;
  cancelling: boolean;
  deleting: boolean;
  minimumBytes: number;
  keeperRule: DuplicateKeeperRuleId;
}>();

let scopeRequest = 0;
const resolvingScope = ref(false);
function discardScopeResolution() {
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

const emit = defineEmits<{
  error: [error: unknown];
  openExclusions: [];
  find: [locations: DuplicateScanLocation[]];
  cancel: [];
  openEntry: [scanId: number, path: string];
  reveal: [path: string];
  delete: [entries: DuplicateFileEntry[]];
  loadMore: [category: FileCategoryId, loadAll?: boolean];
  updateMinimum: [minimumBytes: number];
  updateKeeperRule: [keeperRule: DuplicateKeeperRuleId];
}>();

const aiStore = useAiStore();
const storageScopeStore = useStorageScopeStore();
const duplicateFilesStore = useDuplicateFilesStore();
const storageScanPreferencesStore = useStorageScanPreferencesStore();
const scopeId = STORAGE_SCOPE_IDS.duplicateFiles;
const minimumOptions = ByteSizeService.presetOptions(DUPLICATE_FILE_MINIMUM_PRESETS);
const savedScope = storageScopeStore.selectedPaths[scopeId];
const selectedScopePaths = ref<string[]>(
  Array.isArray(savedScope)
    ? [...savedScope]
    : savedScope
      ? [savedScope]
      : props.result?.roots
        ? [...props.result.roots]
        : props.disk
          ? [props.disk.mountPoint]
          : []
);
const selectedScopeKeys = new Set(selectedScopePaths.value.map(PathUtils.comparisonKey));
const selectedProtectedPaths = ref<string[]>(
  [...(storageScopeStore.duplicateFileProtectedPaths ?? props.result?.protectedRoots ?? [])].filter(path =>
    selectedScopeKeys.has(PathUtils.comparisonKey(path))
  )
);
const activeCategory = ref<FileCategoryId>(FILE_CATEGORY_IDS.all);
const selectedPaths = ref<string[]>([]);
const confirmOpen = ref(false);
const pendingDeleteEntries = ref<DuplicateFileEntry[]>([]);
const deleteRequested = ref(false);
const pendingSmartSelectionRule = ref<DuplicateKeeperRuleId | null>(null);
const smartSelecting = computed(() => pendingSmartSelectionRule.value !== null);

const groups = computed(() => props.result?.groups ?? []);
const categoryOptions = computed(() => {
  const counts = Object.fromEntries(FILE_CATEGORY_FILTER_ORDER.map(category => [category, 0])) as Record<
    FileCategoryId,
    number
  >;
  counts[FILE_CATEGORY_IDS.all] = groups.value.length;
  for (const group of groups.value) counts[DuplicateFileGroupUtils.category(group)] += 1;
  // Keep the filter structure stable even when a category has no matches.
  // This matches the large-file page and prevents controls such as Video from
  // appearing or disappearing as streamed duplicate groups arrive.
  return FILE_CATEGORY_FILTER_ORDER.map(value => ({
    value,
    label: t(`fileCategories.${value}`),
    count: counts[value],
  }));
});
const filteredGroups = computed(() => {
  if (activeCategory.value === FILE_CATEGORY_IDS.all) return groups.value;
  return groups.value.filter(group => DuplicateFileGroupUtils.category(group) === activeCategory.value);
});
const selectedEntries = computed(() => DuplicateFileSelectionUtils.selectedEntries(groups.value, selectedPaths.value));
const selectedBytes = computed(() => DuplicateFileGroupUtils.totalAllocatedBytes(selectedEntries.value));
const pendingDeleteBytes = computed(() => DuplicateFileGroupUtils.totalAllocatedBytes(pendingDeleteEntries.value));
const pendingSummaryLabel = computed(() => {
  if (pendingDeleteEntries.value.length === 1) return pendingDeleteEntries.value[0]?.name ?? '';
  return t(
    'duplicateFiles.copyCount',
    { count: FormatService.integer(pendingDeleteEntries.value.length) },
    pendingDeleteEntries.value.length
  );
});
const canStart = computed(() => selectedScopePaths.value.length > 0);
const minimumLabel = computed(
  () =>
    minimumOptions.find(option => option.bytes === props.minimumBytes)?.label ??
    ByteSizeService.bytes(props.minimumBytes)
);
const resultMatchesScope = computed(() =>
  Boolean(
    props.result &&
    PathUtils.sameRootScope(props.result.roots, selectedScopePaths.value) &&
    PathUtils.sameRootScope(props.result.protectedRoots, selectedProtectedPaths.value) &&
    StorageScanPreferenceUtils.sameExcludedFolders(
      storageScanPreferencesStore.pathsForScope('duplicateFiles'),
      duplicateFilesStore.resultExcludedFolders
    ) &&
    StorageScanPreferenceUtils.sameExcludedNames(
      storageScanPreferencesStore.namesForScope('duplicateFiles'),
      duplicateFilesStore.resultExcludedNames
    )
  )
);
const resultHasRelevantExclusions = computed(
  () =>
    duplicateFilesStore.resultExcludedNames.length > 0 ||
    StorageScanPreferenceUtils.hasExcludedFolderInScanRoots(
      props.result?.roots ?? [],
      duplicateFilesStore.resultExcludedFolders
    )
);
const progressTitle = computed(() => {
  if (props.cancelling) return t('loading.cancelling');
  if (props.progress?.currentStage === 'validatingFiles') return t('duplicateFiles.validatingCandidates');
  if (props.progress?.currentStage === 'hashingFiles') return t('duplicateFiles.comparingContents');
  return t('duplicateFiles.scanning');
});
const progressBytesLabel = computed(() => t(duplicateProgressBytesLabelKey(props.progress?.currentStage)));
const summaryMetricLabel = computed(() =>
  t(props.resultComplete ? 'duplicateFiles.summaryReclaimable' : 'duplicateFiles.summaryReclaimableScanning')
);

onMounted(() => {
  void storageScanPreferencesStore.initialize().catch(error => emit('error', error));
});

function clearPendingResultActions() {
  pendingSmartSelectionRule.value = null;
  selectedPaths.value = [];
  pendingDeleteEntries.value = [];
  deleteRequested.value = false;
  confirmOpen.value = false;
}

watch(resultMatchesScope, matches => {
  if (!matches) clearPendingResultActions();
});

watch(groups, nextGroups => {
  // Retain only cleanable selections that still exist in the current result.
  // A rescan can change a path's policy after the user changes protected roots.
  const cleanable = new Set(
    nextGroups.flatMap(group =>
      group.entries
        .filter(entry => entry.deletePolicy === DUPLICATE_ENTRY_DELETE_POLICIES.cleanable)
        .map(entry => entry.path)
    )
  );
  selectedPaths.value = selectedPaths.value.filter(path => cleanable.has(path));
});

watch(
  () => [props.hasMore, props.loadingMore, props.resultComplete, groups.value.length] as const,
  ([hasMore, loadingMore, resultComplete]) => {
    const rule = pendingSmartSelectionRule.value;
    if (!rule || loadingMore) return;
    if (!resultComplete) {
      pendingSmartSelectionRule.value = null;
      return;
    }
    if (hasMore) {
      emit('loadMore', FILE_CATEGORY_IDS.all, true);
      return;
    }
    selectedPaths.value = DuplicateFileSelectionUtils.suggestedPaths(groups.value, rule);
    pendingSmartSelectionRule.value = null;
  },
  { flush: 'post' }
);

watch(
  () => props.result?.scanId,
  () => {
    pendingSmartSelectionRule.value = null;
  }
);

watch(
  () => props.disk?.mountPoint,
  mountPoint => {
    if (mountPoint && storageScopeStore.selectedPaths[scopeId] === undefined && !selectedScopePaths.value.length) {
      selectedScopePaths.value = [PathUtils.display(mountPoint)];
    }
  }
);

watch(
  () => props.deleting,
  (deleting, wasDeleting) => {
    if (!deleteRequested.value || deleting || !wasDeleting) return;
    deleteRequested.value = false;
    confirmOpen.value = false;
    pendingDeleteEntries.value = [];
  }
);

function explainFile(group: DuplicateGroup, entry: DuplicateFileEntry) {
  if (props.busy || props.deleting || !props.resultComplete || !resultMatchesScope.value) return;
  const context = duplicateFileAiContext(
    group,
    entry,
    OperatingSystemService.isWindows() ? 'windows' : OperatingSystemService.isLinux() ? 'linux' : 'macos'
  );
  if (context) void aiStore.show(context, locale.value);
}

watch(
  () => [props.result?.scanId, props.busy, props.deleting, props.resultComplete, resultMatchesScope.value] as const,
  () => aiStore.dismissModule('duplicateFiles')
);

async function start() {
  if (scanBusy.value || props.deleting || !canStart.value) return;
  const requested = [...selectedScopePaths.value];
  const protectedPaths = [...selectedProtectedPaths.value];
  const protectedKeys = new Set(protectedPaths.map(PathUtils.comparisonKey));
  const request = ++scopeRequest;
  resolvingScope.value = true;
  try {
    const directories = await FolderSelectionService.resolveSelectedDirectories(
      requested,
      () => request !== scopeRequest
    );
    if (
      request !== scopeRequest ||
      !PathUtils.sameSelectedPaths(requested, selectedScopePaths.value) ||
      !PathUtils.sameSelectedPaths(protectedPaths, selectedProtectedPaths.value)
    )
      return;
    const protectedTargets = PathUtils.uniquePaths(
      directories
        .filter(entry => protectedKeys.has(PathUtils.comparisonKey(entry.requestedPath)))
        .map(entry => entry.path)
    );
    selectedScopePaths.value = PathUtils.uniquePaths(directories.map(entry => entry.path));
    selectedProtectedPaths.value = protectedTargets;
    // Persist aliases and their protection together so an interrupted save cannot
    // restore a cleanable target that the user explicitly marked as protected.
    storageScopeStore.selectPaths(scopeId, selectedScopePaths.value, props.disks, protectedTargets);
    clearPendingResultActions();
    const protectedTargetKeys = new Set(protectedTargets.map(PathUtils.comparisonKey));
    emit(
      'find',
      selectedScopePaths.value.map(path => ({
        path,
        mode: protectedTargetKeys.has(PathUtils.comparisonKey(path))
          ? DUPLICATE_SCAN_LOCATION_MODES.protected
          : DUPLICATE_SCAN_LOCATION_MODES.cleanable,
      }))
    );
  } catch (error) {
    if (request === scopeRequest) emit('error', error);
  } finally {
    await nextTick();
    if (request === scopeRequest) resolvingScope.value = false;
  }
}

function selectScope(value: unknown) {
  if (!Array.isArray(value) || !value.every(path => typeof path === 'string')) return;
  // Selection only configures the next scan. Streamed result roots must never replace it.
  selectedScopePaths.value = value.map(PathUtils.display);
  const selectedKeys = new Set(selectedScopePaths.value.map(PathUtils.comparisonKey));
  selectedProtectedPaths.value = selectedProtectedPaths.value.filter(path =>
    selectedKeys.has(PathUtils.comparisonKey(path))
  );
  storageScopeStore.selectPaths(scopeId, selectedScopePaths.value, props.disks);
  storageScopeStore.setDuplicateFileProtectedPaths(selectedProtectedPaths.value);
}

function updateProtectedPaths(paths: string[]) {
  selectedProtectedPaths.value = PathUtils.uniquePaths(paths);
  storageScopeStore.setDuplicateFileProtectedPaths(selectedProtectedPaths.value);
}

function removeScopeFolder(path: string) {
  storageScopeStore.removeFolder(path);
  selectScope(selectedScopePaths.value.filter(item => PathUtils.comparisonKey(item) !== PathUtils.comparisonKey(path)));
}

function updateMinimum(value: unknown) {
  const minimumBytes = Number(value);
  if (minimumBytes === props.minimumBytes || !minimumOptions.some(option => option.bytes === minimumBytes)) return;
  emit('updateMinimum', minimumBytes);
}

function applySmartSelection(rule = props.keeperRule) {
  if (props.hasMore) {
    pendingSmartSelectionRule.value = rule;
    if (!props.loadingMore) emit('loadMore', FILE_CATEGORY_IDS.all, true);
    return;
  }
  selectedPaths.value = DuplicateFileSelectionUtils.suggestedPaths(groups.value, rule);
}

function toggleSmartSelection() {
  if (selectedEntries.value.length) {
    pendingSmartSelectionRule.value = null;
    selectedPaths.value = [];
    return;
  }
  applySmartSelection();
}

function selectKeeperRule(value: DuplicateKeeperRuleId) {
  if (!Object.values(DUPLICATE_KEEPER_RULE_IDS).includes(value)) return;
  emit('updateKeeperRule', value);
  applySmartSelection(value);
}

function requestDelete(entries: DuplicateFileEntry[]) {
  if (props.busy || props.deleting || !resultMatchesScope.value || !entries.length) return;
  pendingDeleteEntries.value = entries;
  confirmOpen.value = true;
}

function confirmDelete() {
  if (props.busy || props.deleting || !resultMatchesScope.value || !pendingDeleteEntries.value.length) return;
  deleteRequested.value = true;
  emit('delete', pendingDeleteEntries.value);
  // Keep the confirmation visible as an activity dialog until the Store
  // settles. Successful rows disappear through the result update, while
  // failed selections remain available for another attempt.
  void nextTick(() => {
    if (!props.deleting) deleteRequested.value = false;
  });
}
</script>

<template>
  <MdPageShell class="duplicate-page @container/duplicates" content-mode="workspace" :title="t('duplicateFiles.title')">
    <template #overlay><MdAiWorkspace module="duplicateFiles" /></template>
    <template #actions>
      <div class="header-actions">
        <label class="size-filter header-size-filter">
          <span>{{ t('duplicateFiles.minimumSize') }}</span>
          <Select
            :model-value="String(minimumBytes)"
            :disabled="scanBusy || deleting"
            @update:model-value="updateMinimum"
          >
            <SelectTrigger class="w-28" size="sm" :aria-label="t('duplicateFiles.minimumSize')">
              <SelectValue>≥ {{ minimumLabel }}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem v-for="option in minimumOptions" :key="option.bytes" :value="String(option.bytes)">
                ≥ {{ option.label }}
              </SelectItem>
            </SelectContent>
          </Select>
        </label>
        <MdStorageScopeSelect
          :model-value="selectedScopePaths"
          multiple
          protection-enabled
          :protected-paths="selectedProtectedPaths"
          :disks="disks"
          :recent-folders="storageScopeStore.recentFolders"
          :standard-folders="storageScopeStore.standardFolders"
          :disabled="scanBusy || deleting"
          @error="emit('error', $event)"
          @remove-folder="removeScopeFolder"
          @update:protected-paths="updateProtectedPaths"
          @update:model-value="selectScope"
        />
        <Button
          v-if="result"
          class="scan-button"
          :variant="resultMatchesScope ? 'outline' : 'default'"
          type="button"
          :disabled="scanBusy || deleting || !canStart"
          @click="start"
        >
          <MdIcon
            :class="{ 'icon-spin': scanBusy }"
            :name="scanBusy || resultMatchesScope ? ICON_NAMES.refresh : ICON_NAMES.duplicateFiles"
            :size="17"
          />
          {{ t(resultMatchesScope ? 'duplicateFiles.rescan' : 'duplicateFiles.start') }}
        </Button>
      </div>
    </template>

    <template v-if="!scanBusy && result && resultComplete" #footer>
      <MdSelectionActionBar
        :selected-label="t('duplicateFiles.selected')"
        :selected-value="
          t(
            'duplicateFiles.copyCount',
            { count: FormatService.integer(selectedEntries.length) },
            selectedEntries.length
          )
        "
        :space-label="t('common.estimatedRelease')"
        :space-value="ByteSizeService.bytes(selectedBytes)"
        :action-label="t('duplicateFiles.batchDelete')"
        :disabled="!resultMatchesScope || !selectedEntries.length"
        :busy="deleting"
        @action="requestDelete(selectedEntries)"
      >
        <template #action-icon><MdIcon :name="ICON_NAMES.trash" :size="16" /></template>
      </MdSelectionActionBar>
    </template>

    <MdResultWorkspace>
      <template v-if="result" #summary>
        <MdResultSummary
          :title="
            t(
              'duplicateFiles.summaryCount',
              { count: FormatService.integer(result.totalGroupCount) },
              result.totalGroupCount
            )
          "
          :metric-label="summaryMetricLabel"
          :metric-value="ByteSizeService.bytes(result.reclaimableBytes)"
        >
          <template v-if="resultComplete && resultHasRelevantExclusions" #status>
            <MdScanExclusionLink :hint="t('storageScanExclusions.resultHint')" @open="emit('openExclusions')" />
          </template>
          <template #actions>
            <div class="summary-actions">
              <MdDuplicateSmartSelectButton
                :keeper-rule="keeperRule"
                :selected-count="selectedEntries.length"
                :busy="smartSelecting"
                :disabled="
                  !resultMatchesScope || !groups.length || scanBusy || deleting || smartSelecting || !resultComplete
                "
                @toggle="toggleSmartSelection"
                @select-rule="selectKeeperRule"
              />
            </div>
          </template>
        </MdResultSummary>
      </template>

      <template v-if="result" #header>
        <MdResultFilterToolbar>
          <MdFileCategoryFilter
            v-model="activeCategory"
            class="min-w-0 flex-1"
            :options="categoryOptions"
            :disabled="scanBusy"
          />
        </MdResultFilterToolbar>
      </template>

      <div class="result-content" :inert="scanBusy || undefined" :aria-busy="scanBusy">
        <template v-if="result">
          <MdDuplicateFileGroups
            v-show="filteredGroups.length > 0"
            v-model:selected-paths="selectedPaths"
            :scan-id="result.scanId"
            :category="activeCategory"
            :groups="filteredGroups"
            :keeper-rule="keeperRule"
            :selection-disabled="scanBusy || deleting || !resultMatchesScope"
            :open-disabled="scanBusy || deleting || !resultComplete"
            :delete-disabled="scanBusy || deleting || !resultComplete || !resultMatchesScope"
            :has-more="hasMore"
            :loading-more="loadingMore"
            :remaining-group-count="Math.max(0, (result?.returnedGroupCount ?? 0) - groups.length)"
            @open-entry="emit('openEntry', result.scanId, $event.path)"
            @reveal="emit('reveal', $event)"
            @delete="requestDelete([$event])"
            @explain="explainFile"
            @load-more="emit('loadMore', $event)"
          />
          <MdEmptyState
            v-if="!filteredGroups.length"
            compact
            :icon-name="ICON_NAMES.duplicateFiles"
            :title="t('duplicateFiles.noResults')"
            :description="t('duplicateFiles.noResultsDescription')"
          >
            <Button
              v-if="hasMore && resultComplete"
              size="sm"
              type="button"
              variant="ghost"
              :disabled="scanBusy || loadingMore"
              @click="emit('loadMore', activeCategory)"
            >
              {{ loadingMore ? t('loading.processing') : t('common.loadMore') }}
            </Button>
          </MdEmptyState>
        </template>
        <MdEmptyState
          v-else
          :icon-name="ICON_NAMES.duplicateFiles"
          :title="t('duplicateFiles.emptyTitle')"
          :description="t('duplicateFiles.emptyDescription', { size: minimumLabel })"
        >
          <div class="empty-primary-actions">
            <Button v-if="canStart" size="lg" type="button" :disabled="scanBusy || deleting" @click="start">
              <MdIcon :name="ICON_NAMES.duplicateFiles" :size="17" />
              {{ t('duplicateFiles.start') }}
            </Button>
          </div>
        </MdEmptyState>
      </div>

      <MdDelayedOperationWorkspace :active="scanBusy" mode="overlay" role="status" aria-live="polite">
        <MdOperationProgress
          :icon-name="ICON_NAMES.duplicateFiles"
          :title="progressTitle"
          :progress="progress"
          :path-label="t('loading.currentAnalysisDirectory')"
          :preparing-text="t('loading.preparingAnalysisDirectory')"
          :hint="t('duplicateFiles.scanHint')"
          :bytes-label="progressBytesLabel"
          :cancelable="true"
          :cancel-disabled="cancelling"
          @cancel="cancelScan"
        />
      </MdDelayedOperationWorkspace>
    </MdResultWorkspace>

    <MdDestructiveActionDialog
      v-model:open="confirmOpen"
      :title="t(pendingDeleteEntries.length === 1 ? 'duplicateFiles.deleteSingleTitle' : 'duplicateFiles.deleteTitle')"
      :description="
        t(
          pendingDeleteEntries.length === 1
            ? 'duplicateFiles.deleteSingleDescription'
            : 'duplicateFiles.deleteDescription'
        )
      "
      :summary-label="pendingSummaryLabel"
      :summary-value="ByteSizeService.bytes(pendingDeleteBytes)"
      :note="t('duplicateFiles.deleteSafetyNote')"
      :cancel-label="t('common.cancel')"
      :confirm-label="t('duplicateFiles.batchDelete')"
      :busy="deleting"
      @confirm="confirmDelete"
    />
  </MdPageShell>
</template>

<style scoped>
@reference "@assets/main.css";
.duplicate-page {
  height: 100%;
  min-height: 0;
  overflow: hidden;
}
.header-actions {
  display: flex;
  min-width: 0;
  max-width: 100%;
  flex-wrap: nowrap;
  align-items: center;
  justify-content: flex-end;
  gap: 10px;
}

.header-actions :deep(.scope-select) {
  min-width: 120px;
  max-width: 176px;
  flex: 1 1 176px;
}

.scan-button {
  flex: none;
  white-space: nowrap;
}

.summary-actions {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 8px;
}

.empty-primary-actions {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
}

.result-content {
  display: flex;
  min-height: 0;
  flex: 1;
  flex-direction: column;
}
.size-filter {
  display: flex;
  height: 40px;
  flex: none;
  align-items: center;
  gap: 6px;
  border-radius: var(--radius-sm);
  padding-inline-start: 9px;
  @apply bg-transparent transition-colors hover:bg-muted/55;
}

.size-filter > span {
  @apply text-muted-foreground;
  font-size: var(--font-content-meta);
}

.size-filter :deep([data-slot='select-trigger']) {
  height: 100%;
  border: 0;
  background: transparent;
  box-shadow: none;
}
</style>
