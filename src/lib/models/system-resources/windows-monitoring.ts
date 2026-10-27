export type WindowsMonitoringState = 'notInstalled' | 'ready' | 'unavailable' | 'incompatible' | 'unsupportedHardware';
export interface WindowsDependencyIssue {
  reason: 'missing' | 'incompatible' | 'certificateUntrusted' | 'verificationFailed' | 'unavailable';
  nativeCode?: string;
}
export interface WindowsMonitoringObservation {
  state: WindowsMonitoringState;
  driverAvailable: boolean;
  serviceInstalled: boolean;
  dependencyIssue?: WindowsDependencyIssue;
}
export type WindowsMonitoringOutcome =
  | 'ready'
  | 'removed'
  | 'unsupportedHardware'
  | 'busy'
  | 'verificationFailed'
  | 'certificateUntrusted'
  | 'userCancelled'
  | 'installationFailed'
  | 'serviceUnavailable'
  | 'restartRequired'
  | 'driverRemovalFailed'
  | 'removalFailed';
export interface WindowsMonitoringSetupResult {
  outcome: WindowsMonitoringOutcome;
  observation: WindowsMonitoringObservation;
  nativeCode?: string;
}
