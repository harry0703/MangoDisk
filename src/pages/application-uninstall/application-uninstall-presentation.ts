import type { ApplicationUninstallDiagnostic, ApplicationUninstallInstallerKind } from '@/lib/models/application';

export type ApplicationSizeHintKey =
  'applicationUninstall.windowsAppPackageSizeHint' | 'applicationUninstall.applicationSizeEstimateHint';

/**
 * Windows AppX inventory can include shared package files whose logical size is not attributable
 * to one application. Its dedicated hint explains that limitation; other installer kinds retain
 * the general measured-or-estimated size explanation.
 */
export function applicationSizeHintKey(
  installerKind: ApplicationUninstallInstallerKind | null
): ApplicationSizeHintKey {
  return installerKind === 'windowsAppx'
    ? 'applicationUninstall.windowsAppPackageSizeHint'
    : 'applicationUninstall.applicationSizeEstimateHint';
}

/** Keep the explanation tied to observed evidence; a Settings button does not prove that
 * Windows can launch the registered uninstaller. Unknown failures retain the neutral fallback.
 */
export function applicationUnavailableTitleKey(diagnostic: ApplicationUninstallDiagnostic | null): string {
  switch (diagnostic) {
    case 'executableMissing':
      return 'applicationUninstall.uninstallerMissing';
    case 'executableAccessDenied':
      return 'applicationUninstall.uninstallerAccessDenied';
    default:
      return 'applicationUninstall.uninstallEntryUnavailableDescription';
  }
}
