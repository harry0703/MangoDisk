import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ApplicationCloseBatchResult } from '@/lib/models/application-close';
import type {
  ApplicationLeftoverScanResult,
  ApplicationUninstallCandidate,
  ApplicationUninstallBatchPlan,
  ApplicationUninstallBatchResult,
  ApplicationUninstallExecutionProgress,
  ApplicationUninstallScanResult,
} from '@/lib/models/application';
import type { TraversalProgress } from '@/lib/models/progress';
import { ApplicationService } from '@/lib/services/application-service';

import { useAppStore } from './app-store';
import { useApplicationStore } from './application-store';
import { useHistoryStore } from './history-store';

const authorizationPrompt = 'MangoDisk needs administrator permission to uninstall this app';

const plan: ApplicationUninstallBatchPlan = {
  schemaVersion: 1,
  batchId: 'batch-1',
  batchHash: 'batch-hash-1',
  createdAtMs: 1,
  catalogRevision: 'revision-1',
  plans: [
    {
      schemaVersion: 2,
      planId: 'plan-1',
      planHash: 'hash-1',
      createdAtMs: 1,
      applicationId: 'application-1',
      catalogRevision: 'revision-1',
      items: [
        {
          componentId: 'component-binary',
          kind: 'applicationBinary',
          expectedBytes: 256,
          expectedFileCount: 1,
          expectedSnapshotFingerprint: 'snapshot-1',
        },
      ],
      expectedBytes: 256,
    },
  ],
  expectedBytes: 256,
};

const preview: ApplicationUninstallBatchResult = {
  batchId: plan.batchId,
  expectedBytes: 256,
  previewedBytes: 256,
  releasedBytes: 0,
  selectedApplicationCount: 1,
  previewedApplicationCount: 1,
  affectedApplicationCount: 0,
  failedApplicationCount: 0,
  previewedItemCount: 1,
  affectedItemCount: 0,
  failedItemCount: 0,
  releasedBytesIsEstimate: false,
  restartRequired: false,
  dryRun: true,
  results: [
    {
      planId: plan.plans[0].planId,
      applicationId: plan.plans[0].applicationId,
      applicationName: 'Fixture App',
      expectedBytes: 256,
      previewedBytes: 256,
      releasedBytes: 0,
      previewedItemCount: 1,
      affectedItemCount: 0,
      failedItemCount: 0,
      releasedBytesIsEstimate: false,
      restartRequired: false,
      dryRun: true,
      actions: [
        {
          componentId: 'component-binary',
          kind: 'applicationBinary',
          status: 'previewed',
          reason: null,
          expectedBytes: 256,
          releasedBytes: 0,
        },
      ],
      historySaved: false,
    },
  ],
};

const catalog: ApplicationUninstallScanResult = {
  schemaVersion: 4,
  scannedAtMs: 2,
  supported: true,
  executionSupported: true,
  catalogActionable: true,
  inventoryComplete: true,
  catalogRevision: 'revision-2',
  candidates: [],
  readyCount: 0,
  blockedCount: 0,
  hiddenCount: 0,
  relatedDirectoryCount: 0,
  relatedPathScanElapsedMs: 0,
  elapsedMs: 1,
};

const closeResult: ApplicationCloseBatchResult = {
  mode: 'graceful',
  matchedProcessCount: 1,
  requestedProcessCount: 1,
  remainingProcessCount: 0,
  failedTargetCount: 0,
  targets: [
    {
      targetId: 'application-1',
      status: 'completed',
      matchedProcessCount: 1,
      requestedProcessCount: 1,
      remainingProcesses: [],
    },
  ],
  elapsedMs: 25,
};

const applicationCandidate: ApplicationUninstallCandidate = {
  applicationId: 'application-1',
  primaryIdentifier: 'com.example.fixture',
  systemKind: 'unclassified',
  sourceIdentities: [{ source: 'macosBundle', identifier: 'com.example.fixture' }],
  name: 'Fixture App',
  version: '1.0.0',
  publisher: 'Example',
  estimatedBytes: 256,
  lastUsedAtMs: null,
  installedAtMs: null,
  platform: 'macosBundle',
  installerKind: null,
  executionMode: null,
  capability: 'ready',
  recordState: 'installed',
  uninstallDiagnostic: null,
  applicationPath: '/Applications/Fixture App.app',
  possibleRelatedPaths: [],
  iconPath: null,
  runningProcesses: [],
  totalBytes: 256,
  defaultSelectedBytes: 256,
  associatedDataComplete: true,
  components: [],
};

describe('application uninstall workflow', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.restoreAllMocks();
    vi.spyOn(useAppStore(), 'refreshSystemDisk').mockResolvedValue(true);
  });

  it('removes one verified record without scanning or replacing remaining candidates', () => {
    const store = useApplicationStore();
    const removed = { ...applicationCandidate, applicationId: 'removed', capability: 'viewOnly' as const };
    store.uninstallCatalog = {
      ...catalog,
      candidates: [removed, applicationCandidate],
      readyCount: 1,
      blockedCount: 1,
    };
    const remaining = store.uninstallCatalog.candidates[1];
    const scan = vi.spyOn(ApplicationService, 'scanUninstallCatalog');
    store.uninstallPlan = plan;
    store.removeUninstallCatalogRecord('removed');
    expect(store.uninstallCatalog.candidates).toEqual([remaining]);
    expect(store.uninstallCatalog.candidates[0]).toBe(remaining);
    expect(store.uninstallCatalog.readyCount).toBe(1);
    expect(store.uninstallCatalog.blockedCount).toBe(0);
    expect(store.uninstallCatalog.scannedAtMs).toBe(catalog.scannedAtMs);
    expect(store.uninstallPlan).toBeNull();
    expect(scan).not.toHaveBeenCalled();
    store.removeUninstallCatalogRecord('removed');
    expect(store.uninstallCatalog.candidates).toHaveLength(1);
  });

  it('publishes the catalog snapshot updated by application close', async () => {
    let resolveClose:
      | ((result: { closeResult: ApplicationCloseBatchResult; catalog: ApplicationUninstallScanResult }) => void)
      | undefined;
    vi.spyOn(ApplicationService, 'closeUninstallApplications').mockImplementation(
      () =>
        new Promise(resolve => {
          resolveClose = resolve;
        })
    );
    const store = useApplicationStore();
    store.uninstallCatalog = catalog;

    const pending = store.closeUninstallApplications(['application-1'], 'graceful');
    await vi.waitFor(() => expect(resolveClose).toBeDefined());
    expect(store.closingUninstallApplications).toBe(true);
    expect(store.uninstallCloseResult).toBeNull();

    resolveClose?.({
      closeResult,
      catalog: { ...catalog, catalogRevision: 'revision-3' },
    });
    await pending;
    expect(store.closingUninstallApplications).toBe(false);
    expect(store.uninstallCatalog?.catalogRevision).toBe('revision-3');
    expect(store.uninstallCloseResult).toEqual(closeResult);
  });

  it('retains a plan only after its dry-run passes', async () => {
    const selections = [{ applicationId: 'application-1', componentIds: ['component-binary'] }];
    const prepare = vi.spyOn(ApplicationService, 'prepareUninstallBatch').mockResolvedValue({ plan, preview });
    const store = useApplicationStore();
    store.uninstallCatalog = catalog;

    await store.prepareUninstall(selections);

    expect(prepare).toHaveBeenCalledWith(selections, catalog.catalogRevision);
    expect(store.uninstallPlan).toEqual(plan);
    expect(store.uninstallPreview).toEqual(preview);
    expect(store.preparingUninstall).toBe(false);
  });

  it('does not expose confirmation when preflight reports a failed item', async () => {
    const failedPreview: ApplicationUninstallBatchResult = {
      ...preview,
      previewedBytes: 0,
      previewedApplicationCount: 0,
      failedApplicationCount: 1,
      previewedItemCount: 0,
      failedItemCount: 1,
      results: [
        {
          ...preview.results[0],
          previewedBytes: 0,
          previewedItemCount: 0,
          failedItemCount: 1,
          actions: [
            {
              ...preview.results[0].actions[0],
              status: 'failed',
              reason: 'componentChanged',
            },
          ],
        },
      ],
    };
    vi.spyOn(ApplicationService, 'prepareUninstallBatch').mockResolvedValue({ plan, preview: failedPreview });
    const store = useApplicationStore();
    store.uninstallCatalog = catalog;

    await store.prepareUninstall([{ applicationId: 'application-1', componentIds: ['component-binary'] }]);

    expect(store.uninstallPlan).toBeNull();
    expect(store.uninstallPreview).toEqual(failedPreview);
  });

  it('ignores a preparation result after the confirmation is cancelled', async () => {
    let resolvePreparation:
      ((value: { plan: ApplicationUninstallBatchPlan; preview: ApplicationUninstallBatchResult }) => void) | undefined;
    vi.spyOn(ApplicationService, 'prepareUninstallBatch').mockImplementation(
      () =>
        new Promise(resolve => {
          resolvePreparation = resolve;
        })
    );
    const store = useApplicationStore();
    store.uninstallCatalog = catalog;
    const pending = store.prepareUninstall([{ applicationId: 'application-1', componentIds: ['component-binary'] }]);

    store.clearPreparedUninstall();
    expect(store.preparingUninstall).toBe(false);
    resolvePreparation?.({ plan, preview });
    await pending;

    expect(store.uninstallPlan).toBeNull();
    expect(store.uninstallPreview).toBeNull();
    expect(store.preparingUninstall).toBe(false);
  });

  it('refreshes history and applies the result without rescanning the catalog', async () => {
    const result: ApplicationUninstallBatchResult = {
      ...preview,
      dryRun: false,
      previewedBytes: 0,
      previewedApplicationCount: 0,
      affectedApplicationCount: 1,
      previewedItemCount: 0,
      releasedBytes: 256,
      affectedItemCount: 1,
      results: [
        {
          ...preview.results[0],
          dryRun: false,
          previewedBytes: 0,
          previewedItemCount: 0,
          releasedBytes: 256,
          affectedItemCount: 1,
          historySaved: true,
          actions: [
            {
              ...preview.results[0].actions[0],
              status: 'completed',
              releasedBytes: 256,
            },
          ],
        },
      ],
    };
    const progress: ApplicationUninstallExecutionProgress = {
      stage: 'uninstalling',
      currentApplicationId: 'application-1',
      completedApplications: [],
      completedApplicationCount: 0,
      totalApplicationCount: 1,
      affectedApplicationCount: 0,
      failedApplicationCount: 0,
      releasedBytes: 0,
      elapsedMs: 10,
    };
    const execute = vi
      .spyOn(ApplicationService, 'executeUninstallBatchWithProgress')
      .mockImplementation(async (_plan, _dryRun, _authorizationPrompt, handler) => {
        handler(progress);
        expect(useApplicationStore().uninstallExecutionProgress).toEqual(progress);
        return result;
      });
    const scan = vi.spyOn(ApplicationService, 'scanUninstallCatalog');
    const history = useHistoryStore();
    const loadHistory = vi.spyOn(history, 'load').mockResolvedValue();
    const refreshDisk = vi.mocked(useAppStore().refreshSystemDisk);
    const store = useApplicationStore();
    store.uninstallCatalog = { ...catalog, candidates: [applicationCandidate], readyCount: 1 };
    store.uninstallPlan = plan;
    store.uninstallPreview = preview;

    await store.executePreparedUninstall(authorizationPrompt);

    expect(execute).toHaveBeenCalledWith(plan, false, authorizationPrompt, expect.any(Function));
    expect(loadHistory).toHaveBeenCalledWith({ reportError: false });
    expect(refreshDisk).toHaveBeenCalledOnce();
    expect(scan).not.toHaveBeenCalled();
    expect(store.uninstallLastResult).toEqual(result);
    expect(store.uninstallCatalog?.candidates).toEqual([]);
    expect(store.uninstallPlan).toBeNull();
    expect(store.uninstallPreview).toBeNull();
    expect(store.executingUninstall).toBe(false);
    expect(store.uninstallExecutionProgress).toBeNull();
  });

  it('keeps the catalog actionable when execution changes no application', async () => {
    const result: ApplicationUninstallBatchResult = {
      ...preview,
      dryRun: false,
      failedApplicationCount: 1,
      failedItemCount: 1,
      results: [
        {
          ...preview.results[0],
          dryRun: false,
          failedItemCount: 1,
          actions: [
            {
              ...preview.results[0].actions[0],
              status: 'failed',
              reason: 'nativeInstallerFailed',
            },
          ],
        },
      ],
    };
    vi.spyOn(ApplicationService, 'executeUninstallBatchWithProgress').mockResolvedValue(result);
    const scan = vi.spyOn(ApplicationService, 'scanUninstallCatalog');
    const loadHistory = vi.spyOn(useHistoryStore(), 'load').mockResolvedValue();
    const appStore = useAppStore();
    const reportError = vi.spyOn(appStore, 'reportError');
    const store = useApplicationStore();
    store.uninstallCatalog = { ...catalog, candidates: [applicationCandidate], readyCount: 1 };
    store.uninstallPlan = plan;
    store.uninstallPreview = preview;

    await store.executePreparedUninstall(authorizationPrompt);

    expect(store.uninstallLastResult).toEqual(result);
    expect(store.uninstallCatalog?.candidates).toEqual([applicationCandidate]);
    expect(scan).not.toHaveBeenCalled();
    expect(loadHistory).toHaveBeenCalledWith({ reportError: false });
    expect(reportError).not.toHaveBeenCalled();
    expect(appStore.errorCode).toBeNull();
  });

  it('releases execution progress after an uninstall command fails', async () => {
    const progress: ApplicationUninstallExecutionProgress = {
      stage: 'uninstalling',
      currentApplicationId: 'application-1',
      completedApplications: [],
      completedApplicationCount: 0,
      totalApplicationCount: 1,
      affectedApplicationCount: 0,
      failedApplicationCount: 0,
      releasedBytes: 0,
      elapsedMs: 10,
    };
    vi.spyOn(ApplicationService, 'executeUninstallBatchWithProgress').mockImplementation(
      async (_plan, _dryRun, _authorizationPrompt, handler) => {
        handler(progress);
        throw new Error('fixture failure');
      }
    );
    const appStore = useAppStore();
    const reportError = vi.spyOn(appStore, 'reportError').mockImplementation(() => undefined);
    const store = useApplicationStore();
    store.uninstallPlan = plan;
    store.uninstallPreview = preview;

    await store.executePreparedUninstall(authorizationPrompt);

    expect(reportError).toHaveBeenCalledOnce();
    expect(store.executingUninstall).toBe(false);
    expect(store.uninstallExecutionProgress).toBeNull();
    expect(store.uninstallPlan).toEqual(plan);
  });

  it('requests cooperative cancellation only while an uninstall batch is active', async () => {
    const cancel = vi.spyOn(ApplicationService, 'cancelUninstallExecution').mockResolvedValue();
    const store = useApplicationStore();

    await store.cancelUninstallExecution();
    expect(cancel).not.toHaveBeenCalled();

    store.executingUninstall = true;
    await store.cancelUninstallExecution();

    expect(cancel).toHaveBeenCalledOnce();
    expect(store.cancellingUninstall).toBe(true);
  });

  it('closes a prepared batch when cancellation finishes during validation', async () => {
    let rejectExecution: ((reason?: unknown) => void) | undefined;
    vi.spyOn(ApplicationService, 'executeUninstallBatchWithProgress').mockImplementation(
      () =>
        new Promise((_, reject) => {
          rejectExecution = reject;
        })
    );
    vi.spyOn(ApplicationService, 'cancelUninstallExecution').mockImplementation(async () => {
      rejectExecution?.({ code: 'operationCancelled', details: {}, retryable: true });
    });
    const appStore = useAppStore();
    const reportError = vi.spyOn(appStore, 'reportError');
    const store = useApplicationStore();
    store.uninstallPlan = plan;
    store.uninstallPreview = preview;

    const pending = store.executePreparedUninstall(authorizationPrompt);
    await vi.waitFor(() => expect(rejectExecution).toBeDefined());
    await store.cancelUninstallExecution();
    await pending;

    expect(reportError).not.toHaveBeenCalled();
    expect(store.uninstallPlan).toBeNull();
    expect(store.uninstallPreview).toBeNull();
    expect(store.uninstallCancellationRevision).toBe(1);
    expect(store.executingUninstall).toBe(false);
    expect(store.cancellingUninstall).toBe(false);
  });

  it('closes immediately when Core detaches from an active system uninstaller', async () => {
    const cancelledResult: ApplicationUninstallBatchResult = {
      ...preview,
      dryRun: false,
      previewedApplicationCount: 0,
      previewedItemCount: 0,
      results: [
        {
          ...preview.results[0],
          dryRun: false,
          previewedBytes: 0,
          previewedItemCount: 0,
          actions: [
            {
              ...preview.results[0].actions[0],
              status: 'cancelled',
              reason: 'externalUninstallerContinuing',
            },
          ],
        },
      ],
    };
    vi.spyOn(ApplicationService, 'executeUninstallBatchWithProgress').mockResolvedValue(cancelledResult);
    const scan = vi.spyOn(ApplicationService, 'scanUninstallCatalog');
    const loadHistory = vi.spyOn(useHistoryStore(), 'load').mockResolvedValue();
    const store = useApplicationStore();
    store.uninstallCatalog = { ...catalog, candidates: [applicationCandidate] };
    store.uninstallPlan = plan;
    store.uninstallPreview = preview;

    await store.executePreparedUninstall(authorizationPrompt);

    expect(scan).not.toHaveBeenCalled();
    expect(loadHistory).toHaveBeenCalledWith({ reportError: false });
    expect(store.uninstallCatalog?.candidates).toEqual([applicationCandidate]);
    expect(store.uninstallLastResult).toEqual(cancelledResult);
    expect(store.executingUninstall).toBe(false);
    expect(store.cancellingUninstall).toBe(false);
  });

  it('allows another uninstall without rescanning the catalog', async () => {
    const result: ApplicationUninstallBatchResult = {
      ...preview,
      dryRun: false,
      affectedApplicationCount: 1,
      affectedItemCount: 1,
      releasedBytes: 256,
      results: [
        {
          ...preview.results[0],
          dryRun: false,
          affectedItemCount: 1,
          releasedBytes: 256,
          actions: [
            {
              ...preview.results[0].actions[0],
              status: 'completed',
              releasedBytes: 256,
            },
          ],
        },
      ],
    };
    const remainingCandidate: ApplicationUninstallCandidate = {
      ...applicationCandidate,
      applicationId: 'application-2',
      primaryIdentifier: 'com.example.second',
      name: 'Second App',
    };
    vi.spyOn(ApplicationService, 'executeUninstallBatchWithProgress').mockResolvedValue(result);
    const scan = vi.spyOn(ApplicationService, 'scanUninstallCatalog');
    const prepare = vi.spyOn(ApplicationService, 'prepareUninstallBatch').mockResolvedValue({ plan, preview });
    vi.spyOn(useHistoryStore(), 'load').mockResolvedValue();
    const store = useApplicationStore();
    store.uninstallCatalog = {
      ...catalog,
      candidates: [applicationCandidate, remainingCandidate],
      readyCount: 2,
    };
    store.uninstallPlan = plan;
    store.uninstallPreview = preview;

    await store.executePreparedUninstall(authorizationPrompt);

    expect(store.executingUninstall).toBe(false);
    expect(store.uninstallLastResult).toEqual(result);
    expect(store.uninstallPlan).toBeNull();
    expect(store.uninstallPreview).toBeNull();
    expect(store.uninstallExecutionProgress).toBeNull();
    expect(store.uninstallCatalog?.candidates).toEqual([remainingCandidate]);

    await store.prepareUninstall([{ applicationId: 'application-2', componentIds: ['component-binary'] }]);
    expect(scan).not.toHaveBeenCalled();
    expect(prepare).toHaveBeenCalledWith(
      [{ applicationId: 'application-2', componentIds: ['component-binary'] }],
      catalog.catalogRevision
    );
  });

  it('exposes catalog progress only while the user-requested scan is running', async () => {
    const progress: TraversalProgress = {
      operationId: 7,
      currentStage: 'analyzing',
      currentPath: '/Applications/Fixture App.app',
      itemsScanned: 1,
      bytesScanned: 256,
      completedSteps: 1,
      totalSteps: 2,
      foundItems: 1,
      foundBytes: 256,
      elapsedMs: 10,
    };
    let reportProgress: ((value: TraversalProgress) => void) | undefined;
    const unlisten = vi.fn();
    const listen = vi.spyOn(ApplicationService, 'listenUninstallProgress').mockImplementation(async handler => {
      reportProgress = handler;
      return unlisten;
    });
    vi.spyOn(ApplicationService, 'scanUninstallCatalog').mockImplementation(async () => {
      reportProgress?.(progress);
      expect(useApplicationStore().uninstallProgress).toEqual(progress);
      return catalog;
    });
    const store = useApplicationStore();

    await store.scanUninstallCatalog();

    expect(listen).toHaveBeenCalledOnce();
    expect(unlisten).toHaveBeenCalledOnce();
    expect(store.uninstallCatalog).toEqual(catalog);
    expect(store.uninstallProgress).toBeNull();
    expect(store.scanningUninstallCatalog).toBe(false);
  });

  it('cancels a running catalog scan without reporting cancellation as an error', async () => {
    let rejectScan: ((reason?: unknown) => void) | undefined;
    vi.spyOn(ApplicationService, 'listenUninstallProgress').mockResolvedValue(vi.fn());
    const scan = vi.spyOn(ApplicationService, 'scanUninstallCatalog').mockImplementation(
      () =>
        new Promise((_, reject) => {
          rejectScan = reject;
        })
    );
    const cancel = vi.spyOn(ApplicationService, 'cancelUninstallCatalogScan').mockImplementation(async () => {
      rejectScan?.({ code: 'operationCancelled', details: {}, retryable: true });
    });
    const appStore = useAppStore();
    const reportError = vi.spyOn(appStore, 'reportError');
    const store = useApplicationStore();

    const pending = store.scanUninstallCatalog();
    await vi.waitFor(() => expect(scan).toHaveBeenCalledOnce());
    await store.cancelUninstallCatalogScan();
    await pending;

    expect(cancel).toHaveBeenCalledOnce();
    expect(reportError).not.toHaveBeenCalled();
    expect(store.scanningUninstallCatalog).toBe(false);
    expect(store.cancellingUninstallCatalog).toBe(false);
  });

  it('reports a non-cancellation scan failure that races with a cancel request', async () => {
    let rejectScan: ((reason?: unknown) => void) | undefined;
    vi.spyOn(ApplicationService, 'listenUninstallProgress').mockResolvedValue(vi.fn());
    vi.spyOn(ApplicationService, 'scanUninstallCatalog').mockImplementation(
      () =>
        new Promise((_, reject) => {
          rejectScan = reject;
        })
    );
    vi.spyOn(ApplicationService, 'cancelUninstallCatalogScan').mockImplementation(async () => {
      rejectScan?.({ code: 'operationFailed', details: {}, retryable: true });
    });
    const appStore = useAppStore();
    const reportError = vi.spyOn(appStore, 'reportError');
    const store = useApplicationStore();

    const pending = store.scanUninstallCatalog();
    await vi.waitFor(() => expect(rejectScan).toBeDefined());
    await store.cancelUninstallCatalogScan();
    await pending;

    expect(reportError).toHaveBeenCalledOnce();
  });
});

describe('application leftover workflow', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.restoreAllMocks();
    vi.spyOn(useAppStore(), 'refreshSystemDisk').mockResolvedValue(true);
  });

  it('clears leftover scan and execution results together', () => {
    const store = useApplicationStore();
    store.leftovers = {
      schemaVersion: 2,
      scannedAtMs: 1,
      supported: true,
      inventoryComplete: true,
      accessLimited: false,
      candidates: [],
      totalBytes: 0,
      totalFileCount: 0,
      skippedCount: 0,
      elapsedMs: 1,
    };
    store.lastResult = {
      planId: 'plan-1',
      expectedBytes: 0,
      releasedBytes: 0,
      affectedItemCount: 0,
      failedItemCount: 0,
      dryRun: false,
      actions: [],
      historySaved: false,
    };

    store.clearLeftoverResults();

    expect(store.leftovers).toBeNull();
    expect(store.lastResult).toBeNull();
  });

  it('does not retain stale candidates when an inventory refresh fails', async () => {
    const previousScan: ApplicationLeftoverScanResult = {
      schemaVersion: 2,
      scannedAtMs: 1,
      supported: true,
      inventoryComplete: true,
      accessLimited: false,
      candidates: [],
      totalBytes: 0,
      totalFileCount: 0,
      skippedCount: 0,
      elapsedMs: 1,
    };
    vi.spyOn(ApplicationService, 'scanLeftovers').mockRejectedValue(new Error('fixture failure'));
    const appStore = useAppStore();
    const reportError = vi.spyOn(appStore, 'reportError').mockImplementation(() => undefined);
    const store = useApplicationStore();
    store.leftovers = previousScan;

    await store.scanLeftovers();

    expect(store.leftovers).toBeNull();
    expect(reportError).toHaveBeenCalledOnce();
    expect(store.scanningLeftovers).toBe(false);
  });

  it('requests cancellation only while leftover deletion is active', async () => {
    const cancel = vi.spyOn(ApplicationService, 'cancelLeftoverDeletion').mockResolvedValue();
    const store = useApplicationStore();

    await store.cancelLeftoverDeletion();
    expect(cancel).not.toHaveBeenCalled();

    store.deletingLeftovers = true;
    await store.cancelLeftoverDeletion();

    expect(cancel).toHaveBeenCalledOnce();
  });

  it('removes completed leftovers without starting another scan', async () => {
    const completedCandidate = {
      candidateId: 'completed',
      applicationIdentifier: 'com.example.completed',
      applicationName: 'Completed',
      source: 'applicationSupport' as const,
      path: '/Library/Caches/com.example.completed',
      bytes: 100,
      fileCount: 1,
      modifiedAtMs: null,
      confidence: 'high' as const,
      defaultSelected: true,
      evidence: ['installedOwnerAbsent' as const],
      snapshotFingerprint: 'completed-snapshot',
    };
    const failedCandidate = {
      ...completedCandidate,
      candidateId: 'failed',
      applicationIdentifier: 'com.example.failed',
      applicationName: 'Failed',
      path: '/Library/Caches/com.example.failed',
      bytes: 200,
      fileCount: 2,
      snapshotFingerprint: 'failed-snapshot',
    };
    vi.spyOn(ApplicationService, 'deleteLeftoversPermanently').mockResolvedValue({
      planId: 'plan-1',
      expectedBytes: 300,
      releasedBytes: 100,
      affectedItemCount: 1,
      failedItemCount: 1,
      dryRun: false,
      actions: [
        {
          candidateId: 'completed',
          applicationIdentifier: 'com.example.completed',
          applicationName: 'Completed',
          status: 'completed',
          reason: null,
          expectedBytes: 100,
          releasedBytes: 100,
        },
        {
          candidateId: 'failed',
          applicationIdentifier: 'com.example.failed',
          applicationName: 'Failed',
          status: 'failed',
          reason: 'candidateChanged',
          expectedBytes: 200,
          releasedBytes: 0,
        },
      ],
      historySaved: true,
    });
    const rescan = vi.spyOn(ApplicationService, 'scanLeftovers');
    vi.spyOn(useHistoryStore(), 'load').mockResolvedValue();
    const refreshDisk = vi.mocked(useAppStore().refreshSystemDisk);
    const store = useApplicationStore();
    store.leftovers = {
      schemaVersion: 2,
      scannedAtMs: 1,
      supported: true,
      inventoryComplete: true,
      accessLimited: false,
      candidates: [completedCandidate, failedCandidate],
      totalBytes: 300,
      totalFileCount: 3,
      skippedCount: 0,
      elapsedMs: 1,
    };

    await store.deleteLeftoversPermanently([completedCandidate, failedCandidate]);

    expect(rescan).not.toHaveBeenCalled();
    expect(refreshDisk).toHaveBeenCalledOnce();
    expect(store.leftovers!.candidates).toEqual([failedCandidate]);
    expect(store.leftovers!.totalBytes).toBe(200);
    expect(store.leftovers!.totalFileCount).toBe(2);
  });

  it('removes partially changed leftover snapshots and refreshes saved history quietly', async () => {
    const candidate = {
      candidateId: 'partial',
      applicationIdentifier: 'com.example.partial',
      applicationName: 'Partial',
      source: 'applicationSupport' as const,
      path: '/Library/Caches/com.example.partial',
      bytes: 200,
      fileCount: 2,
      modifiedAtMs: null,
      confidence: 'high' as const,
      defaultSelected: true,
      evidence: ['installedOwnerAbsent' as const],
      snapshotFingerprint: 'partial-snapshot',
    };
    vi.spyOn(ApplicationService, 'deleteLeftoversPermanently').mockResolvedValue({
      planId: 'plan-partial',
      expectedBytes: 200,
      releasedBytes: 100,
      affectedItemCount: 0,
      failedItemCount: 1,
      dryRun: false,
      actions: [
        {
          candidateId: candidate.candidateId,
          applicationIdentifier: candidate.applicationIdentifier,
          applicationName: candidate.applicationName,
          status: 'failed',
          reason: 'permanentDeleteFailed',
          expectedBytes: 200,
          releasedBytes: 100,
        },
      ],
      historySaved: true,
    });
    const historyLoad = vi.spyOn(useHistoryStore(), 'load').mockResolvedValue();
    const store = useApplicationStore();
    store.leftovers = {
      schemaVersion: 2,
      scannedAtMs: 1,
      supported: true,
      inventoryComplete: true,
      accessLimited: false,
      candidates: [candidate],
      totalBytes: candidate.bytes,
      totalFileCount: candidate.fileCount,
      skippedCount: 0,
      elapsedMs: 1,
    };

    await store.deleteLeftoversPermanently([candidate]);

    expect(store.leftovers!.candidates).toEqual([]);
    expect(store.leftovers!.totalBytes).toBe(0);
    expect(store.leftovers!.totalFileCount).toBe(0);
    expect(historyLoad).toHaveBeenCalledWith({ reportError: false });
  });
});
