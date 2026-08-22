<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { computed, ref, watch } from 'vue';

import MdLoadMoreButton from '@/components/custom/md-load-more-button.vue';
import MdFileEntryContextMenu from '@/components/custom/md-file-entry-context-menu.vue';
import MdIconAction from '@/components/custom/md-icon-action.vue';
import MdMiddleEllipsis from '@/components/custom/md-middle-ellipsis.vue';
import MdNativeFileIcon from '@/components/custom/md-native-file-icon.vue';
import MdResultCheckbox from '@/components/custom/md-result-checkbox.vue';
import MdResultTable from '@/components/custom/md-result-table.vue';
import MdResultTableHierarchy from '@/components/custom/md-result-table-hierarchy.vue';
import MdResultTableRow from '@/components/custom/md-result-table-row.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { Button } from '@/components/ui/button';
import { ICON_NAMES } from '@/lib/models/ui';
import {
  DUPLICATE_GROUP_KINDS,
  DUPLICATE_ENTRY_RENDER_BATCH_SIZE,
  DUPLICATE_GROUP_RENDER_BATCH_SIZE,
  type DuplicateFileEntry,
  type DuplicateGroup,
  type DuplicateKeeperRuleId,
} from '@/lib/models/duplicate-file';
import { FILE_CATEGORY_IDS, type FileCategoryId } from '@/lib/models/file-category';
import { DuplicateFileSelectionUtils } from '@/lib/utils/duplicate-file-selection';
import { DuplicateFileGroupUtils } from '@/lib/utils/duplicate-file-group';
import { ByteSizeService } from '@/lib/services/byte-size-service';
import { FormatUtils } from '@/lib/utils/format';
import { PathUtils } from '@/lib/utils/path';
import { RenderBatchUtils } from '@/lib/utils/render-batch';

const { locale, t } = useI18n({ useScope: 'global' });

const props = withDefaults(
  defineProps<{
    scanId: number;
    category: FileCategoryId;
    groups: DuplicateGroup[];
    keeperRule: DuplicateKeeperRuleId;
    selectedPaths: string[];
    aiRecommendedPaths?: string[];
    selectionDisabled: boolean;
    openDisabled: boolean;
    deleteDisabled: boolean;
    hasMore: boolean;
    loadingMore: boolean;
    remainingGroupCount: number;
  }>(),
  {
    aiRecommendedPaths: () => [],
  }
);
const emit = defineEmits<{
  openEntry: [entry: DuplicateFileEntry];
  reveal: [path: string];
  delete: [entry: DuplicateFileEntry];
  loadMore: [category: FileCategoryId];
  'update:selectedPaths': [paths: string[]];
}>();

const groupsScroll = ref<InstanceType<typeof MdResultTable> | null>(null);
const collapsedGroupIds = ref<ReadonlySet<string>>(new Set());
const selectedPathSet = computed(() => new Set(props.selectedPaths));
const aiRecommendedPathSet = computed(() => new Set(props.aiRecommendedPaths));
const keeperPathByGroup = computed(
  () =>
    new Map(
      props.groups.map(group => [
        group.id,
        DuplicateFileSelectionUtils.keeper(group.entries, props.keeperRule)?.path ?? null,
      ])
    )
);
const appliedSelectionGroupIds = computed(
  () =>
    new Set(
      props.groups
        .filter(group => {
          const keeperPath = keeperPathByGroup.value.get(group.id);
          return group.entries.every(entry => selectedPathSet.value.has(entry.path) === (entry.path !== keeperPath));
        })
        .map(group => group.id)
    )
);
const visibleGroupCount = ref(DUPLICATE_GROUP_RENDER_BATCH_SIZE);
const visibleEntryCounts = ref<Record<string, number>>({});
const revealRequestedPage = ref(false);
const visibleGroups = computed(() => RenderBatchUtils.visibleItems(props.groups, visibleGroupCount.value));
const remainingVisibleGroupCount = computed(() =>
  RenderBatchUtils.remainingCountAcrossPages(props.groups.length, visibleGroups.value.length, props.remainingGroupCount)
);
const groupLabels = computed(() => DuplicateFileGroupUtils.displayLabels(props.groups));
const unselectedCountByGroup = computed(
  () =>
    new Map(
      props.groups.map(group => [
        group.id,
        group.entries.reduce((count, entry) => count + Number(!selectedPathSet.value.has(entry.path)), 0),
      ])
    )
);

watch(
  () => [props.scanId, props.category] as const,
  () => {
    // Streaming and pagination can replace the array during one scan. Reset
    // only for a new scan or category so updates do not interrupt reading.
    visibleGroupCount.value = DUPLICATE_GROUP_RENDER_BATCH_SIZE;
    visibleEntryCounts.value = {};
    revealRequestedPage.value = false;
    collapsedGroupIds.value = new Set();
    groupsScroll.value?.scrollTo({ top: 0 });
  },
  { flush: 'post' }
);

watch(
  () => props.groups.length,
  (nextCount, previousCount) => {
    if (!revealRequestedPage.value || nextCount <= previousCount) return;

    // Reveal a user-requested page as soon as it arrives. Keeping the loaded
    // page hidden would require a second click and make the counter alternate
    // between locally hidden groups and groups not fetched from Core yet.
    visibleGroupCount.value = RenderBatchUtils.nextVisibleCount(
      visibleGroupCount.value,
      nextCount,
      DUPLICATE_GROUP_RENDER_BATCH_SIZE
    );
    revealRequestedPage.value = false;
  },
  { flush: 'post' }
);

watch(
  () => [props.loadingMore, props.hasMore] as const,
  ([loading, hasMore]) => {
    // A matching page clears the request in the group-count watcher. Clear it
    // here only when pagination reaches the end without finding a match.
    if (!loading && !hasMore) revealRequestedPage.value = false;
  },
  { flush: 'post' }
);

function isSelected(path: string) {
  return selectedPathSet.value.has(path);
}

function isOnlyKeeper(entry: DuplicateFileEntry, group: DuplicateGroup) {
  return !selectedPathSet.value.has(entry.path) && unselectedCountByGroup.value.get(group.id) === 1;
}

function toggleEntry(entry: DuplicateFileEntry, group: DuplicateGroup, selected: boolean) {
  emit(
    'update:selectedPaths',
    DuplicateFileSelectionUtils.updateEntrySelection(props.selectedPaths, entry, group, selected)
  );
}

function toggleGroupSelection(group: DuplicateGroup) {
  emit(
    'update:selectedPaths',
    DuplicateFileSelectionUtils.toggleGroupCopies(props.selectedPaths, group, props.keeperRule)
  );
}

function isGroupSelectionApplied(group: DuplicateGroup) {
  return appliedSelectionGroupIds.value.has(group.id);
}

function toggleGroup(groupId: string) {
  const next = new Set(collapsedGroupIds.value);
  if (next.has(groupId)) next.delete(groupId);
  else next.add(groupId);
  collapsedGroupIds.value = next;
}

function visibleEntries(group: DuplicateGroup): DuplicateFileEntry[] {
  const limit = visibleEntryCounts.value[group.id] ?? DUPLICATE_ENTRY_RENDER_BATCH_SIZE;
  return RenderBatchUtils.visibleItems(group.entries, limit);
}

function remainingEntryCount(group: DuplicateGroup): number {
  return RenderBatchUtils.remainingCount(group.entries.length, visibleEntries(group).length);
}

function loadMoreEntries(group: DuplicateGroup) {
  const current = visibleEntryCounts.value[group.id] ?? DUPLICATE_ENTRY_RENDER_BATCH_SIZE;
  visibleEntryCounts.value = {
    ...visibleEntryCounts.value,
    [group.id]: RenderBatchUtils.nextVisibleCount(current, group.entries.length, DUPLICATE_ENTRY_RENDER_BATCH_SIZE),
  };
}

function loadMoreGroups() {
  if (visibleGroupCount.value < props.groups.length) {
    visibleGroupCount.value = RenderBatchUtils.nextVisibleCount(
      visibleGroupCount.value,
      props.groups.length,
      DUPLICATE_GROUP_RENDER_BATCH_SIZE
    );
    return;
  }
  if (props.loadingMore || !props.hasMore) return;
  revealRequestedPage.value = true;
  emit('loadMore', props.category);
}
</script>

<template>
  <MdResultTable ref="groupsScroll" class="duplicate-groups">
    <section v-for="group in visibleGroups" :key="group.id" class="duplicate-group">
      <header class="group-header" @click="toggleGroup(group.id)">
        <MdNativeFileIcon
          :path="group.entries[0]?.path ?? ''"
          :name="group.entries[0]?.name ?? ''"
          :directory="group.kind === DUPLICATE_GROUP_KINDS.directory"
          directory-mode="generic"
        />
        <button class="group-disclosure" type="button" :aria-expanded="!collapsedGroupIds.has(group.id)">
          <span class="group-copy">
            <strong class="md-result-primary">
              {{ groupLabels.get(group.id) }}
            </strong>
            <small>
              {{
                t(
                  group.kind === DUPLICATE_GROUP_KINDS.directory
                    ? 'duplicateFiles.directoryGroupSummary'
                    : 'duplicateFiles.groupSummary',
                  {
                    count: FormatUtils.integer(group.entries.length),
                    size: ByteSizeService.bytes(group.bytesPerFile),
                    files: t(
                      'common.fileCount',
                      { count: FormatUtils.integer(group.fileCountPerEntry) },
                      group.fileCountPerEntry
                    ),
                    reclaimable: ByteSizeService.bytes(group.reclaimableBytes),
                  },
                  group.entries.length
                )
              }}
            </small>
          </span>
        </button>
        <Button
          class="group-select-action"
          size="sm"
          variant="ghost"
          type="button"
          :data-applied="isGroupSelectionApplied(group)"
          :disabled="selectionDisabled"
          :aria-label="
            t(
              isGroupSelectionApplied(group)
                ? 'duplicateFiles.clearGroupSelectionHint'
                : 'duplicateFiles.selectGroupHint'
            )
          "
          @click.stop="toggleGroupSelection(group)"
        >
          <MdIcon :name="isGroupSelectionApplied(group) ? ICON_NAMES.check : ICON_NAMES.duplicateFiles" :size="14" />
          <span class="group-select-label">
            {{
              t(
                isGroupSelectionApplied(group) ? 'duplicateFiles.groupSelectionApplied' : 'duplicateFiles.selectGroup',
                { count: FormatUtils.integer(Math.max(0, group.entries.length - 1)) },
                Math.max(0, group.entries.length - 1)
              )
            }}
          </span>
        </Button>
        <MdIcon
          class="group-chevron"
          :class="{ collapsed: collapsedGroupIds.has(group.id) }"
          :name="ICON_NAMES.chevronUp"
          :size="16"
        />
      </header>

      <MdResultTableHierarchy v-show="!collapsedGroupIds.has(group.id)">
        <MdFileEntryContextMenu
          v-for="entry in visibleEntries(group)"
          :key="entry.path"
          :open-disabled="openDisabled"
          :delete-disabled="deleteDisabled"
          @open="emit('openEntry', entry)"
          @reveal="emit('reveal', entry.path)"
          @delete="emit('delete', entry)"
        >
          <MdResultTableRow
            class="member-row grid-cols-[18px_minmax(120px,1fr)_112px] @5xl/duplicates:grid-cols-[18px_minmax(160px,1fr)_128px]"
            :data-selected="isSelected(entry.path)"
          >
            <MdResultCheckbox
              :aria-label="entry.path"
              :checked="isSelected(entry.path)"
              :disabled="selectionDisabled || isOnlyKeeper(entry, group)"
              @update:checked="toggleEntry(entry, group, $event)"
            />
            <span class="member-primary">
              <span class="member-path">
                <MdMiddleEllipsis :text="PathUtils.display(entry.path)" :tail-length="32" />
              </span>
              <span
                v-if="aiRecommendedPathSet.has(entry.path)"
                class="ai-badge"
                :title="t('duplicateFiles.aiAdvisorBadge')"
              >
                <MdIcon :name="ICON_NAMES.smartSelect" :size="11" />
                <span>{{ t('duplicateFiles.aiAdvisorBadge') }}</span>
              </span>
              <span class="member-actions">
                <MdIconAction
                  variant="ghost"
                  :label="t('common.showInFileManager')"
                  @click.prevent="emit('reveal', entry.path)"
                >
                  <MdIcon :name="ICON_NAMES.folder" :size="16" />
                </MdIconAction>
                <MdIconAction
                  variant="ghost"
                  :label="t('common.deletePermanently')"
                  destructive
                  :disabled="deleteDisabled"
                  @click.prevent="emit('delete', entry)"
                >
                  <MdIcon :name="ICON_NAMES.trash" :size="16" />
                </MdIconAction>
              </span>
            </span>
            <span class="member-date">{{ FormatUtils.dateTime(entry.modifiedAtMs, locale) }}</span>
          </MdResultTableRow>
        </MdFileEntryContextMenu>
        <template v-if="remainingEntryCount(group)" #footer>
          <MdLoadMoreButton
            v-if="remainingEntryCount(group)"
            :remaining-label="
              t(
                'common.fileCount',
                { count: FormatUtils.integer(remainingEntryCount(group)) },
                remainingEntryCount(group)
              )
            "
            @load-more="loadMoreEntries(group)"
          />
        </template>
      </MdResultTableHierarchy>
    </section>
    <MdLoadMoreButton
      v-if="remainingVisibleGroupCount > 0 && (visibleGroups.length < groups.length || hasMore)"
      class="group-load-more"
      :remaining-label="
        category === FILE_CATEGORY_IDS.all
          ? t(
              'duplicateFiles.groupCount',
              { count: FormatUtils.integer(remainingVisibleGroupCount) },
              remainingVisibleGroupCount
            )
          : undefined
      "
      :disabled="loadingMore"
      :loading="loadingMore"
      @load-more="loadMoreGroups"
    />
  </MdResultTable>
</template>

<style scoped>
@reference "@assets/main.css";

.duplicate-groups {
  min-height: 0;
  flex: 1;
}

.duplicate-group {
  overflow: hidden;
  background: transparent;
}

.group-load-more {
  min-height: 38px;
  border-top-width: 0;
  background: transparent;
  padding: 4px 10px;
}

.group-header {
  position: relative;
  display: grid;
  min-height: var(--layout-result-group-height);
  grid-template-columns: 36px minmax(0, 1fr) auto 24px;
  align-items: center;
  gap: 11px;
  padding-block: 4px;
  padding-inline: var(--result-table-content-inline-padding);
  @apply text-muted-foreground;
  cursor: pointer;
}

.group-header::before {
  position: absolute;
  top: var(--result-item-background-inset, 3px);
  right: 0;
  bottom: var(--result-item-background-inset, 3px);
  left: 0;
  border-radius: 7px;
  content: '';
  @apply bg-muted/38;
  pointer-events: none;
}

.group-header > * {
  position: relative;
}

.group-disclosure {
  min-width: 0;
  border: 0;
  padding: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
}

.group-header:has(.group-disclosure:focus-visible)::before {
  box-shadow: inset 0 0 0 1px var(--focus-ring-subtle);
  box-shadow: inset 0 0 0 1px color-mix(in oklab, var(--ring) 52%, transparent);
}

.group-disclosure:focus-visible {
  outline: none;
}

.group-copy {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 3px;
}

.group-copy > strong {
  @apply text-card-foreground;
}

.group-copy strong {
  overflow: hidden;
  font-size: var(--font-content-primary);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.group-copy small {
  overflow: hidden;
  @apply text-muted-foreground;
  font-size: var(--font-content-secondary);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.group-chevron {
  justify-self: center;
  pointer-events: none;
  @apply text-muted-foreground transition-transform duration-200;
}

.group-chevron.collapsed {
  transform: rotate(180deg);
}

.group-select-action {
  height: 30px;
  gap: 5px;
  border-width: 0;
  border-radius: 7px;
  padding: 0 7px;
  @apply bg-transparent text-muted-foreground shadow-none hover:text-primary;
  font-size: var(--font-content-secondary);
  font-weight: 500;
  white-space: nowrap;
}

.group-select-action[data-applied='true'] {
  @apply bg-transparent text-primary hover:text-primary;
}

.group-select-action:hover {
  background: var(--surface-primary-subtle);
}

.group-select-action :deep(svg) {
  flex: none;
}

.member-row {
  display: grid;
  min-height: var(--layout-result-child-row-height);
  align-items: center;
  gap: 10px;
  padding-block: 1px;
}

.duplicate-group :deep(.hierarchy-items::before) {
  width: 1px;
  @apply bg-border/65;
}

.member-primary {
  position: relative;
  display: flex;
  min-width: 0;
  align-items: center;
}

.member-path {
  min-width: 0;
  flex: 1;
  overflow: hidden;
  @apply text-card-foreground;
  font-size: var(--font-content-body);
  font-weight: 400;
}

.member-actions {
  position: absolute;
  right: 0;
  display: flex;
  gap: 2px;
  opacity: 0;
  pointer-events: none;
  transition: opacity 0.14s ease;
}

.ai-badge {
  display: inline-flex;
  flex: none;
  align-items: center;
  gap: 3px;
  border-radius: var(--radius-sm);
  padding: 1px 6px;
  margin-left: 6px;
  font-size: var(--font-content-meta);
  font-weight: 500;
  line-height: 1.3;
  @apply border border-primary/20 bg-primary/10 text-primary;
}

.member-row:is(:hover, :has(:focus-visible)) .member-actions {
  opacity: 1;
  pointer-events: auto;
}

.member-row:is(:hover, :has(:focus-visible)) .member-primary {
  padding-right: 64px;
}

.member-date {
  overflow: hidden;
  @apply text-muted-foreground;
  font-size: 10px;
  font-variant-numeric: tabular-nums;
  text-overflow: ellipsis;
  white-space: nowrap;
}

@container duplicates (max-width: 700px) {
  .member-row {
    grid-template-columns: 18px minmax(0, 1fr);
  }

  .member-date {
    display: none;
  }
}

@container duplicates (max-width: 560px) {
  .group-header {
    grid-template-columns: 32px minmax(0, 1fr) 30px 20px;
    gap: 8px;
  }

  .group-select-action {
    width: 30px;
    padding: 0;
  }

  .group-select-label {
    display: none;
  }
}
</style>
