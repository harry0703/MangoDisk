<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { toast } from 'vue-sonner';

import MdAiAction from '@/components/custom/md-ai-action.vue';
import { systemMaintenanceAiContext } from './system-maintenance-ai-context';
import MdCategoryFilter from '@/components/custom/md-category-filter.vue';
import MdCatalogList from '@/components/custom/md-catalog-list.vue';
import MdCatalogListItem from '@/components/custom/md-catalog-list-item.vue';
import MdConfirmDialog from '@/components/custom/md-confirm-dialog.vue';
import MdEmptyState from '@/components/custom/md-empty-state.vue';
import MdOperationProgress from '@/components/custom/md-operation-progress.vue';
import MdOperationWorkspace from '@/components/custom/md-operation-workspace.vue';
import MdAiWorkspace from '@/layouts/components/md-ai-workspace.vue';
import { useAiStore } from '@/stores/ai-store';
import MdPageShell from '@/components/custom/md-page-shell.vue';
import MdResultFilterToolbar from '@/components/custom/md-result-filter-toolbar.vue';
import MdResultWorkspace from '@/components/custom/md-result-workspace.vue';
import MdSpinner from '@/components/custom/md-spinner.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { Button } from '@/components/ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import type {
  SystemMaintenanceCategory,
  SystemMaintenanceItem,
  SystemMaintenanceJob,
} from '@/lib/models/system-maintenance';
import { ICON_NAMES, OPERATION_PROGRESS_CLOCK_INTERVAL_MS } from '@/lib/models/ui';
import { useSystemMaintenanceStore } from '@/stores/system-maintenance-store';

const { t, locale } = useI18n({ useScope: 'global' });
const aiStore = useAiStore();
const store = useSystemMaintenanceStore();
type MaintenanceFilter = 'all' | 'recommended' | SystemMaintenanceCategory;

const activeCategory = ref<MaintenanceFilter>('all');
const maintenanceScroll = ref<InstanceType<typeof MdCatalogList> | null>(null);
const confirmationOpen = ref(false);
const pendingExecutionId = ref<string | null>(null);
const clockMs = ref(Date.now());
let clockTimer: ReturnType<typeof setInterval> | undefined;
const categories: SystemMaintenanceCategory[] = ['systemRepair', 'searchAndInterface', 'network'];

const busy = computed(() => store.scanning || store.executing);
const visibleItems = computed(() =>
  (store.catalog?.items ?? []).filter(item => {
    if (activeCategory.value === 'recommended') return item.status === 'recommended';
    return activeCategory.value === 'all' || item.category === activeCategory.value;
  })
);
const pendingExecutionItem = computed(
  () => store.catalog?.items.find(item => item.taskId === pendingExecutionId.value) ?? null
);
const categoryOptions = computed(() => [
  {
    value: 'all',
    label: t('systemMaintenance.categories.all'),
    count: store.catalog?.summary.itemCount ?? 0,
  },
  {
    value: 'recommended',
    label: t('systemMaintenance.categories.recommended'),
    count: store.catalog?.summary.recommendedCount ?? 0,
  },
  ...categories
    .map(category => ({
      value: category,
      label: t(`systemMaintenance.categories.${category}`),
      count: store.catalog?.items.filter(item => item.category === category).length ?? 0,
    }))
    .filter(option => option.count > 0),
]);

watch(
  () => store.lastResult,
  result => {
    if (!result) return;
    const name = itemMessageById(result.taskId, 'name');
    if (result.failureReason === 'userCancelled') {
      if (result.mutationState === 'mayHaveChanged') {
        toast.warning(t('systemMaintenance.feedback.cancelledMayHaveChanged', { name }));
      } else {
        toast.info(t('systemMaintenance.feedback.cancelled', { name }));
      }
      return;
    }
    if (result.status === 'failed') {
      if (result.mutationState === 'mayHaveChanged') {
        toast.warning(t('systemMaintenance.feedback.mayHaveChanged', { name }));
        return;
      }
      const reason = result.failureReason ?? 'platformFailure';
      toast.warning(t(`systemMaintenance.feedback.failures.${reason}`, { name }));
      return;
    }
    if (result.requiresRestart) {
      toast.success(t('systemMaintenance.feedback.restartRequired', { name }));
      return;
    }
    toast.success(t(`systemMaintenance.feedback.${result.status}`, { name }));
  }
);

function explainItem(item: SystemMaintenanceItem) {
  if (!store.catalog) return;
  const context = systemMaintenanceAiContext(
    item,
    itemMessage(item, 'name'),
    itemMessage(item, 'description'),
    store.catalog.platform
  );
  void aiStore.show(context, locale.value);
}

function itemMessage(item: SystemMaintenanceItem, field: 'description' | 'name'): string {
  return itemMessageById(item.taskId, field);
}

function itemMessageById(taskId: string, field: 'description' | 'name'): string {
  return t(`systemMaintenance.items.${taskId.replaceAll('.', '_')}.${field}`);
}

function statusLabel(item: SystemMaintenanceItem): string {
  if (item.status === 'unavailable' && item.diagnostic) {
    return t(`systemMaintenance.diagnostics.${item.diagnostic}`);
  }
  return t(`systemMaintenance.statuses.${item.status}`);
}

function updateCategory(value: string) {
  if (value !== 'all' && value !== 'recommended' && !categories.includes(value as SystemMaintenanceCategory)) return;
  activeCategory.value = value as MaintenanceFilter;
  maintenanceScroll.value?.scrollTo({ top: 0 });
}

function requestExecution(item: SystemMaintenanceItem) {
  if (item.riskLevel === 'caution') {
    pendingExecutionId.value = item.taskId;
    confirmationOpen.value = true;
    return;
  }
  void store.execute(t('systemMaintenance.authorizationPrompt'), item.taskId);
}

function taskJob(item: SystemMaintenanceItem): SystemMaintenanceJob | null {
  return store.executionForTask(item.taskId);
}

function isRetryableJob(job: SystemMaintenanceJob | null): boolean {
  return job?.status === 'finished' && job.result?.status === 'failed' && job.result.mutationState === 'notChanged';
}

function showsElevation(item: SystemMaintenanceItem): boolean {
  const job = taskJob(item);
  return (
    item.requiresElevation &&
    item.status !== 'healthy' &&
    item.status !== 'unavailable' &&
    (job?.status !== 'finished' || isRetryableJob(job))
  );
}

function jobLabel(job: SystemMaintenanceJob): string {
  if (job.status === 'queued') return t('systemMaintenance.cancelQueued');
  if (job.status === 'cancelling') return t('loading.cancelling');
  if (job.status === 'finished') return t('systemMaintenance.retryOne');
  if (job.cancelable) return t('systemMaintenance.cancelExecution');
  return t('systemMaintenance.executingOne');
}

function terminalJobLabel(job: SystemMaintenanceJob): string {
  if (job.result?.status === 'started') return t('systemMaintenance.systemProcessing');
  if (job.result?.status === 'completed') return t('systemMaintenance.completedOne');
  return t('systemMaintenance.refreshRequired');
}

function terminalJobIcon(job: SystemMaintenanceJob) {
  if (job.result?.status === 'failed') return ICON_NAMES.info;
  if (job.result?.status === 'started') return ICON_NAMES.clock;
  return ICON_NAMES.check;
}

function progressPhaseLabel(job: SystemMaintenanceJob): string {
  return t(`systemMaintenance.progress.phases.${job.progress?.phase ?? 'preparing'}`);
}

function elapsedLabel(job: SystemMaintenanceJob): string {
  const startedAtMs = job.startedAtMs ?? job.queuedAtMs;
  const seconds = Math.max(0, Math.floor((clockMs.value - startedAtMs) / 1000));
  if (seconds < 60) return t('systemMaintenance.progress.elapsedSeconds', { seconds });
  return t('systemMaintenance.progress.elapsedMinutesSeconds', {
    minutes: Math.floor(seconds / 60),
    seconds: seconds % 60,
  });
}

function progressSummary(job: SystemMaintenanceJob): string {
  const values: string[] = [];
  if (job.progress?.currentStep && job.progress.totalSteps) {
    values.push(
      t('systemMaintenance.progress.step', {
        current: job.progress.currentStep,
        total: job.progress.totalSteps,
      })
    );
  }
  values.push(progressPhaseLabel(job));
  if (job.progress?.percent !== null && job.progress?.percent !== undefined) {
    values.push(`${job.progress.percent}%`);
  }
  values.push(elapsedLabel(job));
  return values.join(' · ');
}

function progressWidth(job: SystemMaintenanceJob): string | undefined {
  return job.progress?.percent === null || job.progress?.percent === undefined ? undefined : `${job.progress.percent}%`;
}

function isItemActionDisabled(item: SystemMaintenanceItem): boolean {
  const job = taskJob(item);
  return store.scanning || Boolean(job && job.status !== 'finished' && !job.cancelable);
}

function confirmExecution() {
  const taskId = pendingExecutionId.value;
  pendingExecutionId.value = null;
  confirmationOpen.value = false;
  if (taskId) void store.execute(t('systemMaintenance.authorizationPrompt'), taskId);
}

function handleItemAction(item: SystemMaintenanceItem) {
  const job = taskJob(item);
  if (job && job.status !== 'finished') {
    if (job.cancelable) void store.cancelExecution(job.executionId);
    return;
  }
  requestExecution(item);
}

onMounted(() => {
  clockTimer = setInterval(() => {
    clockMs.value = Date.now();
  }, OPERATION_PROGRESS_CLOCK_INTERVAL_MS);
  void store.initialize();
});

onUnmounted(() => {
  if (clockTimer) clearInterval(clockTimer);
});
watch(
  () => [store.catalog, store.lastResult, store.scanning, store.executing],
  () => aiStore.dismissModule('systemMaintenance')
);
</script>

<template>
  <MdPageShell class="maintenance-page" content-mode="workspace" :title="t('systemMaintenance.title')">
    <template #overlay><MdAiWorkspace module="systemMaintenance" /></template>
    <template #actions>
      <Button variant="outline" :disabled="busy" @click="store.scan()">
        <MdIcon :name="ICON_NAMES.refresh" :size="17" />
        {{ t('systemMaintenance.rescan') }}
      </Button>
    </template>

    <MdResultWorkspace v-if="store.catalog" class="maintenance-workspace">
      <template #header>
        <MdResultFilterToolbar>
          <MdCategoryFilter
            :model-value="activeCategory"
            :options="categoryOptions"
            :disabled="store.scanning"
            :accessibility-label="t('systemMaintenance.filterCategory')"
            @update:model-value="updateCategory"
          />
        </MdResultFilterToolbar>
      </template>

      <MdCatalogList ref="maintenanceScroll">
        <MdEmptyState
          v-if="!visibleItems.length"
          :icon-name="ICON_NAMES.systemMaintenance"
          :title="t('systemMaintenance.empty.title')"
          :description="t('systemMaintenance.empty.description')"
          compact
        />
        <section v-else>
          <MdCatalogListItem
            v-for="item in visibleItems"
            :key="item.taskId"
            class="md-ai-hover-row"
            :title="itemMessage(item, 'name')"
          >
            <template #description>
              <span class="item-details">
                <small
                  v-if="taskJob(item)?.status === 'running' || taskJob(item)?.status === 'cancelling'"
                  class="item-progress-summary"
                >
                  {{ progressSummary(taskJob(item)!) }}
                </small>
                <small v-else class="item-description">{{ itemMessage(item, 'description') }}</small>
                <span
                  v-if="taskJob(item)?.status === 'running' || taskJob(item)?.status === 'cancelling'"
                  class="item-progress-track"
                  :class="{ 'is-indeterminate': progressWidth(taskJob(item)!) === undefined }"
                  aria-hidden="true"
                >
                  <i class="md-operational-motion" :style="{ width: progressWidth(taskJob(item)!) }" />
                </span>
              </span>
            </template>
            <template #actions>
              <MdAiAction
                :name="itemMessage(item, 'name')"
                :disabled="store.scanning || Boolean(taskJob(item) && taskJob(item)?.status !== 'finished')"
                @explain="explainItem(item)"
              />
              <Tooltip v-if="showsElevation(item)">
                <TooltipTrigger as-child>
                  <span class="item-admin" :aria-label="t('systemMaintenance.statuses.requiresElevation')">
                    <MdIcon :name="ICON_NAMES.shield" :size="14" />
                  </span>
                </TooltipTrigger>
                <TooltipContent side="top" :side-offset="6">
                  {{ t('systemMaintenance.statuses.requiresElevation') }}
                </TooltipContent>
              </Tooltip>
              <span v-if="item.status === 'healthy'" class="item-state is-healthy">
                <MdIcon :name="ICON_NAMES.check" :size="15" />
                {{ statusLabel(item) }}
              </span>
              <span v-else-if="item.status === 'unavailable'" class="item-state">
                {{ statusLabel(item) }}
              </span>
              <span
                v-else-if="taskJob(item)?.status === 'finished' && !isRetryableJob(taskJob(item))"
                class="item-state"
                :class="{ 'is-healthy': taskJob(item)?.result?.status === 'completed' }"
              >
                <MdIcon :name="terminalJobIcon(taskJob(item)!)" :size="15" />
                {{ terminalJobLabel(taskJob(item)!) }}
              </span>
              <Button
                v-else
                class="item-execute-button"
                variant="outline"
                size="sm"
                :disabled="isItemActionDisabled(item)"
                @click="handleItemAction(item)"
              >
                <MdSpinner
                  v-if="taskJob(item)?.status === 'running' || taskJob(item)?.status === 'cancelling'"
                  size="small"
                />
                {{ taskJob(item) ? jobLabel(taskJob(item)!) : t('systemMaintenance.executeOne') }}
              </Button>
            </template>
          </MdCatalogListItem>
        </section>
      </MdCatalogList>
    </MdResultWorkspace>

    <MdOperationWorkspace v-else-if="store.scanning || !store.scanFailed">
      <MdOperationProgress
        :icon-name="ICON_NAMES.systemMaintenance"
        :title="t('systemMaintenance.scanning')"
        :progress="null"
        :path-label="t('systemMaintenance.scanning')"
        :preparing-text="t('systemMaintenance.scanningDescription')"
        :hint="t('systemMaintenance.scanningDescription')"
        :show-traversal-details="false"
        :show-step-progress="false"
        :cancelable="false"
        :cancel-disabled="true"
      />
    </MdOperationWorkspace>

    <MdOperationWorkspace v-else>
      <MdEmptyState
        :icon-name="ICON_NAMES.info"
        :title="t('systemMaintenance.scanFailedTitle')"
        :description="t('systemMaintenance.scanFailedDescription')"
      >
        <Button variant="outline" @click="store.retryScan()">
          <MdIcon :name="ICON_NAMES.refresh" :size="17" />
          {{ t('common.retry') }}
        </Button>
      </MdEmptyState>
    </MdOperationWorkspace>

    <MdConfirmDialog
      v-model:open="confirmationOpen"
      :title="t('systemMaintenance.confirmation.title')"
      :description="t('systemMaintenance.confirmation.description')"
      :cancel-label="t('common.cancel')"
      :confirm-label="t('systemMaintenance.confirmation.confirm')"
      size="standard"
      @confirm="confirmExecution"
    >
      <div v-if="pendingExecutionItem" class="maintenance-confirm-item">
        <strong>{{ itemMessage(pendingExecutionItem, 'name') }}</strong>
        <span>{{ itemMessage(pendingExecutionItem, 'description') }}</span>
      </div>
    </MdConfirmDialog>
  </MdPageShell>
</template>

<style scoped>
@reference "@assets/main.css";

.maintenance-page :deep(.md-page-content) {
  gap: 0;
}

.maintenance-workspace {
  background: var(--card);
}

.maintenance-confirm-item {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 3px;
  border-width: 1px;
  border-radius: 9px;
  padding: 10px 12px;
  @apply border-border/70 bg-muted/40;
}

.maintenance-confirm-item strong {
  font-size: var(--font-content-primary);
  font-weight: 600;
}

.maintenance-confirm-item span {
  @apply text-muted-foreground;
  font-size: var(--font-content-secondary);
  line-height: 1.5;
}

.item-details {
  position: relative;
  display: flex;
  width: 100%;
  min-width: 0;
  height: 20px;
  align-items: center;
  padding-bottom: 3px;
}

.item-details small {
  font-size: var(--font-content-secondary);
}

.item-description {
  overflow: hidden;
  color: var(--muted-foreground);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.item-progress-summary {
  overflow: hidden;
  color: var(--primary);
  font-size: var(--font-content-meta);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.item-progress-track {
  position: absolute;
  bottom: 0;
  left: 0;
  overflow: hidden;
  width: min(420px, 72%);
  height: 2px;
  border-radius: 999px;
  background: color-mix(in oklab, var(--primary) 14%, transparent);
}

.item-progress-track i {
  display: block;
  height: 100%;
  border-radius: inherit;
  background: var(--primary);
  transition: width 240ms ease-out;
}

.item-progress-track.is-indeterminate i {
  width: 34%;
  animation: maintenance-progress-slide 1.35s ease-in-out infinite;
}

@keyframes maintenance-progress-slide {
  from {
    transform: translateX(-110%);
  }
  to {
    transform: translateX(310%);
  }
}

.item-admin {
  display: grid;
  width: 24px;
  height: 24px;
  flex: none;
  place-items: center;
  color: var(--muted-foreground);
}

.item-state {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  color: var(--muted-foreground);
  font-size: var(--font-content-meta);
  white-space: nowrap;
}

.item-state.is-healthy {
  color: var(--success);
}

.item-execute-button {
  min-width: 72px;
  border-color: color-mix(in oklab, var(--primary) 28%, var(--border));
  color: var(--primary);
}
</style>
