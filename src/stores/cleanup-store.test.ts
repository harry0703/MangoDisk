import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ApplicationCloseBatchResult } from '@/lib/models/application-close';
import {
  CLEANUP_RULE_IDS,
  CLEANUP_SCAN_SCOPE_MODES,
  STANDARD_CLEANUP_SCAN_SCOPE,
  type CleanupExecutionProgress,
  type CleanupResult,
  type CleanupScanScope,
  type CleanupScanResult,
  type CleanupSourceDetail,
  type ScanRuleResult,
} from '@/lib/models/cleanup';
import type { TraversalProgress } from '@/lib/models/progress';
import { LOG_DOMAINS, LOG_EVENTS } from '@/lib/models/telemetry';
import { CleanupService } from '@/lib/services/cleanup-service';
import { DiskService } from '@/lib/services/disk-service';
import { LoggerService } from '@/lib/services/logger-service';

import { useAppStore } from './app-store';
import { useCleanupStore } from './cleanup-store';
import { useHistoryStore } from './history-store';

const previewResult: CleanupResult = {
  planId: 'plan-1',
  planHash: 'plan-hash-1',
  expectedBytes: 128,
  releasedBytes: 0,
  affectedItemCount: 0,
  failedItemCount: 0,
  dryRun: true,
  actions: [],
  record: {
    schemaVersion: 2,
    operationId: 'operation-1',
    category: 'deepCleanup',
    startedAtMs: 1,
    finishedAtMs: 2,
    outcome: 'completed',
    dryRun: true,
    selectedItemCount: 1,
    affectedItemCount: 0,
    expectedBytes: 128,
    releasedBytes: 0,
    releasedBytesIsEstimate: false,
    failedItemCount: 0,
    details: {
      type: 'deepCleanup',
      payload: {
        cleanup: {
          selectedRuleIds: ['rule-1'],
          expectedBytes: 128,
          actions: [],
        },
        applicationLeftovers: null,
      },
    },
  },
  historySaved: false,
};

const closeResult: ApplicationCloseBatchResult = {
  mode: 'graceful',
  matchedProcessCount: 1,
  requestedProcessCount: 1,
  remainingProcessCount: 0,
  failedTargetCount: 0,
  targets: [
    {
      targetId: 'rule-1',
      status: 'completed',
      matchedProcessCount: 1,
      requestedProcessCount: 1,
      remainingProcesses: [],
    },
  ],
  elapsedMs: 25,
};

type CleanupSourceFixture = Pick<CleanupSourceDetail, 'path' | 'bytes'> & Partial<CleanupSourceDetail>;
type CleanupRuleFixture = Pick<ScanRuleResult, 'ruleId'> &
  Partial<Omit<ScanRuleResult, 'sources'>> & {
    sources?: CleanupSourceFixture[];
  };
type CleanupScanFixture = Partial<Omit<CleanupScanResult, 'disk' | 'rules'>> & {
  disk?: Partial<CleanupScanResult['disk']>;
  rules?: CleanupRuleFixture[];
};

function cleanupScan(fixture: CleanupScanFixture = {}): CleanupScanResult {
  const { disk: diskFixture = {}, rules: ruleFixtures = [], ...scanFixture } = fixture;
  const rules = ruleFixtures.map<ScanRuleResult>(rule => ({
    category: 'system',
    group: 'system',
    risk: 'safe',
    defaultSelected: false,
    recommendedSelected: false,
    bytes: 0,
    fileCount: 0,
    available: true,
    selectable: true,
    status: 'found',
    runningProcesses: [],
    requiresAppClose: false,
    sourceCount: rule.sources?.length ?? 0,
    sourcesTruncated: false,
    scanElapsedMs: 0,
    ...rule,
    ruleId: rule.ruleId,
    sources: (rule.sources ?? []).map(source => ({
      ...source,
      fileCount: source.fileCount ?? 1,
      modifiedAtMs: source.modifiedAtMs ?? null,
      blockReason: source.blockReason ?? null,
      path: source.path,
      bytes: source.bytes,
    })),
  }));
  return {
    schemaVersion: '2',
    customScanId: null,
    scannedAtMs: 1,
    applicationIcons: [],
    warningCount: 0,
    safeBytes: 0,
    reclaimableBytes: 0,
    applicabilityElapsedMs: 0,
    applicableRuleCount: rules.length,
    filteredRuleCount: 0,
    inventoryApplicationCount: 0,
    inventoryProcessCount: 0,
    elapsedMs: 0,
    ...scanFixture,
    disk: {
      name: 'Fixture',
      mountPoint: '/',
      totalBytes: 1_000,
      availableBytes: 500,
      usedBytes: 500,
      ...diskFixture,
    },
    rules,
  };
}

describe('cleanup workflow completion', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.restoreAllMocks();
  });

  it('keeps cleanup blocked until an application-close result is ready', async () => {
    let resolveClose: ((result: ApplicationCloseBatchResult) => void) | undefined;
    vi.spyOn(CleanupService, 'closeApplications').mockImplementation(
      () =>
        new Promise(resolve => {
          resolveClose = resolve;
        })
    );
    const store = useCleanupStore();

    const pending = store.closeApplications(['rule-1'], 'graceful');
    expect(store.closingApplications).toBe(true);
    expect(store.applicationCloseResult).toBeNull();
    expect(await store.execute(true)).toBe(false);

    resolveClose?.(closeResult);
    await pending;
    expect(store.closingApplications).toBe(false);
    expect(store.applicationCloseResult).toEqual(closeResult);
  });

  it('keeps the final scan progress available for the following workflow phase', async () => {
    const finalProgress: TraversalProgress = {
      operationId: 7,
      currentStage: 'analyzing',
      currentPath: '/fixture/cache',
      itemsScanned: 42,
      bytesScanned: 8_192,
      completedSteps: 3,
      totalSteps: 3,
      foundItems: 2,
      foundBytes: 1_024,
      elapsedMs: 2_500,
    };
    const scanResult = cleanupScan({
      disk: {
        name: 'Fixture',
        mountPoint: '/',
        totalBytes: 1_000,
        availableBytes: 500,
        usedBytes: 500,
      },
      rules: [],
      safeBytes: 0,
      reclaimableBytes: 0,
    });
    const store = useCleanupStore();
    store.scanProgress = { ...finalProgress, operationId: 6 };
    vi.spyOn(CleanupService, 'scanWithProgress').mockImplementation(async (_scanScope, onProgress) => {
      expect(store.scanProgress).toBeNull();
      onProgress(finalProgress);
      return scanResult;
    });

    const completed = await store.scanCandidates();

    expect(completed).toBe(true);
    expect(store.scanProgress).toEqual(finalProgress);
    expect(store.loading).toBe(false);
  });

  it('initializes recommendations with only user-selectable sources', async () => {
    const store = useCleanupStore();
    const scanResult = cleanupScan({
      disk: {
        name: 'Fixture',
        mountPoint: '/',
        totalBytes: 1_000,
        availableBytes: 500,
        usedBytes: 500,
      },
      rules: [
        {
          ruleId: 'rule-1',
          recommendedSelected: true,
          selectable: true,
          bytes: 600,
          sourcesTruncated: false,
          sources: [
            { path: '/Applications/Ready.app', bytes: 400, blockReason: null },
            { path: '/Applications/Running.app', bytes: 200, blockReason: 'requiresClose' },
          ],
        },
      ],
      safeBytes: 0,
      reclaimableBytes: 600,
    });
    vi.spyOn(CleanupService, 'scanWithProgress').mockResolvedValue(scanResult);

    expect(await store.scanCandidates()).toBe(true);

    expect(store.selectedRuleIds).toEqual(['rule-1']);
    expect(store.sourceSelections).toEqual([{ ruleId: 'rule-1', mode: 'include', paths: ['/Applications/Ready.app'] }]);
    expect(store.selectedBytes).toBe(400);
  });

  it('replaces only the matching privileged rule in the active scan snapshot', async () => {
    const rule: ScanRuleResult = {
      ruleId: CLEANUP_RULE_IDS.windowsPreviousInstallations,
      category: 'system',
      group: 'system',
      risk: 'recoverable',
      defaultSelected: false,
      recommendedSelected: false,
      bytes: 0,
      fileCount: 1,
      available: true,
      selectable: false,
      status: 'requiresElevation',
      runningProcesses: [],
      requiresAppClose: false,
      sources: [],
      sourceCount: 0,
      sourcesTruncated: false,
      scanElapsedMs: 1,
    };
    const measured: ScanRuleResult = { ...rule, bytes: 9_715_531_776, selectable: true, status: 'found' };
    const store = useCleanupStore();
    store.scan = cleanupScan({
      scannedAtMs: 42,
      rules: [rule],
      reclaimableBytes: 0,
      safeBytes: 0,
    });
    vi.spyOn(CleanupService, 'scanPreviousInstallationsWithPrivileges').mockResolvedValue(measured);

    expect(await store.scanPreviousInstallationsWithPrivileges()).toBe(true);
    expect(store.scan.rules).toEqual([measured]);
    expect(store.scan.reclaimableBytes).toBe(9_715_531_776);
    expect(store.selectedRuleIds).toEqual([]);
    expect(store.privilegedScanRuleId).toBeNull();
  });

  it('reports completion after Core returns a cleanup result', async () => {
    const execute = vi.spyOn(CleanupService, 'executeWithProgress').mockResolvedValue(previewResult);
    const store = useCleanupStore();
    store.selectedRuleIds = ['rule-1'];

    const completed = await store.execute(true, '00000000-0000-4000-8000-000000000001');

    expect(completed).toBe(true);
    expect(execute).toHaveBeenCalledWith(
      ['rule-1'],
      [],
      true,
      STANDARD_CLEANUP_SCAN_SCOPE,
      '00000000-0000-4000-8000-000000000001',
      expect.any(Function)
    );
    expect(store.result).toEqual(previewResult);
    expect(store.loading).toBe(false);
  });

  it('replaces selection order with the Core execution queue', async () => {
    const store = useCleanupStore();
    const observedQueues: string[][] = [];
    const progress: CleanupExecutionProgress = {
      stage: 'validating',
      plannedRuleIds: ['rule-1', 'rule-2'],
      currentRuleId: null,
      currentItemPath: null,
      currentRuleAffectedItemCount: 0,
      currentRuleReleasedBytes: 0,
      completedRuleResults: [],
      validatedRuleCount: 0,
      completedRuleCount: 0,
      totalRuleCount: 2,
      checkedItemCount: 0,
      checkedBytes: 0,
      affectedItemCount: 0,
      releasedBytes: 0,
      elapsedMs: 1,
    };
    vi.spyOn(CleanupService, 'executeWithProgress').mockImplementation(
      async (_ruleIds, _sourceSelections, _dryRun, _scanScope, _operationId, handler) => {
        handler(progress);
        observedQueues.push([...store.executionRuleIds]);
        return previewResult;
      }
    );
    store.selectedRuleIds = ['rule-2', 'rule-1'];

    await store.execute(true, '00000000-0000-4000-8000-000000000002');

    expect(observedQueues).toEqual([['rule-1', 'rule-2']]);
  });

  it('sends source-level overrides to Core', async () => {
    const execute = vi.spyOn(CleanupService, 'executeWithProgress').mockResolvedValue(previewResult);
    const store = useCleanupStore();
    store.selectedRuleIds = ['rule-1'];
    store.sourceSelections = [{ ruleId: 'rule-1', mode: 'include', paths: ['/cache/selected'] }];

    const completed = await store.execute(true, '00000000-0000-4000-8000-000000000003');

    expect(completed).toBe(true);
    expect(execute).toHaveBeenCalledWith(
      ['rule-1'],
      [{ ruleId: 'rule-1', mode: 'include', paths: ['/cache/selected'] }],
      true,
      STANDARD_CLEANUP_SCAN_SCOPE,
      '00000000-0000-4000-8000-000000000003',
      expect.any(Function)
    );
  });

  it('reuses the selected volume scope for cleanup execution', async () => {
    const scanResult = cleanupScan({
      disk: {
        name: 'Fixture',
        mountPoint: '/',
        totalBytes: 1_000,
        availableBytes: 500,
        usedBytes: 500,
      },
      rules: [],
      safeBytes: 0,
      reclaimableBytes: 0,
    });
    const scope: CleanupScanScope = {
      mode: CLEANUP_SCAN_SCOPE_MODES.selectedVolumes,
      volumeMountPoints: ['/Volumes/Projects'],
    };
    vi.spyOn(CleanupService, 'scanWithProgress').mockResolvedValue(scanResult);
    const execute = vi.spyOn(CleanupService, 'executeWithProgress').mockResolvedValue(previewResult);
    const store = useCleanupStore();

    expect(await store.scanCandidates(scope)).toBe(true);
    store.selectedRuleIds = ['rule-1'];
    expect(await store.execute(true, '00000000-0000-4000-8000-000000000004')).toBe(true);

    expect(store.scanScope).toEqual(scope);
    expect(execute).toHaveBeenCalledWith(
      ['rule-1'],
      [],
      true,
      scope,
      '00000000-0000-4000-8000-000000000004',
      expect.any(Function)
    );
  });

  it('binds custom cleanup execution to the Core scan session', async () => {
    const scanResult = cleanupScan({
      customScanId: 42,
      disk: {
        name: 'Fixture',
        mountPoint: '/',
        totalBytes: 1_000,
        availableBytes: 500,
        usedBytes: 500,
      },
      rules: [],
      safeBytes: 0,
      reclaimableBytes: 0,
    });
    const rules = [
      {
        schemaVersion: 1 as const,
        id: 'temporary-files',
        name: 'Temporary files',
        roots: ['/fixture'],
        namePatterns: ['*.tmp'],
        minimumBytes: null,
        maximumBytes: null,
        modifiedTime: { mode: 'any' as const },
        recursive: true,
        removeEmptyDirectories: false,
      },
    ];
    const scope: CleanupScanScope = {
      mode: CLEANUP_SCAN_SCOPE_MODES.custom,
      includeStandardRules: false,
      rules,
    };
    vi.spyOn(CleanupService, 'scanWithProgress').mockResolvedValue(scanResult);
    const execute = vi.spyOn(CleanupService, 'executeWithProgress').mockResolvedValue(previewResult);
    const store = useCleanupStore();

    expect(await store.scanCandidates(scope)).toBe(true);
    store.selectedRuleIds = ['custom.temporary-files'];
    expect(await store.execute(true, '00000000-0000-4000-8000-000000000005')).toBe(true);

    const authorizedScope: CleanupScanScope = {
      mode: CLEANUP_SCAN_SCOPE_MODES.custom,
      includeStandardRules: false,
      scanId: 42,
      rules,
    };
    expect(store.scanScope).toEqual(authorizedScope);
    expect(execute).toHaveBeenCalledWith(
      ['custom.temporary-files'],
      [],
      true,
      authorizedScope,
      '00000000-0000-4000-8000-000000000005',
      expect.any(Function)
    );
  });

  it('reports failure so a following destructive workflow does not start', async () => {
    vi.spyOn(CleanupService, 'executeWithProgress').mockRejectedValue(new Error('fixture failure'));
    const appStore = useAppStore();
    const reportError = vi.spyOn(appStore, 'reportError').mockImplementation(() => undefined);
    const store = useCleanupStore();
    store.selectedRuleIds = ['rule-1'];

    const completed = await store.execute(true);

    expect(completed).toBe(false);
    expect(reportError).toHaveBeenCalledOnce();
    expect(store.loading).toBe(false);
    expect(store.executionProgress).toBeNull();
    expect(store.executionStartedAtMs).toBeNull();
  });

  it('requests cooperative cancellation only for an active destructive cleanup', async () => {
    const cancel = vi.spyOn(CleanupService, 'cancelExecution').mockResolvedValue();
    const store = useCleanupStore();

    await store.cancelExecution();
    expect(cancel).not.toHaveBeenCalled();

    store.loading = true;
    store.operation = 'cleaning';
    await store.cancelExecution();

    expect(cancel).toHaveBeenCalledOnce();
  });

  it('does not report a user-requested cancellation as an execution failure', async () => {
    vi.spyOn(CleanupService, 'executeWithProgress').mockRejectedValue({
      code: 'operationCancelled',
      details: {},
      retryable: true,
    });
    const appStore = useAppStore();
    const reportError = vi.spyOn(appStore, 'reportError');
    const store = useCleanupStore();
    store.selectedRuleIds = ['rule-1'];

    const completed = await store.execute(false);

    expect(completed).toBe(false);
    expect(reportError).not.toHaveBeenCalled();
    expect(store.loading).toBe(false);
  });

  it('applies completed actions without starting another scan', async () => {
    const refreshedDisk = {
      name: 'Fixture',
      mountPoint: '/',
      totalBytes: 1_000,
      availableBytes: 500,
      usedBytes: 500,
    };
    const executionResult: CleanupResult = {
      ...previewResult,
      dryRun: false,
      releasedBytes: 128,
      affectedItemCount: 1,
      historySaved: true,
      actions: [
        {
          ruleId: 'rule-1',
          actionKind: 'delete',
          status: 'completed',
          reasonCode: null,
          bytesExpected: 128,
          releasedBytes: 128,
          affectedItemCount: 1,
          failedItemCount: 0,
          runningProcesses: [],
        },
      ],
    };
    vi.spyOn(CleanupService, 'executeWithProgress').mockResolvedValue(executionResult);
    const rescan = vi.spyOn(CleanupService, 'scanWithProgress');
    const refreshDisk = vi.spyOn(DiskService, 'getSystemDisk').mockResolvedValue(refreshedDisk);
    vi.spyOn(useHistoryStore(), 'load').mockResolvedValue();
    const store = useCleanupStore();
    store.scan = cleanupScan({
      disk: {
        name: 'Fixture',
        mountPoint: '/',
        totalBytes: 1_000,
        availableBytes: 372,
        usedBytes: 628,
      },
      rules: [
        {
          ruleId: 'rule-1',
          risk: 'safe',
          bytes: 128,
          fileCount: 1,
          selectable: true,
          status: 'found',
          sources: [{ path: '/cache/a', bytes: 128, fileCount: 1 }],
          sourceCount: 1,
        },
      ],
      safeBytes: 128,
      reclaimableBytes: 128,
    });
    store.selectedRuleIds = ['rule-1'];

    const completed = await store.execute(false);

    expect(completed).toBe(true);
    expect(rescan).not.toHaveBeenCalled();
    expect(refreshDisk).toHaveBeenCalledOnce();
    expect(useAppStore().disk).toEqual(refreshedDisk);
    expect(store.scan.disk).toEqual(refreshedDisk);
    expect(store.scan.rules[0]).toMatchObject({
      bytes: 0,
      fileCount: 0,
      selectable: false,
      status: 'clean',
    });
    expect(store.scan.reclaimableBytes).toBe(0);
    expect(store.selectedRuleIds).toEqual([]);
    expect(store.operation).toBe('idle');
  });

  it('subtracts partial effects and deselects a source-scoped rule whose details became stale', async () => {
    const executionResult: CleanupResult = {
      ...previewResult,
      dryRun: false,
      releasedBytes: 50,
      affectedItemCount: 1,
      failedItemCount: 1,
      actions: [
        {
          ruleId: 'rule-1',
          actionKind: 'delete',
          status: 'partial',
          reasonCode: 'itemsSkipped',
          bytesExpected: 100,
          releasedBytes: 50,
          affectedItemCount: 1,
          failedItemCount: 1,
          runningProcesses: [],
        },
      ],
    };
    vi.spyOn(CleanupService, 'executeWithProgress').mockResolvedValue(executionResult);
    vi.spyOn(DiskService, 'getSystemDisk').mockResolvedValue({
      name: 'Fixture',
      mountPoint: '/',
      totalBytes: 1_000,
      availableBytes: 500,
      usedBytes: 500,
    });
    const store = useCleanupStore();
    store.scan = cleanupScan({
      rules: [
        {
          ruleId: 'rule-1',
          risk: 'safe',
          bytes: 300,
          fileCount: 3,
          selectable: true,
          status: 'found',
          requiresAppClose: false,
          sources: [
            { path: '/cache/a', bytes: 100, fileCount: 1 },
            { path: '/cache/b', bytes: 200, fileCount: 2 },
          ],
          sourceCount: 2,
          sourcesTruncated: false,
        },
      ],
      safeBytes: 300,
      reclaimableBytes: 300,
      disk: {
        name: 'Fixture',
        mountPoint: '/',
        totalBytes: 1_000,
        availableBytes: 400,
        usedBytes: 600,
      },
    });
    store.selectedRuleIds = ['rule-1'];
    store.sourceSelections = [{ ruleId: 'rule-1', mode: 'include', paths: ['/cache/a'] }];

    const completed = await store.execute(false);

    expect(completed).toBe(true);
    expect(store.scan.rules[0]).toMatchObject({
      bytes: 250,
      fileCount: 2,
      sources: [],
      sourcesTruncated: true,
      selectable: true,
      status: 'found',
    });
    expect(store.scan.safeBytes).toBe(250);
    expect(store.scan.reclaimableBytes).toBe(250);
    expect(store.selectedRuleIds).toEqual([]);
    expect(store.sourceSelections).toEqual([]);
  });

  it('keeps a partially changed whole-rule selection within its original scope', async () => {
    const executionResult: CleanupResult = {
      ...previewResult,
      dryRun: false,
      releasedBytes: 100,
      affectedItemCount: 1,
      failedItemCount: 1,
      actions: [
        {
          ruleId: 'rule-1',
          actionKind: 'delete',
          status: 'partial',
          reasonCode: 'itemsSkipped',
          bytesExpected: 300,
          releasedBytes: 100,
          affectedItemCount: 1,
          failedItemCount: 1,
          runningProcesses: [],
        },
      ],
    };
    vi.spyOn(CleanupService, 'executeWithProgress').mockResolvedValue(executionResult);
    vi.spyOn(DiskService, 'getSystemDisk').mockRejectedValue(new Error('secondary refresh unavailable'));
    vi.spyOn(LoggerService, 'warn').mockImplementation(() => undefined);
    const store = useCleanupStore();
    store.scan = cleanupScan({
      rules: [
        {
          ruleId: 'rule-1',
          risk: 'safe',
          bytes: 300,
          fileCount: 3,
          selectable: true,
          status: 'found',
          requiresAppClose: false,
          sources: [],
          sourceCount: 0,
          sourcesTruncated: true,
        },
      ],
      safeBytes: 300,
      reclaimableBytes: 300,
      disk: {
        name: 'Fixture',
        mountPoint: '/',
        totalBytes: 1_000,
        availableBytes: 400,
        usedBytes: 600,
      },
    });
    store.selectedRuleIds = ['rule-1'];

    const completed = await store.execute(false);

    expect(completed).toBe(true);
    expect(store.selectedRuleIds).toEqual(['rule-1']);
    expect(store.sourceSelections).toEqual([]);
  });

  it('keeps a completed cleanup successful when the disk refresh fails', async () => {
    const executionResult: CleanupResult = {
      ...previewResult,
      dryRun: false,
    };
    vi.spyOn(CleanupService, 'executeWithProgress').mockResolvedValue(executionResult);
    const refreshError = new Error('disk refresh failed');
    vi.spyOn(DiskService, 'getSystemDisk').mockRejectedValue(refreshError);
    const warn = vi.spyOn(LoggerService, 'warn').mockImplementation(() => undefined);
    const appStore = useAppStore();
    const reportError = vi.spyOn(appStore, 'reportError');
    const store = useCleanupStore();
    store.selectedRuleIds = ['rule-1'];

    const completed = await store.execute(false);

    expect(completed).toBe(true);
    expect(store.result).toEqual(executionResult);
    expect(reportError).not.toHaveBeenCalled();
    expect(warn).toHaveBeenCalledWith(LOG_DOMAINS.applicationShell, LOG_EVENTS.diskRefreshFailed, {
      code: 'operationFailed',
    });
  });

  it('switches safely between whole-rule, exclude, and empty source selection', () => {
    const store = useCleanupStore();
    store.scan = cleanupScan({
      rules: [
        {
          ruleId: 'rule-1',
          selectable: true,
          bytes: 300,
          sourcesTruncated: false,
          sources: [
            { path: '/cache/a', bytes: 100 },
            { path: '/cache/b', bytes: 200 },
          ],
        },
      ],
    });
    store.selectedRuleIds = ['rule-1'];

    store.toggleSource('rule-1', '/cache/a');
    expect(store.sourceSelections).toEqual([{ ruleId: 'rule-1', mode: 'exclude', paths: ['/cache/a'] }]);
    expect(store.selectedBytes).toBe(200);

    store.toggleSource('rule-1', '/cache/a');
    expect(store.sourceSelections).toEqual([]);
    expect(store.selectedRuleIds).toEqual(['rule-1']);

    store.setRulesSelected(['rule-1'], false);
    store.toggleSource('rule-1', '/cache/a');
    expect(store.sourceSelections).toEqual([{ ruleId: 'rule-1', mode: 'include', paths: ['/cache/a'] }]);
    store.toggleSource('rule-1', '/cache/a');
    expect(store.sourceSelections).toEqual([]);
    expect(store.selectedRuleIds).toEqual([]);
  });

  it('bulk-selects only sources that the UI allows users to select', () => {
    const store = useCleanupStore();
    store.scan = cleanupScan({
      rules: [
        {
          ruleId: 'rule-1',
          selectable: true,
          bytes: 600,
          sourcesTruncated: false,
          sources: [
            { path: '/Applications/Ready.app', bytes: 400, blockReason: null },
            { path: '/Applications/Running.app', bytes: 200, blockReason: 'requiresClose' },
          ],
        },
      ],
    });

    store.setRulesSelected(['rule-1'], true);

    expect(store.selectedRuleIds).toEqual(['rule-1']);
    expect(store.sourceSelections).toEqual([{ ruleId: 'rule-1', mode: 'include', paths: ['/Applications/Ready.app'] }]);
    expect(store.selectedBytes).toBe(400);

    store.setRulesSelected(['rule-1'], false);
    expect(store.selectedRuleIds).toEqual([]);
    expect(store.sourceSelections).toEqual([]);
  });

  it('selects project sources requiring closure individually and in bulk without selecting limited sources', () => {
    const store = useCleanupStore();
    store.scan = cleanupScan({
      rules: [
        {
          ruleId: 'project.rust-build-artifacts',
          category: 'project',
          selectable: true,
          bytes: 600,
          sourcesTruncated: false,
          sources: [
            { path: '/fixture/worktree/target', bytes: 200, blockReason: 'requiresClose' },
            { path: '/fixture/project/target', bytes: 400, blockReason: null },
            { path: '/fixture/limited/target', bytes: 100, blockReason: 'incompleteMeasurement' },
          ],
        },
      ],
    });
    const ruleId = 'project.rust-build-artifacts';
    store.toggleSource(ruleId, '/fixture/worktree/target');
    expect(store.selectedRuleIds).toEqual([ruleId]);
    expect(store.selectedBytes).toBe(200);
    store.toggleSource(ruleId, '/fixture/limited/target');
    expect(store.selectedBytes).toBe(200);
    store.setRulesSelected([ruleId], true);
    expect(store.sourceSelections).toEqual([
      { ruleId, mode: 'include', paths: ['/fixture/worktree/target', '/fixture/project/target'] },
    ]);
    expect(store.selectedBytes).toBe(600);
  });

  it('does not select rules that require a privileged scan', () => {
    const store = useCleanupStore();
    store.scan = cleanupScan({
      rules: [
        {
          ruleId: CLEANUP_RULE_IDS.windowsPreviousInstallations,
          selectable: false,
          status: 'requiresElevation',
          bytes: 0,
          sourcesTruncated: false,
          sources: [],
        },
      ],
    });

    store.setRulesSelected([CLEANUP_RULE_IDS.windowsPreviousInstallations], true);

    expect(store.selectedRuleIds).toEqual([]);
    expect(store.sourceSelections).toEqual([]);
  });
});
