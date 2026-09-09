import { defineStore } from 'pinia';

import type { ApplicationCloseBatchResult, ApplicationCloseMode } from '@/lib/models/application-close';
import type {
  ApplicationLeftoverCandidate,
  ApplicationLeftoverResult,
  ApplicationLeftoverScanResult,
  ApplicationUninstallBatchPlan,
  ApplicationUninstallBatchResult,
  ApplicationUninstallBatchSelection,
  ApplicationUninstallExecutionProgress,
  ApplicationUninstallScanResult,
} from '@/lib/models/application';
import type { TraversalProgress } from '@/lib/models/progress';
import { ApplicationService } from '@/lib/services/application-service';
import { LoggerService } from '@/lib/services/logger-service';
import { MacOsPermissionService } from '@/lib/services/macos-permission-service';
import * as ApplicationUninstallResultUtils from '@/lib/utils/application-uninstall-result';
import { parseCommandError } from '@/lib/utils/error';

import { useAppStore } from './app-store';
import { useHistoryStore } from './history-store';

interface ApplicationState {
  leftovers: ApplicationLeftoverScanResult | null;
  lastResult: ApplicationLeftoverResult | null;
  scanningLeftovers: boolean;
  deletingLeftovers: boolean;
  uninstallCatalog: ApplicationUninstallScanResult | null;
  scanningUninstallCatalog: boolean;
  cancellingUninstallCatalog: boolean;
  uninstallProgress: TraversalProgress | null;
  uninstallPlan: ApplicationUninstallBatchPlan | null;
  uninstallPreview: ApplicationUninstallBatchResult | null;
  uninstallLastResult: ApplicationUninstallBatchResult | null;
  preparingUninstall: boolean;
  uninstallPreparationRevision: number;
  executingUninstall: boolean;
  cancellingUninstall: boolean;
  uninstallCancellationRevision: number;
  uninstallExecutionProgress: ApplicationUninstallExecutionProgress | null;
  closingUninstallApplications: boolean;
  uninstallCloseResult: ApplicationCloseBatchResult | null;
}

export const useApplicationStore = defineStore('applications', {
  state: (): ApplicationState => ({
    leftovers: null,
    lastResult: null,
    scanningLeftovers: false,
    deletingLeftovers: false,
    uninstallCatalog: null,
    scanningUninstallCatalog: false,
    cancellingUninstallCatalog: false,
    uninstallProgress: null,
    uninstallPlan: null,
    uninstallPreview: null,
    uninstallLastResult: null,
    preparingUninstall: false,
    uninstallPreparationRevision: 0,
    executingUninstall: false,
    cancellingUninstall: false,
    uninstallCancellationRevision: 0,
    uninstallExecutionProgress: null,
    closingUninstallApplications: false,
    uninstallCloseResult: null,
  }),
  actions: {
    clearLeftoverResults() {
      this.leftovers = null;
      this.lastResult = null;
    },
    async closeUninstallApplications(
      applicationIds: string[],
      mode: ApplicationCloseMode
    ): Promise<ApplicationCloseBatchResult | null> {
      const catalogRevision = this.uninstallCatalog?.catalogRevision;
      if (
        this.scanningUninstallCatalog ||
        this.preparingUninstall ||
        this.executingUninstall ||
        this.closingUninstallApplications ||
        !catalogRevision ||
        !applicationIds.length
      ) {
        return null;
      }
      const appStore = useAppStore();
      this.closingUninstallApplications = true;
      this.uninstallCloseResult = null;
      appStore.clearError();
      try {
        const { closeResult, catalog } = await ApplicationService.closeUninstallApplications(
          applicationIds,
          mode,
          catalogRevision
        );
        // Core updates the same trusted catalog snapshot after verifying the
        // selected processes stopped. This avoids an expensive full inventory
        // scan while keeping uninstall preflight and the UI on one revision.
        this.uninstallCatalog = catalog;
        // Publish the result only after the close operation has left the busy
        // state. The dialog may immediately continue into uninstall preflight.
        this.closingUninstallApplications = false;
        this.uninstallCloseResult = closeResult;
        return closeResult;
      } catch (error) {
        appStore.reportError(error);
        return null;
      } finally {
        this.closingUninstallApplications = false;
      }
    },
    removeUninstallCatalogRecord(applicationId: string) {
      const catalog = this.uninstallCatalog;
      if (!catalog) return;
      this.uninstallCatalog = ApplicationUninstallResultUtils.removeApplications(catalog, new Set([applicationId]));
      this.clearPreparedUninstall();
      LoggerService.info(
        'application-uninstall',
        `record_removed_from_catalog removed_count=${catalog.candidates.length - this.uninstallCatalog.candidates.length} remaining_count=${this.uninstallCatalog.candidates.length} rescan=false`
      );
    },
    clearPreparedUninstall() {
      if (this.executingUninstall) return;
      this.uninstallPreparationRevision += 1;
      this.preparingUninstall = false;
      this.uninstallPlan = null;
      this.uninstallPreview = null;
    },
    async prepareUninstall(selections: ApplicationUninstallBatchSelection[]) {
      if (
        this.scanningUninstallCatalog ||
        this.preparingUninstall ||
        this.executingUninstall ||
        this.closingUninstallApplications ||
        !selections.length
      )
        return;
      const appStore = useAppStore();
      const catalogRevision = this.uninstallCatalog?.catalogRevision;
      if (!catalogRevision) {
        appStore.reportError(new Error('application uninstall catalog revision is unavailable'));
        return;
      }
      this.preparingUninstall = true;
      const preparationRevision = ++this.uninstallPreparationRevision;
      this.uninstallPlan = null;
      this.uninstallPreview = null;
      appStore.clearError();
      try {
        const { plan, preview } = await ApplicationService.prepareUninstallBatch(selections, catalogRevision);
        if (preparationRevision !== this.uninstallPreparationRevision) return;
        this.uninstallPreview = preview;
        if (!preview.failedItemCount) this.uninstallPlan = plan;
      } catch (error) {
        if (preparationRevision === this.uninstallPreparationRevision) appStore.reportError(error);
      } finally {
        if (preparationRevision === this.uninstallPreparationRevision) this.preparingUninstall = false;
      }
    },
    async executePreparedUninstall(authorizationPrompt: string) {
      if (
        this.scanningUninstallCatalog ||
        this.preparingUninstall ||
        this.executingUninstall ||
        this.closingUninstallApplications ||
        !this.uninstallPlan
      )
        return;
      const appStore = useAppStore();
      const plan = this.uninstallPlan;
      let result: ApplicationUninstallBatchResult | null = null;
      let latestElapsedMs = 0;
      this.executingUninstall = true;
      this.cancellingUninstall = false;
      this.uninstallExecutionProgress = null;
      this.uninstallLastResult = null;
      this.uninstallCloseResult = null;
      appStore.clearError();
      try {
        result = await ApplicationService.executeUninstallBatchWithProgress(
          plan,
          false,
          authorizationPrompt,
          progress => {
            latestElapsedMs = progress.elapsedMs;
            this.uninstallExecutionProgress = progress;
          }
        );
      } catch (error) {
        // Cancellation during the read-only validation stage returns the
        // typed Core cancellation error because no application has started.
        // Keep that user-requested outcome silent and close the stale plan.
        if (this.cancellingUninstall && parseCommandError(error)?.code === 'operationCancelled') {
          this.uninstallPlan = null;
          this.uninstallPreview = null;
          this.uninstallCancellationRevision += 1;
        } else {
          appStore.reportError(error);
        }
      }
      if (!result) {
        this.executingUninstall = false;
        this.cancellingUninstall = false;
        this.uninstallExecutionProgress = null;
        return;
      }

      // Event delivery and command resolution use separate Tauri channels.
      // Materialize the final typed snapshot from the authoritative result so
      // every application row reaches a terminal state even if the last event
      // is delivered after the invoke promise resolves.
      this.uninstallExecutionProgress = {
        stage: 'finalizing',
        currentApplicationId: null,
        completedApplications: result.results.map(application => ({
          applicationId: application.applicationId,
          status: application.actions.some(action => action.status === 'cancelled')
            ? 'cancelled'
            : application.failedItemCount
              ? 'failed'
              : 'completed',
          releasedBytes: application.releasedBytes,
        })),
        completedApplicationCount: result.results.length,
        totalApplicationCount: result.selectedApplicationCount,
        affectedApplicationCount: result.affectedApplicationCount,
        failedApplicationCount: result.failedApplicationCount,
        releasedBytes: result.releasedBytes,
        elapsedMs: latestElapsedMs,
      };
      /*
       * Core verifies the selected registrations before returning. Apply that
       * authoritative target result immediately so the dialog can close
       * without waiting for a second full inventory scan. A later uninstall
       * still performs Core preflight validation and reports the uncommon case
       * where another installer changed the cached registration or component.
       */
      if (this.uninstallCatalog) {
        this.uninstallCatalog = ApplicationUninstallResultUtils.apply(this.uninstallCatalog, result);
      }
      void useHistoryStore().load({ reportError: false });
      this.uninstallLastResult = result;
      await appStore.refreshSystemDisk();
      this.uninstallPlan = null;
      this.uninstallPreview = null;
      this.executingUninstall = false;
      this.cancellingUninstall = false;
      this.uninstallExecutionProgress = null;
    },
    async cancelUninstallExecution() {
      if (!this.executingUninstall || this.cancellingUninstall) return;
      this.cancellingUninstall = true;
      try {
        await ApplicationService.cancelUninstallExecution();
      } catch (error) {
        useAppStore().reportError(error);
        this.cancellingUninstall = false;
      }
    },
    async scanUninstallCatalog() {
      if (
        this.scanningUninstallCatalog ||
        this.preparingUninstall ||
        this.executingUninstall ||
        this.closingUninstallApplications
      )
        return;
      const appStore = useAppStore();
      this.scanningUninstallCatalog = true;
      this.cancellingUninstallCatalog = false;
      this.uninstallPreparationRevision += 1;
      this.uninstallPlan = null;
      this.uninstallPreview = null;
      this.uninstallLastResult = null;
      this.uninstallCloseResult = null;
      this.uninstallProgress = null;
      appStore.clearError();
      let unlisten: (() => void) | undefined;
      try {
        unlisten = await ApplicationService.listenUninstallProgress(progress => {
          this.uninstallProgress = progress;
        });
        this.uninstallCatalog = await ApplicationService.scanUninstallCatalog();
      } catch (error) {
        // Only the typed Core cancellation outcome is intentionally silent.
        // An unrelated failure racing with a cancel click must remain visible.
        if (parseCommandError(error)?.code !== 'operationCancelled') appStore.reportError(error);
      } finally {
        unlisten?.();
        this.uninstallProgress = null;
        this.scanningUninstallCatalog = false;
        this.cancellingUninstallCatalog = false;
      }
    },
    async cancelUninstallCatalogScan() {
      if (!this.scanningUninstallCatalog || this.cancellingUninstallCatalog) return;
      this.cancellingUninstallCatalog = true;
      try {
        await ApplicationService.cancelUninstallCatalogScan();
      } catch (error) {
        useAppStore().reportError(error);
        this.cancellingUninstallCatalog = false;
      }
    },
    async scanLeftovers() {
      if (this.scanningLeftovers || this.deletingLeftovers) return;
      const appStore = useAppStore();
      this.scanningLeftovers = true;
      // A failed identity refresh must not put candidates from an older
      // application inventory back into an actionable result.
      this.leftovers = null;
      this.lastResult = null;
      appStore.clearError();
      try {
        this.leftovers = await ApplicationService.scanLeftovers();
        await MacOsPermissionService.recordApplicationDataAccess(this.leftovers);
      } catch (error) {
        appStore.reportError(error);
      } finally {
        this.scanningLeftovers = false;
      }
    },
    async deleteLeftoversPermanently(
      candidates: ApplicationLeftoverCandidate[],
      deepCleanupOperationId = crypto.randomUUID()
    ) {
      if (this.scanningLeftovers || this.deletingLeftovers || !candidates.length) return;
      const appStore = useAppStore();
      this.deletingLeftovers = true;
      this.lastResult = null;
      appStore.clearError();
      try {
        const result = await ApplicationService.deleteLeftoversPermanently(
          candidates.map(candidate => ({
            candidateId: candidate.candidateId,
            expectedBytes: candidate.bytes,
            expectedFileCount: candidate.fileCount,
            expectedSnapshotFingerprint: candidate.snapshotFingerprint,
          })),
          false,
          deepCleanupOperationId
        );
        this.lastResult = result;
        const invalidatedCandidateIds = new Set(
          result.actions
            .filter(action => action.status === 'completed' || action.releasedBytes > 0)
            .map(action => action.candidateId)
        );
        if (this.leftovers && invalidatedCandidateIds.size) {
          // Any verified partial deletion invalidates the candidate fingerprint
          // and byte estimate. Remove that stale row just like a completed row;
          // a later explicit scan can publish a fresh actionable candidate.
          const remaining = this.leftovers.candidates.filter(
            candidate => !invalidatedCandidateIds.has(candidate.candidateId)
          );
          this.leftovers = {
            ...this.leftovers,
            candidates: remaining,
            totalBytes: remaining.reduce((total, candidate) => total + candidate.bytes, 0),
            totalFileCount: remaining.reduce((total, candidate) => total + candidate.fileCount, 0),
          };
        }
        if (result.historySaved) await useHistoryStore().load({ reportError: false });
        await appStore.refreshSystemDisk();
      } catch (error) {
        appStore.reportError(error);
      } finally {
        this.deletingLeftovers = false;
      }
    },
    async cancelLeftoverDeletion() {
      if (!this.deletingLeftovers) return;
      try {
        await ApplicationService.cancelLeftoverDeletion();
      } catch (error) {
        useAppStore().reportError(error);
        throw error;
      }
    },
  },
});
