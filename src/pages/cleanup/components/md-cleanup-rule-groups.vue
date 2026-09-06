<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';

import MdLoadMoreButton from '@/components/custom/md-load-more-button.vue';
import MdApplicationIcon from '@/components/custom/md-application-icon.vue';
import MdIconAction from '@/components/custom/md-icon-action.vue';
import MdMiddleEllipsis from '@/components/custom/md-middle-ellipsis.vue';
import MdResultCategoryItem from '@/components/custom/md-result-category-item.vue';
import MdResultMasterDetail from '@/components/custom/md-result-master-detail.vue';
import MdResultCheckbox from '@/components/custom/md-result-checkbox.vue';
import MdResultDetailHeader from '@/components/custom/md-result-detail-header.vue';
import MdResultItemContent from '@/components/custom/md-result-item-content.vue';
import MdResultTable from '@/components/custom/md-result-table.vue';
import MdResultTableHierarchy from '@/components/custom/md-result-table-hierarchy.vue';
import MdResultTableRow from '@/components/custom/md-result-table-row.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { Button } from '@/components/ui/button';
import type { ApplicationLeftoverCandidate, ApplicationLeftoverScanResult } from '@/lib/models/application';
import {
  CLEANUP_RULE_IDS,
  type CleanupResultGroup,
  type CleanupSourceSelection,
  type PresentedScanRuleResult,
} from '@/lib/models/cleanup';
import { ICON_NAMES } from '@/lib/models/ui';
import { ApplicationIconService } from '@/lib/services/application-icon-service';
import * as CleanupRuleSelectionUtils from '@/lib/utils/cleanup-rule-selection';
import { ByteSizeService } from '@/lib/services/byte-size-service';
import * as FormatUtils from '@/lib/utils/format';
import * as PathUtils from '@/lib/utils/path';
import * as RenderBatchUtils from '@/lib/utils/render-batch';

import { applicationLeftoverGroupSelection, groupApplicationLeftovers } from '../application-leftover-groups';
import { hasCleanupRuleDetails, isAggregateOnlyCleanupRule } from '../cleanup-rule-details';
import { cleanupGroupIcon, cleanupRuleIcon } from '../cleanup-rule-icon';
import { buildCleanupResultCategories, type CleanupResultCategory } from '../cleanup-result-categories';
import MdAiAction from '@/components/custom/md-ai-action.vue';
import { OperatingSystemService } from '@/lib/services/operating-system-service';
import { cleanupAiContext } from '../cleanup-ai-context';
import { useAiStore } from '@/stores/ai-store';

const LEFTOVER_VIEW_ID = 'application-leftovers';
const CLEANUP_CHILD_INITIAL_RENDER_COUNT = 10;
const CLEANUP_CHILD_RENDER_BATCH_SIZE = 50;
type CleanupViewId = CleanupResultGroup | typeof LEFTOVER_VIEW_ID;
type ApplicationLeftoverGroup = ReturnType<typeof groupApplicationLeftovers>[number];
type CleanupNavigationItem =
  | { kind: 'category'; id: CleanupResultGroup; category: CleanupResultCategory }
  | { kind: 'leftovers'; id: typeof LEFTOVER_VIEW_ID };

const { locale, t } = useI18n({ useScope: 'global' });
const aiStore = useAiStore();
function explainRule(rule: PresentedScanRuleResult) {
  const context = cleanupAiContext(rule, OperatingSystemService.isWindows() ? 'windows' : 'macos');
  if (context) void aiStore.show(context, locale.value);
}
const props = withDefaults(
  defineProps<{
    busy: boolean;
    leftovers: ApplicationLeftoverScanResult | null;
    rules: PresentedScanRuleResult[];
    selectedLeftoverIds: string[];
    selectedRuleIds: string[];
    sourceSelections: CleanupSourceSelection[];
    privilegedScanRuleId: string | null;
    embedded?: boolean;
  }>(),
  { embedded: false }
);
const emit = defineEmits<{
  open: [path: string];
  selectLeftoverGroup: [candidateIds: string[], selected: boolean];
  selectAll: [ruleIds: string[], selected: boolean];
  toggleLeftover: [candidate: ApplicationLeftoverCandidate];
  toggleSource: [ruleId: string, path: string];
  privilegedScan: [];
}>();

const activeViewId = ref<CleanupViewId>('system');
const expandedRuleIds = ref<ReadonlySet<string>>(new Set());
const expandedLeftoverIds = ref<ReadonlySet<string>>(new Set());
const visibleSourceCounts = ref<Record<string, number>>({});
const visibleLeftoverCounts = ref<Record<string, number>>({});
const applicationIcons = ref<ReadonlyMap<string, string>>(new Map());
const detailList = ref<InstanceType<typeof MdResultTable> | null>(null);
let iconRequestVersion = 0;

const categories = computed(() =>
  buildCleanupResultCategories(props.rules, props.selectedRuleIds, props.sourceSelections)
);
const activeCategory = computed(() => categories.value.find(category => category.id === activeViewId.value) ?? null);
const activeRuleRows = computed(() =>
  (activeCategory.value?.rules ?? []).map(rule => ({
    rule,
    selectedBytes: selectedBytes(rule),
    selection: ruleSelectionLevel(rule),
  }))
);
const applicationOptimizationRule = computed(
  () => activeCategory.value?.rules.find(rule => isUniversalBinaryRule(rule)) ?? null
);
const applicationOptimizationSources = computed(() => applicationOptimizationRule.value?.sources ?? []);
const showingApplicationOptimization = computed(() => activeCategory.value?.id === 'applicationOptimization');
const showingLeftovers = computed(() => activeViewId.value === LEFTOVER_VIEW_ID);
const leftoverCandidates = computed(() => props.leftovers?.candidates ?? []);
const selectedLeftoverSet = computed(() => new Set(props.selectedLeftoverIds));
const leftoverGroups = computed(() =>
  groupApplicationLeftovers(leftoverCandidates.value).map(group => ({
    ...group,
    selection: applicationLeftoverGroupSelection(group.candidateIds, selectedLeftoverSet.value),
  }))
);
const hasLeftoverView = computed(() => Boolean(props.leftovers?.supported && leftoverGroups.value.length));
const navigationItems = computed<CleanupNavigationItem[]>(() => {
  const items: CleanupNavigationItem[] = categories.value.map(category => ({
    kind: 'category',
    id: category.id,
    category,
  }));
  if (!hasLeftoverView.value) return items;

  // Application leftovers belong with other application maintenance entries,
  // not after specialized AI and developer categories. Prefer placing them
  // after browser data; fall back to application caches when browsers are absent.
  let anchorIndex = items.findIndex(item => item.kind === 'category' && item.id === 'browser');
  if (anchorIndex < 0) {
    anchorIndex = items.findIndex(item => item.kind === 'category' && item.id === 'application');
  }
  items.splice(anchorIndex >= 0 ? anchorIndex + 1 : Math.min(3, items.length), 0, {
    kind: 'leftovers',
    id: LEFTOVER_VIEW_ID,
  });
  return items;
});
const leftoverSelection = computed(() =>
  applicationLeftoverGroupSelection(
    leftoverCandidates.value.map(candidate => candidate.candidateId),
    selectedLeftoverSet.value
  )
);
const selectedLeftoverBytes = computed(() =>
  leftoverCandidates.value.reduce(
    (total, candidate) => total + (selectedLeftoverSet.value.has(candidate.candidateId) ? candidate.bytes : 0),
    0
  )
);

function categoryTitle(category: CleanupResultGroup): string {
  return t(`cleanup.categoryTitles.${category}`);
}

function categoryItemCount(count: number): string {
  return t('cleanup.categoryItemCount', { count: FormatUtils.integer(count) }, count);
}

function categoryRuleIds(category: CleanupResultCategory): string[] {
  return category.rules.filter(rule => rule.selectable).map(rule => rule.ruleId);
}

function isUniversalBinaryRule(rule: PresentedScanRuleResult): boolean {
  return rule.ruleId === CLEANUP_RULE_IDS.macosUniversalBinaries;
}

function ruleSelectionLevel(rule: PresentedScanRuleResult) {
  return CleanupRuleSelectionUtils.ruleSelectionLevel(rule, props.selectedRuleIds, props.sourceSelections);
}

function sourceSelected(ruleId: string, path: string): boolean {
  return CleanupRuleSelectionUtils.sourceSelected(ruleId, path, props.selectedRuleIds, props.sourceSelections);
}

function sourceBlockReason(reason: PresentedScanRuleResult['sources'][number]['blockReason']): string {
  return reason === 'requiresClose' ? t('cleanup.sourceRequiresClose') : t('cleanup.sourceAccessLimited');
}

function selectedBytes(rule: PresentedScanRuleResult): number {
  return CleanupRuleSelectionUtils.selectedBytesForRule(rule, props.selectedRuleIds, props.sourceSelections);
}

function ruleValueDetail(
  rule: PresentedScanRuleResult,
  selection: 'all' | 'partial' | 'none',
  selectedRuleBytes: number
): string {
  if (rule.status === 'requiresElevation') return t('cleanup.privilegedScan.sizePending');
  if (selection === 'none') return t('cleanup.cleanableFound');
  if (selectedRuleBytes !== rule.bytes) return t('cleanup.totalSize', { size: ByteSizeService.bytes(rule.bytes) });
  return t('cleanup.selected');
}

function visibleRuleSources(rule: PresentedScanRuleResult) {
  const visibleCount = visibleSourceCounts.value[rule.ruleId] ?? CLEANUP_CHILD_INITIAL_RENDER_COUNT;
  return RenderBatchUtils.visibleItems(rule.sources, visibleCount);
}

function remainingRuleSourceCount(rule: PresentedScanRuleResult): number {
  return RenderBatchUtils.remainingCount(rule.sources.length, visibleRuleSources(rule).length);
}

function loadMoreRuleSources(rule: PresentedScanRuleResult) {
  const current = visibleSourceCounts.value[rule.ruleId] ?? CLEANUP_CHILD_INITIAL_RENDER_COUNT;
  visibleSourceCounts.value = {
    ...visibleSourceCounts.value,
    [rule.ruleId]: RenderBatchUtils.nextVisibleCount(current, rule.sources.length, CLEANUP_CHILD_RENDER_BATCH_SIZE),
  };
}

function visibleLeftoverCandidates(group: ApplicationLeftoverGroup) {
  const visibleCount = visibleLeftoverCounts.value[group.applicationIdentifier] ?? CLEANUP_CHILD_INITIAL_RENDER_COUNT;
  return RenderBatchUtils.visibleItems(group.candidates, visibleCount);
}

function remainingLeftoverCandidateCount(group: ApplicationLeftoverGroup): number {
  return RenderBatchUtils.remainingCount(group.candidates.length, visibleLeftoverCandidates(group).length);
}

function loadMoreLeftoverCandidates(group: ApplicationLeftoverGroup) {
  const current = visibleLeftoverCounts.value[group.applicationIdentifier] ?? CLEANUP_CHILD_INITIAL_RENDER_COUNT;
  visibleLeftoverCounts.value = {
    ...visibleLeftoverCounts.value,
    [group.applicationIdentifier]: RenderBatchUtils.nextVisibleCount(
      current,
      group.candidates.length,
      CLEANUP_CHILD_RENDER_BATCH_SIZE
    ),
  };
}

function runningProcessWarning(rule: PresentedScanRuleResult): string {
  if (!rule.runningProcesses.length) return t('cleanup.requiresClose');
  return t('cleanup.requiresCloseProcesses', {
    processes: FormatUtils.list(rule.runningProcesses, locale.value),
  });
}

function toggleCategory(category: CleanupResultCategory, checked: boolean) {
  emit('selectAll', categoryRuleIds(category), checked);
}

function toggleRule(rule: PresentedScanRuleResult, checked: boolean) {
  emit('selectAll', [rule.ruleId], checked);
}

async function toggleRuleDetails(rule: PresentedScanRuleResult) {
  if (!hasCleanupRuleDetails(rule)) return;
  const next = new Set(expandedRuleIds.value);
  if (next.has(rule.ruleId)) {
    next.delete(rule.ruleId);
  } else {
    next.add(rule.ruleId);
  }
  expandedRuleIds.value = next;

  if (!next.has(rule.ruleId) || !isUniversalBinaryRule(rule)) return;
  const requestVersion = ++iconRequestVersion;
  const icons = await ApplicationIconService.resolve(rule.sources.map(source => source.path));
  if (requestVersion === iconRequestVersion) applicationIcons.value = icons;
}

function toggleLeftoverGroupDetails(applicationIdentifier: string) {
  const next = new Set(expandedLeftoverIds.value);
  if (next.has(applicationIdentifier)) next.delete(applicationIdentifier);
  else next.add(applicationIdentifier);
  expandedLeftoverIds.value = next;
}

function toggleLeftoverGroup(candidateIds: string[]) {
  const selection = applicationLeftoverGroupSelection(candidateIds, selectedLeftoverSet.value);
  emit('selectLeftoverGroup', candidateIds, selection !== 'all');
}

function toggleAllLeftovers(checked: boolean) {
  emit(
    'selectLeftoverGroup',
    leftoverCandidates.value.map(candidate => candidate.candidateId),
    checked
  );
}

function handleApplicationIconError(path: string) {
  const icons = new Map(applicationIcons.value);
  icons.delete(path);
  applicationIcons.value = icons;
}

function applicationName(path: string): string {
  return PathUtils.fileName(path).replace(/\.app$/iu, '');
}

async function loadApplicationOptimizationIcons() {
  const paths = applicationOptimizationSources.value.map(source => source.path);
  if (!paths.length) return;
  const requestVersion = ++iconRequestVersion;
  const icons = await ApplicationIconService.resolve(paths);
  if (requestVersion === iconRequestVersion) applicationIcons.value = icons;
}

watch(
  () => navigationItems.value.map(item => item.id),
  viewIds => {
    if (!viewIds.includes(activeViewId.value)) activeViewId.value = viewIds[0] ?? 'system';
  },
  { immediate: true }
);

watch(
  activeViewId,
  async () => {
    await nextTick();
    detailList.value?.scrollTo({ top: 0 });
    visibleSourceCounts.value = {};
    visibleLeftoverCounts.value = {};

    if (showingApplicationOptimization.value) await loadApplicationOptimizationIcons();

    // A category with one cleanup rule has no useful intermediate level.
    // Reveal its locations immediately so the result behaves like a direct
    // category-to-item browser while preserving explicit disclosure for
    // categories containing several independent rules.
    const rules = activeCategory.value?.rules ?? [];
    if (rules.length === 1 && hasCleanupRuleDetails(rules[0])) {
      expandedRuleIds.value = new Set([...expandedRuleIds.value, rules[0].ruleId]);
    }
  },
  { immediate: true }
);
</script>

<template>
  <MdResultMasterDetail :class="{ embedded }" :empty="!navigationItems.length">
    <template v-if="navigationItems.length" #navigation>
      <template v-for="item in navigationItems" :key="item.id">
        <MdResultCategoryItem
          v-if="item.kind === 'category'"
          :active="activeViewId === item.id"
          :title="categoryTitle(item.category.id)"
          :description="`${ByteSizeService.bytes(item.category.bytes)} · ${categoryItemCount(item.category.rules.length)}`"
          :icon-name="cleanupGroupIcon(item.category.id)"
          :selected-summary="
            item.category.selection === 'none' ? undefined : ByteSizeService.bytes(item.category.selectedBytes)
          "
          :selected-aria-label="t(`cleanup.selectionState.${item.category.selection}`)"
          @select="activeViewId = item.id"
        />

        <MdResultCategoryItem
          v-else-if="leftovers"
          :active="showingLeftovers"
          :title="t('applicationLeftovers.categoryTitle')"
          :description="`${ByteSizeService.bytes(leftovers.totalBytes)} · ${categoryItemCount(leftoverGroups.length)}`"
          :icon-name="ICON_NAMES.application"
          :selected-summary="leftoverSelection === 'none' ? undefined : ByteSizeService.bytes(selectedLeftoverBytes)"
          :selected-aria-label="t(`cleanup.selectionState.${leftoverSelection}`)"
          @select="activeViewId = LEFTOVER_VIEW_ID"
        />
      </template>
    </template>

    <section v-if="showingLeftovers && leftovers" class="cleanup-details">
      <MdResultDetailHeader
        :title="t('applicationLeftovers.categoryTitle')"
        :selection="leftoverSelection"
        :select-label="t('cleanup.selectAll')"
        :disabled="busy || !leftoverCandidates.length"
        @update:selected="toggleAllLeftovers"
      >
        <template #metric>
          <small>{{ t('cleanup.selected') }} / {{ t('cleanup.cleanableFound') }}</small>
          <strong>{{ ByteSizeService.bytes(selectedLeftoverBytes) }}</strong>
          <i>/ {{ ByteSizeService.bytes(leftovers.totalBytes) }}</i>
        </template>
      </MdResultDetailHeader>

      <div v-if="leftovers.accessLimited || !leftovers.inventoryComplete" class="detail-warning">
        <MdIcon :name="ICON_NAMES.info" :size="15" />
        {{
          leftovers.accessLimited
            ? t('applicationLeftovers.accessLimitedDescription')
            : t('applicationLeftovers.incompleteDescription')
        }}
      </div>

      <MdResultTable ref="detailList" class="detail-list">
        <div class="detail-list-content">
          <article v-for="group in leftoverGroups" :key="group.applicationIdentifier" class="rule-card">
            <MdResultTableRow
              layout="item"
              class="rule-summary md-ai-hover-row"
              :data-selected="group.selection !== 'none'"
            >
              <MdResultCheckbox
                :checked="group.selection === 'all'"
                :indeterminate="group.selection === 'partial'"
                :disabled="busy"
                :aria-label="t('applicationLeftovers.selectCandidate', { name: group.applicationName })"
                @update:checked="toggleLeftoverGroup(group.candidateIds)"
              />
              <button
                class="rule-disclosure"
                type="button"
                :aria-expanded="expandedLeftoverIds.has(group.applicationIdentifier)"
                @click="toggleLeftoverGroupDetails(group.applicationIdentifier)"
              >
                <MdResultItemContent
                  :title="group.applicationName"
                  :description="`${group.applicationIdentifier} · ${t(
                    'applicationLeftovers.locationCount',
                    { count: group.candidates.length },
                    group.candidates.length
                  )}`"
                  :value="ByteSizeService.bytes(group.bytes)"
                  :expandable="true"
                  :expanded="expandedLeftoverIds.has(group.applicationIdentifier)"
                >
                  <template #icon><MdIcon :name="ICON_NAMES.application" :size="20" /></template>
                </MdResultItemContent>
              </button>
            </MdResultTableRow>

            <MdResultTableHierarchy v-if="expandedLeftoverIds.has(group.applicationIdentifier)">
              <MdResultTableRow
                v-for="candidate in visibleLeftoverCandidates(group)"
                :key="candidate.candidateId"
                class="source-row"
                :data-selected="selectedLeftoverSet.has(candidate.candidateId)"
              >
                <MdResultCheckbox
                  :checked="selectedLeftoverSet.has(candidate.candidateId)"
                  :disabled="busy"
                  :aria-label="
                    t('applicationLeftovers.selectLocation', {
                      name: t(`applicationLeftovers.sources.${candidate.source}`),
                    })
                  "
                  @update:checked="emit('toggleLeftover', candidate)"
                />
                <span class="source-primary">
                  <span class="source-main">
                    <strong class="md-result-primary">{{
                      t(`applicationLeftovers.sources.${candidate.source}`)
                    }}</strong>
                    <MdMiddleEllipsis :text="PathUtils.display(candidate.path)" :tail-length="22" />
                    <small>{{ t('cleanup.sourceFiles', { count: FormatUtils.integer(candidate.fileCount) }) }}</small>
                  </span>
                  <span class="source-actions">
                    <MdIconAction
                      variant="ghost"
                      :label="t('common.showInFileManager')"
                      @click="emit('open', candidate.path)"
                    >
                      <MdIcon :name="ICON_NAMES.folder" :size="16" />
                    </MdIconAction>
                  </span>
                </span>
                <strong class="md-result-primary">{{ ByteSizeService.bytes(candidate.bytes) }}</strong>
              </MdResultTableRow>
              <template v-if="remainingLeftoverCandidateCount(group)" #footer>
                <MdLoadMoreButton
                  class="source-load-more"
                  :remaining-label="
                    t(
                      'common.locationCount',
                      { count: FormatUtils.integer(remainingLeftoverCandidateCount(group)) },
                      remainingLeftoverCandidateCount(group)
                    )
                  "
                  :disabled="busy"
                  @load-more="loadMoreLeftoverCandidates(group)"
                />
              </template>
            </MdResultTableHierarchy>
          </article>
          <div v-if="!leftoverGroups.length" class="empty-detail">
            <MdIcon :name="ICON_NAMES.check" :size="22" />
            <strong>{{ t('applicationLeftovers.emptyTitle') }}</strong>
            <small>{{ t('applicationLeftovers.emptyDescription') }}</small>
          </div>
        </div>
      </MdResultTable>
    </section>

    <section
      v-else-if="activeCategory && showingApplicationOptimization && applicationOptimizationRule"
      class="cleanup-details"
    >
      <MdResultDetailHeader
        :title="categoryTitle(activeCategory.id)"
        :description="t('cleanup.categoryDescriptions.applicationOptimization')"
        :selection="activeCategory.selection"
        :select-label="t('cleanup.selectAll')"
        :disabled="busy || !categoryRuleIds(activeCategory).length"
        @update:selected="toggleCategory(activeCategory, $event)"
      >
        <template #metric>
          <small>{{ t('cleanup.selected') }} / {{ t('cleanup.cleanableFound') }}</small>
          <strong>{{ ByteSizeService.bytes(activeCategory.selectedBytes) }}</strong>
          <i>/ {{ ByteSizeService.bytes(activeCategory.bytes) }}</i>
        </template>
      </MdResultDetailHeader>

      <MdResultTable ref="detailList" class="detail-list">
        <div class="detail-list-content application-list-content">
          <MdResultTableRow
            v-for="source in applicationOptimizationSources"
            :key="source.path"
            class="application-row"
            :data-selected="sourceSelected(applicationOptimizationRule.ruleId, source.path)"
          >
            <MdResultCheckbox
              :checked="!source.blockReason && sourceSelected(applicationOptimizationRule.ruleId, source.path)"
              :disabled="busy || Boolean(source.blockReason)"
              :aria-label="t('cleanup.selectSource', { path: source.path })"
              @update:checked="emit('toggleSource', applicationOptimizationRule.ruleId, source.path)"
            />
            <MdApplicationIcon
              :src="applicationIcons.get(source.path)"
              :size="44"
              :artwork-size="40"
              @error="handleApplicationIconError(source.path)"
            />
            <span class="application-primary">
              <span class="application-main">
                <strong class="md-result-primary">{{ applicationName(source.path) }}</strong>
                <small v-if="source.blockReason">{{ sourceBlockReason(source.blockReason) }}</small>
                <small v-else>{{ t('cleanup.applicationComponentCount', { count: source.fileCount }) }}</small>
              </span>
              <span class="source-actions">
                <MdIconAction variant="ghost" :label="t('common.showInFileManager')" @click="emit('open', source.path)">
                  <MdIcon :name="ICON_NAMES.folder" :size="16" />
                </MdIconAction>
              </span>
            </span>
            <strong class="application-size md-result-primary">{{ ByteSizeService.bytes(source.bytes) }}</strong>
          </MdResultTableRow>
        </div>
      </MdResultTable>
    </section>

    <section v-else-if="activeCategory" class="cleanup-details">
      <MdResultDetailHeader
        :title="categoryTitle(activeCategory.id)"
        :selection="activeCategory.selection"
        :select-label="t('cleanup.selectAll')"
        :disabled="busy || !categoryRuleIds(activeCategory).length"
        @update:selected="toggleCategory(activeCategory, $event)"
      >
        <template #metric>
          <small>{{ t('cleanup.selected') }} / {{ t('cleanup.cleanableFound') }}</small>
          <strong>{{ ByteSizeService.bytes(activeCategory.selectedBytes) }}</strong>
          <i>/ {{ ByteSizeService.bytes(activeCategory.bytes) }}</i>
        </template>
      </MdResultDetailHeader>

      <MdResultTable ref="detailList" class="detail-list">
        <div class="detail-list-content">
          <article
            v-for="row in activeRuleRows"
            :key="row.rule.ruleId"
            class="rule-card"
            :class="{
              compact: activeCategory.id === 'userCache',
              'requires-elevation': row.rule.status === 'requiresElevation',
            }"
          >
            <MdResultTableRow
              layout="item"
              class="rule-summary md-ai-hover-row"
              :class="row.selection"
              :data-selected="row.selection !== 'none'"
            >
              <MdResultCheckbox
                :checked="row.selection === 'all'"
                :indeterminate="row.selection === 'partial'"
                :disabled="busy || !row.rule.selectable"
                :aria-label="t('cleanup.selectRule', { name: row.rule.name })"
                @update:checked="toggleRule(row.rule, $event)"
              />
              <MdResultItemContent
                :disclosure-label="row.rule.name"
                :disclosure-disabled="!hasCleanupRuleDetails(row.rule)"
                :title="row.rule.name"
                :badge="activeCategory.id !== 'userCache' && row.rule.risk === 'safe' ? t('common.safe') : undefined"
                badge-tone="positive"
                :value="
                  row.rule.status === 'requiresElevation'
                    ? t('cleanup.privilegedScan.required')
                    : ByteSizeService.bytes(row.selection === 'none' ? row.rule.bytes : row.selectedBytes)
                "
                :value-detail="ruleValueDetail(row.rule, row.selection, row.selectedBytes)"
                :value-tone="row.rule.status === 'requiresElevation' ? 'warning' : 'default'"
                :expandable="hasCleanupRuleDetails(row.rule)"
                :expanded="expandedRuleIds.has(row.rule.ruleId)"
                @toggle="toggleRuleDetails(row.rule)"
              >
                <template #icon>
                  <MdIcon
                    :class="{ 'recoverable-rule-icon': row.rule.risk === 'recoverable' }"
                    :name="cleanupRuleIcon(row.rule.ruleId, row.rule.group)"
                    :size="20"
                  />
                </template>
                <template v-if="row.rule.category !== 'custom'" #actions>
                  <MdAiAction :name="row.rule.name" :disabled="busy" @explain="explainRule(row.rule)" />
                </template>
              </MdResultItemContent>
              <Button
                v-if="row.rule.status === 'requiresElevation'"
                class="privileged-scan-button"
                variant="outline"
                size="sm"
                type="button"
                :disabled="busy || privilegedScanRuleId !== null"
                @click="emit('privilegedScan')"
              >
                <span
                  v-if="privilegedScanRuleId === row.rule.ruleId"
                  class="privileged-scan-spinner icon-spin md-operational-motion"
                  aria-hidden="true"
                />
                <MdIcon v-else :name="ICON_NAMES.shield" :size="15" />
                {{
                  privilegedScanRuleId === row.rule.ruleId
                    ? t('cleanup.privilegedScan.checking')
                    : t('cleanup.privilegedScan.action')
                }}
              </Button>
            </MdResultTableRow>

            <MdResultTableHierarchy v-if="expandedRuleIds.has(row.rule.ruleId) && hasCleanupRuleDetails(row.rule)">
              <div class="rule-details">
                <p v-if="row.rule.description" class="rule-description">{{ row.rule.description }}</p>
                <p v-if="row.rule.impact" class="rule-detail-note impact">
                  <MdIcon :name="ICON_NAMES.info" :size="13" />
                  <span>{{ row.rule.impact }}</span>
                </p>
                <p v-if="row.rule.requiresAppClose" class="rule-detail-note warning">
                  <MdIcon :name="ICON_NAMES.info" :size="13" />
                  <span>{{ runningProcessWarning(row.rule) }}</span>
                </p>
                <p v-if="isAggregateOnlyCleanupRule(row.rule)" class="rule-detail-note">
                  <MdIcon :name="ICON_NAMES.info" :size="13" />
                  <span>{{ t('cleanup.aggregateOnlyDetails') }}</span>
                </p>
              </div>
              <MdResultTableRow
                v-for="source in visibleRuleSources(row.rule)"
                :key="source.path"
                class="source-row"
                :class="{ 'with-artwork': isUniversalBinaryRule(row.rule) }"
                :data-selected="sourceSelected(row.rule.ruleId, source.path)"
              >
                <MdResultCheckbox
                  :checked="!source.blockReason && sourceSelected(row.rule.ruleId, source.path)"
                  :disabled="busy || Boolean(source.blockReason)"
                  :aria-label="t('cleanup.selectSource', { path: source.path })"
                  @update:checked="emit('toggleSource', row.rule.ruleId, source.path)"
                />
                <MdApplicationIcon
                  v-if="isUniversalBinaryRule(row.rule)"
                  :src="applicationIcons.get(source.path)"
                  :size="38"
                  :artwork-size="34"
                  @error="handleApplicationIconError(source.path)"
                />
                <span class="source-primary">
                  <span class="source-main">
                    <MdMiddleEllipsis :text="PathUtils.display(source.path)" :tail-length="22" />
                    <small>
                      {{ t('cleanup.sourceFiles', { count: FormatUtils.integer(source.fileCount) }) }}
                      <template v-if="source.modifiedAtMs">
                        ·
                        {{
                          t('cleanup.sourceModified', {
                            time: FormatUtils.dateTime(source.modifiedAtMs, locale),
                          })
                        }}</template
                      >
                      <template v-if="source.blockReason"> · {{ sourceBlockReason(source.blockReason) }}</template>
                    </small>
                  </span>
                  <span class="source-actions">
                    <MdIconAction
                      variant="ghost"
                      :label="t('common.showInFileManager')"
                      @click="emit('open', source.path)"
                    >
                      <MdIcon :name="ICON_NAMES.folder" :size="16" />
                    </MdIconAction>
                  </span>
                </span>
                <strong class="md-result-primary">{{ ByteSizeService.bytes(source.bytes) }}</strong>
              </MdResultTableRow>
              <template v-if="remainingRuleSourceCount(row.rule)" #footer>
                <MdLoadMoreButton
                  class="source-load-more"
                  :remaining-label="
                    t(
                      'common.locationCount',
                      { count: FormatUtils.integer(remainingRuleSourceCount(row.rule)) },
                      remainingRuleSourceCount(row.rule)
                    )
                  "
                  :disabled="busy"
                  @load-more="loadMoreRuleSources(row.rule)"
                />
              </template>
            </MdResultTableHierarchy>
          </article>
        </div>
      </MdResultTable>
    </section>

    <div v-else class="cleanup-details empty">
      <span class="cleanup-empty-icon">
        <MdIcon :name="ICON_NAMES.sparkles" :size="24" />
      </span>
      <strong>{{ t('cleanup.emptyCleanTitle') }}</strong>
      <small>{{ t('cleanup.emptyCleanDescription') }}</small>
    </div>
  </MdResultMasterDetail>
</template>

<style scoped src="./md-cleanup-rule-groups.css"></style>
