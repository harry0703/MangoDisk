<script setup lang="ts">
import { computed, nextTick, ref, shallowRef, watch, type ComponentPublicInstance } from 'vue';
import { defaultRangeExtractor, observeElementRect, useVirtualizer, type Range } from '@tanstack/vue-virtual';
import { useI18n } from 'vue-i18n';
import MdApplicationResourceRow from './md-application-resource-row.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import { useTrayPanelStore } from '@/stores/tray-panel-store';
import {
  resourceValue,
  sortResourceRows,
  updateResourceRows,
} from '@/lib/utils/system-resources/application-resource-list';
import type { ResourceListRow, ResourceSort } from '@/lib/models/system-resources/application-resource-list';
import type {
  ProcessMemorySummary,
  ProcessCpuSummary,
  MetricStatus,
} from '@/lib/models/system-resources/system-resources';

const props = withDefaults(
  defineProps<{
    summary: ProcessMemorySummary | ProcessCpuSummary | null;
    metric?: 'cpu' | 'memory';
    active?: boolean;
    status?: MetricStatus;
  }>(),
  { metric: 'memory', status: 'ready', active: true }
);
const { t, locale } = useI18n({ useScope: 'global' });
const store = useTrayPanelStore();
void store.loadSort();
const sort = computed(() => store.sortPreferences[props.metric]);
const expandedId = ref<string | null>(null);
const focusedId = ref<string | null>(null);
const pendingApplications = new Set<string>();
const viewport = ref<HTMLElement | null>(null);
const rows = shallowRef<ResourceListRow[]>([]);
const hasHistory = computed(() => props.summary !== null);
const sampleStatus = computed(() => t(`systemStatus.${props.status}`));
const largestValue = computed(() =>
  Math.max(0, ...rows.value.filter(r => r.available).map(r => resourceValue(r.application)))
);

function update(forceSort = false) {
  const applications = props.summary?.applications ?? [];
  const present = new Set(applications.map(application => application.id));
  const discardMissing = props.metric === 'memory' && props.status === 'ready' && props.summary !== null;
  const retain = (id: string) => !discardMissing || present.has(id) || pendingApplications.has(id);
  // Ranking stability must not keep an exited memory application indefinitely.
  // A failed sample is not evidence of exit, and accepted operations must settle first.
  if (expandedId.value && !retain(expandedId.value)) expandedId.value = null;
  if (focusedId.value && !retain(focusedId.value)) focusedId.value = null;
  const locked = expandedId.value !== null;
  const next = updateResourceRows(rows.value, applications, locked).filter(row => retain(row.application.id));
  rows.value = locked && !forceSort ? next : sortResourceRows(next, sort.value, locale.value);
}
function operationPending(id: string, pending: boolean) {
  if (pending) pendingApplications.add(id);
  else {
    pendingApplications.delete(id);
    update();
  }
}
watch([() => props.summary, () => props.status], () => update(), { immediate: true });
watch([sort, locale], () => update());
watch(
  () => props.active,
  active => {
    if (!active) {
      expandedId.value = null;
      focusedId.value = null;
      update();
      // Hidden WebViews can suppress the scroll event, so re-enable the virtualizer
      // with a fresh offset rather than relying on that event to reset its range.
      if (viewport.value) viewport.value.scrollTop = 0;
    }
  }
);
function toggle(id: string) {
  expandedId.value = expandedId.value === id ? null : id;
  // Collapsing waits for the next sample before resuming automatic sorting.
}
function chooseSort(column: ResourceSort['column']) {
  void store.sortResources(props.metric, column);
  // Only an explicit user sort may move a row while details are open.
  update(true);
  void nextTick(() => {
    const index = rows.value.findIndex(row => row.application.id === expandedId.value);
    virtualizer.value.scrollToIndex(Math.max(0, index), { align: 'start' });
  });
}
const retainedIndices = computed<number[]>(previous => {
  const next = [expandedId.value, focusedId.value]
    .map(id => rows.value.findIndex(row => row.application.id === id))
    .filter(index => index >= 0);
  return previous?.length === next.length && next.every((index, n) => index === previous[n]) ? previous : next;
});
const rowKeys = computed<string[]>(previous => {
  const next = rows.value.map(row => row.application.id);
  return previous?.length === next.length && next.every((key, index) => key === previous[index]) ? previous : next;
});
const itemKey = computed(() => {
  const keys = rowKeys.value;
  return (index: number) => keys[index]!;
});
function retainRange(retained: number[]) {
  return (range: Range) => [...new Set([...defaultRangeExtractor(range), ...retained])].sort((a, b) => a - b);
}
const retainedRange = computed(() => retainRange(retainedIndices.value));
const observeVisibleRect: typeof observeElementRect<HTMLElement> = (instance, update) =>
  observeElementRect(instance, rect => {
    // Hidden native panels and layout-less test environments must not erase the viewport.
    if (rect.height > 0) update(rect);
  });
const virtualizer = useVirtualizer(
  computed(() => ({
    count: rows.value.length,
    enabled: props.active,
    getScrollElement: () => viewport.value,
    getItemKey: itemKey.value,
    estimateSize: () => 40,
    observeElementRect: observeVisibleRect,
    initialRect: { width: 360, height: 400 },
    overscan: 4,
    rangeExtractor: retainedRange.value,
  }))
);
const visibleRows = computed(() =>
  virtualizer.value.getVirtualItems().flatMap(item => {
    const row = rows.value[item.index];
    return row ? [{ item, row }] : [];
  })
);
function measure(element: Element | ComponentPublicInstance | null) {
  const node = element && '$el' in element ? element.$el : element;
  if (node instanceof HTMLElement && node.getBoundingClientRect().height > 0) virtualizer.value.measureElement(node);
}
</script>

<template>
  <section class="application-resources" :aria-label="t('monitoring.applicationName')">
    <div class="sort-columns" role="group" :aria-label="t('monitoring.applicationName')">
      <div v-for="column in ['name', 'usage'] as const" :key="column" class="sort-column">
        <button
          :aria-label="
            t('monitoring.sortBy', {
              column: t(
                column === 'name'
                  ? 'monitoring.applicationName'
                  : metric === 'cpu'
                    ? 'systemStatus.cpu'
                    : 'systemStatus.memory'
              ),
              direction: t(
                sort.column === column && sort.direction === 'ascending'
                  ? 'monitoring.sortDescending'
                  : column === 'name' || sort.column === column
                    ? 'monitoring.sortAscending'
                    : 'monitoring.sortDescending'
              ),
            })
          "
          :aria-pressed="sort.column === column"
          @click="chooseSort(column)"
        >
          {{
            t(
              column === 'name'
                ? 'monitoring.applicationName'
                : metric === 'cpu'
                  ? 'systemStatus.cpu'
                  : 'systemStatus.memory'
            )
          }}
          <MdIcon
            :class="{ 'inactive-arrow': sort.column !== column }"
            :name="sort.direction === 'ascending' ? ICON_NAMES.chevronUp : ICON_NAMES.chevronDown"
            :size="12"
          />
        </button>
        <MdTooltip
          v-if="column === 'name' && hasHistory && ['failed', 'disconnected', 'unsupported'].includes(status)"
          :active="active"
          :text="sampleStatus"
        >
          <span class="reading-error" tabindex="0" role="status" :aria-label="sampleStatus">{{ sampleStatus }}</span>
        </MdTooltip>
      </div>
    </div>
    <p v-if="status !== 'ready' && !hasHistory" class="list-empty" role="status">
      {{ t(status === 'loading' ? 'monitoring.loadingApplications' : `systemStatus.${status}`) }}
    </p>
    <p v-else-if="!summary" class="list-empty" role="status">{{ t('monitoring.loadingApplications') }}</p>
    <p v-else-if="!rows.length" class="list-empty">{{ t('monitoring.noApplications') }}</p>
    <div
      v-else
      ref="viewport"
      class="list-viewport scrollbar-stable-end"
      tabindex="0"
      :aria-label="t('monitoring.applicationName')"
    >
      <ol :style="{ height: `${virtualizer.getTotalSize()}px` }">
        <MdApplicationResourceRow
          v-for="{ item, row } in visibleRows"
          :key="row.application.id"
          :ref="measure"
          :data-index="item.index"
          :data-application-id="row.application.id"
          :style="{ position: 'absolute', top: `${item.start}px`, width: '100%' }"
          :application="row.application"
          :available="row.available"
          :members="row.members"
          :metric="metric"
          :active="active"
          :expanded="expandedId === row.application.id"
          :share="row.available && largestValue > 0 ? (resourceValue(row.application) / largestValue) * 100 : 0"
          @focusin="focusedId = row.application.id"
          @focusout="focusedId = null"
          @toggle="toggle(row.application.id)"
          @operation-pending="pending => operationPending(row.application.id, pending)"
        />
      </ol>
    </div>
  </section>
</template>

<style scoped>
@reference "@assets/main.css";
.application-resources {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  overflow: hidden;
}
.sort-columns {
  display: flex;
  justify-content: space-between;
  flex: none;
  margin: 0 12px 4px 0;
}
.sort-column {
  display: flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
}
.sort-columns button {
  @apply text-muted-foreground;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border-radius: 4px;
  padding: 3px 6px;
  font-size: 12px;
  cursor: pointer;
}
.sort-columns button[aria-pressed='true'] {
  @apply text-foreground;
}
.sort-columns button:hover {
  @apply bg-accent;
}
.inactive-arrow {
  visibility: hidden;
}
.list-viewport {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding-right: 12px;
  overflow-anchor: none;
}
ol {
  position: relative;
  list-style: none;
  padding: 0;
  margin: 0;
}
button:focus-visible,
.list-viewport:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: -2px;
}
.list-empty {
  @apply text-muted-foreground;
  padding: 20px 0;
  text-align: center;
  font-size: 12px;
  display: grid;
  place-items: center;
  flex: 1;
  min-height: 0;
}
.reading-error {
  @apply rounded bg-muted text-muted-foreground;
  font-size: 9px;
  padding: 0 4px;
  flex: none;
  cursor: help;
}
.reading-error:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}
</style>
