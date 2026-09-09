import type { ApplicationUninstallBatchResult, ApplicationUninstallScanResult } from '@/lib/models/application';
export function apply(
  catalog: ApplicationUninstallScanResult,
  result: ApplicationUninstallBatchResult
): ApplicationUninstallScanResult {
  if (result.dryRun) return catalog;
  const removedApplicationIds = new Set(
    result.results
      .filter(application =>
        application.actions.some(
          action =>
            (action.status === 'completed' &&
              (action.kind === 'applicationBinary' || action.kind === 'nativeInstaller')) ||
            // The native postflight can verify removal even when the vendor returns an error.
            // Remove only that stale row; the result and history retain their failed status.
            (action.status === 'failed' &&
              action.kind === 'nativeInstaller' &&
              action.reason === 'nativeInstallerFailedAfterRemoval')
        )
      )
      .map(application => application.applicationId)
  );
  if (!removedApplicationIds.size) return catalog;
  return removeApplications(catalog, removedApplicationIds);
}

/** Reconcile verified removals while preserving the scan snapshot and unaffected rows. */
export function removeApplications(
  catalog: ApplicationUninstallScanResult,
  removedApplicationIds: ReadonlySet<string>
): ApplicationUninstallScanResult {
  const candidates = catalog.candidates.filter(candidate => !removedApplicationIds.has(candidate.applicationId));
  if (candidates.length === catalog.candidates.length) return catalog;
  const readyCount = candidates.filter(
    candidate =>
      candidate.capability === 'ready' ||
      (candidate.platform === 'windowsRegistry' && candidate.capability === 'requiresElevation')
  ).length;
  return {
    ...catalog,
    candidates,
    readyCount,
    blockedCount: candidates.length - readyCount,
  };
}
