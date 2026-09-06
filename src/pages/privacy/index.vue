<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { toast } from 'vue-sonner';

import MdEmptyState from '@/components/custom/md-empty-state.vue';
import MdOperationProgress from '@/components/custom/md-operation-progress.vue';
import MdOperationWorkspace from '@/components/custom/md-operation-workspace.vue';
import MdAiWorkspace from '@/layouts/components/md-ai-workspace.vue';
import { useAiStore } from '@/stores/ai-store';
import MdPageShell from '@/components/custom/md-page-shell.vue';
import MdPermissionGuidance from '@/components/custom/md-permission-guidance.vue';
import MdResultWorkspace from '@/components/custom/md-result-workspace.vue';
import MdSelectionActionBar from '@/components/custom/md-selection-action-bar.vue';
import MdSelectionMode, { type MdSelectionModeOption } from '@/components/custom/md-selection-mode.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { Button } from '@/components/ui/button';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import type { ApplicationCloseMode } from '@/lib/models/application-close';
import type { PrivacyItem, PrivacyTimeRange } from '@/lib/models/privacy';
import type { TraversalProgress } from '@/lib/models/progress';
import { MACOS_PRIVACY_DESTINATION_IDS } from '@/lib/models/macos-permissions';
import { ICON_NAMES } from '@/lib/models/ui';
import { ApplicationIconService } from '@/lib/services/application-icon-service';
import { MacOsPermissionService } from '@/lib/services/macos-permission-service';
import * as FormatUtils from '@/lib/utils/format';
import {
  actionablePrivacyTokens,
  privacySelectionMode,
  recommendedPrivacyTokens,
  summarizePrivacySelection,
  type PrivacySelectionMode,
} from '@/lib/utils/privacy-selection';
import { useAppStore } from '@/stores/app-store';
import { usePrivacyStore } from '@/stores/privacy-store';

import { OperatingSystemService } from '@/lib/services/operating-system-service';
import { privacyAiContext } from './privacy-ai-context';
import MdPrivacyResultList from './components/md-privacy-result-list.vue';
import MdPrivacyDetailDialog from './components/md-privacy-detail-dialog.vue';
import MdPrivacyResultDialog from './components/md-privacy-result-dialog.vue';
import MdPrivacyPlanDialog from './components/md-privacy-plan-dialog.vue';

const aiStore = useAiStore();
const { t, locale } = useI18n({ useScope: 'global' });
const store = usePrivacyStore();
const confirmationOpen = ref(false);
const resultOpen = ref(false);
const permissionPromptOpen = ref(false);
const permissionPromptShown = ref(false);
const detailOpen = ref(false);
const detailItem = ref<PrivacyItem | null>(null);
const sourceIconUrls = ref<Readonly<Record<string, string>>>({});
let iconResolutionRevision = 0;

const busy = computed(
  () => store.scanning || store.preparing || store.closingBrowsers || store.refreshingBrowserStatus || store.executing
);
const selectionSummary = computed(() => summarizePrivacySelection(store.scanResult?.items ?? [], store.selectedTokens));
const privacyItems = computed(() => store.scanResult?.items ?? []);
const recommendedTokens = computed(() => recommendedPrivacyTokens(privacyItems.value));
const allActionableTokens = computed(() => actionablePrivacyTokens(privacyItems.value));
const recommendedSummary = computed(() => summarizePrivacySelection(privacyItems.value, recommendedTokens.value));
const allActionableSummary = computed(() => summarizePrivacySelection(privacyItems.value, allActionableTokens.value));
const selectionMode = computed(() => privacySelectionMode(privacyItems.value, store.selectedTokens));
const selectionModeOptions = computed<MdSelectionModeOption[]>(() => {
  const options: MdSelectionModeOption[] = [
    {
      value: 'smart',
      label: `${t('privacy.selectionMode.smart')} · ${t(
        'privacy.traceCount',
        { count: FormatUtils.integer(recommendedSummary.value.itemCount) },
        recommendedSummary.value.itemCount
      )}`,
    },
    {
      value: 'all',
      label: `${t('privacy.selectionMode.all')} · ${t(
        'privacy.traceCount',
        { count: FormatUtils.integer(allActionableSummary.value.itemCount) },
        allActionableSummary.value.itemCount
      )}`,
    },
    { value: 'none', label: t('privacy.selectionMode.none') },
  ];
  if (selectionMode.value === 'manual') {
    options.push({ value: 'manual', label: t('privacy.selectionMode.manual'), disabled: true });
  }
  return options;
});
const selectedCount = computed(() => selectionSummary.value.itemCount);
const selectedValue = computed(() =>
  selectionSummary.value.pendingScanCount
    ? t('privacy.selectedPendingSummary', {
        count: FormatUtils.integer(selectedCount.value),
        pending: FormatUtils.integer(selectionSummary.value.pendingScanCount),
      })
    : t('privacy.traceCount', { count: FormatUtils.integer(selectedCount.value) }, selectedCount.value)
);
const hasPermissionRequired = computed(
  () => store.scanResult?.coverage.some(source => source.capability === 'permissionRequired') ?? false
);
const showPermissionGuidance = computed(() => hasPermissionRequired.value && MacOsPermissionService.isMacOs());
const sourceIconPaths = computed<Readonly<Record<string, string>>>(() =>
  Object.fromEntries(
    (store.scanResult?.coverage ?? []).flatMap(source =>
      source.iconPath ? [[source.sourceId, source.iconPath] as const] : []
    )
  )
);
const sourceIconUrlsByName = computed<Readonly<Record<string, string>>>(() =>
  Object.fromEntries(
    (store.scanResult?.coverage ?? []).flatMap(source => {
      const icon = sourceIconUrls.value[source.sourceId];
      return icon ? [[source.sourceName, icon] as const] : [];
    })
  )
);
const resultIncompleteCount = computed(
  () => store.result?.items.filter(item => item.status === 'failed' || item.status === 'cancelled').length ?? 0
);
const privacyProgress = computed<TraversalProgress | null>(() => {
  const progress = store.scanProgress;
  if (!progress) return null;
  return {
    operationId: 0,
    currentStage: 'analyzing',
    currentPath: progress.sourceName ?? '',
    itemsScanned: progress.completedSources,
    bytesScanned: 0,
    completedSteps: progress.completedSources,
    totalSteps: progress.totalSources,
    foundItems: 0,
    foundBytes: 0,
    elapsedMs: 0,
  };
});
const privacyScanTitle = computed(() =>
  store.scanProgress?.sourceName
    ? t('privacy.scanningSource', { source: store.scanProgress.sourceName })
    : t('privacy.scanning')
);

watch(
  showPermissionGuidance,
  needsPermission => {
    if (!needsPermission) {
      permissionPromptOpen.value = false;
      return;
    }
    if (permissionPromptShown.value) return;
    permissionPromptShown.value = true;
    permissionPromptOpen.value = true;
  },
  { immediate: true }
);

watch(
  () => store.scanResult?.scanId,
  (scanId, previousScanId) => {
    if (previousScanId && scanId !== previousScanId) {
      detailOpen.value = false;
      detailItem.value = null;
    }
  }
);

watch(
  () => store.scanResult?.coverage.map(source => [source.sourceId, source.iconPath] as const) ?? [],
  sources => {
    const revision = ++iconResolutionRevision;
    const paths = sources.flatMap(([, path]) => (path ? [path] : []));
    sourceIconUrls.value = {};
    if (!paths.length) return;

    void ApplicationIconService.resolveIncrementally(paths, icons => {
      if (revision !== iconResolutionRevision) return;
      sourceIconUrls.value = Object.fromEntries(
        sources.flatMap(([sourceId, path]) => {
          const icon = path ? icons.get(path) : undefined;
          return icon ? [[sourceId, icon] as const] : [];
        })
      );
    });
  },
  { immediate: true }
);

function updateTimeRange(value: unknown) {
  if (['lastHour', 'today', 'lastSevenDays', 'allTime'].includes(String(value))) {
    store.setTimeRange(value as PrivacyTimeRange);
  }
}

function updateSelectionMode(value: unknown) {
  if (!['smart', 'all', 'none'].includes(String(value))) return;
  const mode = value as Exclude<PrivacySelectionMode, 'manual'>;
  store.setSelection(mode === 'smart' ? recommendedTokens.value : mode === 'all' ? allActionableTokens.value : []);
}

function explainItem(item: PrivacyItem) {
  void aiStore.show(
    privacyAiContext(
      item,
      t(`privacy.kinds.${item.kind}`),
      store.timeRange,
      OperatingSystemService.isWindows() ? 'windows' : 'macos'
    ),
    locale.value
  );
}

function showDetails(item: PrivacyItem) {
  detailItem.value = item;
  detailOpen.value = true;
}

async function prepareExecution() {
  const plan = await store.prepare();
  if (plan) confirmationOpen.value = true;
}

async function execute(excludedSourceIds: string[] = []) {
  confirmationOpen.value = false;
  await store.execute(excludedSourceIds);
  if (!store.result) return;
  if (!store.result.items.length) {
    toast.warning(t('privacy.feedback.noRemainingSelection'));
    return;
  }
  resultOpen.value = true;
  if (resultIncompleteCount.value) {
    toast.warning(
      t('privacy.feedback.completedWithWarnings', {
        count: store.result.affectedItemCount,
        failed: resultIncompleteCount.value,
      })
    );
  } else {
    toast.success(t('privacy.feedback.completed', { count: store.result.affectedItemCount }));
  }
}

function updateConfirmationOpen(value: boolean) {
  confirmationOpen.value = value;
  if (!value) store.clearPlan();
}

function closeBrowsers(sourceIds: string[], mode: ApplicationCloseMode) {
  void store.closeBrowsers(sourceIds, mode);
}

function refreshBrowserStatus(sourceIds: string[]) {
  void store.refreshBrowserStatus(sourceIds);
}

async function continueAfterBrowserClose(excludedSourceIds: string[]) {
  // Core refreshes only the selected candidates whose process was closed. A full catalog scan here
  // would delay the result dialog and make one cleanup feel like two separate operations.
  await execute(excludedSourceIds);
}

async function openPrivacySettings(): Promise<boolean> {
  try {
    await MacOsPermissionService.openPrivacySettings(MACOS_PRIVACY_DESTINATION_IDS.fullDiskAccess);
    return true;
  } catch (error) {
    useAppStore().reportError(error);
    return false;
  }
}
watch(
  () => [store.scanResult, busy.value, store.timeRange],
  () => aiStore.dismissModule('privacy')
);
</script>

<template>
  <MdPageShell class="privacy-page" content-mode="workspace" :title="t('privacy.title')">
    <template #overlay><MdAiWorkspace module="privacy" /></template>
    <template #actions>
      <Select :key="locale" :model-value="store.timeRange" :disabled="busy" @update:model-value="updateTimeRange">
        <SelectTrigger class="time-range h-9" :aria-label="t('privacy.timeRangeLabel')">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="lastHour">{{ t('privacy.timeRanges.lastHour') }}</SelectItem>
          <SelectItem value="today">{{ t('privacy.timeRanges.today') }}</SelectItem>
          <SelectItem value="lastSevenDays">{{ t('privacy.timeRanges.lastSevenDays') }}</SelectItem>
          <SelectItem value="allTime">{{ t('privacy.timeRanges.allTime') }}</SelectItem>
        </SelectContent>
      </Select>
      <Button v-if="store.scanResult" variant="outline" type="button" :disabled="busy" @click="store.scan()">
        <MdIcon :name="ICON_NAMES.refresh" :size="17" />
        {{ t('privacy.rescan') }}
      </Button>
    </template>

    <template v-if="store.scanResult" #footer>
      <MdSelectionActionBar
        :selected-label="t('privacy.selectedSummary')"
        :selected-value="selectedValue"
        emphasize-selected-value
        :action-label="t('privacy.clearSelected')"
        :disabled="!store.selectedTokens.length"
        :busy="busy"
        @action="prepareExecution"
      >
        <template #options>
          <div class="privacy-footer-options">
            <MdPermissionGuidance
              v-if="showPermissionGuidance"
              v-model="permissionPromptOpen"
              :summary="t('privacy.permission.summary')"
              :title="t('privacy.permission.title')"
              :description="t('privacy.permission.description')"
              :instructions="t('privacy.permission.instructions')"
              :skip-label="t('privacy.permission.skip')"
              :open-settings-label="t('privacy.permission.openSettings')"
              :open-settings="openPrivacySettings"
            />
            <MdSelectionMode
              :busy="busy"
              :display-value="t(`privacy.selectionMode.${selectionMode}`)"
              :label="t('privacy.selectionMode.label')"
              :model-value="selectionMode"
              :options="selectionModeOptions"
              @update:model-value="updateSelectionMode"
            />
          </div>
        </template>
        <template #action-icon>
          <MdIcon :name="ICON_NAMES.shield" :size="18" />
        </template>
      </MdSelectionActionBar>
    </template>

    <MdOperationWorkspace v-if="store.scanning">
      <MdOperationProgress
        :icon-name="ICON_NAMES.shield"
        :title="store.cancellingScan ? t('privacy.cancelling') : privacyScanTitle"
        :progress="privacyProgress"
        :path-label="t('privacy.scanning')"
        :preparing-text="t('privacy.scanningDescription')"
        :hint="t('privacy.scanningDescription')"
        :show-traversal-details="false"
        show-step-progress
        :cancelable="true"
        :cancel-disabled="store.cancellingScan"
        @cancel="store.cancelScan()"
      />
    </MdOperationWorkspace>

    <MdResultWorkspace v-else-if="store.scanResult" class="privacy-results">
      <div class="privacy-result-content">
        <MdPrivacyResultList
          :busy="busy"
          :items="store.scanResult.items"
          :permission-label="showPermissionGuidance ? t('privacy.permission.badge') : undefined"
          :selected-tokens="store.selectedTokens"
          :source-icon-urls="sourceIconUrls"
          @update:selected-tokens="store.setSelection"
          @show-details="showDetails"
          @explain="explainItem"
        />
      </div>
    </MdResultWorkspace>

    <MdOperationWorkspace v-else>
      <MdEmptyState
        :icon-name="ICON_NAMES.shield"
        :title="store.result ? t('privacy.completedTitle') : t('privacy.idleTitle')"
        :description="
          store.result
            ? t('privacy.completedDescription', { count: store.result.affectedItemCount })
            : t('privacy.idleDescription')
        "
      >
        <Button size="lg" type="button" :disabled="busy" @click="store.scan()">
          <MdIcon :name="ICON_NAMES.search" :size="17" />
          {{ t('privacy.scan') }}
        </Button>
      </MdEmptyState>
    </MdOperationWorkspace>

    <MdPrivacyPlanDialog
      v-if="store.plan"
      :model-value="confirmationOpen"
      :plan="store.plan"
      :busy="busy"
      :closing-browsers="store.closingBrowsers"
      :close-result="store.closeResult"
      :browser-status-result="store.browserStatusResult"
      :source-icon-paths="sourceIconPaths"
      :source-icon-urls-by-name="sourceIconUrlsByName"
      @close-browsers="closeBrowsers"
      @refresh-browser-status="refreshBrowserStatus"
      @continue="continueAfterBrowserClose"
      @execute="execute"
      @update:model-value="updateConfirmationOpen"
    />

    <MdPrivacyResultDialog v-model="resultOpen" :plan="store.completedPlan" :result="store.result" />
    <MdPrivacyDetailDialog
      v-if="store.scanResult"
      v-model="detailOpen"
      :scan-id="store.scanResult.scanId"
      :item="detailItem"
    />
  </MdPageShell>
</template>

<style scoped>
@reference "@assets/main.css";

.time-range {
  width: 170px;
  @apply border-border/70 bg-card/35 shadow-none hover:border-border hover:bg-card/55;
}
.time-range[data-state='open'] {
  @apply border-border bg-card/55 ring-0;
}
.time-range:focus-visible {
  @apply border-ring ring-3 ring-ring/20;
}
.privacy-result-content {
  display: flex;
  min-height: 0;
  flex: 1;
  flex-direction: column;
  overflow: hidden;
}
.privacy-footer-options {
  display: flex;
  width: 100%;
  min-width: 0;
  flex-wrap: nowrap;
  align-items: center;
  justify-content: flex-end;
  gap: 8px 12px;
}

.privacy-footer-options :deep(.permission-summary) {
  flex: 1 1 auto;
}

.privacy-footer-options :deep(.selection-mode) {
  flex: none;
}
</style>
