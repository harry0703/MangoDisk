<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { toast } from 'vue-sonner';

import MdDialogContent from '@/components/custom/md-dialog-content.vue';
import MdEmptyState from '@/components/custom/md-empty-state.vue';
import MdLoadMoreButton from '@/components/custom/md-load-more-button.vue';
import MdOperationProgress from '@/components/custom/md-operation-progress.vue';
import MdOperationWorkspace from '@/components/custom/md-operation-workspace.vue';
import MdPageShell from '@/components/custom/md-page-shell.vue';
import MdResultFilterToolbar from '@/components/custom/md-result-filter-toolbar.vue';
import MdResultSearch from '@/components/custom/md-result-search.vue';
import MdResultSummary from '@/components/custom/md-result-summary.vue';
import MdResultTable from '@/components/custom/md-result-table.vue';
import MdResultWorkspace from '@/components/custom/md-result-workspace.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { Button } from '@/components/ui/button';
import { Dialog, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import type {
  StartupArtifact,
  StartupCatalog,
  StartupChangePlan,
  StartupChangeResult,
  StartupDesiredState,
  StartupOwnerGroup,
} from '@/lib/models/startup';
import { MACOS_PRIVACY_DESTINATION_IDS } from '@/lib/models/macos-permissions';
import { ICON_NAMES } from '@/lib/models/ui';
import { ApplicationIconService } from '@/lib/services/application-icon-service';
import { ClipboardService } from '@/lib/services/clipboard-service';
import { MacOsPermissionService } from '@/lib/services/macos-permission-service';
import { MacOsSystemSettingsService } from '@/lib/services/macos-system-settings-service';
import { OperatingSystemService } from '@/lib/services/operating-system-service';
import { AiAdvisorService } from '@/lib/services/ai-advisor-service';
import { FormatUtils } from '@/lib/utils/format';
import { RenderBatchUtils } from '@/lib/utils/render-batch';
import { useAppStore } from '@/stores/app-store';

import MdStartupRow from './components/md-startup-row.vue';
import { startupGroupIconUrl } from './startup-brand-icon';

import {
  defaultStartupGroups,
  displayedArtifactsForGroup,
  filterAndSortStartupGroups,
  indexStartupArtifacts,
  manageableArtifactsForGroup,
  needsBackgroundTaskPermission,
  nextStartupDesiredState,
  removableOrphanArtifactsForGroup,
  startupFilterCounts,
  startupGroupManageableState,
  startupGroupStartTiming,
  startupGroupSubtitle,
  startupRevealPath,
  startupPlanRequiresReview,
  type StartupStateFilter,
} from './startup-view';

const STARTUP_RENDER_BATCH_SIZE = 120;

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

const { locale, t } = useI18n({ useScope: 'global' });
const appStore = useAppStore();
const query = ref('');
const stateFilter = ref<StartupStateFilter>('all');
const expandedGroupId = ref<string | null>(null);
const aiAnalyzing = ref(false);
const aiRecommendedIds = ref<string[]>([]);
let aiAbortController: AbortController | null = null;
let aiStaggerTimer: ReturnType<typeof setTimeout> | null = null;

const aiRecommendedSet = computed(() => new Set(aiRecommendedIds.value));

function isGroupAiRecommended(group: StartupOwnerGroup): boolean {
  return aiRecommendedSet.value.has(group.groupId) || group.itemIds.some(id => aiRecommendedSet.value.has(id));
}

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

async function startAiAdvisor() {
  if (aiAnalyzing.value) {
    abortAiAnalysis();
    return;
  }

  abortAiAnalysis();
  aiAnalyzing.value = true;
  aiRecommendedIds.value = [];

  const controller = new AbortController();
  aiAbortController = controller;

  try {
    const config = {
      apiKey: appStore.settings.aiApiKey,
      baseUrl: appStore.settings.aiApiBaseUrl,
      model: appStore.settings.aiModel,
    };

    const startupPayload = defaultGroups.value.map(group => ({
      ...group,
      id: group.groupId,
      name: group.name,
    }));
    const recommended = await AiAdvisorService.analyzeStartupItems(startupPayload, config, controller.signal);
    if (controller.signal.aborted) return;

    const resolvedIds = recommended.map(id => {
      if (defaultGroups.value.some(g => g.groupId === id)) return id;
      const num = Number(id);
      if (!isNaN(num) && startupPayload[num]) return startupPayload[num].groupId;
      return id;
    });

    if (!resolvedIds.length) {
      aiAnalyzing.value = false;
      aiAbortController = null;
      return;
    }

    // Smooth staggered animation (50ms interval)
    const idsToAdd = [...resolvedIds];
    let index = 0;
    const batchSize = Math.max(1, Math.floor(idsToAdd.length / 10));

    const processBatch = () => {
      if (controller.signal.aborted) return;
      const nextBatch = idsToAdd.slice(index, index + batchSize);
      index += batchSize;

      aiRecommendedIds.value = [...new Set([...aiRecommendedIds.value, ...nextBatch])];

      if (index < idsToAdd.length) {
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
const changeOpen = ref(false);
const permissionPromptOpen = ref(false);
const permissionPromptShown = ref(false);
const iconUrls = ref<ReadonlyMap<string, string>>(new Map());
const visibleCount = ref(STARTUP_RENDER_BATCH_SIZE);
const activeChangeItemIds = ref<ReadonlySet<string>>(new Set());
const quickChangeRequest = ref<{ itemIds: string[]; desiredState: StartupDesiredState } | null>(null);
const changeFeedback = ref<{ displayName: string; desiredState: StartupDesiredState } | null>(null);
const copiedActionKey = ref<string | null>(null);
let copyFeedbackTimer: ReturnType<typeof setTimeout> | null = null;
const isWindows = OperatingSystemService.isWindows();
const isMacOs = OperatingSystemService.isMacOs();

const artifactsById = computed(() => indexStartupArtifacts(props.catalog?.artifacts ?? []));
const defaultGroups = computed(() => defaultStartupGroups(props.catalog?.groups ?? [], artifactsById.value));
const filterCounts = computed(() => startupFilterCounts(defaultGroups.value, artifactsById.value));
const filteredGroups = computed(() =>
  filterAndSortStartupGroups(defaultGroups.value, artifactsById.value, query.value, stateFilter.value, locale.value)
);
const visibleGroups = computed(() => RenderBatchUtils.visibleItems(filteredGroups.value, visibleCount.value));
const remainingResultCount = computed(() =>
  RenderBatchUtils.remainingCount(filteredGroups.value.length, visibleGroups.value.length)
);
const changeBusy = computed(
  () => props.preparingChange || props.executingChange || props.cancellingChange || Boolean(props.pendingPlan)
);
const backgroundTasksNeedPermission = computed(() =>
  needsBackgroundTaskPermission(isMacOs, props.catalog?.coverage ?? [])
);
const pendingPlanRequiresElevation = computed(() =>
  Boolean(props.pendingPlan?.items.some(item => item.requiresElevation))
);
const pendingPlanOnlyAffectsFutureLaunches = computed(
  () => props.pendingPlan?.desiredState === 'disabled' && Boolean(props.pendingPlan.items.length)
);
const pendingPlanRemovesOrphans = computed(() => props.pendingPlan?.desiredState === 'removed');
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

watch([() => props.catalog?.scanId, query, stateFilter], () => {
  // Startup catalogs can still contain thousands of hidden system entries.
  // Reset progressive rendering after each visible result change to keep scrolling responsive.
  abortAiAnalysis();
  aiRecommendedIds.value = [];
  visibleCount.value = STARTUP_RENDER_BATCH_SIZE;
});

watch(
  () => defaultGroups.value.map(group => group.iconPath).filter((path): path is string => Boolean(path)),
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
    const request = quickChangeRequest.value;
    if (!plan || !request) return;
    quickChangeRequest.value = null;
    if (startupPlanRequiresReview(plan, request.itemIds.length)) {
      changeOpen.value = true;
      return;
    }
    emit('executeChange');
  }
);

watch(
  () => props.preparingChange,
  (preparing, wasPreparing) => {
    if (!preparing && wasPreparing && !props.pendingPlan && quickChangeRequest.value) {
      clearActiveChange();
      changeFeedback.value = null;
    }
  }
);

watch(
  () => props.executingChange,
  (executing, wasExecuting) => {
    if (executing || !wasExecuting) return;
    clearActiveChange();
    if (!props.lastChangeResult) {
      changeOpen.value = false;
      changeFeedback.value = null;
    }
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
      toast.warning(
        t(
          feedback?.desiredState === 'enabled'
            ? 'startup.change.partialEnableResult'
            : feedback?.desiredState === 'removed'
              ? 'startup.cleanup.partialResult'
              : 'startup.change.partialDisableResult',
          {
            name: feedback?.displayName ?? t('startup.title'),
            changed: result.changedCount,
            failed: result.failedCount,
          }
        )
      );
    } else {
      const messageKey =
        feedback?.desiredState === 'enabled'
          ? 'startup.change.enableSuccessResult'
          : feedback?.desiredState === 'removed'
            ? 'startup.cleanup.successResult'
            : 'startup.change.disableSuccessResult';
      toast.success(t(messageKey, { name: feedback?.displayName ?? t('startup.title') }));
    }
    changeFeedback.value = null;
    changeOpen.value = false;
    clearActiveChange();
  }
);

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
  if (changeFeedback.value?.desiredState === 'removed') return false;
  return manageableArtifacts(group).some(artifact => activeChangeItemIds.value.has(artifact.itemId));
}

function requestOrphanRemoval(group: StartupOwnerGroup) {
  requestChange(
    removableOrphanArtifactsForGroup(group, artifactsById.value).map(artifact => artifact.itemId),
    'removed',
    group.name
  );
}

function requestGroupChange(group: StartupOwnerGroup) {
  requestChange(
    manageableArtifacts(group).map(artifact => artifact.itemId),
    groupDesiredState(group),
    group.name
  );
}

function requestArtifactChange(artifact: StartupArtifact) {
  requestChange([artifact.itemId], artifactDesiredState(artifact), artifact.displayName);
}

function requestChange(itemIds: string[], desiredState: StartupDesiredState, displayName: string) {
  if (!itemIds.length || changeBusy.value) return;
  const selection = { itemIds, desiredState };
  activeChangeItemIds.value = new Set(itemIds);
  quickChangeRequest.value = selection;
  changeFeedback.value = { displayName, desiredState };
  emit('prepareChange', selection);
}

function clearActiveChange() {
  activeChangeItemIds.value = new Set();
  quickChangeRequest.value = null;
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

async function confirmBackgroundTaskPrivacySettings() {
  if (await openBackgroundTaskPrivacySettings()) permissionPromptOpen.value = false;
}

async function openLoginItemsSettings() {
  try {
    await MacOsSystemSettingsService.openLoginItems();
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
  abortAiAnalysis();
});

function updateChangeOpen(open: boolean) {
  if (props.executingChange) {
    if (!open) emit('cancelChangeExecution');
    return;
  }
  changeOpen.value = open;
  if (!open) {
    emit('cancelChange');
    clearActiveChange();
    changeFeedback.value = null;
  }
}
</script>

<template>
  <MdPageShell class="@container/startup" content-mode="workspace" :title="t('startup.title')">
    <template v-if="catalog && !scanning" #actions>
      <div class="header-actions">
        <Button
          variant="outline"
          type="button"
          :disabled="changeBusy"
          @click="startAiAdvisor"
        >
          <MdIcon
            :class="{ 'icon-spin': aiAnalyzing }"
            :name="aiAnalyzing ? ICON_NAMES.refresh : ICON_NAMES.smartSelect"
            :size="15"
            class="text-primary"
          />
          {{ aiAnalyzing ? t('largeFiles.selectionMode.analyzing') : t('largeFiles.selectionMode.smart') }}
        </Button>
        <Button variant="outline" type="button" :disabled="changeBusy || aiAnalyzing" @click="emit('scan')">
          <MdIcon :name="ICON_NAMES.refresh" :size="16" />
          {{ t('startup.rescan') }}
        </Button>
      </div>
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
          :title="t('startup.summary.programs', { count: FormatUtils.integer(defaultGroups.length) })"
          :metric-label="t('startup.summary.enabled')"
          :metric-value="FormatUtils.integer(filterCounts.enabled)"
        >
          <template #actions>
            <button
              v-if="backgroundTasksNeedPermission"
              class="summary-permission"
              type="button"
              @click="openBackgroundTaskPrivacySettings"
            >
              {{ t('startup.summary.permissionRequired') }}
              <MdIcon :name="ICON_NAMES.external" :size="13" />
            </button>
          </template>
        </MdResultSummary>
      </template>

      <template v-if="catalog" #header>
        <MdResultFilterToolbar>
          <div class="startup-filters scrollbar-hidden" :aria-label="t('startup.filterLabel')">
            <button
              v-for="filterId in ['all', 'enabled', 'disabled'] as const"
              :key="filterId"
              type="button"
              :class="{ active: stateFilter === filterId }"
              @click="stateFilter = filterId"
            >
              {{ t('startup.filters.' + filterId) }}
              <span>{{ FormatUtils.integer(filterCounts[filterId]) }}</span>
            </button>
          </div>
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
        v-else-if="!defaultGroups.length"
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
          :busy="changeBusy"
          :changing="isChanging(group)"
          :copied-action-key="copiedActionKey"
          :ai-recommended="isGroupAiRecommended(group)"
          :ai-recommended-item-ids="aiRecommendedIds"
          @toggle-expanded="expandedGroupId = expandedGroupId === group.groupId ? null : group.groupId"
          @toggle-group="requestGroupChange(group)"
          @toggle-artifact="requestArtifactChange"
          @remove-orphans="requestOrphanRemoval(group)"
          @reveal="emit('open', $event)"
          @copy="copyStartupValue"
          @open-system-settings="openLoginItemsSettings"
        />

        <MdLoadMoreButton
          v-if="remainingResultCount"
          :remaining-label="t('startup.remainingResults', { count: FormatUtils.integer(remainingResultCount) })"
          @load-more="loadMoreResults"
        />
      </MdResultTable>
    </MdResultWorkspace>

    <Dialog v-model:open="permissionPromptOpen">
      <MdDialogContent class="startup-permission-dialog p-0 sm:max-w-md">
        <DialogHeader class="px-5 pt-5 pr-14 pb-3">
          <DialogTitle>{{ t('startup.permission.title') }}</DialogTitle>
          <DialogDescription>{{ t('startup.permission.description') }}</DialogDescription>
        </DialogHeader>
        <p class="permission-instructions">
          <MdIcon :name="ICON_NAMES.info" :size="16" />
          {{ t('startup.permission.instructions') }}
        </p>
        <DialogFooter class="border-t border-border/70 px-5 py-3.5">
          <Button variant="outline" type="button" @click="permissionPromptOpen = false">
            {{ t('startup.permission.skip') }}
          </Button>
          <Button type="button" @click="confirmBackgroundTaskPrivacySettings">
            <MdIcon :name="ICON_NAMES.external" :size="15" />
            {{ t('startup.permission.openSettings') }}
          </Button>
        </DialogFooter>
      </MdDialogContent>
    </Dialog>

    <Dialog :open="changeOpen" @update:open="updateChangeOpen">
      <MdDialogContent class="startup-change-dialog max-h-[78vh] overflow-hidden p-0 sm:max-w-lg">
        <DialogHeader class="px-5 pt-5 pr-14 pb-3">
          <DialogTitle>{{
            t(pendingPlanRemovesOrphans ? 'startup.cleanup.title' : 'startup.change.title')
          }}</DialogTitle>
          <DialogDescription :class="{ 'sr-only': !pendingPlan }">
            {{
              pendingPlan
                ? t('startup.change.descriptions.' + pendingPlan.desiredState, {
                    count: pendingPlan.items.length,
                  })
                : t('startup.change.checking')
            }}
          </DialogDescription>
        </DialogHeader>

        <div class="change-plan-body scrollbar-stable-end">
          <div v-if="preparingChange" class="change-loading" role="status">
            <span class="change-spinner md-operational-motion" aria-hidden="true" />
            {{ t('startup.change.checking') }}
          </div>
          <template v-else-if="pendingPlan">
            <div
              v-if="pendingPlanRequiresElevation || pendingPlanOnlyAffectsFutureLaunches || pendingPlanRemovesOrphans"
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
              <p v-if="pendingPlanRemovesOrphans">
                <MdIcon :name="ICON_NAMES.info" :size="15" />
                {{ t('startup.cleanup.guidance') }}
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

        <DialogFooter class="border-t border-border/70 px-5 py-3.5">
          <Button
            variant="outline"
            type="button"
            :disabled="cancellingChange"
            @click="preparingChange || executingChange ? emit('cancelChangeExecution') : updateChangeOpen(false)"
          >
            {{ cancellingChange ? t('startup.cancelling') : t('common.cancel') }}
          </Button>
          <Button
            :variant="pendingPlanRemovesOrphans ? 'destructive' : 'default'"
            type="button"
            :disabled="!pendingPlan?.items.length || preparingChange || executingChange"
            :aria-busy="executingChange"
            @click="emit('executeChange')"
          >
            <span
              v-if="executingChange"
              class="change-action-spinner change-spinner md-operational-motion"
              aria-hidden="true"
            />
            {{
              executingChange
                ? t('startup.change.applying')
                : t(pendingPlanRemovesOrphans ? 'startup.cleanup.confirm' : 'startup.change.confirm')
            }}
          </Button>
        </DialogFooter>
      </MdDialogContent>
    </Dialog>
  </MdPageShell>
</template>

<style scoped>
@reference "@assets/main.css";

.header-actions {
  display: flex;
  min-width: 0;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
}

.summary-permission {
  display: flex;
  align-items: center;
  gap: 5px;
  border: 0;
  padding: 4px 0;
  background: transparent;
  color: var(--primary);
  font: inherit;
  font-size: 12px;
  cursor: pointer;
}

.summary-permission:hover {
  text-decoration: underline;
}

.permission-instructions {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin: 0 20px 18px;
  border-radius: 10px;
  padding: 10px 12px;
  background: color-mix(in oklab, var(--primary) 5%, var(--card));
  color: var(--muted-foreground);
  font-size: 12px;
  line-height: 1.5;
}

.permission-instructions :deep(svg) {
  flex: none;
  margin-top: 1px;
  color: var(--primary);
}

.startup-filters {
  display: flex;
  min-width: 0;
  gap: 4px;
  overflow-x: auto;
}

.startup-filters button {
  display: flex;
  flex: none;
  align-items: center;
  gap: 6px;
  border: 0;
  border-radius: 9px;
  min-height: 32px;
  padding: 5px 9px;
  background: transparent;
  color: var(--muted-foreground);
  font: inherit;
  font-size: 13px;
  cursor: pointer;
}

.startup-filters button:hover,
.startup-filters button.active {
  @apply bg-primary/10 text-primary;
}

.startup-filters button:focus-visible {
  @apply outline-none ring-2 ring-inset ring-ring/40;
}

.startup-filters button span {
  min-width: 16px;
  padding: 2px;
  color: var(--muted-foreground);
  font-size: 11px;
  text-align: center;
}

.change-spinner {
  flex: none;
  border: 1.5px solid color-mix(in oklab, currentColor 26%, transparent);
  border-top-color: currentColor;
  border-radius: 50%;
  animation: startup-change-spin 0.72s linear infinite;
}

.change-action-spinner {
  width: 13px;
  height: 13px;
}

.startup-change-dialog {
  display: grid;
  grid-template-rows: auto minmax(0, 1fr) auto;
}

.change-plan-body {
  min-height: 90px;
  overflow-y: auto;
  padding: 0 20px 16px;
}

.change-loading {
  display: flex;
  min-height: 84px;
  align-items: center;
  gap: 8px;
  color: var(--muted-foreground);
  font-size: 12px;
}

.change-loading .change-spinner {
  width: 15px;
  height: 15px;
}

@keyframes startup-change-spin {
  to {
    transform: rotate(360deg);
  }
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
  background: color-mix(in oklab, var(--primary) 5%, var(--card));
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

@container startup (max-width: 520px) {
  .startup-filters {
    gap: 2px;
  }
}
</style>
