import { describe, expect, it } from 'vitest';

import type {
  ApplicationUninstallBatchResult,
  ApplicationUninstallCandidate,
  ApplicationUninstallScanResult,
} from '@/lib/models/application';

import * as ApplicationUninstallResultUtils from './application-uninstall-result';

function candidate(
  applicationId: string,
  capability: ApplicationUninstallCandidate['capability'] = 'ready'
): ApplicationUninstallCandidate {
  return {
    applicationId,
    primaryIdentifier: `com.example.${applicationId}`,
    systemKind: 'unclassified',
    sourceIdentities: [{ source: 'macosBundle', identifier: `com.example.${applicationId}` }],
    name: applicationId,
    version: null,
    publisher: null,
    estimatedBytes: 100,
    lastUsedAtMs: null,
    installedAtMs: null,
    platform: 'macosBundle',
    installerKind: null,
    executionMode: null,
    capability,
    recordState: 'installed',
    uninstallDiagnostic: null,
    applicationPath: `/Applications/${applicationId}.app`,
    possibleRelatedPaths: [],
    iconPath: null,
    runningProcesses: [],
    totalBytes: 100,
    defaultSelectedBytes: 100,
    associatedDataComplete: true,
    components: [],
  };
}

function catalog(candidates: ApplicationUninstallCandidate[]): ApplicationUninstallScanResult {
  return {
    schemaVersion: 4,
    scannedAtMs: 1,
    supported: true,
    executionSupported: true,
    catalogActionable: true,
    inventoryComplete: true,
    catalogRevision: 'revision-1',
    candidates,
    readyCount: candidates.filter(item => item.capability === 'ready').length,
    blockedCount: candidates.filter(item => item.capability !== 'ready').length,
    hiddenCount: 0,
    relatedDirectoryCount: 0,
    relatedPathScanElapsedMs: 0,
    elapsedMs: 1,
  };
}

function result(status: 'completed' | 'failed', kind: 'applicationBinary' | 'cache'): ApplicationUninstallBatchResult {
  return {
    batchId: 'batch-1',
    expectedBytes: 100,
    previewedBytes: 0,
    releasedBytes: status === 'completed' ? 100 : 0,
    selectedApplicationCount: 1,
    previewedApplicationCount: 0,
    affectedApplicationCount: status === 'completed' ? 1 : 0,
    failedApplicationCount: status === 'failed' ? 1 : 0,
    previewedItemCount: 0,
    affectedItemCount: status === 'completed' ? 1 : 0,
    failedItemCount: status === 'failed' ? 1 : 0,
    releasedBytesIsEstimate: false,
    restartRequired: false,
    dryRun: false,
    results: [
      {
        planId: 'plan-1',
        applicationId: 'removed',
        applicationName: 'Removed',
        expectedBytes: 100,
        previewedBytes: 0,
        releasedBytes: status === 'completed' ? 100 : 0,
        previewedItemCount: 0,
        affectedItemCount: status === 'completed' ? 1 : 0,
        failedItemCount: status === 'failed' ? 1 : 0,
        releasedBytesIsEstimate: false,
        restartRequired: false,
        dryRun: false,
        actions: [
          {
            componentId: 'component-1',
            kind,
            status,
            reason: status === 'failed' ? 'permanentDeleteFailed' : null,
            expectedBytes: 100,
            releasedBytes: status === 'completed' ? 100 : 0,
          },
        ],
        historySaved: true,
      },
    ],
  };
}

describe('application uninstall result synchronization', () => {
  it('removes an application only after its primary component is removed', () => {
    const snapshot = catalog([candidate('removed'), candidate('remaining', 'applicationRunning')]);
    const updated = ApplicationUninstallResultUtils.apply(snapshot, result('completed', 'applicationBinary'));

    expect(updated.candidates.map(item => item.applicationId)).toEqual(['remaining']);
    expect(updated.readyCount).toBe(0);
    expect(updated.blockedCount).toBe(1);
  });

  it('retains applications after failed or secondary-only actions', () => {
    const snapshot = catalog([candidate('removed')]);

    expect(ApplicationUninstallResultUtils.apply(snapshot, result('failed', 'applicationBinary')).candidates).toEqual(
      snapshot.candidates
    );
    expect(ApplicationUninstallResultUtils.apply(snapshot, result('completed', 'cache')).candidates).toEqual(
      snapshot.candidates
    );
  });
});

describe('native failures after verified registration removal', () => {
  it('removes only the verified native row while retaining the failure result', () => {
    const scan = catalog([candidate('removed'), candidate('untouched')]);
    const failure = result('failed', 'applicationBinary');
    failure.results[0]!.actions[0]!.kind = 'nativeInstaller';
    failure.results[0]!.actions[0]!.reason = 'nativeInstallerFailedAfterRemoval';
    const updated = ApplicationUninstallResultUtils.apply(scan, failure);
    expect(updated.candidates.map(item => item.applicationId)).toEqual(['untouched']);
    expect(updated.scannedAtMs).toBe(scan.scannedAtMs);
    expect(failure.failedApplicationCount).toBe(1);
    expect(failure.results[0]!.actions[0]!.status).toBe('failed');
    expect(failure.releasedBytes).toBe(0);
    failure.results[0]!.actions[0]!.reason = 'nativeInstallerFailed';
    expect(ApplicationUninstallResultUtils.apply(scan, failure)).toBe(scan);
    failure.results[0]!.actions[0]!.reason = 'nativeInstallerFailedAfterRemoval';
    failure.results[0]!.actions[0]!.kind = 'applicationBinary';
    expect(ApplicationUninstallResultUtils.apply(scan, failure)).toBe(scan);
    failure.results[0]!.actions[0]!.kind = 'nativeInstaller';
    failure.dryRun = true;
    expect(ApplicationUninstallResultUtils.apply(scan, failure)).toBe(scan);
  });
});
