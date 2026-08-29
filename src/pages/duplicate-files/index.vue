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
import { DUPLICATE_FILE_MINIMUM_PRESETS, DUPLICATE_KEEPER_RULE_IDS } from '@/lib/models/duplicate-file';
import { STORAGE_SCOPE_IDS } from '@/lib/models/storage-scope';
import { ICON_NAMES } from '@/lib/models/ui';
import type { DuplicateFileEntry, DuplicateFilesResult, DuplicateKeeperRuleId } from '@/lib/models/duplicate-file';
import type { DiskInfo } from '@/lib/models/disk';
import type { TraversalProgress } from '@/lib/models/progress';
import type { FileCategoryId } from '@/lib/models/file-category';
import { DuplicateFileSelectionUtils } from '@/lib/utils/duplicate-file-selection';
import { DuplicateFileGroupUtils } from '@/lib/utils/duplicate-file-group';
import { ByteSizeService } from '@/lib/services/byte-size-service';
import { AiAdvisorService } from '@/lib/services/ai-advisor-service';
import { FormatUtils } from '@/lib/utils/format';
import { PathUtils } from '@/lib/utils/path';
import { useStorageScopeStore } from '@/stores/storage-scope-store';
import { useAppStore } from '@/stores/app-store';

import MdDuplicateFileGroups from './components/md-duplicate-file-groups.vue';
import MdDuplicateSmartSelectButton from './components/md-duplicate-smart-select-button.vue';

const { t } = useI18n({ useScope: 'global' });
const appStore = useAppStore();

const props = defineProps<{
  disk: DiskInfo | null;
  disks: DiskInfo[];
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

const emit = defineEmits<{
  error: [error: unknown];
  find: [path: string];
  cancel: [];
  openEntry: [scanId: number, path: string];
  reveal: [path: string];
  delete: [entries: DuplicateFileEntry[]];
  loadMore: [category: FileCategoryId];
  updateMinimum: [minimumBytes: number];
  updateKeeperRule: [keeperRule: DuplicateKeeperRuleId];
}>();

const storageScopeStore = useStorageScopeStore();
const scopeId = STORAGE_SCOPE_IDS.duplicateFiles;
const minimumOptions = ByteSizeService.presetOptions(DUPLICATE_FILE_MINIMUM_PRESETS);
const selectedScopePath = ref(
  PathUtils.display(storageScopeStore.selectedPath(scopeId) || props.result?.roots[0] || props.disk?.mountPoint || '')
);
const activeCategory = ref<FileCategoryId>(FILE_CATEGORY_IDS.all);
const selectedPaths = ref<string[]>([]);
const aiAnalyzing = ref(false);
const aiRecommendedPaths = ref<string[]>([]);
const confirmOpen = ref(false);
const pendingDeleteEntries = ref<DuplicateFileEntry[]>([]);
const deleteRequested = ref(false);
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
    { count: FormatUtils.integer(pendingDeleteEntries.value.length) },
    pendingDeleteEntries.value.length
  );
});
const canStart = computed(() => Boolean(selectedScopePath.value));
const minimumLabel = computed(
  () =>
    minimumOptions.find(option => option.bytes === props.minimumBytes)?.label ??
    ByteSizeService.bytes(props.minimumBytes)
);
const resultMatchesScope = computed(
  () =>
    props.result?.roots.length === 1 &&
    PathUtils.comparisonKey(props.result.roots[0] ?? '') === PathUtils.comparisonKey(selectedScopePath.value)
);
const progressTitle = computed(() => {
  if (props.cancelling) return t('loading.cancelling');
  if (props.progress?.currentStage === 'validatingFiles') return t('duplicateFiles.validatingCandidates');
  if (props.progress?.currentStage === 'hashingFiles') return t('duplicateFiles.comparingContents');
  return t('duplicateFiles.scanning');
});
const summaryMetricLabel = computed(() =>
  t(props.resultComplete ? 'duplicateFiles.summaryReclaimable' : 'duplicateFiles.summaryReclaimableScanning')
);
watch(groups, nextGroups => {
  // Retain only selections that still exist in the current result.
  const existing = new Set(nextGroups.flatMap(group => group.entries.map(entry => entry.path)));
  selectedPaths.value = selectedPaths.value.filter(path => existing.has(path));
  aiRecommendedPaths.value = aiRecommendedPaths.value.filter(path => existing.has(path));
});

watch(
  () => props.disk?.mountPoint,
  mountPoint => {
    if (mountPoint && !selectedScopePath.value) selectedScopePath.value = PathUtils.display(mountPoint);
  },
  { immediate: true }
);

watch(
  () => props.result?.roots[0],
  root => {
    if (root) selectedScopePath.value = PathUtils.display(root);
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

function start() {
  if (props.busy || props.deleting || !canStart.value) return;
  abortAiAnalysis();
  aiRecommendedPaths.value = [];
  selectedPaths.value = [];
  pendingDeleteEntries.value = [];
  emit('find', selectedScopePath.value);
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

function updateMinimum(value: unknown) {
  const minimumBytes = Number(value);
  if (minimumBytes === props.minimumBytes || !minimumOptions.some(option => option.bytes === minimumBytes)) return;
  abortAiAnalysis();
  aiRecommendedPaths.value = [];
  emit('updateMinimum', minimumBytes);
}

function applySmartSelection(rule = props.keeperRule) {
  selectedPaths.value = DuplicateFileSelectionUtils.suggestedPaths(groups.value, rule);
}

function toggleSmartSelection() {
  abortAiAnalysis();
  if (selectedPaths.value.length) {
    selectedPaths.value = [];
    aiRecommendedPaths.value = [];
    return;
  }
  applySmartSelection();
}

function selectKeeperRule(value: DuplicateKeeperRuleId) {
  if (!Object.values(DUPLICATE_KEEPER_RULE_IDS).includes(value)) return;
  abortAiAnalysis();
  aiRecommendedPaths.value = [];
  emit('updateKeeperRule', value);
  applySmartSelection(value);
}

async function runAiSmartSelect() {
  if (props.busy || props.deleting || !groups.value.length || !props.resultComplete) return;
  abortAiAnalysis();

  aiAnalyzing.value = true;
  aiRecommendedPaths.value = [];
  selectedPaths.value = [];

  const controller = new AbortController();
  aiAbortController = controller;

  try {
    const config = {
      apiKey: appStore.settings.aiApiKey,
      baseUrl: appStore.settings.aiApiBaseUrl,
      model: appStore.settings.aiModel,
    };
    const recommended = await AiAdvisorService.analyzeDuplicateFiles(groups.value, config, controller.signal);
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

function requestDelete(entries: DuplicateFileEntry[]) {
  if (props.busy || props.deleting || !entries.length) return;
  abortAiAnalysis();
  pendingDeleteEntries.value = entries;
  confirmOpen.value = true;
}

function confirmDelete() {
  if (props.busy || props.deleting || !pendingDeleteEntries.value.length) return;
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
    <template #actions>
      <div class="header-actions">
        <label class="size-filter header-size-filter">
          <span>{{ t('duplicateFiles.minimumSize') }}</span>
          <Select :model-value="String(minimumBytes)" :disabled="busy || deleting" @update:model-value="updateMinimum">
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
          :model-value="selectedScopePath"
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
          class="scan-button"
          :variant="resultMatchesScope ? 'outline' : 'default'"
          type="button"
          :disabled="busy || deleting || !canStart"
          @click="start"
        >
          <MdIcon
            :class="{ 'icon-spin': busy }"
            :name="busy || resultMatchesScope ? ICON_NAMES.refresh : ICON_NAMES.duplicateFiles"
            :size="17"
          />
          {{ t(resultMatchesScope ? 'duplicateFiles.rescan' : 'duplicateFiles.start') }}
        </Button>
      </div>
    </template>

    <template v-if="!busy && result && resultComplete" #footer>
      <MdSelectionActionBar
        :selected-label="t('duplicateFiles.selected')"
        :selected-value="
          t('duplicateFiles.copyCount', { count: FormatUtils.integer(selectedEntries.length) }, selectedEntries.length)
        "
        :space-label="t('common.estimatedRelease')"
        :space-value="ByteSizeService.bytes(selectedBytes)"
        :action-label="t('duplicateFiles.batchDelete')"
        :disabled="!selectedEntries.length"
        :busy="deleting || aiAnalyzing"
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
              { count: FormatUtils.integer(result.totalGroupCount) },
              result.totalGroupCount
            )
          "
          :metric-label="summaryMetricLabel"
          :metric-value="ByteSizeService.bytes(result.reclaimableBytes)"
        >
          <template #actions>
            <MdDuplicateSmartSelectButton
              :keeper-rule="keeperRule"
              :selected-count="selectedPaths.length"
              :analyzing="aiAnalyzing"
              :disabled="!groups.length || busy || deleting || !resultComplete"
              @toggle="toggleSmartSelection"
              @select-rule="selectKeeperRule"
              @ai-select="runAiSmartSelect"
            />
          </template>
        </MdResultSummary>
      </template>

      <template v-if="result" #header>
        <MdResultFilterToolbar>
          <MdFileCategoryFilter
            v-model="activeCategory"
            class="min-w-0 flex-1"
            :options="categoryOptions"
            :disabled="busy"
          />
        </MdResultFilterToolbar>
      </template>

      <div class="result-content" :inert="busy ? '' : undefined" :aria-busy="busy">
        <template v-if="result">
          <MdDuplicateFileGroups
            v-show="filteredGroups.length > 0"
            v-model:selected-paths="selectedPaths"
            :ai-recommended-paths="aiRecommendedPaths"
            :scan-id="result.scanId"
            :category="activeCategory"
            :groups="filteredGroups"
            :keeper-rule="keeperRule"
            :selection-disabled="busy || deleting"
            :open-disabled="busy || deleting || !resultComplete"
            :delete-disabled="busy || deleting || !resultComplete"
            :has-more="hasMore"
            :loading-more="loadingMore"
            :remaining-group-count="Math.max(0, (result?.returnedGroupCount ?? 0) - groups.length)"
            @open-entry="emit('openEntry', result.scanId, $event.path)"
            @reveal="emit('reveal', $event)"
            @delete="requestDelete([$event])"
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
              :disabled="busy || loadingMore"
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
          <Button v-if="canStart" size="lg" type="button" :disabled="busy || deleting" @click="start">
            <MdIcon :name="ICON_NAMES.duplicateFiles" :size="17" />
            {{ t('duplicateFiles.start') }}
          </Button>
        </MdEmptyState>
      </div>

      <MdDelayedOperationWorkspace :active="busy" mode="overlay" role="status" aria-live="polite">
        <MdOperationProgress
          :icon-name="ICON_NAMES.duplicateFiles"
          :title="progressTitle"
          :progress="progress"
          :path-label="t('loading.currentAnalysisDirectory')"
          :preparing-text="t('loading.preparingAnalysisDirectory')"
          :hint="t('duplicateFiles.scanHint')"
          :cancelable="true"
          :cancel-disabled="cancelling"
          @cancel="emit('cancel')"
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
