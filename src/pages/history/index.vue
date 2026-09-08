<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { computed, ref } from 'vue';

import MdEmptyState from '@/components/custom/md-empty-state.vue';
import MdDestructiveActionDialog from '@/components/custom/md-destructive-action-dialog.vue';
import MdDialogContent from '@/components/custom/md-dialog-content.vue';
import MdDialogFooter from '@/components/custom/md-dialog-footer.vue';
import MdDialogHeader from '@/components/custom/md-dialog-header.vue';
import MdMiddleEllipsis from '@/components/custom/md-middle-ellipsis.vue';
import MdPageShell from '@/components/custom/md-page-shell.vue';
import MdResultTable from '@/components/custom/md-result-table.vue';
import MdResultTableRow from '@/components/custom/md-result-table-row.vue';
import MdResultWorkspace from '@/components/custom/md-result-workspace.vue';
import MdStatusBadge from '@/components/custom/md-status-badge.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { Button } from '@/components/ui/button';
import { Dialog, DialogDescription, DialogTitle } from '@/components/ui/dialog';
import { ICON_NAMES, type IconName } from '@/lib/models/ui';
import type { ApplicationUninstallApplicationDetails, PresentedOperationRecord } from '@/lib/models/history';
import type { ApplicationLeftoverActionResult, ApplicationUninstallActionResult } from '@/lib/models/application';
import { ByteSizeService } from '@/lib/services/byte-size-service';
import * as FormatUtils from '@/lib/utils/format';
import * as PathUtils from '@/lib/utils/path';

const { locale, t } = useI18n({ useScope: 'global' });

defineProps<{ history: PresentedOperationRecord[]; busy: boolean }>();
const emit = defineEmits<{ clear: [] }>();

const detailOpen = ref(false);
const clearConfirmOpen = ref(false);
const selectedRecord = ref<PresentedOperationRecord | null>(null);
const selectedDetails = computed(() => selectedRecord.value?.details.payload ?? null);
const usesTallDetailDialog = computed(() => (selectedRecord.value?.selectedItemCount ?? 0) > 4);
const selectedUninstallApplications = computed<ApplicationUninstallApplicationDetails[]>(() => {
  const record = selectedRecord.value;
  if (record?.details.type !== 'applicationUninstall') return [];
  return record.details.payload.applications;
});

function openDetails(record: PresentedOperationRecord) {
  selectedRecord.value = record;
  detailOpen.value = true;
}

function releasedBytesAreEstimated(record: PresentedOperationRecord): boolean {
  return record.releasedBytesIsEstimate;
}

function displayedReleasedBytes(record: PresentedOperationRecord): number {
  if (record.details.type === 'applicationUninstall') {
    // Native uninstall bytes are estimates, but their action results still
    // identify which applications actually completed. Summing those results
    // prevents cancelled applications from falling back to the batch estimate.
    return record.details.payload.applications.reduce(
      (total, application) => total + uninstallApplicationReleasedBytes(application),
      0
    );
  }
  return releasedBytesAreEstimated(record) ? record.expectedBytes : (record.releasedBytes ?? 0);
}

function releasedBytesLabel(record: PresentedOperationRecord): string {
  return t(releasedBytesAreEstimated(record) ? 'history.estimated' : 'history.actual');
}

function operationTitle(record: PresentedOperationRecord): string {
  return t(`history.categories.${record.category}`);
}

function operationIcon(record: PresentedOperationRecord): IconName {
  if (record.category === 'deepCleanup') return ICON_NAMES.deepCleanup;
  if (record.category === 'largeFileCleanup') return ICON_NAMES.largeFiles;
  if (record.category === 'duplicateFileCleanup') return ICON_NAMES.duplicateFiles;
  if (record.category === 'startupManagement') return ICON_NAMES.startup;
  if (record.category === 'systemOptimization') return ICON_NAMES.systemOptimization;
  if (record.category === 'privacyCleanup') return ICON_NAMES.shield;
  return ICON_NAMES.uninstall;
}

function countBasedRecord(record: PresentedOperationRecord): boolean {
  return (
    record.category === 'startupManagement' ||
    record.category === 'systemOptimization' ||
    record.category === 'privacyCleanup'
  );
}

function confirmClearHistory() {
  clearConfirmOpen.value = false;
  emit('clear');
}

function selectedItemCount(record: PresentedOperationRecord): number {
  return record.selectedItemCount;
}

function recordSummary(record: PresentedOperationRecord): string {
  if (record.category === 'startupManagement') {
    return t('history.startupRecordSummary', {
      selected: FormatUtils.integer(record.selectedItemCount),
      changed: FormatUtils.integer(record.affectedItemCount),
    });
  }
  if (record.category === 'systemOptimization') {
    return t('history.systemOptimizationRecordSummary', {
      selected: FormatUtils.integer(record.selectedItemCount),
      changed: FormatUtils.integer(record.affectedItemCount),
    });
  }
  if (record.category === 'privacyCleanup') {
    return t('history.privacyRecordSummary', {
      selected: FormatUtils.integer(record.selectedItemCount),
      changed: FormatUtils.integer(record.affectedItemCount),
    });
  }
  const key = record.category === 'applicationUninstall' ? 'history.uninstallRecordSummary' : 'history.recordSummary';
  return t(
    key,
    {
      items: FormatUtils.integer(selectedItemCount(record)),
      files: FormatUtils.integer(record.affectedItemCount),
    },
    record.category === 'applicationUninstall' ? record.selectedItemCount : record.affectedItemCount
  );
}

function startupHistoryItemMessage(
  item: Extract<PresentedOperationRecord, { category: 'startupManagement' }>['details']['payload']['items'][number]
): string {
  if (item.failureReason) return t(`history.startupFailureReasons.${item.failureReason}`);
  return t(`history.startupStatuses.${item.status}`);
}

function systemOptimizationItemName(settingId: string): string {
  return t(`systemOptimization.items.${settingId.replaceAll('.', '_')}.name`);
}

function systemOptimizationItemMessage(
  item: Extract<PresentedOperationRecord, { category: 'systemOptimization' }>['details']['payload']['items'][number]
): string {
  if (item.failureReason) return t(`history.systemOptimizationFailureReasons.${item.failureReason}`);
  return t(`history.systemOptimizationStatuses.${item.status}`);
}

function systemOptimizationItemAction(
  item: Extract<PresentedOperationRecord, { category: 'systemOptimization' }>['details']['payload']['items'][number],
  restoration: boolean
): string {
  if (item.status === 'failed') return t('history.systemOptimizationChangeFailed');
  return (item.desiredOptimized ?? !restoration)
    ? t('history.systemOptimizationApplied')
    : t('history.systemOptimizationRestored');
}

function leftoverActionMessage(action: ApplicationLeftoverActionResult): string {
  if (action.reason) return t(`history.applicationLeftoverReasons.${action.reason}`);
  return t(`history.applicationLeftoverStatuses.${action.status}`);
}

function uninstallActionMessage(action: ApplicationUninstallActionResult): string {
  if (action.reason === 'externalUninstallerContinuing') {
    return t('history.applicationUninstallReasons.externalUninstallerContinuing');
  }
  if (action.reason) return t(`history.applicationUninstallReasons.${action.reason}`);
  if (action.status === 'cancelled') return t('history.applicationUninstallStatuses.cancelled');
  return t(`history.applicationUninstallStatuses.${action.status}`);
}

function uninstallApplicationFailed(application: ApplicationUninstallApplicationDetails): boolean {
  return application.actions.some(action => action.status === 'failed');
}

function uninstallApplicationCancelled(application: ApplicationUninstallApplicationDetails): boolean {
  return application.actions.some(action => action.status === 'cancelled');
}

function uninstallApplicationMessage(application: ApplicationUninstallApplicationDetails): string {
  const failedAction = application.actions.find(action => action.status === 'failed');
  if (failedAction) return uninstallActionMessage(failedAction);
  const cancelledAction = application.actions.find(action => action.status === 'cancelled');
  if (cancelledAction) return uninstallActionMessage(cancelledAction);
  if (application.restartRequired) return t('applicationUninstall.restartRequired');
  return t('history.applicationUninstallStatuses.completed');
}

function uninstallApplicationExpectedBytes(application: ApplicationUninstallApplicationDetails): number {
  return application.actions.reduce((total, action) => total + action.expectedBytes, 0);
}

function uninstallApplicationReleasedBytes(application: ApplicationUninstallApplicationDetails): number {
  return application.actions.reduce((total, action) => total + action.releasedBytes, 0);
}

function displayedUninstallApplicationBytes(application: ApplicationUninstallApplicationDetails): number {
  return uninstallApplicationReleasedBytes(application);
}

function fileCleanupActionMessage(status: 'deleted' | 'failed'): string {
  return t(status === 'failed' ? 'history.deleteFailed' : 'history.deleted');
}
</script>

<template>
  <MdPageShell class="@container/history" content-mode="workspace" content-width="wide" :title="t('history.title')">
    <template v-if="history.length" #actions>
      <Button
        class="clear-history-button"
        variant="ghost"
        type="button"
        :disabled="busy"
        @click="clearConfirmOpen = true"
      >
        <MdIcon :name="ICON_NAMES.trash" :size="14" />
        {{ t('history.clear') }}
      </Button>
    </template>

    <MdResultWorkspace v-if="!history.length" class="history-empty-workspace">
      <MdEmptyState
        :icon-name="ICON_NAMES.history"
        :title="t('history.emptyTitle')"
        :description="t('history.emptyDescription')"
      />
    </MdResultWorkspace>

    <MdResultWorkspace v-else>
      <MdResultTable class="history-list" header-variant="plain">
        <template #header>
          <div class="history-list-header" aria-hidden="true">
            <span>{{ t('history.operation') }}</span>
            <span>{{ t('history.planned') }}</span>
            <span>{{ t('history.resultSpace') }}</span>
          </div>
        </template>
        <MdResultTableRow v-for="record in history" :key="record.operationId" class="history-record-row">
          <button class="record" type="button" @click="openDetails(record)">
            <span class="record-icon" :class="{ preview: record.dryRun }">
              <MdIcon :name="record.dryRun ? ICON_NAMES.search : operationIcon(record)" :size="18" />
            </span>
            <span class="record-main">
              <span>
                <strong class="md-result-primary">{{ operationTitle(record) }}</strong>
                <MdStatusBadge v-if="record.failedItemCount" size="compact" tone="warning">
                  {{ t('history.statusWarnings') }}
                </MdStatusBadge>
              </span>
              <small>
                {{ FormatUtils.dateTime(record.startedAtMs, locale) }} ·
                {{ recordSummary(record) }}
              </small>
            </span>
            <strong class="record-byte">
              {{
                countBasedRecord(record)
                  ? FormatUtils.integer(record.selectedItemCount)
                  : ByteSizeService.bytes(record.expectedBytes)
              }}
            </strong>
            <strong class="record-byte">
              {{
                countBasedRecord(record)
                  ? FormatUtils.integer(record.affectedItemCount)
                  : ByteSizeService.bytes(displayedReleasedBytes(record))
              }}
            </strong>
            <MdIcon class="record-chevron" :name="ICON_NAMES.chevronRight" :size="17" />
          </button>
        </MdResultTableRow>
      </MdResultTable>
    </MdResultWorkspace>

    <Dialog v-model:open="detailOpen">
      <MdDialogContent
        class="min-h-0 grid-rows-[auto_auto_minmax(0,1fr)_auto]"
        :height="usesTallDetailDialog ? 'tall' : 'auto'"
        size="wide"
      >
        <MdDialogHeader>
          <DialogTitle>{{ t('history.detailTitle') }}</DialogTitle>
          <DialogDescription>{{ t('history.detailDescription') }}</DialogDescription>
        </MdDialogHeader>

        <div v-if="selectedRecord && selectedDetails" class="detail-overview">
          <div v-if="countBasedRecord(selectedRecord)" class="detail-summary">
            <span
              ><small>{{ t('history.selectedItems') }}</small
              ><strong>{{ FormatUtils.integer(selectedRecord.selectedItemCount) }}</strong></span
            >
            <span
              ><small>{{ t('history.changedItems') }}</small
              ><strong>{{ FormatUtils.integer(selectedRecord.affectedItemCount) }}</strong></span
            >
            <span
              ><small>{{ t('history.failedItems') }}</small
              ><strong :class="{ warning: selectedRecord.failedItemCount }">{{
                FormatUtils.integer(selectedRecord.failedItemCount)
              }}</strong></span
            >
          </div>
          <div v-else class="detail-summary">
            <span
              ><small>{{ t('history.expected') }}</small
              ><strong>{{ ByteSizeService.bytes(selectedRecord.expectedBytes) }}</strong></span
            >
            <span
              ><small>{{ releasedBytesLabel(selectedRecord) }}</small
              ><strong>{{ ByteSizeService.bytes(displayedReleasedBytes(selectedRecord)) }}</strong></span
            >
            <span
              ><small>{{
                t(
                  selectedRecord.category === 'applicationUninstall'
                    ? 'history.affectedApplications'
                    : 'history.affectedItems'
                )
              }}</small
              ><strong>{{ FormatUtils.integer(selectedRecord.affectedItemCount) }}</strong></span
            >
            <span
              ><small>{{
                t(
                  selectedRecord.category === 'applicationUninstall'
                    ? 'history.failedApplications'
                    : 'history.failedItems'
                )
              }}</small
              ><strong :class="{ warning: selectedRecord.failedItemCount }">{{
                FormatUtils.integer(selectedRecord.failedItemCount)
              }}</strong></span
            >
          </div>
          <div class="detail-meta">
            <span>{{ FormatUtils.dateTime(selectedRecord.startedAtMs, locale) }}</span>
            <span>
              {{ t('history.duration') }}
              {{
                t(
                  'loading.elapsedSeconds',
                  {
                    count: FormatUtils.integer(
                      FormatUtils.elapsedSeconds(selectedRecord.startedAtMs, selectedRecord.finishedAtMs)
                    ),
                  },
                  FormatUtils.elapsedSeconds(selectedRecord.startedAtMs, selectedRecord.finishedAtMs)
                )
              }}
            </span>
          </div>
        </div>

        <MdResultTable v-if="selectedRecord && selectedDetails" class="detail-actions">
          <template #header>
            <h3>
              {{
                t(
                  selectedRecord.details.type === 'applicationUninstall'
                    ? 'history.uninstalledApplications'
                    : selectedRecord.details.type === 'systemOptimization'
                      ? 'history.systemSettings'
                      : selectedRecord.details.type === 'privacyCleanup'
                        ? 'history.privacyItems'
                        : selectedRecord.details.type === 'startupManagement'
                          ? 'history.startupItems'
                          : 'history.cleanupItems'
                )
              }}
            </h3>
          </template>
          <template
            v-if="
              selectedRecord.details.type === 'deepCleanup' && selectedRecord.details.payload.cleanup?.actions.length
            "
          >
            <div
              v-for="action in selectedRecord.details.payload.cleanup.actions"
              :key="action.ruleId"
              class="detail-action"
            >
              <span class="action-status" :class="{ warning: action.failedItemCount }">
                <MdIcon :name="action.failedItemCount ? ICON_NAMES.info : ICON_NAMES.check" :size="13" />
              </span>
              <span
                ><strong>{{ action.name }}</strong
                ><small>{{ action.message }}</small></span
              >
              <span>
                <small>{{ t('history.expected') }} {{ ByteSizeService.bytes(action.bytesExpected) }}</small>
                <strong>{{ t('history.actual') }} {{ ByteSizeService.bytes(action.releasedBytes) }}</strong>
              </span>
            </div>
          </template>
          <template v-else-if="selectedRecord.details.type === 'startupManagement'">
            <div v-for="item in selectedRecord.details.payload.items" :key="item.itemId" class="detail-action">
              <span class="action-status" :class="{ warning: item.status === 'failed' }">
                <MdIcon :name="item.status === 'failed' ? ICON_NAMES.info : ICON_NAMES.check" :size="13" />
              </span>
              <span>
                <strong>{{ item.displayName || '—' }}</strong>
                <small>{{ startupHistoryItemMessage(item) }}</small>
              </span>
              <span>
                <small>{{ t(`startup.configuredStates.${item.previousState}`) }}</small>
                <strong>→ {{ t(`startup.configuredStates.${item.desiredState}`) }}</strong>
              </span>
            </div>
          </template>
          <template v-else-if="selectedRecord.details.type === 'systemOptimization'">
            <div v-for="item in selectedRecord.details.payload.items" :key="item.settingId" class="detail-action">
              <span class="action-status" :class="{ warning: item.status === 'failed' }">
                <MdIcon :name="item.status === 'failed' ? ICON_NAMES.info : ICON_NAMES.check" :size="13" />
              </span>
              <span>
                <strong>{{ systemOptimizationItemName(item.settingId) }}</strong>
                <small>{{ systemOptimizationItemMessage(item) }}</small>
              </span>
              <span>
                <strong>{{ systemOptimizationItemAction(item, selectedRecord.details.payload.restoration) }}</strong>
              </span>
            </div>
          </template>
          <template v-else-if="selectedRecord.details.type === 'privacyCleanup'">
            <div
              v-for="item in selectedRecord.details.payload.items"
              :key="`${item.sourceId}:${item.kind}`"
              class="detail-action"
            >
              <span class="action-status" :class="{ warning: item.status === 'failed' || item.status === 'cancelled' }">
                <MdIcon
                  :name="item.status === 'failed' || item.status === 'cancelled' ? ICON_NAMES.info : ICON_NAMES.check"
                  :size="13"
                />
              </span>
              <span>
                <strong>{{ t(`privacy.kinds.${item.kind}`) }}</strong>
                <small>{{ t(`history.privacyStatuses.${item.status}`) }}</small>
              </span>
              <span
                ><strong>{{ FormatUtils.integer(item.affectedItemCount) }}</strong></span
              >
            </div>
          </template>
          <template
            v-if="
              selectedRecord.details.type === 'deepCleanup' &&
              selectedRecord.details.payload.applicationLeftovers?.actions.length
            "
          >
            <div
              v-for="action in selectedRecord.details.payload.applicationLeftovers.actions"
              :key="action.candidateId"
              class="detail-action"
            >
              <span class="action-status" :class="{ warning: action.status === 'failed' }">
                <MdIcon :name="action.status === 'failed' ? ICON_NAMES.info : ICON_NAMES.check" :size="13" />
              </span>
              <span
                ><strong>{{ action.applicationName || action.applicationIdentifier || action.candidateId }}</strong
                ><small>{{ leftoverActionMessage(action) }}</small></span
              >
              <span>
                <small>{{ t('history.expected') }} {{ ByteSizeService.bytes(action.expectedBytes) }}</small>
                <strong>{{ t('history.actual') }} {{ ByteSizeService.bytes(action.releasedBytes) }}</strong>
              </span>
            </div>
          </template>
          <template
            v-else-if="selectedRecord.details.type === 'applicationUninstall' && selectedUninstallApplications.length"
          >
            <div
              v-for="application in selectedUninstallApplications"
              :key="application.applicationId"
              class="detail-action"
            >
              <span
                class="action-status"
                :class="{
                  warning: uninstallApplicationFailed(application) || uninstallApplicationCancelled(application),
                }"
              >
                <MdIcon
                  :name="
                    uninstallApplicationFailed(application)
                      ? ICON_NAMES.info
                      : uninstallApplicationCancelled(application)
                        ? ICON_NAMES.minus
                        : ICON_NAMES.check
                  "
                  :size="13"
                />
              </span>
              <span
                ><strong :title="application.applicationIdentifier">{{ application.applicationName }}</strong
                ><small>{{ uninstallApplicationMessage(application) }}</small></span
              >
              <span>
                <small
                  >{{ t('history.expected') }}
                  {{ ByteSizeService.bytes(uninstallApplicationExpectedBytes(application)) }}</small
                >
                <strong v-if="uninstallApplicationCancelled(application)">
                  {{ t('history.notCounted') }}
                </strong>
                <strong v-else
                  >{{ releasedBytesLabel(selectedRecord) }}
                  {{ ByteSizeService.bytes(displayedUninstallApplicationBytes(application)) }}</strong
                >
              </span>
            </div>
          </template>
          <template
            v-if="
              selectedRecord.details.type === 'largeFileCleanup' ||
              selectedRecord.details.type === 'duplicateFileCleanup'
            "
          >
            <div v-for="item in selectedRecord.details.payload.items" :key="item.path" class="detail-action">
              <span class="action-status" :class="{ warning: item.status === 'failed' }">
                <MdIcon :name="item.status === 'failed' ? ICON_NAMES.info : ICON_NAMES.check" :size="13" />
              </span>
              <span class="detail-action-content">
                <strong><MdMiddleEllipsis :text="PathUtils.fileName(item.path)" /></strong>
                <small><MdMiddleEllipsis :text="PathUtils.display(item.path)" :tail-length="32" /></small>
              </span>
              <span class="detail-action-result">
                <strong>{{ fileCleanupActionMessage(item.status) }}</strong>
              </span>
            </div>
            <p v-if="selectedRecord.details.payload.omittedItemCount" class="detail-omitted">
              {{
                t(
                  'history.moreItemsOmitted',
                  {
                    count: FormatUtils.integer(selectedRecord.details.payload.omittedItemCount),
                  },
                  selectedRecord.details.payload.omittedItemCount
                )
              }}
            </p>
          </template>
        </MdResultTable>

        <MdDialogFooter>
          <Button variant="outline" type="button" @click="detailOpen = false">{{ t('common.close') }}</Button>
        </MdDialogFooter>
      </MdDialogContent>
    </Dialog>

    <MdDestructiveActionDialog
      v-model:open="clearConfirmOpen"
      :title="t('history.clearConfirmTitle')"
      :description="t('history.clearConfirmDescription')"
      :summary-label="t('history.clearConfirmScope')"
      :summary-value="t('history.clearConfirmCount', { count: FormatUtils.integer(history.length) }, history.length)"
      :note="t('history.clearConfirmNote')"
      :cancel-label="t('common.cancel')"
      :confirm-label="t('history.clearConfirmAction')"
      :busy="busy"
      @confirm="confirmClearHistory"
    />
  </MdPageShell>
</template>

<style scoped>
@reference "@assets/main.css";

.clear-history-button {
  @apply border-0 bg-transparent text-muted-foreground shadow-none hover:text-destructive;
}

.clear-history-button:hover {
  background: var(--surface-destructive-subtle);
}

.history-list {
  --result-table-content-inline-padding: 0px;
}

.history-list-header,
.record {
  display: grid;
  grid-template-columns: 30px minmax(0, 1fr) 96px 96px 18px;
  align-items: center;
  gap: 12px;
}

.history-list-header {
  width: 100%;
  min-height: 36px;
  padding: 7px 14px;
  @apply text-muted-foreground;
  font-size: var(--font-content-secondary);
}

.history-list-header span:first-child {
  grid-column: 1 / 3;
}

.history-list-header span:nth-child(n + 2) {
  text-align: right;
}

.record {
  width: 100%;
  min-height: 52px;
  border: 0;
  padding: 5px 14px;
  background: transparent;
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
  @apply focus-visible:outline-none;
}

.record-icon {
  display: grid;
  width: 28px;
  height: 28px;
  place-items: center;
  @apply text-success;
}

.record-icon.preview {
  @apply text-primary;
}

.record-main {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 2px;
}

.record-main > span {
  display: flex;
  align-items: center;
  gap: 8px;
}

.record-main strong {
  font-size: var(--font-content-primary);
}

.record-main small {
  overflow: hidden;
  @apply text-muted-foreground;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--font-content-secondary);
}

.record-byte {
  min-width: 0;
  text-align: right;
  font-size: var(--font-content-primary);
}

.record-chevron {
  @apply text-muted-foreground;
}

@container history (max-width: 720px) {
  .record {
    grid-template-columns: 28px minmax(0, 1fr) 18px;
  }

  .record-byte {
    display: none;
  }

  .history-list :deep(.result-table-header) {
    display: none;
  }
}

.detail-overview {
  padding: 0 24px;
}
.detail-summary {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 9px;
}
.detail-summary > span {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 5px;
  border-radius: 10px;
  padding: 12px;
  @apply bg-muted;
}
.detail-summary small,
.detail-meta,
.detail-action small {
  @apply text-muted-foreground;
  font-size: 10px;
}
.detail-summary strong {
  font-size: 17px;
}
.detail-summary strong.warning {
  @apply text-warning-foreground;
}
.detail-meta {
  display: flex;
  justify-content: space-between;
  padding: 11px 2px 14px;
}
.detail-actions {
  --result-table-content-inline-padding: 0px;

  min-height: 0;
  margin: 0 24px 20px;
  overflow: hidden;
  border-width: 1px;
  border-radius: 11px;
  @apply border-border;
}
.detail-actions :deep(.result-table-header) {
  @apply border-border bg-muted/45;
}
.detail-actions h3 {
  margin: 0;
  padding: 11px 14px;
  font-size: 12px;
}
.detail-action {
  display: grid;
  grid-template-columns: 24px minmax(0, 1fr) max-content;
  align-items: center;
  gap: 9px;
  border-top-width: 1px;
  padding: 11px 13px;
  @apply border-border;
}
.detail-action:first-of-type {
  border-top: 0;
}
.detail-action > span:nth-child(2),
.detail-action > span:last-child {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 3px;
}
.detail-action > span:last-child {
  align-items: flex-end;
  padding-left: 8px;
  white-space: nowrap;
}
.detail-action-content {
  overflow: hidden;
}
.detail-action-content > strong,
.detail-action-content > small {
  display: block;
  min-width: 0;
  width: 100%;
}
.detail-action strong {
  font-size: 11px;
}
.action-status {
  display: grid;
  width: 21px;
  height: 21px;
  place-items: center;
  border-radius: 50%;
  @apply text-success;
  background: var(--surface-success-subtle);
}
.action-status.warning {
  @apply text-warning-foreground;
  background: var(--surface-warning-subtle);
}
.detail-omitted {
  margin: 8px 0 0;
  @apply text-center text-muted-foreground;
  font-size: var(--font-content-secondary);
}
</style>
