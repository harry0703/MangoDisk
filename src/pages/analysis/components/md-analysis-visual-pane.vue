<script setup lang="ts">
import { FormatService } from '@/lib/services/presentation/format-service';
import { useI18n } from 'vue-i18n';
import { computed, onDeactivated, ref, watch } from 'vue';

import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdScanExclusionLink from '@/components/custom/md-scan-exclusion-link.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { ANALYSIS_CHART_MAX_DEPTH, ANALYSIS_SCAN_MODES, ANALYSIS_VIEW_IDS } from '@/lib/models/storage/analysis';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import type {
  AnalysisRemainderSelection,
  AnalysisResult,
  AnalysisViewId,
  DirectoryEntryInfo,
} from '@/lib/models/storage/analysis';
import { ByteSizeService } from '@/lib/services/settings/byte-size-service';

import MdAnalysisSunburst from './md-analysis-sunburst.vue';
import MdAnalysisTreemap from './md-analysis-treemap.vue';
import MdAnalysisOtherDialog from './md-analysis-other-dialog.vue';

const { t } = useI18n({ useScope: 'global' });
const treemapDepth = defineModel<number>('treemapDepth', { default: 1 });
const sunburstDepth = defineModel<number>('sunburstDepth', { default: 3 });

const props = defineProps<{
  result: AnalysisResult;
  exclusionsActive: boolean;
  entries: DirectoryEntryInfo[];
  folderCount: number;
  viewMode: AnalysisViewId;
  openDisabled: boolean;
  deleteDisabled: boolean;
  deletingPath?: string | null;
  hoveredEntryPath?: string | null;
}>();

const emit = defineEmits<{
  activate: [entry: DirectoryEntryInfo];
  openEntry: [entry: DirectoryEntryInfo];
  reveal: [path: string];
  delete: [entry: DirectoryEntryInfo];
  hoverEntry: [path: string | null];
  navigate: [path: string];
  refreshDirectory: [path: string];
  openExclusions: [];
  'update:viewMode': [viewMode: AnalysisViewId];
}>();
const remainderSelection = ref<AnalysisRemainderSelection | null>(null);
function showRemainder(selection: AnalysisRemainderSelection) {
  if (props.openDisabled) return;
  emit('hoverEntry', null);
  remainderSelection.value = selection;
}
watch([() => props.result, () => props.openDisabled, () => props.viewMode], () => {
  remainderSelection.value = null;
});
onDeactivated(() => {
  remainderSelection.value = null;
});
const minimumDepth = computed(() => (props.viewMode === ANALYSIS_VIEW_IDS.treemap ? 1 : 2));
const depth = computed(() => (props.viewMode === ANALYSIS_VIEW_IDS.treemap ? treemapDepth.value : sunburstDepth.value));
const depthLevels = computed(() =>
  Array.from({ length: ANALYSIS_CHART_MAX_DEPTH - minimumDepth.value + 1 }, (_, index) => index + minimumDepth.value)
);
function updateDepth(value: unknown) {
  const level = Number(value);
  if (!Number.isInteger(level) || level < minimumDepth.value || level > ANALYSIS_CHART_MAX_DEPTH) return;
  if (props.viewMode === ANALYSIS_VIEW_IDS.treemap) treemapDepth.value = level;
  else sunburstDepth.value = level;
}
</script>

<template>
  <section class="visual-pane">
    <header class="md-workspace-toolbar">
      <div class="space-summary">
        <p
          :aria-label="
            t(
              'analysis.folderSpaceSummary',
              { folders: FormatService.integer(folderCount), size: ByteSizeService.bytes(result.totalBytes) },
              folderCount
            )
          "
        >
          <strong>{{ ByteSizeService.bytes(result.totalBytes) }}</strong>
          <MdTooltip
            :text="
              t(
                result.scanMode === ANALYSIS_SCAN_MODES.fast
                  ? 'analysis.scanMode.fastHint'
                  : 'analysis.scanMode.standardDescription'
              )
            "
          >
            <span>{{
              t(result.scanMode === ANALYSIS_SCAN_MODES.fast ? 'analysis.fileSize' : 'analysis.diskUsage')
            }}</span>
          </MdTooltip>
        </p>
        <slot name="coverage" />
        <MdScanExclusionLink
          v-if="exclusionsActive"
          :hint="t('analysis.exclusionHint')"
          @open="emit('openExclusions')"
        />
      </div>
      <div class="chart-controls">
        <Select :model-value="String(depth)" :disabled="openDisabled" @update:model-value="updateDepth">
          <SelectTrigger size="sm" class="chart-depth" :aria-label="t('analysis.sunburstDepth')">
            <SelectValue>{{ t('analysis.sunburstLevels', { count: depth }, depth) }}</SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem v-for="level in depthLevels" :key="level" :value="String(level)">
              {{ t('analysis.sunburstLevels', { count: level }, level) }}
            </SelectItem>
          </SelectContent>
        </Select>
        <div class="view-switcher" role="group" :aria-label="t('analysis.result')">
          <button
            type="button"
            :class="{ active: props.viewMode === ANALYSIS_VIEW_IDS.treemap }"
            :aria-pressed="props.viewMode === ANALYSIS_VIEW_IDS.treemap"
            @click="emit('update:viewMode', ANALYSIS_VIEW_IDS.treemap)"
          >
            <MdIcon :name="ICON_NAMES.chartTreemap" :size="15" />
            {{ t('analysis.treemap') }}
          </button>
          <button
            type="button"
            :class="{ active: props.viewMode === ANALYSIS_VIEW_IDS.sunburst }"
            :aria-pressed="props.viewMode === ANALYSIS_VIEW_IDS.sunburst"
            @click="emit('update:viewMode', ANALYSIS_VIEW_IDS.sunburst)"
          >
            <MdIcon :name="ICON_NAMES.chartSunburst" :size="15" />
            {{ t('analysis.sunburst') }}
          </button>
        </div>
      </div>
    </header>

    <KeepAlive>
      <MdAnalysisTreemap
        v-if="props.viewMode === ANALYSIS_VIEW_IDS.treemap"
        :depth="treemapDepth"
        :result="result"
        :open-disabled="openDisabled"
        :delete-disabled="deleteDisabled"
        :deleting-path="deletingPath"
        :hovered-entry-path="hoveredEntryPath"
        @hover-entry="emit('hoverEntry', $event)"
        @navigate="emit('navigate', $event)"
        @activate="emit('activate', $event)"
        @open-entry="emit('openEntry', $event)"
        @reveal="emit('reveal', $event)"
        @delete="emit('delete', $event)"
        @show-remainder="showRemainder"
      />
      <MdAnalysisSunburst
        v-else
        :depth="sunburstDepth"
        :result="result"
        :delete-disabled="deleteDisabled"
        :deleting-path="deletingPath"
        :open-disabled="openDisabled"
        :hovered-entry-path="hoveredEntryPath"
        @navigate="emit('navigate', $event)"
        @hover-entry="emit('hoverEntry', $event)"
        @open-entry="emit('openEntry', $event)"
        @reveal="emit('reveal', $event)"
        @delete="emit('delete', $event)"
        @show-remainder="showRemainder"
      />
    </KeepAlive>
    <MdAnalysisOtherDialog
      :scan-id="result.scanId"
      :scan-mode="result.scanMode"
      :selection="remainderSelection"
      @close="remainderSelection = null"
      @navigate="emit('navigate', $event)"
      @refresh-directory="emit('refreshDirectory', $event)"
    />
  </section>
</template>

<style scoped>
@reference "@assets/main.css";

.visual-pane {
  container: analysis-visual / inline-size;
  display: flex;
  min-width: 0;
  min-height: 0;
  flex-direction: column;
  overflow: hidden;
}

.visual-pane > header {
  display: flex;
  height: auto;
  flex: none;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
  padding: 2px 12px;
}

.chart-controls {
  display: flex;
  min-width: 0;
  margin-left: auto;
  align-items: center;
  gap: 8px;
  max-width: 100%;
  flex-wrap: wrap;
  justify-content: flex-end;
}

.chart-depth {
  width: auto;
  height: var(--layout-workspace-control-height);
  flex: none;
  font-size: var(--font-content-secondary);
  @apply border-border/50 bg-transparent text-muted-foreground shadow-none;
}

.chart-depth:hover:not(:disabled),
.chart-depth[data-state='open'] {
  @apply border-border bg-accent/35 text-foreground;
}

.chart-depth[data-state='open'] {
  @apply ring-0;
}

.chart-depth:focus-visible {
  @apply border-ring/60 ring-2 ring-ring/25;
}

.chart-depth :deep(svg) {
  opacity: 0.4;
}

.chart-depth:hover:not(:disabled) :deep(svg),
.chart-depth:focus-visible :deep(svg),
.chart-depth[data-state='open'] :deep(svg) {
  opacity: 0.55;
}

.space-summary {
  display: flex;
  min-width: 0;
  align-items: baseline;
  gap: 8px;
  max-width: 100%;
  flex-wrap: wrap;
  row-gap: 4px;
}

.visual-pane header p {
  display: flex;
  min-width: 0;
  align-items: baseline;
  gap: 8px;
  margin: 0;
  overflow: hidden;
  @apply text-muted-foreground;
  font-size: var(--font-content-secondary);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.space-summary strong {
  @apply text-card-foreground;
  font-size: var(--font-content-section-title);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.space-summary p > span {
  overflow: hidden;
  text-overflow: ellipsis;
}

.view-switcher {
  display: flex;
  height: var(--layout-workspace-control-height);
  flex: none;
  overflow: hidden;
  border-width: 1px;
  border-radius: 8px;
  @apply border-border;
}

.view-switcher button {
  display: flex;
  height: 100%;
  align-items: center;
  gap: 6px;
  border: 0;
  border-left-width: 1px;
  padding: 0 11px;
  @apply border-border bg-card text-muted-foreground transition-colors duration-200;
  font: inherit;
  font-size: var(--font-content-secondary);
  cursor: pointer;
}

.view-switcher button:first-child {
  border-left: 0;
}

.view-switcher button:hover:not(.active) {
  @apply border-primary/40 bg-accent/65 text-accent-foreground;
}

.view-switcher button.active {
  background: var(--surface-primary-subtle);
  @apply text-primary-text;
}

.view-switcher button:focus-visible {
  position: relative;
  z-index: 1;
  @apply outline-none ring-2 ring-inset ring-ring/45;
}
</style>
