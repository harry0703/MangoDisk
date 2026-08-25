<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { computed, nextTick, ref, watch } from 'vue';

import MdDelayedOperationWorkspace from '@/components/custom/md-delayed-operation-workspace.vue';
import MdStorageScopeSelect from '@/components/custom/md-storage-scope-select.vue';
import MdEmptyState from '@/components/custom/md-empty-state.vue';
import MdFileCategoryFilter from '@/components/custom/md-file-category-filter.vue';
import MdOperationProgress from '@/components/custom/md-operation-progress.vue';
import MdPageShell from '@/components/custom/md-page-shell.vue';
import MdResultFilterToolbar from '@/components/custom/md-result-filter-toolbar.vue';
import MdResultSummary from '@/components/custom/md-result-summary.vue';
import MdResultWorkspace from '@/components/custom/md-result-workspace.vue';
import MdSelectionActionBar from '@/components/custom/md-selection-action-bar.vue';
import MdDestructiveActionDialog from '@/components/custom/md-destructive-action-dialog.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { Button } from '@/components/ui/button';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { FILE_CATEGORY_FILTER_ORDER, FILE_CATEGORY_IDS } from '@/lib/models/file-category';
import { LARGE_FILE_MINIMUM_PRESETS } from '@/lib/models/large-file';
import { STORAGE_SCOPE_IDS } from '@/lib/models/storage-scope';
import { ICON_NAMES } from '@/lib/models/ui';
import type { DiskInfo } from '@/lib/models/disk';
import type { TraversalProgress } from '@/lib/models/progress';
import type { FileCategoryId } from '@/lib/models/file-category';
import type { LargeFileEntry, LargeFilesResult, LargeFilesSelectionMode } from '@/lib/models/large-file';
import { DiskUtils } from '@/lib/utils/disk';
import { FileTypeUtils } from '@/lib/utils/file-type';
import { ByteSizeService } from '@/lib/services/byte-size-service';
import { AiAdvisorService } from '@/lib/services/ai-advisor-service';
import { FormatUtils } from '@/lib/utils/format';
import { LargeFileEntryUtils } from '@/lib/utils/large-file-entry';
import { PathUtils } from '@/lib/utils/path';
import { useStorageScopeStore } from '@/stores/storage-scope-store';
import { useAppStore } from '@/stores/app-store';

import MdLargeFileList from './components/md-large-file-list.vue';
import MdLargeFilesSelectionMode from './components/md-large-files-selection-mode.vue';

const { t } = useI18n({ useScope: 'global' });

const props = defineProps<{
  disk: DiskInfo | null;
  disks: DiskInfo[];
  result: LargeFilesResult | null;
  progress: TraversalProgress | null;
  minimumBytes: number;
  busy: boolean;
  cancelling: boolean;
  deleting: boolean;
}>();

const emit = defineEmits<{
  find: [path: string | undefined, refresh?: boolean];
  cancel: [];
  error: [error: unknown];
  updateMinimum: [minimumBytes: number];
  openEntry: [scanId: number, path: string];
  reveal: [path: string];
  deleteMany: [entries: LargeFileEntry[]];
}>();

const storageScopeStore = useStorageScopeStore();
const appStore = useAppStore();
const scopeId = STORAGE_SCOPE_IDS.largeFiles;
const minimumOptions = ByteSizeService.presetOptions(LARGE_FILE_MINIMUM_PRESETS);
const selectedScopePath = ref(
  PathUtils.display(storageScopeStore.selectedPath(scopeId) || props.result?.root || props.disk?.mountPoint || '')
);
const activeCategory = ref<FileCategoryId>(FILE_CATEGORY_IDS.all);
const selectedPaths = ref<string[]>([]);
const selectionMode = ref<LargeFilesSelectionMode>('none');
const aiAnalyzing = ref(false);
const aiRecommendedPaths = ref<string[]>([]);
const pendingDelete = ref<LargeFileEntry[]>([]);
const confirmOpen = ref(false);
const deleteRequested = ref(false);
let aiAbortController: AbortController | null = null;
let aiStaggerTimer: ReturnType<typeof setTimeout> | null = null;

const activeDisk = computed(() =>
  DiskUtils.findForPath(
    props.disks,
    props.result?.root || selectedScopePath.value || props.disk?.mountPoint || '',
    props.disk
  )
);
const resultMatchesScope = computed(
  () =>
    Boolean(props.result?.root && selectedScopePath.value) &&
    PathUtils.comparisonKey(props.result?.root ?? '') === PathUtils.comparisonKey(selectedScopePath.value)
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
  return t('common.fileCount', { count: FormatUtils.integer(pendingDelete.value.length) }, pendingDelete.value.length);
});

watch(
  () => props.disk?.mountPoint,
  mountPoint => {
    if (mountPoint && !selectedScopePath.value) {
      selectedScopePath.value = PathUtils.display(mountPoint);
    }
  },
  { immediate: true }
);
watch(
  () => props.result?.root,
  root => {
    if (!root) return;
    selectedScopePath.value = PathUtils.display(root);
  }
);
watch(minimumEntries, entries => {
  const existingPaths = new Set(entries.map(entry => entry.path));
  selectedPaths.value = selectedPaths.value.filter(path => existingPaths.has(path));
  aiRecommendedPaths.value = aiRecommendedPaths.value.filter(path => existingPaths.has(path));
  syncSelectionMode(selectedPaths.value);
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

function syncSelectionMode(paths: string[]) {
  const count = paths.length;
  const total = minimumEntries.value.length;
  if (count === 0) {
    selectionMode.value = 'none';
    return;
  }
  if (total > 0 && count === total) {
    selectionMode.value = 'all';
    return;
  }
  if (
    aiRecommendedPaths.value.length > 0 &&
    count === aiRecommendedPaths.value.length &&
    aiRecommendedPaths.value.every(path => paths.includes(path))
  ) {
    selectionMode.value = 'smart';
    return;
  }
  selectionMode.value = 'manual';
}

function updateSelectedPaths(paths: string[]) {
  if (aiAnalyzing.value) {
    abortAiAnalysis();
  }
  selectedPaths.value = paths;
  syncSelectionMode(paths);
}

async function handleSelectionModeChange(value: unknown) {
  if (!['smart', 'all', 'none'].includes(String(value))) return;
  const mode = String(value) as LargeFilesSelectionMode;

  abortAiAnalysis();

  if (mode === 'none') {
    selectionMode.value = 'none';
    aiRecommendedPaths.value = [];
    selectedPaths.value = [];
    return;
  }

  if (mode === 'all') {
    selectionMode.value = 'all';
    aiRecommendedPaths.value = [];
    selectedPaths.value = minimumEntries.value.map(entry => entry.path);
    return;
  }

  if (mode === 'smart') {
    selectionMode.value = 'smart';
    aiAnalyzing.value = true;
    aiRecommendedPaths.value = [];
    selectedPaths.value = [];

    const controller = new AbortController();
    aiAbortController = controller;

    try {
      const config = {
        apiKey: appStore.settings.aiApiKey,
        baseUrl: appStore.settings.aiApiBaseUrl,
        model: appStore.settings.aiModel
      };
      const recommended = await AiAdvisorService.analyzeLargeFiles(minimumEntries.value, config, controller.signal);
      if (controller.signal.aborted) return;

      aiRecommendedPaths.value = recommended;
      if (!recommended.length) {
        selectedPaths.value = [];
        aiAnalyzing.value = false;
        aiAbortController = null;
        return;
      }

      // Smooth staggered selection animation (50ms interval)
      const pathsToSelect = [...recommended];
      let index = 0;
      const batchSize = Math.max(1, Math.floor(pathsToSelect.length / 20));

      const processBatch = () => {
        if (controller.signal.aborted) return;
        const nextBatch = pathsToSelect.slice(index, index + batchSize);
        index += batchSize;
        selectedPaths.value = [...new Set([...selectedPaths.value, ...nextBatch])];

        if (index < pathsToSelect.length) {
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

function start(refresh = false) {
  if (props.busy || props.deleting || !selectedScopePath.value) return;
  abortAiAnalysis();
  aiRecommendedPaths.value = [];
  selectionMode.value = 'none';
  emit('find', selectedScopePath.value, refresh);
}

function updateMinimum(value: unknown) {
  const minimumBytes = Number(value);
  if (minimumBytes === props.minimumBytes || !minimumOptions.some(option => option.bytes === minimumBytes)) {
    return;
  }

  // Increasing the threshold is an in-memory filter. A lower threshold may
  // need rows that were omitted from the published result, so request them
  // immediately after persisting the new preference. Core normally serves
  // this from the existing 50 MB in-memory scan result without traversing the disk.
  abortAiAnalysis();
  emit('updateMinimum', minimumBytes);
  if (props.result && minimumBytes < props.result.minimumBytes && selectedScopePath.value) {
    emit('find', selectedScopePath.value, false);
  }
}

function selectScope(value: unknown) {
  if (typeof value !== 'string' || !value) return;
  // Scope selection configures the next explicit scan and never starts one.
  selectedScopePath.value = PathUtils.display(value);
  storageScopeStore.select(scopeId, selectedScopePath.value, props.disks);
}

function removeScopeFolder(path: string) {
  const removingCurrent = PathUtils.comparisonKey(path) === PathUtils.comparisonKey(selectedScopePath.value);
  storageScopeStore.removeFolder(path);
  if (!removingCurrent) return;

  const fallback = PathUtils.display(props.disk?.mountPoint || props.disks[0]?.mountPoint || '');
  selectedScopePath.value = fallback;
  if (fallback) storageScopeStore.select(scopeId, fallback, props.disks);
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
    <template #actions>
      <div class="header-actions">
        <label v-if="!result" class="size-filter header-size-filter">
          <span>{{ t('largeFiles.minimumSize') }}</span>
          <Select :model-value="String(minimumBytes)" :disabled="busy || deleting" @update:model-value="updateMinimum">
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
          :model-value="selectedScopePath || activeDisk?.mountPoint || ''"
          :disks="disks"
          :recent-folders="storageScopeStore.recentFolders"
          :standard-folders="storageScopeStore.standardFolders"
          :disabled="busy || deleting"
          @error="emit('error', $event)"
          @remove-folder="removeScopeFolder"
          @update:model-value="selectScope"
        />
        <Button
          v-if="result"
          class="search-button"
          :variant="resultMatchesScope ? 'outline' : 'default'"
          type="button"
          :disabled="busy || deleting || !selectedScopePath"
          @click="start(resultMatchesScope)"
        >
          <MdIcon
            :class="{ 'icon-spin': busy }"
            :name="busy || resultMatchesScope ? ICON_NAMES.refresh : ICON_NAMES.largeFiles"
            :size="17"
          />
          {{ t(resultMatchesScope ? 'largeFiles.rescan' : 'largeFiles.start') }}
        </Button>
      </div>
    </template>

    <template v-if="!busy && result" #footer>
      <MdSelectionActionBar
        :selected-label="t('largeFiles.selected')"
        :selected-value="
          t('common.fileCount', { count: FormatUtils.integer(selectedEntries.length) }, selectedEntries.length)
        "
        :space-label="t('common.estimatedRelease')"
        :space-value="ByteSizeService.bytes(selectedBytes)"
        :action-label="t('largeFiles.batchDelete')"
        :disabled="!selectedEntries.length"
        :busy="deleting || aiAnalyzing"
        @action="requestDelete(selectedEntries)"
      >
        <template #options>
          <MdLargeFilesSelectionMode
            :busy="busy || deleting"
            :mode="selectionMode"
            :analyzing="aiAnalyzing"
            @change="handleSelectionModeChange"
          />
        </template>
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
                count: FormatUtils.integer(resultSummaryCount),
                size: minimumLabel,
              },
              resultSummaryCount
            )
          "
          :metric-label="t('largeFiles.summarySpace')"
          :metric-value="ByteSizeService.bytes(resultSummaryBytes)"
        >
          <template #actions>
            <label class="size-filter summary-size-filter">
              <span>{{ t('largeFiles.minimumSize') }}</span>
              <Select
                :model-value="String(minimumBytes)"
                :disabled="busy || deleting"
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
          </template>
        </MdResultSummary>
      </template>

      <template v-if="result" #header>
        <MdResultFilterToolbar>
          <MdFileCategoryFilter v-model="activeCategory" :options="categoryOptions" :disabled="busy" />
        </MdResultFilterToolbar>
      </template>

      <template v-if="result?.truncated" #notice>
        {{ t('common.limitedResults') }}
      </template>

      <div class="result-content" :inert="busy ? '' : undefined" :aria-busy="busy">
        <template v-if="result">
          <MdLargeFileList
            v-show="filteredEntries.length > 0"
            :selected-paths="selectedPaths"
            :ai-recommended-paths="aiRecommendedPaths"
            :entries="filteredEntries"
            :open-disabled="busy || deleting"
            :delete-disabled="busy || deleting"
            @update:selected-paths="updateSelectedPaths"
            @open-entry="emit('openEntry', result.scanId, $event.path)"
            @reveal="emit('reveal', $event)"
            @delete="requestDelete([$event])"
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
          <Button size="lg" type="button" :disabled="busy || deleting || !selectedScopePath" @click="start(false)">
            <MdIcon :name="ICON_NAMES.largeFiles" :size="17" />
            {{ t('largeFiles.start') }}
          </Button>
        </MdEmptyState>
      </div>

      <MdDelayedOperationWorkspace :active="busy" mode="overlay" role="status" aria-live="polite">
        <MdOperationProgress
          :icon-name="ICON_NAMES.largeFiles"
          :title="cancelling ? t('loading.cancelling') : t('largeFiles.scanning')"
          :progress="progress"
          :path-label="t('loading.currentAnalysisDirectory')"
          :preparing-text="t('loading.preparingAnalysisDirectory')"
          :hint="t('largeFiles.scanHint')"
          :cancelable="true"
          :cancel-disabled="cancelling"
          @cancel="emit('cancel')"
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
  flex-wrap: wrap;
  align-items: center;
  justify-content: flex-end;
  gap: 10px;
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
  @apply bg-muted/55 text-foreground;
}

.summary-size-filter :deep([data-slot='select-trigger']) {
  border: 0;
  background: transparent;
  box-shadow: none;
}
</style>
