import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  SystemMaintenanceCatalog,
  SystemMaintenanceItem,
  SystemMaintenanceJob,
} from '@/lib/models/system-maintenance';
import { LoggerService } from '@/lib/services/logger-service';
import { SystemMaintenanceService } from '@/lib/services/system-maintenance-service';

import { useAppStore } from './app-store';
import { useSystemMaintenanceStore } from './system-maintenance-store';

const recommendedItem: SystemMaintenanceItem = {
  taskId: 'windows.maintenance.legacy-test',
  category: 'systemRepair',
  riskLevel: 'standard',
  status: 'recommended',
  requiresElevation: false,
  requiresRestart: false,
  estimatedDurationSeconds: 5,
  diagnostic: null,
};

function catalog(items: SystemMaintenanceItem[]): SystemMaintenanceCatalog {
  return {
    schemaVersion: 1,
    scanId: 'system-maintenance-scan-1',
    platform: 'windows',
    scannedAtMs: 1,
    elapsedMs: 2,
    items,
    summary: {
      itemCount: items.length,
      recommendedCount: items.filter(item => item.status === 'recommended').length,
      availableCount: items.filter(item => item.status === 'available').length,
      healthyCount: items.filter(item => item.status === 'healthy').length,
      unavailableCount: items.filter(item => item.status === 'unavailable').length,
    },
  };
}

function queuedJob(taskId: string, executionId = `execution-${taskId}`): SystemMaintenanceJob {
  return {
    executionId,
    scanId: 'system-maintenance-scan-1',
    taskId,
    revision: 1,
    status: 'queued',
    cancelable: true,
    queuedAtMs: 10,
    startedAtMs: null,
    finishedAtMs: null,
    progress: null,
    result: null,
  };
}

describe('system maintenance store', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.restoreAllMocks();
    vi.spyOn(LoggerService, 'info').mockImplementation(() => undefined);
    vi.spyOn(LoggerService, 'warn').mockImplementation(() => undefined);
    vi.spyOn(LoggerService, 'error').mockImplementation(() => undefined);
  });

  it('stores the platform catalog after scanning', async () => {
    const availableItem = {
      ...recommendedItem,
      taskId: 'windows.maintenance.on-demand-test',
      status: 'available' as const,
    };
    vi.spyOn(SystemMaintenanceService, 'scan').mockResolvedValue(catalog([recommendedItem, availableItem]));
    const store = useSystemMaintenanceStore();

    await store.scan();

    expect(store.catalog?.items).toHaveLength(2);
  });

  it('leaves the loading state after a failed scan and recovers on retry', async () => {
    const scan = vi
      .spyOn(SystemMaintenanceService, 'scan')
      .mockRejectedValueOnce(new Error('operation busy'))
      .mockResolvedValueOnce(catalog([recommendedItem]));
    const store = useSystemMaintenanceStore();

    await store.scan();

    expect(store.scanning).toBe(false);
    expect(store.scanFailed).toBe(true);
    expect(store.catalog).toBeNull();

    await store.scan();

    expect(scan).toHaveBeenCalledTimes(2);
    expect(store.scanFailed).toBe(false);
    expect(store.catalog?.items).toEqual([recommendedItem]);
  });

  it('shows a retry state when runtime restoration fails and restores the listener before scanning', async () => {
    const listen = vi.spyOn(SystemMaintenanceService, 'listenJobUpdates').mockResolvedValue(() => undefined);
    vi.spyOn(SystemMaintenanceService, 'runtime')
      .mockRejectedValueOnce(new Error('runtime unavailable'))
      .mockResolvedValueOnce({ catalog: null, executions: [] });
    const scan = vi.spyOn(SystemMaintenanceService, 'scan').mockResolvedValue(catalog([recommendedItem]));
    const store = useSystemMaintenanceStore();

    await store.initialize();

    expect(store.initialized).toBe(false);
    expect(store.scanFailed).toBe(true);
    expect(store.catalog).toBeNull();

    await store.retryScan();

    expect(listen).toHaveBeenCalledTimes(1);
    expect(scan).toHaveBeenCalledTimes(1);
    expect(store.initialized).toBe(true);
    expect(store.scanFailed).toBe(false);
    expect(store.catalog?.items).toEqual([recommendedItem]);
  });

  it('executes one actionable task and rejects healthy or unavailable tasks at the adapter boundary', async () => {
    const availableItem = {
      ...recommendedItem,
      taskId: 'windows.maintenance.on-demand-test',
      status: 'available' as const,
    };
    const healthyItem = {
      ...recommendedItem,
      taskId: 'windows.maintenance.healthy-test',
      status: 'healthy' as const,
    };
    const unavailableItem = {
      ...recommendedItem,
      taskId: 'windows.maintenance.unavailable-test',
      status: 'unavailable' as const,
    };
    const execute = vi
      .spyOn(SystemMaintenanceService, 'execute')
      .mockImplementation(async request => queuedJob(request.taskId));
    const store = useSystemMaintenanceStore();
    store.catalog = catalog([recommendedItem, availableItem, healthyItem, unavailableItem]);

    await store.execute('Authorize maintenance', healthyItem.taskId);
    await store.execute('Authorize maintenance', unavailableItem.taskId);

    expect(execute).not.toHaveBeenCalled();

    await store.execute('Authorize maintenance', availableItem.taskId);

    expect(execute).toHaveBeenCalledWith({
      scanId: 'system-maintenance-scan-1',
      taskId: availableItem.taskId,
      authorizationPrompt: 'Authorize maintenance',
    });
    expect(store.executionForTask(availableItem.taskId)?.status).toBe('queued');
  });

  it('restores background jobs and applies event updates independently', async () => {
    const secondItem = {
      ...recommendedItem,
      taskId: 'windows.maintenance.second-test',
    };
    let updateHandler: ((job: SystemMaintenanceJob) => void) | undefined;
    vi.spyOn(SystemMaintenanceService, 'listenJobUpdates').mockImplementation(async handler => {
      updateHandler = handler;
      return () => undefined;
    });
    vi.spyOn(SystemMaintenanceService, 'runtime').mockResolvedValue({
      catalog: catalog([recommendedItem, secondItem]),
      executions: [queuedJob(recommendedItem.taskId, 'execution-1')],
    });
    const store = useSystemMaintenanceStore();

    await store.initialize();
    updateHandler?.({
      ...queuedJob(secondItem.taskId, 'execution-2'),
      status: 'running',
      startedAtMs: 20,
    });

    expect(store.activeExecutions).toHaveLength(2);
    expect(store.executionForTask(recommendedItem.taskId)?.status).toBe('queued');
    expect(store.executionForTask(secondItem.taskId)?.status).toBe('running');
  });

  it('cancels one queued task without changing another active task', async () => {
    const store = useSystemMaintenanceStore();
    const first = queuedJob(recommendedItem.taskId, 'execution-1');
    const second = queuedJob('windows.maintenance.second-test', 'execution-2');
    store.catalog = catalog([recommendedItem, { ...recommendedItem, taskId: second.taskId }]);
    store.executions = { [first.executionId]: first, [second.executionId]: second };
    const cancelled: SystemMaintenanceJob = {
      ...first,
      revision: 2,
      status: 'finished',
      cancelable: false,
      finishedAtMs: 20,
      result: {
        taskId: first.taskId,
        status: 'failed',
        mutationState: 'notChanged',
        verified: false,
        requiresRestart: false,
        failureReason: 'userCancelled',
      },
    };
    vi.spyOn(SystemMaintenanceService, 'cancelExecution').mockResolvedValue(cancelled);

    await store.cancelExecution(first.executionId);

    expect(store.executionForTask(first.taskId)?.status).toBe('finished');
    expect(store.executionForTask(second.taskId)?.executionId).toBe(second.executionId);
  });

  it('keeps terminal state visible and retries only failures that did not change the system', async () => {
    const store = useSystemMaintenanceStore();
    store.catalog = catalog([recommendedItem]);
    const failed: SystemMaintenanceJob = {
      ...queuedJob(recommendedItem.taskId, 'execution-failed'),
      status: 'finished',
      cancelable: false,
      finishedAtMs: 20,
      result: {
        taskId: recommendedItem.taskId,
        status: 'failed',
        mutationState: 'notChanged',
        verified: false,
        requiresRestart: false,
        failureReason: 'platformFailure',
      },
    };
    store.applyJob(failed, false);
    const execute = vi.spyOn(SystemMaintenanceService, 'execute').mockResolvedValue({
      ...queuedJob(recommendedItem.taskId, 'execution-retry'),
      queuedAtMs: 30,
    });

    expect(store.executionForTask(recommendedItem.taskId)?.status).toBe('finished');

    await store.execute('Authorize maintenance', recommendedItem.taskId);

    expect(execute).toHaveBeenCalledOnce();
    expect(store.executionForTask(recommendedItem.taskId)?.executionId).toBe('execution-retry');
  });

  it('ignores terminal jobs retained from an older catalog scan', () => {
    const store = useSystemMaintenanceStore();
    store.catalog = catalog([recommendedItem]);
    const oldJob = {
      ...queuedJob(recommendedItem.taskId, 'execution-old'),
      scanId: 'system-maintenance-scan-old',
    };
    store.executions = { [oldJob.executionId]: oldJob };

    expect(store.executionForTask(recommendedItem.taskId)).toBeNull();
  });

  it('requires a fresh catalog before rerunning a completed task', async () => {
    const store = useSystemMaintenanceStore();
    store.catalog = catalog([recommendedItem]);
    const completed: SystemMaintenanceJob = {
      ...queuedJob(recommendedItem.taskId, 'execution-completed'),
      status: 'finished',
      cancelable: false,
      finishedAtMs: 20,
      result: {
        taskId: recommendedItem.taskId,
        status: 'completed',
        mutationState: 'changed',
        verified: true,
        requiresRestart: false,
        failureReason: null,
      },
    };
    store.applyJob(completed, false);
    const execute = vi.spyOn(SystemMaintenanceService, 'execute');

    await store.execute('Authorize maintenance', recommendedItem.taskId);

    expect(execute).not.toHaveBeenCalled();
    expect(store.executionForTask(recommendedItem.taskId)?.executionId).toBe(completed.executionId);
  });

  it.each(['completed', 'failed', 'cancelled'] as const)(
    'keeps the catalog stable after a live %s task until an explicit rescan',
    async outcome => {
      const store = useSystemMaintenanceStore();
      const secondItem = { ...recommendedItem, taskId: 'windows.maintenance.second-test' };
      store.catalog = catalog([recommendedItem, secondItem]);
      const originalCatalog = store.catalog;
      const originalSecondItem = store.catalog.items[1];
      const scan = vi.spyOn(SystemMaintenanceService, 'scan').mockResolvedValue({
        ...catalog([recommendedItem, secondItem]),
        scanId: 'system-maintenance-scan-2',
      });
      vi.spyOn(useAppStore(), 'refreshSystemDisk').mockResolvedValue(true);
      const job = queuedJob(recommendedItem.taskId);
      store.applyJob({ ...job, status: 'running' });
      store.applyJob({
        ...job,
        revision: 2,
        status: 'finished',
        cancelable: false,
        finishedAtMs: 20,
        result: {
          taskId: job.taskId,
          status: outcome === 'completed' ? 'completed' : 'failed',
          mutationState: outcome === 'completed' ? 'changed' : outcome === 'failed' ? 'mayHaveChanged' : 'notChanged',
          verified: outcome === 'completed',
          requiresRestart: false,
          failureReason: outcome === 'completed' ? null : outcome === 'failed' ? 'serviceDisabled' : 'userCancelled',
        },
      });

      expect(scan).not.toHaveBeenCalled();
      expect(store.scanning).toBe(false);
      expect(store.catalog).toBe(originalCatalog);
      expect(store.catalog?.items[1]).toBe(originalSecondItem);
      expect(store.executionForTask(job.taskId)?.status).toBe('finished');
      expect(store.executionForTask(secondItem.taskId)).toBeNull();

      await store.scan();

      expect(scan).toHaveBeenCalledOnce();
      expect(store.catalog?.scanId).toBe('system-maintenance-scan-2');
      expect(store.executionForTask(job.taskId)).toBeNull();
    }
  );

  it('does not let an older runtime snapshot resurrect a finished task', () => {
    const store = useSystemMaintenanceStore();
    const queued = queuedJob(recommendedItem.taskId, 'execution-1');
    const finished: SystemMaintenanceJob = {
      ...queued,
      revision: 3,
      status: 'finished',
      cancelable: false,
      finishedAtMs: 20,
      result: {
        taskId: queued.taskId,
        status: 'completed',
        mutationState: 'changed',
        verified: true,
        requiresRestart: false,
        failureReason: null,
      },
    };

    store.applyJob(finished, false);
    store.applyJob({ ...queued, status: 'running', startedAtMs: 10 }, false);

    expect(store.executions[finished.executionId]?.status).toBe('finished');
  });

  it('refreshes disk capacity only for a live job that changed the system', async () => {
    const appStore = useAppStore();
    const refreshDisk = vi.spyOn(appStore, 'refreshSystemDisk').mockResolvedValue(true);
    const store = useSystemMaintenanceStore();
    vi.spyOn(store, 'scan').mockResolvedValue();
    const queued = queuedJob(recommendedItem.taskId, 'execution-capacity');
    const finished: SystemMaintenanceJob = {
      ...queued,
      revision: 2,
      status: 'finished',
      cancelable: false,
      finishedAtMs: 20,
      result: {
        taskId: queued.taskId,
        status: 'completed',
        mutationState: 'changed',
        verified: true,
        requiresRestart: false,
        failureReason: null,
      },
    };

    store.applyJob(finished);
    await vi.waitFor(() => expect(refreshDisk).toHaveBeenCalledOnce());

    store.applyJob({
      ...finished,
      executionId: 'execution-unchanged',
      result: { ...finished.result!, mutationState: 'notChanged' },
    });
    store.applyJob({ ...finished, executionId: 'execution-restored', revision: 1 }, false);
    expect(refreshDisk).toHaveBeenCalledOnce();
  });

  it('does not let an older running snapshot regress visible progress', () => {
    const store = useSystemMaintenanceStore();
    const running: SystemMaintenanceJob = {
      ...queuedJob(recommendedItem.taskId, 'execution-1'),
      revision: 4,
      status: 'running',
      startedAtMs: 20,
      progress: {
        phase: 'repairingComponentImage',
        currentStep: 1,
        totalSteps: 2,
        percent: 40,
      },
    };
    store.applyJob(running, false);
    store.applyJob(
      {
        ...running,
        revision: 3,
        progress: { ...running.progress!, percent: 20 },
      },
      false
    );

    expect(store.executions[running.executionId]?.revision).toBe(4);
    expect(store.executions[running.executionId]?.progress?.percent).toBe(40);
  });

  it('bounds retained terminal jobs while preserving the latest results', () => {
    const store = useSystemMaintenanceStore();
    for (let index = 0; index < 65; index += 1) {
      const job = queuedJob(recommendedItem.taskId, `execution-${index}`);
      store.applyJob(
        {
          ...job,
          revision: 2,
          status: 'finished',
          cancelable: false,
          finishedAtMs: 100 + index,
          result: {
            taskId: job.taskId,
            status: 'completed',
            mutationState: 'changed',
            verified: true,
            requiresRestart: false,
            failureReason: null,
          },
        },
        false
      );
    }

    expect(Object.keys(store.executions)).toHaveLength(64);
    expect(store.executions['execution-0']).toBeUndefined();
    expect(store.executions['execution-64']).toBeDefined();
  });
});
