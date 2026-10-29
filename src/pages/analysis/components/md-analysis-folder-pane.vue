<script setup lang="ts">
import { FormatService } from '@/lib/services/presentation/format-service';
import MdSpinner from '@/components/custom/md-spinner.vue';
import MdResultCheckbox from '@/components/custom/md-result-checkbox.vue';
import { Button } from '@/components/ui/button';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdResultTable from '@/components/custom/md-result-table.vue';
import { useI18n } from 'vue-i18n';
import {
  computed,
  nextTick,
  onActivated,
  onBeforeUnmount,
  onDeactivated,
  onMounted,
  ref,
  watch,
  type VNode,
} from 'vue';
import { observeElementRect, useVirtualizer, type Range } from '@tanstack/vue-virtual';
import MdFileEntryContextMenu from '@/components/custom/md-file-entry-context-menu.vue';
import MdAnalysisEntryIcon from './md-analysis-entry-icon.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import type { DirectoryEntryInfo } from '@/lib/models/storage/analysis';
import { ByteSizeService } from '@/lib/services/settings/byte-size-service';
import * as FormatUtils from '@/lib/utils/presentation/format';

const { t } = useI18n({ useScope: 'global' });

const props = withDefaults(
  defineProps<{
    entries: DirectoryEntryInfo[];
    totalBytes: number;
    folderCount: number;
    fileCount: number;
    truncated: boolean;
    openDisabled: boolean;
    deleteDisabled: boolean;
    deletingPath?: string | null;
    hoveredEntryPath?: string | null;
    selectedPaths?: string[];
    selectionMode?: boolean;
    deleting?: boolean;
  }>(),
  { selectedPaths: () => [], selectionMode: false, deletingPath: null, hoveredEntryPath: null, deleting: false }
);

const emit = defineEmits<{
  activate: [entry: DirectoryEntryInfo];
  openEntry: [entry: DirectoryEntryInfo];
  reveal: [path: string];
  delete: [entry: DirectoryEntryInfo];
  hoverEntry: [path: string | null];
  'update:selectedPaths': [paths: string[]];
  'update:selectionMode': [enabled: boolean];
  deleteSelected: [];
}>();

const selectableEntries = computed(() => props.entries.filter(entry => !entry.deletionProtection));
const selectedSet = computed(() => new Set(props.selectedPaths));
const selectedEntries = computed(() => selectableEntries.value.filter(entry => selectedSet.value.has(entry.path)));
const selectedBytes = computed(() => selectedEntries.value.reduce((sum, entry) => sum + entry.bytes, 0));
const allSelected = computed(
  () => selectableEntries.value.length > 0 && selectedEntries.value.length === selectableEntries.value.length
);
function selectEntry(entry: DirectoryEntryInfo, selected: boolean) {
  if (!props.selectionMode || props.deleteDisabled || entry.deletionProtection) return;
  const paths = new Set(props.selectedPaths);
  if (selected) paths.add(entry.path);
  else paths.delete(entry.path);
  emit('update:selectedPaths', [...paths]);
}
function selectAll(selected: boolean) {
  if (!props.selectionMode || props.deleteDisabled) return;
  emit('update:selectedPaths', selected ? selectableEntries.value.map(entry => entry.path) : []);
}

const pane = ref<HTMLElement | null>(null);
watch(
  () => props.selectionMode,
  enabled => {
    const focused = document.activeElement;
    if (
      !enabled &&
      pane.value?.isConnected &&
      (focused === document.body ||
        (focused instanceof HTMLElement &&
          pane.value.contains(focused) &&
          focused.closest('[role="checkbox"], .selection-delete')))
    ) {
      void nextTick(() => pane.value?.querySelector<HTMLButtonElement>('.selection-toggle')?.focus());
    }
  }
);
const table = ref<InstanceType<typeof MdResultTable> | null>(null);
const listElement = ref<HTMLElement | null>(null);
const listWidth = ref(0);
let resizeObserver: ResizeObserver | null = null;
const rowHeight = ref(52);
const viewportHeight = ref(600);
let pendingFocusIndex: number | null = null;
let active = true;
let lastScrollTop = 0;
function rememberScroll() {
  if (active && listElement.value?.isConnected) lastScrollTop = listElement.value.scrollTop;
}
const overscan = 12;
const poolSize = computed(() => Math.ceil(viewportHeight.value / rowHeight.value) + overscan * 2 + 1);
const observeVisibleRect: typeof observeElementRect<HTMLElement> = (instance, update) =>
  observeElementRect(instance, rect => {
    // A collapsed pane must retain its scroll range until it is visible again.
    if (rect.height > 0) {
      viewportHeight.value = rect.height;
      update(rect);
    }
  });
function pooledRange(range: Range) {
  const length = Math.min(range.count, poolSize.value);
  const start = Math.max(0, Math.min(range.startIndex - overscan, range.count - length));
  return Array.from({ length }, (_, offset) => start + offset);
}
const virtualizer = useVirtualizer(
  computed(() => ({
    count: props.entries.length,
    getScrollElement: () => listElement.value,
    getItemKey: (index: number) => props.entries[index]?.path ?? index,
    estimateSize: () => rowHeight.value,
    observeElementRect: observeVisibleRect,
    initialRect: { width: 400, height: 600 },
    rangeExtractor: pooledRange,
    overscan,
  }))
);
const rows = computed(() =>
  virtualizer.value.getVirtualItems().flatMap(row => {
    const entry = props.entries[row.index];
    return entry ? [{ row, entry }] : [];
  })
);
function releaseRecycledFocus(next: VNode, previous: VNode) {
  if (next.props?.['data-entry-key'] === previous.props?.['data-entry-key']) return;
  const element = previous.el;
  const focused = element instanceof HTMLElement ? element.ownerDocument.activeElement : null;
  if (focused instanceof HTMLElement && element instanceof HTMLElement && element.contains(focused)) {
    listElement.value?.focus({ preventScroll: true });
  }
  emit('hoverEntry', null);
}
function activateEntry(event: MouseEvent, entry: DirectoryEntryInfo) {
  // WebKit does not focus buttons on mouse clicks. Establish the row as the
  // keyboard target so End/Arrow navigation cannot fall back to native scrolling.
  if (event.currentTarget instanceof HTMLButtonElement) event.currentTarget.focus({ preventScroll: true });
  emit('activate', entry);
}

function focusPendingRow() {
  if (pendingFocusIndex === null) return;
  const button = listElement.value?.querySelector<HTMLButtonElement>(
    `[data-index="${pendingFocusIndex}"] .folder-entry`
  );
  if (!button) return;
  pendingFocusIndex = null;
  button.focus({ preventScroll: true });
}
// WebKit can deliver a programmatic scroll event after nextTick. Focus only
// once the target virtual row exists, rather than leaving focus on a recycled row.
watch(rows, focusPendingRow, { flush: 'post' });
async function navigateRows(event: KeyboardEvent, index: number) {
  const page = Math.max(1, Math.floor(viewportHeight.value / rowHeight.value));
  const target = {
    ArrowDown: index + 1,
    ArrowUp: index - 1,
    PageDown: index + page,
    PageUp: index - page,
    Home: 0,
    End: props.entries.length - 1,
  }[event.key];
  if (target === undefined || props.openDisabled || !props.entries.length) return;
  event.preventDefault();
  const next = Math.max(0, Math.min(props.entries.length - 1, target));
  pendingFocusIndex = next;
  virtualizer.value.scrollToIndex(next, { align: 'auto' });
  await nextTick();
  focusPendingRow();
}
watch(
  () => props.entries,
  () => {
    pendingFocusIndex = null;
    lastScrollTop = 0;
    emit('hoverEntry', null);
    virtualizer.value.scrollToOffset(0);
  },
  { flush: 'post' }
);

function entryShareStyle(entry: DirectoryEntryInfo) {
  const share = FormatUtils.percent(entry.bytes, props.totalBytes);
  // Hide narrow fragments without exaggerating their actual byte share.
  const visible = (listWidth.value * share) / 100 >= 4;
  return { '--entry-share': visible ? `${share}%` : '0%' };
}

onMounted(() => {
  listElement.value = table.value?.getScrollElement() ?? null;
  const element = listElement.value;
  if (!element) return;
  element.tabIndex = -1;
  element.addEventListener('scroll', rememberScroll, { passive: true });
  const contentHeight = Number.parseFloat(getComputedStyle(element).getPropertyValue('--layout-result-row-height'));
  if (contentHeight > 0) rowHeight.value = contentHeight + 8;
  resizeObserver = new ResizeObserver(([entry]) => {
    if (entry) listWidth.value = entry.contentRect.width;
  });
  resizeObserver.observe(element);
});

onDeactivated(() => {
  active = false;
  pendingFocusIndex = null;
});
onActivated(() => {
  active = true;
  // WebKit resets the native offset when KeepAlive reattaches the viewport.
  void nextTick(() => virtualizer.value.scrollToOffset(lastScrollTop));
});
onBeforeUnmount(() => {
  resizeObserver?.disconnect();
  listElement.value?.removeEventListener('scroll', rememberScroll);
});
</script>

<template>
  <aside ref="pane" class="folder-pane border-b @2xl/analysis:border-r @2xl/analysis:border-b-0">
    <header class="md-workspace-toolbar">
      <span v-if="selectionMode" class="selection-all">
        <MdResultCheckbox
          :checked="allSelected"
          :indeterminate="selectedEntries.length > 0 && !allSelected"
          :disabled="deleteDisabled || !selectableEntries.length"
          :aria-label="t('analysis.selectAll')"
          @update:checked="selectAll"
        />
      </span>
      <template v-if="selectionMode && selectedEntries.length">
        <p class="selection-count" aria-live="polite">
          {{ t('analysis.selectedCount', { count: FormatService.integer(selectedEntries.length) }) }}
        </p>
        <span class="selection-size">{{ ByteSizeService.bytes(selectedBytes) }}</span>
        <Button
          class="selection-delete"
          variant="ghost"
          size="sm"
          :disabled="deleteDisabled"
          :aria-busy="deleting || undefined"
          :aria-label="t(deleting ? 'analysis.deleting' : 'analysis.deleteAction')"
          @click="emit('deleteSelected')"
        >
          <MdSpinner v-if="deleting" />
          <MdIcon v-else :name="ICON_NAMES.trash" :size="14" aria-hidden="true" />
          {{ t(deleting ? 'analysis.deleting' : 'analysis.deleteSelectedAction') }}
        </Button>
      </template>
      <p v-else>
        {{
          t(
            'analysis.folderFileSummary',
            { folders: FormatService.integer(folderCount), files: FormatService.integer(fileCount) },
            fileCount
          )
        }}
      </p>
      <MdTooltip
        v-if="truncated && !selectedEntries.length"
        :text="t('analysis.limitedEntries', { count: FormatService.integer(entries.length) })"
      >
        <button
          type="button"
          class="md-help-action"
          :aria-label="t('analysis.limitedEntries', { count: FormatService.integer(entries.length) })"
        >
          <MdIcon :name="ICON_NAMES.help" :size="14" aria-hidden="true" />
        </button>
      </MdTooltip>
      <Button
        class="selection-toggle"
        variant="ghost"
        size="sm"
        :disabled="selectionMode ? deleting : deleteDisabled || !selectableEntries.length"
        :aria-pressed="selectionMode"
        @click="emit('update:selectionMode', !selectionMode)"
      >
        {{ t(selectionMode ? 'common.cancel' : 'analysis.multiSelect') }}
      </Button>
    </header>
    <MdResultTable ref="table" class="folder-list" synchronous-scroll>
      <div class="virtual-content" :style="{ height: `${virtualizer.getTotalSize()}px` }">
        <div class="virtual-window" :style="{ top: `${rows[0]?.row.start ?? 0}px` }">
          <div
            v-for="{ row, entry } in rows"
            :key="row.index % poolSize"
            :data-index="row.index"
            :data-entry-key="entry.path"
            :style="{ height: `${rowHeight}px` }"
            @vue:before-update="releaseRecycledFocus"
          >
            <MdFileEntryContextMenu
              :entry-key="entry.path"
              :open-disabled="openDisabled"
              :delete-disabled="deleteDisabled || Boolean(entry.deletionProtection)"
              :reveal-disabled="deletingPath === entry.path"
              @open="emit('openEntry', entry)"
              @reveal="emit('reveal', entry.path)"
              @delete="emit('delete', entry)"
            >
              <div
                class="folder-row"
                :class="{
                  'is-highlighted': !openDisabled && hoveredEntryPath === entry.path,
                  'is-selecting': selectionMode,
                }"
                :style="entryShareStyle(entry)"
                @pointerenter="emit('hoverEntry', openDisabled ? null : entry.path)"
                @pointerleave="emit('hoverEntry', null)"
              >
                <MdTooltip v-if="selectionMode" :text="entry.deletionProtection ? t('analysis.protectedItem') : null">
                  <span class="folder-checkbox">
                    <MdResultCheckbox
                      :checked="selectedSet.has(entry.path) && !entry.deletionProtection"
                      :disabled="deleteDisabled || Boolean(entry.deletionProtection)"
                      :aria-label="t('analysis.selectItem', { name: entry.name })"
                      @update:checked="selectEntry(entry, $event)"
                    />
                  </span>
                </MdTooltip>
                <button
                  class="folder-entry"
                  type="button"
                  :disabled="openDisabled"
                  :aria-busy="deletingPath === entry.path || undefined"
                  @click="activateEntry($event, entry)"
                  @dblclick="!entry.isDirectory && emit('openEntry', entry)"
                  @keydown.enter="!entry.isDirectory && emit('openEntry', entry)"
                  @keydown="navigateRows($event, row.index)"
                >
                  <MdAnalysisEntryIcon
                    v-if="!selectionMode"
                    :entry="entry"
                    :deleting="deletingPath === entry.path"
                    compact
                  />
                  <span class="item-copy">
                    <strong class="md-result-primary">{{ entry.name }}</strong>
                    <small>
                      {{
                        deletingPath === entry.path
                          ? t('analysis.deleting')
                          : entry.sizeStatus === 'unavailable'
                            ? t('analysis.coverage.unavailable')
                            : t('common.fileCount', { count: FormatService.integer(entry.fileCount) }, entry.fileCount)
                      }}
                    </small>
                  </span>
                  <span class="item-metrics">
                    <span>
                      <strong class="md-result-primary">{{
                        entry.sizeStatus === 'unavailable'
                          ? t('analysis.coverage.unavailable')
                          : ByteSizeService.bytes(entry.bytes)
                      }}</strong>
                      <small>{{
                        entry.sizeStatus === 'unavailable'
                          ? '—'
                          : entry.bytes > 0 && FormatUtils.percent(entry.bytes, totalBytes) < 1
                            ? '<1%'
                            : `${Math.round(FormatUtils.percent(entry.bytes, totalBytes))}%`
                      }}</small>
                    </span>
                  </span>
                  <span class="chevron">
                    <MdIcon v-if="entry.isDirectory" :name="ICON_NAMES.chevronRight" :size="18" />
                  </span>
                </button>
              </div>
            </MdFileEntryContextMenu>
          </div>
        </div>
      </div>
    </MdResultTable>
  </aside>
</template>

<style scoped>
@reference "@assets/main.css";

.folder-pane {
  display: flex;
  min-width: 0;
  min-height: 0;
  flex-direction: column;
  container-type: inline-size;
  container-name: analysis-list;
  overflow: hidden;
  @apply border-border;
}

.folder-pane > header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 2px 16px 2px calc(16px + var(--layout-scrollbar-width));
}

.folder-pane header p {
  min-width: 0;
  margin: 0;
  overflow: hidden;
  @apply text-muted-foreground;
  font-size: var(--font-content-secondary);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.selection-count {
  flex: 1;
}
.folder-pane header .selection-count {
  @apply text-foreground;
}
.selection-size {
  @apply text-muted-foreground;
  white-space: nowrap;
  font-size: var(--font-content-secondary);
}
.selection-delete {
  flex: none;
  height: 26px;
  gap: 5px;
  border: 0;
  padding-inline: 6px;
  box-shadow: none;
  @apply text-destructive-text;
  font-size: var(--font-content-secondary);
}
.selection-delete:not(:disabled):hover {
  background: var(--surface-destructive-subtle);
  @apply text-destructive-text;
}
.selection-toggle {
  margin-left: auto;
  height: 26px;
  flex: none;
  padding-inline: 6px;
  font-size: var(--font-content-secondary);
}
.selection-all,
.folder-checkbox {
  display: flex;
  width: 30px;
  height: 30px;
  align-items: center;
  justify-content: center;
}
@container analysis-list (width < 380px) {
  .selection-size {
    display: none;
  }
}

.folder-list {
  min-height: 0;
  flex: 1;
}

.folder-list :deep(.result-table-scroll) {
  padding: 0 calc(6px + var(--layout-scrollbar-width)) 8px;
}

.virtual-content {
  position: relative;
}

.virtual-window {
  position: absolute;
  right: 0;
  left: 0;
}

.folder-row {
  position: relative;
  display: grid;
  width: 100%;
  height: 100%;
  grid-template-columns: 30px minmax(0, 1fr);
  align-items: center;
  gap: 8px;
  overflow: hidden;
  border-radius: var(--radius);
  padding: 4px 10px;
  isolation: isolate;
}

.folder-row::before {
  position: absolute;
  z-index: -1;
  inset: 2px auto 2px 0;
  width: var(--entry-share, 0%);
  border-radius: var(--radius) 0 0 var(--radius);
  background: var(--result-item-share-color);
  content: '';
  opacity: var(--result-item-share-opacity);
  pointer-events: none;
}

.folder-row::after {
  /* Keep the shared hover wash behind the proportional fill at its fixed opacity. */
  position: absolute;
  z-index: -2;
  inset: 0;
  border-radius: inherit;
  background: transparent;
  content: '';
  pointer-events: none;
  @apply transition-colors duration-200;
}

.folder-row:hover::after,
.folder-row:has(.folder-entry:focus-visible)::after,
.folder-row.is-highlighted::after {
  @apply bg-muted/60;
}

.folder-entry {
  display: grid;
  grid-column: 1 / -1;
  grid-row: 1;
  min-width: 0;
  min-height: var(--layout-result-row-height);
  grid-template-columns: 30px minmax(0, 1fr) max-content 14px;
  align-items: center;
  gap: 8px;
  border: 0;
  padding: 2px 0;
  background: transparent;
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
}

.folder-row.is-selecting .folder-entry {
  grid-column: 2;
  grid-template-columns: minmax(0, 1fr) max-content 14px;
}

.folder-entry:focus-visible {
  outline: none;
}

.item-copy,
.item-metrics {
  display: flex;
  min-width: 0;
  flex-direction: column;
}

.item-copy {
  gap: 3px;
}

.item-copy strong,
.item-metrics strong {
  @apply text-card-foreground;
}

.item-copy strong {
  overflow: hidden;
  font-size: var(--font-content-primary);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.item-copy small,
.item-metrics small {
  @apply text-muted-foreground;
  font-size: var(--font-content-secondary);
}

.item-metrics {
  align-items: flex-end;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.item-metrics > span {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 3px;
}

.item-metrics strong {
  font-size: var(--font-content-body);
}

.item-metrics small {
  font-size: var(--font-content-meta);
}

.chevron {
  @apply text-muted-foreground;
}

@supports not (container-type: inline-size) {
  @media (min-width: 900px) {
    .folder-pane {
      border-right-width: 1px;
      border-bottom-width: 0;
    }
  }
}
</style>
