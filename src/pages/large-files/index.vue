<script setup lang="ts">
import { FormatService } from '@/lib/services/presentation/format-service';
import { FolderSelectionService } from '@/lib/services/storage/folder-selection-service';
import { useI18n } from 'vue-i18n';
import MdAiWorkspace from '@/layouts/components/md-ai-workspace.vue';
import { useAiStore } from '@/stores/ai-store';
import { largeFileAiContext } from './large-file-ai-context';
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
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { FILE_CATEGORY_FILTER_ORDER, FILE_CATEGORY_IDS } from '@/lib/models/filesystem/file-category';
import {
  LARGE_FILE_MINIMUM_PRESETS,
  LARGE_FILE_SCAN_MODES,
  type LargeFileScanMode,
} from '@/lib/models/storage/large-file';
import { STORAGE_SCOPE_IDS } from '@/lib/models/storage/storage-scope';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import type { VolumeInfo } from '@/lib/models/storage/volume';
import type { TraversalProgress } from '@/lib/models/presentation/progress';
import type { FileCategoryId } from '@/lib/models/filesystem/file-category';
import type { LargeFileEntry, LargeFilesResult } from '@/lib/models/storage/large-file';
import * as FileTypeUtils from '@/lib/utils/filesystem/file-type';
import { ByteSizeService } from '@/lib/services/settings/byte-size-service';
import { OperatingSystemService } from '@/lib/services/settings/operating-system-service';
import * as LargeFileEntryUtils from '@/lib/utils/storage/large-file-entry';
import * as PathUtils from '@/lib/utils/filesystem/path';
import * as StorageScanPreferenceUtils from '@/lib/utils/storage/storage-scan-preference';
import { useStorageScopeStore } from '@/stores/storage-scope-store';
import { useLargeFilesStore } from '@/stores/large-files-store';
import { useStorageScanPreferencesStore } from '@/stores/storage-scan-preferences-store';

import MdLargeFileList from './components/md-large-file-list.vue';
import MdLargeFileScanButton from './components/md-large-file-scan-button.vue';

const { locale, t } = useI18n({ useScope: 'global' });

const props = defineProps<{
  disk: VolumeInfo | null;
  disks: VolumeInfo[];
  result: LargeFilesResult | null;
  progress: TraversalProgress | null;
  minimumBytes: number;
  busy: boolean;
  cancelling: boolean;
  deleting: boolean;
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
  find: [paths: string[], scanMode: LargeFileScanMode];
  cancel: [];
  error: [error: unknown];
  updateMinimum: [minimumBytes: number];
  openEntry: [scanId: number, path: string];
  reveal: [path: string];
  deleteMany: [entries: LargeFileEntry[]];
  openExclusions: [];
}>();

const aiStore = useAiStore();
const storageScopeStore = useStorageScopeStore();
const largeFilesStore = useLargeFilesStore();
const storageScanPreferencesStore = useStorageScanPreferencesStore();
const scopeId = STORAGE_SCOPE_IDS.largeFiles;
const minimumOptions = ByteSizeService.presetOptions(LARGE_FILE_MINIMUM_PRESETS);
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
const activeCategory = ref<FileCategoryId>(FILE_CATEGORY_IDS.all);
const selectedPaths = ref<string[]>([]);
const pendingDelete = ref<LargeFileEntry[]>([]);
const confirmOpen = ref(false);
const deleteRequested = ref(false);
const selectableScanModes = OperatingSystemService.isMacOs();
const requestedScanMode = ref<LargeFileScanMode>(
  selectableScanModes ? LARGE_FILE_SCAN_MODES.quick : LARGE_FILE_SCAN_MODES.complete
);
const scanHint = computed(() =>
  requestedScanMode.value === LARGE_FILE_SCAN_MODES.quick
    ? t('largeFiles.scanMode.quickHint')
    : t('largeFiles.scanMode.completeHint')
);

const resultMatchesScope = computed(() =>
  Boolean(props.result && PathUtils.sameSelectedPaths(props.result.roots, selectedScopePaths.value))
);
const resultMatchesExclusions = computed(
  () =>
    StorageScanPreferenceUtils.sameExcludedFolders(
      storageScanPreferencesStore.pathsForScope('largeFiles'),
      largeFilesStore.resultExcludedFolders
    ) &&
    StorageScanPreferenceUtils.sameExcludedNames(
      storageScanPreferencesStore.namesForScope('largeFiles'),
      largeFilesStore.resultExcludedNames
    )
);
const resultMatchesConfiguration = computed(() => resultMatchesScope.value && resultMatchesExclusions.value);
const resultHasRelevantExclusions = computed(
  () =>
    largeFilesStore.resultExcludedNames.length > 0 ||
    StorageScanPreferenceUtils.hasExcludedFolderInScanRoots(
      props.result?.roots ?? [],
      largeFilesStore.resultExcludedFolders
    )
);
const minimumEntries = computed(() => (props.result?.entries ?? []).filter(entry => entry.bytes >= props.minimumBytes));
const minimumLabel = computed(
  () =>
    minimumOptions.find(option => option.bytes === props.minimumBytes)?.label ??
    ByteSizeService.bytes(props.minimumBytes)
);
const categoryOptions = computed(() => {
  // Count every category in one pass over the result.
  const counts = FileTypeUtils.categoryCounts(minimumEntries.value.map(entry => entry.name));
  return FILE_CATEGORY_FILTER_ORDER.map(value => ({
    value,
    label: t(`fileCategories.${value}`),
    count: counts[value],
  }));
});
const filteredEntries = computed(() => {
  if (activeCategory.value === FILE_CATEGORY_IDS.all) return minimumEntries.value;
  return minimumEntries.value.filter(entry => FileTypeUtils.category(entry.name) === activeCategory.value);
});
// Native totals are exact for the threshold used by the scan. Raising the
// threshold only filters the loaded rows, so the summary must follow that
// visible subset instead of presenting the original scan total as current.
const resultSummaryCount = computed(() => {
  if (props.result?.minimumBytes === props.minimumBytes) return props.result.totalCount;
  return minimumEntries.value.length;
});
const resultSummaryBytes = computed(() => {
  if (props.result?.minimumBytes === props.minimumBytes) return props.result.totalBytes;
  return minimumEntries.value.reduce((total, entry) => total + entry.bytes, 0);
});
const selectedEntries = computed(() => LargeFileEntryUtils.selectedEntries(minimumEntries.value, selectedPaths.value));
const selectedBytes = computed(() => selectedEntries.value.reduce((total, entry) => total + entry.bytes, 0));
const pendingBytes = computed(() => pendingDelete.value.reduce((total, entry) => total + entry.bytes, 0));
const pendingSummaryLabel = computed(() => {
  if (pendingDelete.value.length === 1) return pendingDelete.value[0]?.name ?? '';
  return t(
    'common.fileCount',
    { count: FormatService.integer(pendingDelete.value.length) },
    pendingDelete.value.length
  );
});

watch(
  () => props.disk?.mountPoint,
  mountPoint => {
    if (mountPoint && storageScopeStore.selectedPaths[scopeId] === undefined && !selectedScopePaths.value.length) {
      selectedScopePaths.value = [PathUtils.display(mountPoint)];
    }
  }
);
watch(
  () => props.result?.scanMode,
  scanMode => {
    if (scanMode) requestedScanMode.value = scanMode;
  }
);
watch(minimumEntries, entries => {
  const existingPaths = new Set(entries.map(entry => entry.path));
  selectedPaths.value = selectedPaths.value.filter(path => existingPaths.has(path));
});
watch(
  () => props.deleting,
  (deleting, wasDeleting) => {
    if (!deleteRequested.value || deleting || !wasDeleting) return;
    deleteRequested.value = false;
    confirmOpen.value = false;
    pendingDelete.value = [];
  }
);
onMounted(() => {
  void storageScanPreferencesStore.initialize().catch(error => emit('error', error));
});

function explainFile(entry: LargeFileEntry) {
  if (props.busy || props.deleting) return;
  const context = largeFileAiContext(
    entry,
    OperatingSystemService.isWindows() ? 'windows' : OperatingSystemService.isLinux() ? 'linux' : 'macos'
  );
  if (context) void aiStore.show(context, locale.value);
}

watch(
  () => [props.result?.scanId, props.busy, props.deleting] as const,
  () => aiStore.dismissModule('largeFiles')
);

async function start(scanMode: LargeFileScanMode = requestedScanMode.value) {
  if (scanBusy.value || props.deleting || !selectedScopePaths.value.length) return;
  const requested = [...selectedScopePaths.value];
  const request = ++scopeRequest;
  resolvingScope.value = true;
  try {
    const roots = await FolderSelectionService.filterExistingDirectories(requested, () => request !== scopeRequest);
    if (request !== scopeRequest || !PathUtils.sameSelectedPaths(requested, selectedScopePaths.value)) return;
    selectScope(roots);
    requestedScanMode.value = scanMode;
    emit('find', roots, scanMode);
  } catch (error) {
    if (request === scopeRequest) emit('error', error);
  } finally {
    await nextTick();
    if (request === scopeRequest) resolvingScope.value = false;
  }
}

function updateMinimum(value: unknown) {
  const minimumBytes = Number(value);
  if (minimumBytes === props.minimumBytes || !minimumOptions.some(option => option.bytes === minimumBytes)) {
    return;
  }

  // Every scan retains candidates from the fixed 50 MiB floor. The shell asks Core to filter the
  // active scan by ID, so changing this preference never starts another filesystem traversal.
  emit('updateMinimum', minimumBytes);
}

function selectScope(value: unknown) {
  if (!Array.isArray(value) || !value.every(path => typeof path === 'string')) return;
  // Scope selection configures the next explicit scan and never starts one.
  selectedScopePaths.value = value.map(PathUtils.display);
  storageScopeStore.selectPaths(scopeId, selectedScopePaths.value, props.disks);
}

function removeScopeFolder(path: string) {
  storageScopeStore.removeFolder(path);
  selectScope(selectedScopePaths.value.filter(item => PathUtils.comparisonKey(item) !== PathUtils.comparisonKey(path)));
}

function requestDelete(entries: LargeFileEntry[]) {
  if (props.busy || props.deleting || !entries.length) return;
  pendingDelete.value = entries;
  confirmOpen.value = true;
}

function confirmDelete() {
  if (props.busy || props.deleting || !pendingDelete.value.length) return;
  deleteRequested.value = true;
  emit('deleteMany', pendingDelete.value);
  // Keep the dialog visible while deletion runs so single-file and batch
  // actions expose the same activity indicator. If another operation wins a
  // narrow race, restore the dialog to its actionable state.
  void nextTick(() => {
    if (!props.deleting) deleteRequested.value = false;
  });
}
</script>

<template>
  <MdPageShell class="@container/large-files" content-mode="workspace" :title="t('largeFiles.title')">
    <template #overlay><MdAiWorkspace module="largeFiles" /></template>
    <template #actions>
      <div class="header-actions">
        <label v-if="!result" class="size-filter header-size-filter">
          <span>{{ t('largeFiles.minimumSize') }}</span>
          <Select
            :model-value="String(minimumBytes)"
            :disabled="scanBusy || deleting"
            @update:model-value="updateMinimum"
          >
            <SelectTrigger class="w-28" size="sm" :aria-label="t('largeFiles.minimumSize')">
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
          :disks="disks"
          :recent-folders="storageScopeStore.recentFolders"
          :standard-folders="storageScopeStore.standardFolders"
          :disabled="scanBusy || deleting"
          @error="emit('error', $event)"
          @remove-folder="removeScopeFolder"
          @update:model-value="selectScope"
        />
        <MdLargeFileScanButton
          v-if="result"
          action="rescan"
          :busy="scanBusy || deleting || !selectedScopePaths.length"
          :emphasized="!resultMatchesConfiguration"
          :mode="
            resultMatchesScope
              ? requestedScanMode
              : selectableScanModes
                ? LARGE_FILE_SCAN_MODES.quick
                : LARGE_FILE_SCAN_MODES.complete
          "
          :selectable-modes="selectableScanModes"
          @scan="start"
        />
      </div>
    </template>

    <template v-if="!scanBusy && result" #footer>
      <MdSelectionActionBar
        :selected-label="t('largeFiles.selected')"
        :selected-value="
          t('common.fileCount', { count: FormatService.integer(selectedEntries.length) }, selectedEntries.length)
        "
        :space-label="t('common.estimatedRelease')"
        :space-value="ByteSizeService.bytes(selectedBytes)"
        :action-label="t('largeFiles.batchDelete')"
        :disabled="!resultMatchesConfiguration || !selectedEntries.length"
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
              'largeFiles.summaryCount',
              {
                count: FormatService.integer(resultSummaryCount),
                size: minimumLabel,
              },
              resultSummaryCount
            )
          "
          :metric-label="t('largeFiles.summarySpace')"
          :metric-value="ByteSizeService.bytes(resultSummaryBytes)"
        >
          <template v-if="resultHasRelevantExclusions" #status>
            <MdScanExclusionLink :hint="t('storageScanExclusions.resultHint')" @open="emit('openExclusions')" />
          </template>
          <template #actions>
            <div class="summary-actions">
              <label class="size-filter summary-size-filter">
                <span>{{ t('largeFiles.minimumSize') }}</span>
                <Select
                  :model-value="String(minimumBytes)"
                  :disabled="scanBusy || deleting"
                  @update:model-value="updateMinimum"
                >
                  <SelectTrigger class="w-28" size="sm" :aria-label="t('largeFiles.minimumSize')">
                    <SelectValue>≥ {{ minimumLabel }}</SelectValue>
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem v-for="option in minimumOptions" :key="option.bytes" :value="String(option.bytes)">
                      ≥ {{ option.label }}
                    </SelectItem>
                  </SelectContent>
                </Select>
              </label>
            </div>
          </template>
        </MdResultSummary>
      </template>

      <template v-if="result" #header>
        <MdResultFilterToolbar>
          <MdFileCategoryFilter v-model="activeCategory" :options="categoryOptions" :disabled="scanBusy" />
        </MdResultFilterToolbar>
      </template>

      <template v-if="result?.truncated" #notice>
        {{ t('common.limitedResults') }}
      </template>

      <div class="result-content" :inert="scanBusy || undefined" :aria-busy="scanBusy">
        <template v-if="result">
          <MdLargeFileList
            v-show="filteredEntries.length > 0"
            v-model:selected-paths="selectedPaths"
            :entries="filteredEntries"
            :open-disabled="scanBusy || deleting"
            :delete-disabled="scanBusy || deleting || !resultMatchesConfiguration"
            @open-entry="emit('openEntry', result.scanId, $event.path)"
            @reveal="emit('reveal', $event)"
            @delete="requestDelete([$event])"
            @explain="explainFile($event)"
          />
          <MdEmptyState
            v-if="!filteredEntries.length"
            compact
            :icon-name="ICON_NAMES.fileSearch"
            :title="t('largeFiles.noResults')"
            :description="t('largeFiles.noResultsDescription')"
          />
        </template>
        <MdEmptyState
          v-else
          :icon-name="ICON_NAMES.largeFiles"
          :title="t('largeFiles.emptyTitle')"
          :description="t('largeFiles.emptyDescription', { size: minimumLabel })"
        >
          <div class="empty-primary-actions">
            <MdLargeFileScanButton
              :busy="scanBusy || deleting || !selectedScopePaths.length"
              :mode="requestedScanMode"
              :selectable-modes="selectableScanModes"
              @scan="start"
            />
          </div>
        </MdEmptyState>
      </div>

      <MdDelayedOperationWorkspace :active="scanBusy" mode="overlay" role="status" aria-live="polite">
        <MdOperationProgress
          :icon-name="ICON_NAMES.largeFiles"
          :title="cancelling ? t('loading.cancelling') : t('largeFiles.scanning')"
          :progress="progress"
          :path-label="t('loading.currentAnalysisDirectory')"
          :preparing-text="t('loading.preparingAnalysisDirectory')"
          :hint="scanHint"
          :show-step-progress="false"
          :cancelable="true"
          :cancel-disabled="cancelling"
          @cancel="cancelScan"
        />
      </MdDelayedOperationWorkspace>
    </MdResultWorkspace>

    <MdDestructiveActionDialog
      v-model:open="confirmOpen"
      :title="pendingDelete.length > 1 ? t('largeFiles.batchDeleteTitle') : t('largeFiles.deleteConfirmTitle')"
      :description="
        pendingDelete.length > 1 ? t('largeFiles.batchDeleteDescription') : t('largeFiles.deleteConfirmDescription')
      "
      :summary-label="pendingSummaryLabel"
      :summary-value="ByteSizeService.bytes(pendingBytes)"
      :note="t('largeFiles.deleteSafetyNote')"
      :cancel-label="t('common.cancel')"
      :confirm-label="t('largeFiles.deleteConfirmAction')"
      :busy="deleting"
      @confirm="confirmDelete"
    />
  </MdPageShell>
</template>

<style scoped>
@reference "@assets/main.css";
.header-actions {
  display: flex;
  min-width: 0;
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

.result-content {
  display: flex;
  min-height: 0;
  flex: 1;
  flex-direction: column;
}
.size-filter {
  display: flex;
  flex: none;
  align-items: center;
  gap: 6px;
}
.size-filter > span {
  @apply text-muted-foreground;
  font-size: var(--font-content-meta);
}
.header-size-filter {
  height: 40px;
  border-radius: var(--radius-sm);
  padding-inline-start: 9px;
  @apply bg-transparent transition-colors hover:bg-muted/55;
}

.header-size-filter :deep([data-slot='select-trigger']) {
  height: 100%;
  border: 0;
  background: transparent;
  box-shadow: none;
}

.summary-size-filter {
  height: 34px;
  border-radius: var(--radius-sm);
  padding-inline-start: 10px;
  @apply bg-muted/55 text-foreground transition-colors hover:bg-accent/70;
}

.summary-size-filter:hover > span {
  @apply text-foreground;
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

.summary-size-filter :deep([data-slot='select-trigger']) {
  border: 0;
  background: transparent;
  box-shadow: none;
}

.summary-size-filter :deep([data-slot='select-trigger']:hover) {
  background: transparent;
}

.summary-size-filter:hover :deep([data-slot='select-trigger'] svg) {
  opacity: 0.8;
}
</style>
