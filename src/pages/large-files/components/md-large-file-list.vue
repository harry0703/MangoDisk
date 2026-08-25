<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';

import MdLoadMoreButton from '@/components/custom/md-load-more-button.vue';
import MdFileEntryContextMenu from '@/components/custom/md-file-entry-context-menu.vue';
import MdIconAction from '@/components/custom/md-icon-action.vue';
import MdMiddleEllipsis from '@/components/custom/md-middle-ellipsis.vue';
import MdNativeFileIcon from '@/components/custom/md-native-file-icon.vue';
import MdResultCheckbox from '@/components/custom/md-result-checkbox.vue';
import MdResultTable from '@/components/custom/md-result-table.vue';
import MdResultTableRow from '@/components/custom/md-result-table-row.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { LARGE_FILE_RENDER_BATCH_SIZE, LARGE_FILE_SORT_KEYS } from '@/lib/models/large-file';
import { SORT_DIRECTIONS } from '@/lib/models/sort';
import { ICON_NAMES } from '@/lib/models/ui';
import type { LargeFileEntry } from '@/lib/models/large-file';
import type { SortDirection } from '@/lib/models/sort';
import { ByteSizeService } from '@/lib/services/byte-size-service';
import { FormatUtils } from '@/lib/utils/format';
import { LargeFileEntryUtils, type LargeFileSortKey } from '@/lib/utils/large-file-entry';
import { PathUtils } from '@/lib/utils/path';
import { RenderBatchUtils } from '@/lib/utils/render-batch';

const { locale, t } = useI18n({ useScope: 'global' });

const props = withDefaults(
  defineProps<{
    entries: LargeFileEntry[];
    selectedPaths: string[];
    aiRecommendedPaths?: string[];
    openDisabled: boolean;
    deleteDisabled: boolean;
  }>(),
  {
    aiRecommendedPaths: () => [],
  }
);
const emit = defineEmits<{
  openEntry: [entry: LargeFileEntry];
  reveal: [path: string];
  delete: [entry: LargeFileEntry];
  'update:selectedPaths': [paths: string[]];
}>();

const listScroll = ref<InstanceType<typeof MdResultTable> | null>(null);
const sortKey = ref<LargeFileSortKey>(LARGE_FILE_SORT_KEYS.bytes);
const sortDirection = ref<SortDirection>(SORT_DIRECTIONS.descending);
const visibleCount = ref(LARGE_FILE_RENDER_BATCH_SIZE);
const sortedEntries = computed(() => LargeFileEntryUtils.sorted(props.entries, sortKey.value, sortDirection.value));
const visibleEntries = computed(() => RenderBatchUtils.visibleItems(sortedEntries.value, visibleCount.value));
const remainingCount = computed(() =>
  RenderBatchUtils.remainingCount(sortedEntries.value.length, visibleEntries.value.length)
);
const selectedPathSet = computed(() => new Set(props.selectedPaths));
const aiRecommendedPathSet = computed(() => new Set(props.aiRecommendedPaths));
const selection = computed(() => LargeFileEntryUtils.selectionState(sortedEntries.value, selectedPathSet.value));

watch(
  () => props.entries,
  () => {
    // Category and threshold filters can shrink a long result to a few rows.
    // Reset both virtualization state and scroll position after the DOM update.
    visibleCount.value = LARGE_FILE_RENDER_BATCH_SIZE;
    listScroll.value?.scrollTo({ top: 0 });
  },
  { flush: 'post' }
);

function changeSort(key: LargeFileSortKey) {
  visibleCount.value = LARGE_FILE_RENDER_BATCH_SIZE;
  if (sortKey.value === key) {
    sortDirection.value =
      sortDirection.value === SORT_DIRECTIONS.ascending ? SORT_DIRECTIONS.descending : SORT_DIRECTIONS.ascending;
    return;
  }
  sortKey.value = key;
  sortDirection.value = key === LARGE_FILE_SORT_KEYS.name ? SORT_DIRECTIONS.ascending : SORT_DIRECTIONS.descending;
}

function sortIcon(key: LargeFileSortKey) {
  if (sortKey.value !== key) return ICON_NAMES.arrowUpDown;
  return sortDirection.value === SORT_DIRECTIONS.ascending ? ICON_NAMES.arrowUp : ICON_NAMES.arrowDown;
}

function updateSelection(targetPaths: string[], selected: boolean) {
  emit('update:selectedPaths', LargeFileEntryUtils.updateSelection(props.selectedPaths, targetPaths, selected));
}

function loadMore() {
  visibleCount.value = RenderBatchUtils.nextVisibleCount(
    visibleCount.value,
    sortedEntries.value.length,
    LARGE_FILE_RENDER_BATCH_SIZE
  );
}
</script>

<template>
  <MdResultTable ref="listScroll" class="large-file-list">
    <template #header>
      <div
        class="table-head grid-cols-[18px_minmax(220px,1.45fr)_minmax(120px,0.8fr)_88px_108px] @5xl/large-files:grid-cols-[18px_minmax(260px,1.55fr)_minmax(160px,1fr)_100px_124px]"
      >
        <label :aria-label="t('largeFiles.selectAll')">
          <MdResultCheckbox
            :checked="selection.checked"
            :indeterminate="selection.indeterminate"
            @update:checked="
              updateSelection(
                sortedEntries.map(entry => entry.path),
                $event
              )
            "
          />
        </label>
        <button
          class="md-result-sort"
          type="button"
          :data-active="sortKey === LARGE_FILE_SORT_KEYS.name"
          @click="changeSort(LARGE_FILE_SORT_KEYS.name)"
        >
          {{ t('largeFiles.fileName') }}
          <MdIcon :name="sortIcon(LARGE_FILE_SORT_KEYS.name)" :size="14" />
        </button>
        <span>{{ t('largeFiles.location') }}</span>
        <button
          class="md-result-sort"
          type="button"
          :data-active="sortKey === LARGE_FILE_SORT_KEYS.bytes"
          @click="changeSort(LARGE_FILE_SORT_KEYS.bytes)"
        >
          {{ t('largeFiles.size') }}
          <MdIcon :name="sortIcon(LARGE_FILE_SORT_KEYS.bytes)" :size="14" />
        </button>
        <button
          class="md-result-sort"
          type="button"
          :data-active="sortKey === LARGE_FILE_SORT_KEYS.modified"
          @click="changeSort(LARGE_FILE_SORT_KEYS.modified)"
        >
          {{ t('largeFiles.modified') }}
          <MdIcon :name="sortIcon(LARGE_FILE_SORT_KEYS.modified)" :size="14" />
        </button>
      </div>
    </template>

    <MdFileEntryContextMenu
      v-for="entry in visibleEntries"
      :key="entry.path"
      :open-disabled="openDisabled"
      :delete-disabled="deleteDisabled"
      @open="emit('openEntry', entry)"
      @reveal="emit('reveal', entry.path)"
      @delete="emit('delete', entry)"
    >
      <MdResultTableRow
        class="file-row grid-cols-[18px_minmax(220px,1.45fr)_minmax(120px,0.8fr)_88px_108px] @5xl/large-files:grid-cols-[18px_minmax(260px,1.55fr)_minmax(160px,1fr)_100px_124px]"
        :data-selected="selectedPathSet.has(entry.path)"
      >
        <MdResultCheckbox
          :aria-label="entry.name"
          :checked="selectedPathSet.has(entry.path)"
          @update:checked="updateSelection([entry.path], $event)"
        />
        <div class="file-name">
          <MdNativeFileIcon :path="entry.path" :name="entry.name" compact />
          <strong class="md-result-primary"><MdMiddleEllipsis :text="entry.name" /></strong>
          <span v-if="aiRecommendedPathSet.has(entry.path)" class="ai-badge" :title="t('largeFiles.aiAdvisorBadge')">
            <MdIcon :name="ICON_NAMES.smartSelect" :size="11" />
            <span>{{ t('largeFiles.aiAdvisorBadge') }}</span>
          </span>
          <div class="file-name-actions">
            <MdIconAction variant="ghost" :label="t('common.showInFileManager')" @click="emit('reveal', entry.path)">
              <MdIcon :name="ICON_NAMES.folder" :size="16" />
            </MdIconAction>
            <MdIconAction
              variant="ghost"
              :label="t('common.deletePermanently')"
              destructive
              :disabled="deleteDisabled"
              @click="emit('delete', entry)"
            >
              <MdIcon :name="ICON_NAMES.trash" :size="16" />
            </MdIconAction>
          </div>
        </div>
        <button class="location-button" type="button" :title="entry.parentPath" @click="emit('reveal', entry.path)">
          <MdMiddleEllipsis :text="PathUtils.display(entry.parentPath)" />
        </button>
        <strong class="file-size md-result-primary">{{ ByteSizeService.bytes(entry.bytes) }}</strong>
        <span class="modified">{{ FormatUtils.dateTime(entry.modifiedAtMs, locale) }}</span>
      </MdResultTableRow>
    </MdFileEntryContextMenu>

    <MdLoadMoreButton
      v-if="remainingCount"
      :remaining-label="t('common.fileCount', { count: FormatUtils.integer(remainingCount) }, remainingCount)"
      @load-more="loadMore"
    />
  </MdResultTable>
</template>

<style scoped>
@reference "@assets/main.css";

.large-file-list {
  display: flex;
  min-height: 0;
  flex: 1;
  flex-direction: column;
  overflow: hidden;
}

.table-head,
.file-row {
  display: grid;
  align-items: center;
  column-gap: 12px;
}

.table-head {
  flex: none;
  min-height: var(--layout-result-header-height);
  font-size: var(--font-content-meta);
}

.table-head label {
  display: flex;
  align-items: center;
  cursor: pointer;
}

.table-head button {
  display: flex;
  align-items: center;
  gap: 4px;
  border: 0;
  padding: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  cursor: pointer;
}

.file-row {
  min-height: var(--layout-result-row-height);
  padding-block: 2px;
}

.file-name {
  position: relative;
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 9px;
}

.file-name strong {
  min-width: 0;
  flex: 1;
}

.file-name-actions {
  position: absolute;
  right: 0;
  display: flex;
  align-items: center;
  gap: 2px;
  opacity: 0;
  pointer-events: none;
  transition: opacity 0.14s ease;
}

.file-row:is(:hover, :has(:focus-visible)) .file-name-actions {
  opacity: 1;
  pointer-events: auto;
}

.file-row:is(:hover, :has(:focus-visible)) .file-name strong {
  padding-right: 64px;
}

.file-name strong,
.file-size {
  font-size: var(--font-content-primary);
}

.ai-badge {
  display: inline-flex;
  flex: none;
  align-items: center;
  gap: 3px;
  border-radius: var(--radius-sm);
  padding: 1px 6px;
  font-size: var(--font-content-meta);
  font-weight: 500;
  line-height: 1.3;
  @apply border border-primary/20 bg-primary/10 text-primary;
}

.location-button {
  min-width: 0;
  width: 100%;
  border: 0;
  padding: 4px 0;
  background: transparent;
  @apply text-muted-foreground hover:text-primary;
  font: inherit;
  font-size: var(--font-content-secondary);
  text-align: left;
  cursor: pointer;
}

.location-button:focus-visible {
  border-radius: 4px;
  @apply outline-none ring-2 ring-ring/35;
}

.modified {
  @apply text-muted-foreground;
  font-size: var(--font-content-meta);
}

@container large-files (max-width: 760px) {
  .table-head,
  .file-row {
    grid-template-columns: 18px minmax(0, 1fr) 84px;
    column-gap: 10px;
  }

  .table-head > :nth-child(3),
  .table-head > :nth-child(5),
  .modified {
    display: none;
  }

  .table-head > :nth-child(4) {
    grid-column: 3;
  }

  .file-row {
    min-height: 50px;
    grid-template-rows: minmax(22px, auto) minmax(16px, auto);
    row-gap: 0;
    padding-block: 3px;
  }

  .file-row > :first-child {
    grid-row: 1 / 3;
  }

  .file-name {
    grid-row: 1;
    grid-column: 2;
  }

  .location-button {
    grid-row: 2;
    grid-column: 2;
    padding: 0;
    font-size: var(--font-content-meta);
  }

  .file-size {
    grid-row: 1 / 3;
    grid-column: 3;
    justify-self: end;
  }
}
</style>
