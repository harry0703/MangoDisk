export type SystemSettingsPlatform = 'macos' | 'windows' | 'linux';
export type SystemSettingCategory = 'performance' | 'productivity' | 'privacy' | 'storage' | 'gaming' | 'appearance';
export type SystemSettingSelectionKind = 'oneClick' | 'custom';
export type SystemSettingRiskLevel = 'standard' | 'caution' | 'high';
export type SystemSettingStatus = 'recommended' | 'optimized' | 'unavailable';
export type SystemSettingDiagnosticCode = 'accessDenied' | 'invalidData' | 'unsupported' | 'stateUnavailable';

export interface SystemSettingItem {
  settingId: string;
  category: SystemSettingCategory;
  selectionKind: SystemSettingSelectionKind;
  riskLevel: SystemSettingRiskLevel;
  status: SystemSettingStatus;
  selectedByDefault: boolean;
  restoreAvailable: boolean;
  requiresRestart: boolean;
  requiresElevation: boolean;
  diagnostic: SystemSettingDiagnosticCode | null;
}

export interface SystemSettingsCatalogSummary {
  itemCount: number;
  recommendedCount: number;
  optimizedCount: number;
  selectedCount: number;
  unavailableCount: number;
}

export interface SystemSettingsCatalog {
  schemaVersion: number;
  scanId: string;
  catalogRevision: string;
  platform: SystemSettingsPlatform;
  scannedAtMs: number;
  items: SystemSettingItem[];
  summary: SystemSettingsCatalogSummary;
  elapsedMs: number;
  recoveryAvailable: boolean;
}

export interface SystemSettingsChangeSelection {
  scanId: string;
  items: SystemSettingChangeSelectionItem[];
}

export type SystemSettingTargetState = 'optimized' | 'default';

export interface SystemSettingChangeSelectionItem {
  settingId: string;
  target: SystemSettingTargetState;
}

export type SystemSettingChangeSkipReason =
  | 'alreadyOptimized'
  | 'alreadyDefault'
  | 'catalogExpired'
  | 'settingChanged'
  | 'settingMissing'
  | 'stateUnavailable'
  | 'unsupported';

export interface SystemSettingChangePlanItem {
  settingId: string;
  category: SystemSettingCategory;
  target: SystemSettingTargetState;
  requiresRestart: boolean;
  requiresElevation: boolean;
}

export interface SystemSettingChangeSkippedItem {
  settingId: string;
  reason: SystemSettingChangeSkipReason;
}

export interface SystemSettingsChangePlan {
  schemaVersion: number;
  planId: string;
  scanId: string;
  catalogRevision: string;
  createdAtMs: number;
  expiresAtMs: number;
  items: SystemSettingChangePlanItem[];
  skippedItems: SystemSettingChangeSkippedItem[];
  requiresConfirmation: boolean;
  requiresRestart: boolean;
}

export type SystemSettingChangeOutcomeStatus = 'changed' | 'unchanged' | 'failed';
export type SystemSettingChangeFailureReason =
  'settingChanged' | 'permissionDenied' | 'unsupported' | 'verificationFailed' | 'platformFailure' | 'userCancelled';

export interface SystemSettingChangeItemResult {
  settingId: string;
  status: SystemSettingChangeOutcomeStatus;
  verified: boolean;
  failureReason: SystemSettingChangeFailureReason | null;
}

export interface SystemSettingsChangeResult {
  planId: string;
  changedCount: number;
  failedCount: number;
  requiresRestart: boolean;
  recoveryAvailable: boolean;
  items: SystemSettingChangeItemResult[];
  catalog: SystemSettingsCatalog | null;
}
