<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { toast } from 'vue-sonner';

import MdCategoryFilter from '@/components/custom/md-category-filter.vue';
import MdDialogContent from '@/components/custom/md-dialog-content.vue';
import MdDialogFooter from '@/components/custom/md-dialog-footer.vue';
import MdDialogHeader from '@/components/custom/md-dialog-header.vue';
import MdEmptyState from '@/components/custom/md-empty-state.vue';
import MdLoadMoreButton from '@/components/custom/md-load-more-button.vue';
import MdOperationProgress from '@/components/custom/md-operation-progress.vue';
import MdOperationWorkspace from '@/components/custom/md-operation-workspace.vue';
import MdAiWorkspace from '@/layouts/components/md-ai-workspace.vue';
import { useAiStore } from '@/stores/ai-store';
import MdPageShell from '@/components/custom/md-page-shell.vue';
import MdPermissionGuidance from '@/components/custom/md-permission-guidance.vue';
import MdResultFilterToolbar from '@/components/custom/md-result-filter-toolbar.vue';
import MdResultSearch from '@/components/custom/md-result-search.vue';
import MdResultSummary from '@/components/custom/md-result-summary.vue';
import MdResultTable from '@/components/custom/md-result-table.vue';
import MdResultWorkspace from '@/components/custom/md-result-workspace.vue';
import MdSpinner from '@/components/custom/md-spinner.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { Dialog, DialogDescription, DialogTitle } from '@/components/ui/dialog';
import type {
  StartupArtifact,
  StartupCatalog,
  StartupChangePlan,
  StartupChangeResult,
  StartupDesiredState,
  StartupOwnerGroup,
} from '@/lib/models/startup';
import { MACOS_PRIVACY_DESTINATION_IDS } from '@/lib/models/macos-permissions';
import { LOG_DOMAINS, LOG_EVENTS } from '@/lib/models/telemetry';
import { ICON_NAMES } from '@/lib/models/ui';
import { ApplicationIconService } from '@/lib/services/application-icon-service';
import { ClipboardService } from '@/lib/services/clipboard-service';
import { MacOsPermissionService } from '@/lib/services/macos-permission-service';
import { MacOsSystemSettingsService } from '@/lib/services/macos-system-settings-service';
import { LoggerService } from '@/lib/services/logger-service';
import { OperatingSystemService } from '@/lib/services/operating-system-service';
import { WindowsStartupToolService, type WindowsStartupTool } from '@/lib/services/windows-startup-tool-service';
import * as FormatUtils from '@/lib/utils/format';
import * as RenderBatchUtils from '@/lib/utils/render-batch';

import { startupAiContext } from './startup-ai-context';
import MdStartupRow from './components/md-startup-row.vue';
import { startupGroupIconUrl } from './startup-brand-icon';
import {
  cancelQueuedStartupChanges,
  completeStartupChange,
  createStartupChangeWorkflow,
  dispatchNextStartupChange,
  enqueueStartupWorkflow,
  queuedStartupItemIds,
} from './startup-change-queue';

import {
  displayedStartupGroups,
  displayedArtifactsForGroup,
  filterAndSortStartupGroups,
  indexStartupArtifacts,
  manageableArtifactsForGroup,
  needsBackgroundTaskPermission,
  nextStartupDesiredState,
  removableStartupArtifactsForGroup,
  startupFilterCounts,
  startupGroupManageableState,
  startupGroupStartTiming,
  startupGroupSubtitle,
  startupRevealPath,
  startupPlanRequiresReview,
  type StartupStateFilter,
} from './startup-view';

const STARTUP_RENDER_BATCH_SIZE = 120;
const STARTUP_CHANGE_BATCH_LIMIT = 256;
// A short window lets deliberate consecutive clicks share one expensive native preflight and
// readback while the per-row pending state still responds immediately.
const STARTUP_CHANGE_BATCH_WINDOW_MS = 350;

const props = defineProps<{
  catalog: StartupCatalog | null;
  scanning: boolean;
  cancelling: boolean;
  preparingChange: boolean;
  executingChange: boolean;
  cancellingChange: boolean;
  pendingPlan: StartupChangePlan | null;
  lastChangeResult: StartupChangeResult | null;
}>();
const emit = defineEmits<{
  scan: [];
  cancel: [];
  open: [path: string];
  prepareChange: [selection: { itemIds: string[]; desiredState: StartupDesiredState }];
  cancelChange: [];
  cancelChangeExecution: [];
  executeChange: [];
  error: [error: unknown];
}>();

const aiStore = useAiStore();
const { locale, t } = useI18n({ useScope: 'global' });
const query = ref('');
const stateFilter = ref<StartupStateFilter>('all');
const expandedGroupId = ref<string | null>(null);
const changeOpen = ref(false);
const permissionPromptOpen = ref(false);
const permissionPromptShown = ref(false);
const iconUrls = ref<ReadonlyMap<string, string>>(new Map());
const visibleCount = ref(STARTUP_RENDER_BATCH_SIZE);
const changeWorkflow = ref(createStartupChangeWorkflow());
const changeFeedback = ref<{
  displayName: string;
  desiredState: StartupDesiredState;
  itemCount: number;
} | null>(null);
const copiedActionKey = ref<string | null>(null);
let copyFeedbackTimer: ReturnType<typeof setTimeout> | null = null;
let changeDispatchTimer: ReturnType<typeof setTimeout> | null = null;
const isWindows = OperatingSystemService.isWindows();
const isMacOs = OperatingSystemService.isMacOs();
const showSystemItems = ref(false);

const artifactsById = computed(() => indexStartupArtifacts(props.catalog?.artifacts ?? []));
const displayGroups = computed(() =>
  displayedStartupGroups(props.catalog?.groups ?? [], artifactsById.value, showSystemItems.value)
);
const filterCounts = computed(() => startupFilterCounts(displayGroups.value, artifactsById.value));
const filterOptions = computed(() =>
  (['all', 'enabled', 'disabled', 'leftover'] as const).map(value => ({
    value,
    label: t(`startup.filters.${value}`),
    count: filterCounts.value[value],
  }))
);
const filteredGroups = computed(() =>
  filterAndSortStartupGroups(displayGroups.value, artifactsById.value, query.value, stateFilter.value, locale.value)
);
const visibleGroups = computed(() => RenderBatchUtils.visibleItems(filteredGroups.value, visibleCount.value));
const remainingResultCount = computed(() =>
  RenderBatchUtils.remainingCount(filteredGroups.value.length, visibleGroups.value.length)
);
const changeBusy = computed(
  () => props.preparingChange || props.executingChange || props.cancellingChange || Boolean(props.pendingPlan)
);
const pendingChangeItemIds = computed(() => queuedStartupItemIds(changeWorkflow.value));
const changeQueueBusy = computed(() => changeBusy.value || pendingChangeItemIds.value.size > 0);
const backgroundTasksNeedPermission = computed(() =>
  needsBackgroundTaskPermission(isMacOs, props.catalog?.coverage ?? [])
);
const pendingPlanRequiresElevation = computed(() =>
  Boolean(props.pendingPlan?.items.some(item => item.requiresElevation))
);
const pendingPlanOnlyAffectsFutureLaunches = computed(
  () => props.pendingPlan?.desiredState === 'disabled' && Boolean(props.pendingPlan.items.length)
);
const pendingPlanRemovesItems = computed(() => props.pendingPlan?.desiredState === 'removed');

watch(
  backgroundTasksNeedPermission,
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

function updateStateFilter(value: string) {
  if (value === 'all' || value === 'enabled' || value === 'disabled' || value === 'leftover') {
    stateFilter.value = value;
  }
}

watch([() => props.catalog?.scanId, query, stateFilter, showSystemItems], () => {
  // Startup catalogs can still contain thousands of hidden system entries.
  // Reset progressive rendering after each visible result change to keep scrolling responsive.
  visibleCount.value = STARTUP_RENDER_BATCH_SIZE;
});

watch(
  () => displayGroups.value.map(group => group.iconPath).filter((path): path is string => Boolean(path)),
  paths => {
    void ApplicationIconService.resolveIncrementally(paths, icons => {
      iconUrls.value = icons;
    });
  },
  { immediate: true }
);

watch(
  () => props.pendingPlan,
  plan => {
    const request = changeWorkflow.value.activeChange;
    if (!plan || !request) return;
    if (startupPlanRequiresReview(plan, request.itemIds.length, request.requiresReview)) {
      changeOpen.value = true;
      return;
    }
    emit('executeChange');
  }
);

watch(
  () => props.preparingChange,
  (preparing, wasPreparing) => {
    if (!preparing && wasPreparing && !props.pendingPlan && changeWorkflow.value.activeChange) {
      changeFeedback.value = null;
      completeActiveChange();
    }
  }
);

watch(
  () => props.executingChange,
  (executing, wasExecuting) => {
    if (executing || !wasExecuting || props.lastChangeResult) return;
    changeOpen.value = false;
    changeFeedback.value = null;
    completeActiveChange();
  }
);

watch(
  () => props.lastChangeResult,
  result => {
    if (!result) return;
    const feedback = changeFeedback.value;
    if (!result.catalog) {
      toast.warning(t('startup.change.refreshFailedResult'));
    } else if (result.failedCount) {
      const batchMessage = feedback && feedback.itemCount > 1 && feedback.desiredState !== 'removed';
      toast.warning(
        t(
          batchMessage
            ? feedback.desiredState === 'enabled'
              ? 'startup.change.batchPartialEnableResult'
              : 'startup.change.batchPartialDisableResult'
            : feedback?.desiredState === 'enabled'
              ? 'startup.change.partialEnableResult'
              : feedback?.desiredState === 'removed'
                ? 'startup.remove.partialResult'
                : 'startup.change.partialDisableResult',
          {
            name: feedback?.displayName ?? t('startup.title'),
            changed: result.changedCount,
            failed: result.failedCount,
          }
        )
      );
    } else {
      const batchMessage = feedback && feedback.itemCount > 1 && feedback.desiredState !== 'removed';
      const messageKey = batchMessage
        ? feedback.desiredState === 'enabled'
          ? 'startup.change.batchEnableSuccessResult'
          : 'startup.change.batchDisableSuccessResult'
        : feedback?.desiredState === 'enabled'
          ? 'startup.change.enableSuccessResult'
          : feedback?.desiredState === 'removed'
            ? 'startup.remove.successResult'
            : 'startup.change.disableSuccessResult';
      toast.success(
        t(messageKey, {
          count: result.changedCount,
          name: feedback?.displayName ?? t('startup.title'),
        })
      );
    }
    changeFeedback.value = null;
    changeOpen.value = false;
    completeActiveChange();
  }
);

watch(changeBusy, busy => {
  if (!busy) scheduleNextChange(0);
});

function manageableArtifacts(group: StartupOwnerGroup): StartupArtifact[] {
  return manageableArtifactsForGroup(group, artifactsById.value);
}

function displayedArtifacts(group: StartupOwnerGroup): StartupArtifact[] {
  return displayedArtifactsForGroup(group, artifactsById.value);
}

function groupDisplayState(group: StartupOwnerGroup) {
  return startupGroupManageableState(group, artifactsById.value);
}

function groupDesiredState(group: StartupOwnerGroup): StartupDesiredState {
  return nextStartupDesiredState(groupDisplayState(group));
}

function artifactDesiredState(artifact: StartupArtifact): StartupDesiredState {
  return nextStartupDesiredState(artifact.configuredState);
}

function groupSubtitle(group: StartupOwnerGroup): string | null {
  return startupGroupSubtitle(group);
}

function groupStartTiming(group: StartupOwnerGroup): string {
  return t(`startup.detail.startTiming.${startupGroupStartTiming(group)}`);
}

function nativeIconUrl(path: string | null): string {
  return path ? (iconUrls.value.get(path) ?? '') : '';
}

function groupIconUrl(group: StartupOwnerGroup): string {
  return startupGroupIconUrl(group, displayedArtifacts(group), nativeIconUrl(group.iconPath));
}

function loadMoreResults() {
  visibleCount.value = RenderBatchUtils.nextVisibleCount(
    visibleCount.value,
    filteredGroups.value.length,
    STARTUP_RENDER_BATCH_SIZE
  );
}

function isChanging(group: StartupOwnerGroup): boolean {
  const manageableItemIds = new Set(manageableArtifacts(group).map(artifact => artifact.itemId));
  return [changeWorkflow.value.activeChange, ...changeWorkflow.value.queuedChanges].some(
    change => change?.desiredState !== 'removed' && change?.itemIds.some(itemId => manageableItemIds.has(itemId))
  );
}

function isGroupChangePending(group: StartupOwnerGroup): boolean {
  return displayedArtifacts(group).some(artifact => pendingChangeItemIds.value.has(artifact.itemId));
}

function requestStartupRemoval(group: StartupOwnerGroup) {
  requestChange(
    removableStartupArtifactsForGroup(group, artifactsById.value).map(artifact => artifact.itemId),
    'removed'
  );
}

function requestGroupChange(group: StartupOwnerGroup) {
  const itemIds = manageableArtifacts(group).map(artifact => artifact.itemId);
  requestChange(itemIds, groupDesiredState(group), itemIds.length > 1);
}

function requestArtifactChange(artifact: StartupArtifact) {
  requestChange([artifact.itemId], artifactDesiredState(artifact));
}

function requestChange(itemIds: string[], desiredState: StartupDesiredState, requiresReview = false) {
  if (!itemIds.length || itemIds.some(itemId => changeWorkflow.value.activeChange?.itemIds.includes(itemId))) return;
  changeWorkflow.value = enqueueStartupWorkflow(
    changeWorkflow.value,
    itemIds,
    desiredState,
    STARTUP_CHANGE_BATCH_LIMIT,
    requiresReview
  );
  LoggerService.info(LOG_DOMAINS.startup, LOG_EVENTS.startupChangeQueued, {
    desiredState,
    requiresReview,
    requestedItemCount: itemIds.length,
    queuedBatchCount: changeWorkflow.value.queuedChanges.length,
    pendingItemCount: pendingChangeItemIds.value.size,
  });
  scheduleNextChange(STARTUP_CHANGE_BATCH_WINDOW_MS);
}

function scheduleNextChange(delayMs: number) {
  if (changeDispatchTimer || changeWorkflow.value.activeChange || !changeWorkflow.value.queuedChanges.length) return;
  changeDispatchTimer = setTimeout(() => {
    changeDispatchTimer = null;
    if (changeBusy.value || changeWorkflow.value.activeChange) return;
    const dispatch = dispatchNextStartupChange(changeWorkflow.value);
    const nextChange = dispatch.change;
    if (!nextChange) return;
    changeWorkflow.value = dispatch.workflow;
    LoggerService.info(LOG_DOMAINS.startup, LOG_EVENTS.startupChangeBatchDispatched, {
      desiredState: nextChange.desiredState,
      itemCount: nextChange.itemIds.length,
      requiresReview: Boolean(nextChange.requiresReview),
      remainingBatchCount: dispatch.workflow.queuedChanges.length,
    });
    changeFeedback.value = {
      displayName:
        nextChange.itemIds.length === 1
          ? (artifactsById.value.get(nextChange.itemIds[0]!)?.displayName ?? t('startup.title'))
          : t('startup.change.batchName', { count: nextChange.itemIds.length }),
      desiredState: nextChange.desiredState,
      itemCount: nextChange.itemIds.length,
    };
    emit('prepareChange', { itemIds: nextChange.itemIds, desiredState: nextChange.desiredState });
  }, delayMs);
}

function completeActiveChange() {
  changeWorkflow.value = completeStartupChange(changeWorkflow.value);
  scheduleNextChange(0);
}

function cancelQueuedChanges() {
  const cancellation = cancelQueuedStartupChanges(changeWorkflow.value);
  changeWorkflow.value = cancellation.workflow;
  if (changeDispatchTimer) {
    clearTimeout(changeDispatchTimer);
    changeDispatchTimer = null;
  }
  if (cancellation.cancelledBatchCount) {
    LoggerService.info(LOG_DOMAINS.startup, LOG_EVENTS.startupChangeQueueCancelled, {
      queuedBatchCount: cancellation.cancelledBatchCount,
      queuedItemCount: cancellation.cancelledItemCount,
    });
  }
}

async function openBackgroundTaskPrivacySettings(): Promise<boolean> {
  try {
    await MacOsPermissionService.openPrivacySettings(MACOS_PRIVACY_DESTINATION_IDS.fullDiskAccess);
    return true;
  } catch (error) {
    emit('error', error);
    return false;
  }
}

async function openLoginItemsSettings() {
  try {
    await MacOsSystemSettingsService.openLoginItems();
  } catch (error) {
    emit('error', error);
  }
}

async function openWindowsStartupTool(tool: WindowsStartupTool) {
  try {
    await WindowsStartupToolService.open(tool);
  } catch (error) {
    emit('error', error);
  }
}

async function copyStartupValue(request: { actionKey: string; value: string }) {
  try {
    await ClipboardService.writeText(request.value);
    copiedActionKey.value = request.actionKey;
    if (copyFeedbackTimer) clearTimeout(copyFeedbackTimer);
    copyFeedbackTimer = setTimeout(() => {
      copiedActionKey.value = null;
      copyFeedbackTimer = null;
    }, 1000);
  } catch (error) {
    emit('error', error);
  }
}

onBeforeUnmount(() => {
  if (copyFeedbackTimer) clearTimeout(copyFeedbackTimer);
  if (changeDispatchTimer) clearTimeout(changeDispatchTimer);
});

function cancelActiveChangeExecution() {
  cancelQueuedChanges();
  emit('cancelChangeExecution');
}

function updateChangeOpen(open: boolean) {
  if (props.executingChange) {
    if (!open) cancelActiveChangeExecution();
    return;
  }
  changeOpen.value = open;
  if (!open) {
    emit('cancelChange');
    changeFeedback.value = null;
    completeActiveChange();
  }
}
watch(
  () => [props.catalog, props.scanning, props.preparingChange, props.executingChange],
  () => aiStore.dismissModule('startup')
);
</script>

<template>
  <MdPageShell class="@container/startup" content-mode="workspace" :title="t('startup.title')">
    <template #overlay><MdAiWorkspace module="startup" /></template>
    <template v-if="catalog && !scanning" #actions>
      <Button variant="outline" type="button" :disabled="changeQueueBusy" @click="emit('scan')">
        <MdIcon :name="ICON_NAMES.refresh" :size="16" />
        {{ t('startup.rescan') }}
      </Button>
    </template>

    <MdOperationWorkspace v-if="scanning">
      <MdOperationProgress
        :icon-name="ICON_NAMES.startup"
        :title="cancelling ? t('startup.cancelling') : t('startup.scanning')"
        :progress="null"
        :path-label="t('startup.scanning')"
        :preparing-text="t('startup.scanningDescription')"
        :hint="t('startup.scanningDescription')"
        :show-traversal-details="false"
        :show-step-progress="false"
        :cancelable="true"
        :cancel-disabled="cancelling"
        @cancel="emit('cancel')"
      />
    </MdOperationWorkspace>

    <MdResultWorkspace v-else>
      <template v-if="catalog" #summary>
        <MdResultSummary
          :title="t('startup.summary.programs', { count: FormatUtils.integer(displayGroups.length) })"
          :metric-label="t('startup.summary.enabled')"
          :metric-value="FormatUtils.integer(filterCounts.enabled)"
        >
          <template #actions>
            <label v-if="isWindows" class="flex items-center gap-2 whitespace-nowrap text-sm text-muted-foreground">
              <Checkbox v-model="showSystemItems" :disabled="changeQueueBusy" />
              {{ t('startup.showSystemItems') }}
            </label>
            <MdPermissionGuidance
              v-if="backgroundTasksNeedPermission"
              v-model="permissionPromptOpen"
              :summary="t('startup.summary.permissionRequired')"
              :title="t('startup.permission.title')"
              :description="t('startup.permission.description')"
              :instructions="t('startup.permission.instructions')"
              :skip-label="t('startup.permission.skip')"
              :open-settings-label="t('startup.permission.openSettings')"
              :open-settings="openBackgroundTaskPrivacySettings"
            />
          </template>
        </MdResultSummary>
      </template>

      <template v-if="catalog" #header>
        <MdResultFilterToolbar>
          <MdCategoryFilter
            :model-value="stateFilter"
            :options="filterOptions"
            :accessibility-label="t('startup.filterLabel')"
            :disabled="changeQueueBusy"
            @update:model-value="updateStateFilter"
          />
          <template #aside>
            <MdResultSearch v-model="query" :placeholder="t('startup.searchPlaceholderCompact')" />
          </template>
        </MdResultFilterToolbar>
      </template>

      <MdEmptyState
        v-if="!catalog"
        :icon-name="ICON_NAMES.startup"
        :title="t('startup.emptyTitle')"
        :description="t('startup.emptyDescription')"
      >
        <Button size="lg" type="button" @click="emit('scan')">
          <MdIcon :name="ICON_NAMES.scan" :size="17" />
          {{ t('startup.scan') }}
        </Button>
      </MdEmptyState>

      <MdEmptyState
        v-else-if="!displayGroups.length"
        compact
        :icon-name="ICON_NAMES.check"
        :title="t('startup.noManageableTitle')"
        :description="t('startup.noManageableDescription')"
      >
        <Button
          v-if="backgroundTasksNeedPermission"
          variant="outline"
          type="button"
          @click="openBackgroundTaskPrivacySettings"
        >
          <MdIcon :name="ICON_NAMES.external" :size="14" />
          {{ t('startup.summary.openPrivacySettings') }}
        </Button>
      </MdEmptyState>

      <MdEmptyState
        v-else-if="!filteredGroups.length"
        compact
        :icon-name="ICON_NAMES.search"
        :title="t('startup.noMatchesTitle')"
        :description="t('startup.noMatchesDescription')"
      />

      <MdResultTable v-else>
        <MdStartupRow
          v-for="group in visibleGroups"
          :key="group.groupId"
          :group="group"
          :artifacts="displayedArtifacts(group)"
          :icon-src="groupIconUrl(group)"
          :subtitle="groupSubtitle(group)"
          :start-timing="groupStartTiming(group)"
          :state="groupDisplayState(group)"
          :reveal-path="startupRevealPath(group, artifactsById)"
          :is-windows="isWindows"
          :is-mac-os="isMacOs"
          :expanded="expandedGroupId === group.groupId"
          :busy="isGroupChangePending(group)"
          :changing="isChanging(group)"
          :copied-action-key="copiedActionKey"
          @explain="
            (name, artifacts) =>
              aiStore.show(
                startupAiContext(name, artifacts, t('startup.title'), isWindows ? 'windows' : 'macos'),
                locale
              )
          "
          @toggle-expanded="expandedGroupId = expandedGroupId === group.groupId ? null : group.groupId"
          @toggle-group="requestGroupChange(group)"
          @toggle-artifact="requestArtifactChange"
          @remove-items="requestStartupRemoval(group)"
          @reveal="emit('open', $event)"
          @copy="copyStartupValue"
          @open-system-settings="openLoginItemsSettings"
          @open-windows-tool="openWindowsStartupTool"
        />

        <MdLoadMoreButton
          v-if="remainingResultCount"
          :remaining-label="t('startup.remainingResults', { count: FormatUtils.integer(remainingResultCount) })"
          @load-more="loadMoreResults"
        />
      </MdResultTable>
    </MdResultWorkspace>

    <Dialog :open="changeOpen" @update:open="updateChangeOpen">
      <MdDialogContent class="startup-change-dialog" size="standard">
        <MdDialogHeader>
          <DialogTitle>{{ t(pendingPlanRemovesItems ? 'startup.remove.title' : 'startup.change.title') }}</DialogTitle>
          <DialogDescription :class="{ 'sr-only': !pendingPlan }">
            {{
              pendingPlan
                ? t('startup.change.descriptions.' + pendingPlan.desiredState, {
                    count: pendingPlan.items.length,
                  })
                : t('startup.change.checking')
            }}
          </DialogDescription>
        </MdDialogHeader>

        <div class="change-plan-body scrollbar-stable-end">
          <div v-if="preparingChange" class="change-loading" role="status">
            <MdSpinner />
            {{ t('startup.change.checking') }}
          </div>
          <template v-else-if="pendingPlan">
            <div
              v-if="pendingPlanRequiresElevation || pendingPlanOnlyAffectsFutureLaunches || pendingPlanRemovesItems"
              class="change-guidance"
            >
              <p v-if="pendingPlanRequiresElevation">
                <MdIcon :name="ICON_NAMES.shield" :size="15" />
                {{ t('startup.change.requiresElevation') }}
              </p>
              <p v-if="pendingPlanOnlyAffectsFutureLaunches">
                <MdIcon :name="ICON_NAMES.info" :size="15" />
                {{ t('startup.change.futureOnly') }}
              </p>
              <p v-if="pendingPlanRemovesItems">
                <MdIcon :name="ICON_NAMES.info" :size="15" />
                {{ t('startup.remove.guidance') }}
              </p>
            </div>
            <article v-for="item in pendingPlan.items" :key="item.itemId" class="change-item">
              <div>
                <strong>{{ item.displayName }}</strong>
                <small>{{ t('startup.sourceKinds.' + item.sourceKind) }}</small>
              </div>
              <span>
                {{ t('startup.configuredStates.' + item.previousState) }} →
                {{ t('startup.configuredStates.' + item.desiredState) }}
              </span>
              <p v-for="warning in item.warnings" :key="warning">
                {{ t('startup.change.warnings.' + warning) }}
              </p>
            </article>
            <article v-for="item in pendingPlan.skippedItems" :key="item.itemId" class="change-item skipped">
              <div>
                <strong>{{ item.displayName }}</strong>
                <small>{{ t('startup.change.skipReasons.' + item.reason) }}</small>
              </div>
            </article>
          </template>
        </div>

        <MdDialogFooter>
          <Button
            variant="outline"
            type="button"
            :disabled="cancellingChange"
            @click="preparingChange || executingChange ? cancelActiveChangeExecution() : updateChangeOpen(false)"
          >
            {{ cancellingChange ? t('startup.cancelling') : t('common.cancel') }}
          </Button>
          <Button
            :variant="pendingPlanRemovesItems ? 'destructive' : 'default'"
            type="button"
            :disabled="!pendingPlan?.items.length || preparingChange || executingChange"
            :aria-busy="executingChange"
            @click="emit('executeChange')"
          >
            <MdSpinner v-if="executingChange" size="small" />
            {{
              executingChange
                ? t('startup.change.applying')
                : t(pendingPlanRemovesItems ? 'startup.remove.confirm' : 'startup.change.confirm')
            }}
          </Button>
        </MdDialogFooter>
      </MdDialogContent>
    </Dialog>
  </MdPageShell>
</template>

<style scoped>
@reference "@assets/main.css";

.startup-change-dialog {
  display: grid;
  grid-template-rows: auto minmax(0, 1fr) auto;
}

.change-plan-body {
  min-height: 90px;
  overflow-y: auto;
  padding: 0 var(--layout-dialog-body-inline-padding) 14px;
}

.change-loading {
  display: flex;
  min-height: 84px;
  align-items: center;
  gap: 8px;
  color: var(--muted-foreground);
  font-size: 12px;
}

.change-item {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 3px 12px;
  padding: 8px 2px;
  font-size: 12px;
}

.change-item + .change-item {
  margin-top: 2px;
}

.change-item div {
  display: flex;
  min-width: 0;
  flex-direction: column;
}

.change-item small {
  color: var(--muted-foreground);
}

.change-item p {
  grid-column: 1 / -1;
  margin: 2px 0 0;
  color: var(--destructive);
}

.change-item.skipped {
  grid-template-columns: minmax(0, 1fr);
}

.change-guidance {
  display: grid;
  gap: 7px;
  margin-bottom: 10px;
  border-radius: 10px;
  padding: 10px 12px;
  background: var(--surface-primary-subtle);
}

.change-guidance p {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin: 0;
  color: var(--muted-foreground);
  font-size: 11.5px;
  line-height: 1.45;
}

.change-guidance :deep(svg) {
  flex: none;
  margin-top: 1px;
  color: var(--primary);
}
</style>
