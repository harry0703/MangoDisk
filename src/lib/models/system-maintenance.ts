export type SystemMaintenancePlatform = 'macos' | 'windows';
export type SystemMaintenanceCategory = 'network' | 'searchAndInterface' | 'systemRepair';
export type SystemMaintenanceRiskLevel = 'standard' | 'caution';
export type SystemMaintenanceStatus = 'healthy' | 'recommended' | 'available' | 'unavailable';
export type SystemMaintenanceDiagnosticCode =
  | 'accessDenied'
  | 'applicationRunning'
  | 'checkFailed'
  | 'componentUnavailable'
  | 'toolUnavailable'
  | 'unsupportedVersion';

export interface SystemMaintenanceItem {
  taskId: string;
  category: SystemMaintenanceCategory;
  riskLevel: SystemMaintenanceRiskLevel;
  status: SystemMaintenanceStatus;
  requiresElevation: boolean;
  requiresRestart: boolean;
  estimatedDurationSeconds: number;
  diagnostic: SystemMaintenanceDiagnosticCode | null;
}

export interface SystemMaintenanceCatalogSummary {
  itemCount: number;
  recommendedCount: number;
  availableCount: number;
  healthyCount: number;
  unavailableCount: number;
}

export interface SystemMaintenanceCatalog {
  schemaVersion: number;
  scanId: string;
  platform: SystemMaintenancePlatform;
  scannedAtMs: number;
  elapsedMs: number;
  items: SystemMaintenanceItem[];
  summary: SystemMaintenanceCatalogSummary;
}

export interface SystemMaintenanceExecutionRequest {
  scanId: string;
  taskId: string;
  authorizationPrompt: string;
}

export type SystemMaintenanceExecutionStatus = 'completed' | 'started' | 'failed';
export type SystemMaintenanceFailureReason =
  | 'permissionDenied'
  | 'unsupported'
  | 'verificationFailed'
  | 'verificationPermissionDenied'
  | 'toolUnavailable'
  | 'serviceDisabled'
  | 'serviceUnavailable'
  | 'dependencyUnavailable'
  | 'serviceBusy'
  | 'timedOut'
  | 'platformFailure'
  | 'userCancelled';
export type SystemMaintenanceMutationState = 'notChanged' | 'changed' | 'mayHaveChanged';

export interface SystemMaintenanceExecutionItemResult {
  taskId: string;
  status: SystemMaintenanceExecutionStatus;
  mutationState: SystemMaintenanceMutationState;
  verified: boolean;
  requiresRestart: boolean;
  failureReason: SystemMaintenanceFailureReason | null;
}

export type SystemMaintenanceJobStatus = 'queued' | 'running' | 'cancelling' | 'finished';

export type SystemMaintenancePhase =
  | 'preparing'
  | 'waitingForAuthorization'
  | 'repairingComponentImage'
  | 'checkingSystemFiles'
  | 'checkingStartupDisk'
  | 'checkingSystemDisk'
  | 'rebuildingSearchIndex'
  | 'refreshingShellCaches'
  | 'restartingFinder'
  | 'restartingAudioService'
  | 'restartingServices'
  | 'repairingPrintQueue'
  | 'synchronizingTime'
  | 'rebuildingPerformanceCounters'
  | 'resettingStoreCache'
  | 'refreshingNetwork'
  | 'rebuildingAppAssociations'
  | 'repairingPermissions'
  | 'restoringDefaults'
  | 'verifying';

export interface SystemMaintenanceProgress {
  phase: SystemMaintenancePhase;
  currentStep: number | null;
  totalSteps: number | null;
  percent: number | null;
}

export interface SystemMaintenanceJob {
  executionId: string;
  scanId: string;
  taskId: string;
  revision: number;
  status: SystemMaintenanceJobStatus;
  cancelable: boolean;
  queuedAtMs: number;
  startedAtMs: number | null;
  finishedAtMs: number | null;
  progress: SystemMaintenanceProgress | null;
  result: SystemMaintenanceExecutionItemResult | null;
}

export interface SystemMaintenanceRuntimeState {
  catalog: SystemMaintenanceCatalog | null;
  executions: SystemMaintenanceJob[];
}
