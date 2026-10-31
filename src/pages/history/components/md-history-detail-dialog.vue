<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { Dialog, DialogDescription, DialogTitle } from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import MdDialogContent from '@/components/custom/md-dialog-content.vue';
import MdDialogHeader from '@/components/custom/md-dialog-header.vue';
import MdDialogFooter from '@/components/custom/md-dialog-footer.vue';
import MdResultTable from '@/components/custom/md-result-table.vue';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdMiddleEllipsis from '@/components/custom/md-middle-ellipsis.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import type { ApplicationUninstallApplicationDetails, PresentedOperationRecord } from '@/lib/models/history/history';
import type {
  ApplicationLeftoverActionResult,
  ApplicationUninstallActionResult,
} from '@/lib/models/applications/application';
import { FormatService } from '@/lib/services/presentation/format-service';
import { ByteSizeService } from '@/lib/services/settings/byte-size-service';
import * as FormatUtils from '@/lib/utils/presentation/format';
import * as PathUtils from '@/lib/utils/filesystem/path';
import { expectedActionBytes, releasedActionBytes } from '@/lib/utils/applications/application-uninstall-result';
import { usesItemCounts, displayedReleasedBytes } from '@/lib/utils/history/operation-record';

const { locale, t } = useI18n({ useScope: 'global' });
const props = defineProps<{ selectedRecord: PresentedOperationRecord | null }>();
const open = defineModel<boolean>('open', { required: true });
const selectedDetails = computed(() => props.selectedRecord?.details.payload ?? null);
const deletedFiles = computed(() => {
  const details = props.selectedRecord?.details;
  if (details?.type !== 'largeFileCleanup' && details?.type !== 'duplicateFileCleanup') return null;
  return details.payload.items.filter(item => item.status === 'deleted');
});
const omittedDeletedFileCount = computed(() =>
  Math.max(0, (props.selectedRecord?.affectedItemCount ?? 0) - (deletedFiles.value?.length ?? 0))
);
const usesTallDetailDialog = computed(
  () => (deletedFiles.value?.length ?? props.selectedRecord?.selectedItemCount ?? 0) > 4
);
const selectedUninstallApplications = computed(() => {
  const record = props.selectedRecord;
  return record?.details.type === 'applicationUninstall' ? record.details.payload.applications : [];
});
function releasedBytesLabel(record: PresentedOperationRecord): string {
  return t(record.releasedBytesIsEstimate ? 'history.estimated' : 'history.actual');
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
  return expectedActionBytes(application.actions);
}

function uninstallApplicationReleasedBytes(application: ApplicationUninstallApplicationDetails): number {
  return releasedActionBytes(application.actions);
}

function displayedUninstallApplicationBytes(application: ApplicationUninstallApplicationDetails): number {
  return uninstallApplicationReleasedBytes(application);
}
</script>

<template>
  <Dialog v-model:open="open">
    <MdDialogContent
      class="min-h-0 grid-rows-[auto_auto_minmax(0,1fr)_auto]"
      :height="usesTallDetailDialog ? 'tall' : 'auto'"
      size="wide"
    >
      <MdDialogHeader>
        <DialogTitle>{{
          t(selectedRecord ? `history.categories.${selectedRecord.category}` : 'history.detailTitle')
        }}</DialogTitle>
        <DialogDescription>{{ t('history.detailDescription') }}</DialogDescription>
      </MdDialogHeader>

      <div v-if="selectedRecord && selectedDetails" class="detail-overview">
        <div v-if="usesItemCounts(selectedRecord)" class="detail-summary">
          <span
            ><small>{{ t('history.selectedItems') }}</small
            ><strong>{{ FormatService.integer(selectedRecord.selectedItemCount) }}</strong></span
          >
          <span
            ><small>{{ t('history.changedItems') }}</small
            ><strong>{{ FormatService.integer(selectedRecord.affectedItemCount) }}</strong></span
          >
          <span
            ><small>{{ t('history.failedItems') }}</small
            ><strong :class="{ warning: selectedRecord.failedItemCount }">{{
              FormatService.integer(selectedRecord.failedItemCount)
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
            ><strong>{{ FormatService.integer(selectedRecord.affectedItemCount) }}</strong></span
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
              FormatService.integer(selectedRecord.failedItemCount)
            }}</strong></span
          >
        </div>
        <div class="detail-meta">
          <span>{{ FormatService.dateTime(selectedRecord.startedAtMs, locale) }}</span>
          <span>
            {{ t('history.duration') }}
            {{
              t(
                'loading.elapsedSeconds',
                {
                  count: FormatService.integer(
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
          v-if="selectedRecord.details.type === 'deepCleanup' && selectedRecord.details.payload.cleanup?.actions.length"
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
              ><strong>{{ FormatService.integer(item.affectedItemCount) }}</strong></span
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
              ><MdTooltip :text="application.applicationIdentifier"
                ><strong>{{ application.applicationName }}</strong></MdTooltip
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
            selectedRecord.details.type === 'largeFileCleanup' || selectedRecord.details.type === 'duplicateFileCleanup'
          "
        >
          <p v-if="!deletedFiles?.length && !selectedRecord.affectedItemCount" class="detail-empty">
            {{ t('history.noFilesDeleted') }}
          </p>
          <div v-for="item in deletedFiles" :key="item.path" class="detail-action">
            <span class="action-status">
              <MdIcon :name="ICON_NAMES.check" :size="13" />
            </span>
            <span class="detail-action-content">
              <strong><MdMiddleEllipsis :text="PathUtils.fileName(item.path)" /></strong>
              <small><MdMiddleEllipsis :text="PathUtils.display(item.path)" :tail-length="32" /></small>
            </span>
            <span class="detail-action-result">
              <strong>{{ t('history.deleted') }}</strong>
            </span>
          </div>
          <p v-if="omittedDeletedFileCount" class="detail-omitted">
            {{
              t(
                'history.moreItemsOmitted',
                {
                  count: FormatService.integer(omittedDeletedFileCount),
                },
                omittedDeletedFileCount
              )
            }}
          </p>
        </template>
      </MdResultTable>

      <MdDialogFooter>
        <Button variant="outline" type="button" @click="open = false">{{ t('common.close') }}</Button>
      </MdDialogFooter>
    </MdDialogContent>
  </Dialog>
</template>

<style scoped>
@reference "@assets/main.css";
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
.detail-empty {
  margin: 0;
  padding: 24px 16px;
  @apply text-center text-muted-foreground;
  font-size: var(--font-content-secondary);
  line-height: 1.5;
}
.detail-omitted {
  margin: 8px 0 0;
  @apply text-center text-muted-foreground;
  font-size: var(--font-content-secondary);
}
</style>
