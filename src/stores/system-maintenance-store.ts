import { defineStore } from 'pinia';

import type {
  SystemMaintenanceCatalog,
  SystemMaintenanceExecutionItemResult,
  SystemMaintenanceJob,
} from '@/lib/models/system-maintenance';
import { LOG_DOMAINS, LOG_EVENTS } from '@/lib/models/telemetry';
import { LoggerService } from '@/lib/services/logger-service';
import { SystemMaintenanceService } from '@/lib/services/system-maintenance-service';
import { parseCommandError } from '@/lib/utils/error';

import { useAppStore } from './app-store';

interface SystemMaintenanceState {
  catalog: SystemMaintenanceCatalog | null;
  executions: Record<string, SystemMaintenanceJob>;
  scanning: boolean;
  scanFailed: boolean;
  cancellingScan: boolean;
  initialized: boolean;
  listenerReady: boolean;
  lastResult: SystemMaintenanceExecutionItemResult | null;
}

const MAX_RETAINED_EXECUTIONS = 64;

function isActive(job: SystemMaintenanceJob): boolean {
  return job.status !== 'finished';
}

function isRetryable(job: SystemMaintenanceJob): boolean {
  return job.status === 'finished' && job.result?.status === 'failed' && job.result.mutationState === 'notChanged';
}

const JOB_STATUS_ORDER: Record<SystemMaintenanceJob['status'], number> = {
  queued: 0,
  running: 1,
  cancelling: 2,
  finished: 3,
};

function commandFailureContext(error: unknown): Readonly<Record<string, unknown>> {
  const commandError = parseCommandError(error);
  return {
    code: commandError?.code ?? 'unknown',
    retryable: commandError?.retryable ?? false,
  };
}

function pruneFinishedExecutions(executions: Record<string, SystemMaintenanceJob>) {
  const finished = Object.values(executions)
    .filter(job => job.status === 'finished')
    .sort((left, right) => (left.finishedAtMs ?? left.queuedAtMs) - (right.finishedAtMs ?? right.queuedAtMs));
  for (const job of finished.slice(0, Math.max(0, finished.length - MAX_RETAINED_EXECUTIONS))) {
    delete executions[job.executionId];
  }
}

export const useSystemMaintenanceStore = defineStore('system-maintenance', {
  state: (): SystemMaintenanceState => ({
    catalog: null,
    executions: {},
    scanning: false,
    scanFailed: false,
    cancellingScan: false,
    initialized: false,
    listenerReady: false,
    lastResult: null,
  }),
  getters: {
    activeExecutions(state): SystemMaintenanceJob[] {
      return Object.values(state.executions).filter(isActive);
    },
    executing(): boolean {
      return this.activeExecutions.length > 0;
    },
    executionForTask(state): (taskId: string) => SystemMaintenanceJob | null {
      return taskId => {
        const scanId = state.catalog?.scanId;
        if (!scanId) return null;
        const jobs = Object.values(state.executions).filter(job => job.scanId === scanId && job.taskId === taskId);
        // Core appends every retry to the runtime registry and restoration preserves that order.
        // Selecting the last matching entry also avoids timestamp ties on very fast retries.
        return jobs.at(-1) ?? null;
      };
    },
  },
  actions: {
    async initialize() {
      if (this.initialized) return;
      this.initialized = true;
      try {
        if (!this.listenerReady) {
          await SystemMaintenanceService.listenJobUpdates(job => this.applyJob(job));
          this.listenerReady = true;
        }
        const runtime = await SystemMaintenanceService.runtime();
        if (runtime.catalog) this.catalog = runtime.catalog;
        for (const job of runtime.executions) this.applyJob(job, false);
        LoggerService.info(LOG_DOMAINS.systemMaintenance, LOG_EVENTS.systemMaintenanceRuntimeRestored, {
          executionCount: runtime.executions.length,
          activeExecutionCount: runtime.executions.filter(isActive).length,
          hasCatalog: Boolean(runtime.catalog),
        });
        if (!this.catalog && !this.executing) await this.scan('initialization');
      } catch (error) {
        this.initialized = false;
        this.scanFailed = true;
        LoggerService.error(
          LOG_DOMAINS.systemMaintenance,
          LOG_EVENTS.systemMaintenanceRuntimeRestoreFailed,
          commandFailureContext(error)
        );
        useAppStore().reportError(error);
      }
    },
    async retryScan() {
      // A first-load failure may occur before the runtime listener is ready. Re-enter the complete
      // initialization path in that case so a successful catalog scan is not left without job
      // updates. Ordinary scan failures keep the initialized runtime and retry only the scan.
      if (!this.initialized) {
        await this.initialize();
        return;
      }
      await this.scan();
    },
    applyJob(job: SystemMaintenanceJob, liveUpdate = true) {
      const previous = this.executions[job.executionId];
      // Runtime restoration and desktop events travel through separate IPC channels. Revisions
      // prevent an older snapshot from replacing newer progress while both snapshots remain in the
      // same status; the status check remains a defensive boundary for malformed native updates.
      if (previous && job.revision <= previous.revision) return;
      if (previous && JOB_STATUS_ORDER[job.status] < JOB_STATUS_ORDER[previous.status]) {
        LoggerService.warn(LOG_DOMAINS.systemMaintenance, LOG_EVENTS.systemMaintenanceExecutionUpdateIgnored, {
          executionId: job.executionId,
          previousRevision: previous.revision,
          revision: job.revision,
          previousStatus: previous.status,
          status: job.status,
          reason: 'statusRegression',
        });
        return;
      }
      this.executions[job.executionId] = job;
      pruneFinishedExecutions(this.executions);
      if (previous?.status !== job.status) {
        LoggerService.info(LOG_DOMAINS.systemMaintenance, LOG_EVENTS.systemMaintenanceExecutionStateChanged, {
          executionId: job.executionId,
          taskId: job.taskId,
          revision: job.revision,
          previousStatus: previous?.status ?? null,
          status: job.status,
          cancelable: job.cancelable,
        });
      }
      if (job.status !== 'finished' || !job.result) return;
      this.lastResult = job.result;
      LoggerService.info(LOG_DOMAINS.systemMaintenance, LOG_EVENTS.systemMaintenanceExecutionCompleted, {
        executionId: job.executionId,
        taskId: job.taskId,
        status: job.result.status,
        failureReason: job.result.failureReason,
        mutationState: job.result.mutationState,
        restartRequired: job.result.requiresRestart,
        automaticRescan: false,
      });
      // Live maintenance completion can change filesystem usage outside the cleanup domain.
      // Restored terminal jobs use liveUpdate=false and must not replay this side effect.
      if (liveUpdate && job.result.mutationState === 'changed') {
        void useAppStore().refreshSystemDisk();
      }
      // The job already supplies this row's terminal state. An automatic catalog scan disables
      // every row and replaces the scan ID, hiding that result and making unrelated rows flicker.
      // Keep the catalog until an explicit rescan; Core still checks each task before execution.
    },
    async scan(trigger: 'initialization' | 'manual' = 'manual') {
      if (this.scanning || this.executing) return;
      this.scanning = true;
      this.scanFailed = false;
      this.cancellingScan = false;
      useAppStore().clearError();
      LoggerService.info(LOG_DOMAINS.systemMaintenance, LOG_EVENTS.systemMaintenanceScanStarted, { trigger });
      try {
        const catalog = await SystemMaintenanceService.scan();
        this.catalog = catalog;
        this.scanFailed = false;
        LoggerService.info(LOG_DOMAINS.systemMaintenance, LOG_EVENTS.systemMaintenanceScanCompleted, {
          scanId: catalog.scanId,
          itemCount: catalog.summary.itemCount,
          recommendedCount: catalog.summary.recommendedCount,
          unavailableCount: catalog.summary.unavailableCount,
          elapsedMs: catalog.elapsedMs,
        });
      } catch (error) {
        if (parseCommandError(error)?.code !== 'operationCancelled') {
          this.scanFailed = true;
          LoggerService.error(
            LOG_DOMAINS.systemMaintenance,
            LOG_EVENTS.systemMaintenanceScanFailed,
            commandFailureContext(error)
          );
          useAppStore().reportError(error);
        }
      } finally {
        this.scanning = false;
        this.cancellingScan = false;
      }
    },
    async cancelScan() {
      if (this.cancellingScan || !this.scanning) return;
      this.cancellingScan = true;
      try {
        await SystemMaintenanceService.cancelScan();
      } catch (error) {
        useAppStore().reportError(error);
        this.cancellingScan = false;
      }
    },
    async execute(authorizationPrompt: string, taskId: string) {
      if (!this.catalog || this.scanning) return;
      const previousJob = this.executionForTask(taskId);
      if (previousJob && !isRetryable(previousJob)) return;
      const item = this.catalog.items.find(candidate => candidate.taskId === taskId);
      if (!item || item.status === 'unavailable' || item.status === 'healthy') return;
      this.lastResult = null;
      useAppStore().clearError();
      LoggerService.info(LOG_DOMAINS.systemMaintenance, LOG_EVENTS.systemMaintenanceExecutionRequested, {
        taskId,
        requiresElevation: item.requiresElevation,
      });
      try {
        const job = await SystemMaintenanceService.execute({
          scanId: this.catalog.scanId,
          taskId,
          authorizationPrompt,
        });
        this.applyJob(job, false);
      } catch (error) {
        LoggerService.error(LOG_DOMAINS.systemMaintenance, LOG_EVENTS.systemMaintenanceExecutionRequestFailed, {
          taskId,
          ...commandFailureContext(error),
        });
        useAppStore().reportError(error);
      }
    },
    async cancelExecution(executionId: string) {
      const job = this.executions[executionId];
      if (!job || !job.cancelable || job.status === 'finished') return;
      try {
        const updated = await SystemMaintenanceService.cancelExecution(executionId);
        this.applyJob(updated, false);
      } catch (error) {
        LoggerService.error(LOG_DOMAINS.systemMaintenance, LOG_EVENTS.systemMaintenanceExecutionCancelFailed, {
          executionId,
          taskId: job.taskId,
          ...commandFailureContext(error),
        });
        useAppStore().reportError(error);
      }
    },
  },
});
