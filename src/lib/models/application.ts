export type ApplicationLeftoverSource =
  'sandboxContainer' | 'applicationSupport' | 'preferences' | 'logs' | 'savedState' | 'webData' | 'applicationScripts';
export type ApplicationLeftoverConfidence = 'high';
export type ApplicationLeftoverEvidence =
  | 'containerMetadataVerified'
  | 'formerBundleMissing'
  | 'installedOwnerAbsent'
  | 'exactIdentifierAssociation'
  | 'filesystemSnapshotComplete';

export interface ApplicationLeftoverCandidate {
  candidateId: string;
  applicationIdentifier: string;
  applicationName: string;
  source: ApplicationLeftoverSource;
  path: string;
  bytes: number;
  fileCount: number;
  modifiedAtMs: number | null;
  confidence: ApplicationLeftoverConfidence;
  defaultSelected: boolean;
  evidence: ApplicationLeftoverEvidence[];
  snapshotFingerprint: string;
}

export interface ApplicationLeftoverScanResult {
  schemaVersion: number;
  scannedAtMs: number;
  supported: boolean;
  inventoryComplete: boolean;
  accessLimited: boolean;
  candidates: ApplicationLeftoverCandidate[];
  totalBytes: number;
  totalFileCount: number;
  skippedCount: number;
  elapsedMs: number;
}

export interface ApplicationLeftoverPlanItem {
  candidateId: string;
  expectedBytes: number;
  expectedFileCount: number;
  expectedSnapshotFingerprint: string;
}

export type ApplicationLeftoverActionStatus = 'previewed' | 'completed' | 'cancelled' | 'failed';
export type ApplicationLeftoverActionReason =
  'candidateChanged' | 'ownerReappeared' | 'applicationRunning' | 'permanentDeleteFailed';

export interface ApplicationLeftoverActionResult {
  candidateId: string;
  applicationIdentifier: string;
  applicationName: string;
  status: ApplicationLeftoverActionStatus;
  reason: ApplicationLeftoverActionReason | null;
  expectedBytes: number;
  releasedBytes: number;
}

export interface ApplicationLeftoverResult {
  planId: string;
  expectedBytes: number;
  releasedBytes: number;
  affectedItemCount: number;
  failedItemCount: number;
  dryRun: boolean;
  actions: ApplicationLeftoverActionResult[];
  historySaved: boolean;
}

export type ApplicationUninstallPlatform = 'macosBundle' | 'windowsRegistry';
export type ApplicationUninstallInstallerKind =
  'windowsMsi' | 'windowsAppx' | 'windowsScoop' | 'windowsChocolatey' | 'windowsRegistered';
export type ApplicationUninstallInventorySource =
  'macosBundle' | 'windowsRegistry' | 'windowsMsi' | 'windowsAppx' | 'winget' | 'steam' | 'scoop' | 'chocolatey';
export type ApplicationUninstallExecutionMode = 'silent' | 'interactive' | 'externalClient';
export interface ApplicationUninstallSourceIdentity {
  source: ApplicationUninstallInventorySource;
  identifier: string;
}
export type ApplicationUninstallCapability =
  'ready' | 'applicationRunning' | 'requiresElevation' | 'protectedApplication' | 'viewOnly';
export type ApplicationUninstallRecordState = 'installed' | 'orphanedRegistration';

export type ApplicationUninstallDiagnostic =
  | 'commandMissing'
  | 'commandUnreadable'
  | 'invalidCommand'
  | 'relativeExecutable'
  | 'unresolvedEnvironment'
  | 'executableMissing'
  | 'executableAccessDenied'
  | 'executableProbeFailed'
  | 'invalidExecutable'
  | 'unsupportedCommandHost'
  | 'registrationConflict';

export interface ApplicationUninstallCandidate {
  applicationId: string;
  primaryIdentifier: string;
  sourceIdentities: ApplicationUninstallSourceIdentity[];
  name: string;
  version: string | null;
  publisher: string | null;
  estimatedBytes: number;
  lastUsedAtMs: number | null;
  installedAtMs: number | null;
  platform: ApplicationUninstallPlatform;
  installerKind: ApplicationUninstallInstallerKind | null;
  executionMode: ApplicationUninstallExecutionMode | null;
  capability: ApplicationUninstallCapability;
  recordState: ApplicationUninstallRecordState;
  uninstallDiagnostic: ApplicationUninstallDiagnostic | null;
  applicationPath: string | null;
  possibleRelatedPaths: string[];
  iconPath: string | null;
  runningProcesses: string[];
  totalBytes: number;
  defaultSelectedBytes: number;
  associatedDataComplete: boolean;
  components: ApplicationUninstallComponentSummary[];
}

export interface ApplicationUninstallScanResult {
  schemaVersion: number;
  scannedAtMs: number;
  supported: boolean;
  executionSupported: boolean;
  catalogActionable: boolean;
  inventoryComplete: boolean;
  catalogRevision: string | null;
  candidates: ApplicationUninstallCandidate[];
  readyCount: number;
  blockedCount: number;
  hiddenCount: number;
  relatedDirectoryCount: number;
  relatedPathScanElapsedMs: number;
  elapsedMs: number;
}

export type ApplicationUninstallComponentKind =
  | 'applicationBinary'
  | 'nativeInstaller'
  | 'cache'
  | 'applicationSupport'
  | 'preferences'
  | 'logs'
  | 'savedState'
  | 'sandboxContainer'
  | 'webData';
export type ApplicationUninstallRisk = 'required' | 'rebuildable' | 'userData';

export interface ApplicationUninstallComponentSummary {
  componentId: string;
  kind: ApplicationUninstallComponentKind;
  risk: ApplicationUninstallRisk;
  path: string | null;
  bytes: number;
  fileCount: number;
  defaultSelected: boolean;
}

export interface ApplicationUninstallComponent {
  componentId: string;
  kind: ApplicationUninstallComponentKind;
  risk: ApplicationUninstallRisk;
  path: string | null;
  bytes: number;
  fileCount: number;
  defaultSelected: boolean;
  snapshotFingerprint: string;
}

export interface ApplicationUninstallInspection {
  schemaVersion: number;
  inspectedAtMs: number;
  applicationId: string;
  applicationName: string;
  primaryIdentifier: string;
  platform: ApplicationUninstallPlatform;
  installerKind: ApplicationUninstallInstallerKind | null;
  capability: ApplicationUninstallCapability;
  catalogRevision: string;
  components: ApplicationUninstallComponent[];
  totalBytes: number;
  defaultSelectedBytes: number;
  elapsedMs: number;
}

export interface ApplicationUninstallPlanItem {
  componentId: string;
  kind: ApplicationUninstallComponentKind;
  expectedBytes: number;
  expectedFileCount: number;
  expectedSnapshotFingerprint: string;
}

export interface ApplicationUninstallPlan {
  schemaVersion: number;
  planId: string;
  planHash: string;
  createdAtMs: number;
  applicationId: string;
  catalogRevision: string;
  items: ApplicationUninstallPlanItem[];
  expectedBytes: number;
}

export interface ApplicationUninstallBatchSelection {
  applicationId: string;
  componentIds: string[];
}

export interface ApplicationUninstallBatchPlan {
  schemaVersion: number;
  batchId: string;
  batchHash: string;
  createdAtMs: number;
  catalogRevision: string;
  plans: ApplicationUninstallPlan[];
  expectedBytes: number;
}

export interface ApplicationUninstallBatchPreparation {
  plan: ApplicationUninstallBatchPlan;
  preview: ApplicationUninstallBatchResult;
}

export type ApplicationUninstallExecutionStage = 'validating' | 'uninstalling' | 'finalizing';

export type ApplicationUninstallExecutionItemStatus = 'completed' | 'cancelled' | 'failed';

export interface ApplicationUninstallExecutionItemResult {
  applicationId: string;
  status: ApplicationUninstallExecutionItemStatus;
  releasedBytes: number;
}

export interface ApplicationUninstallExecutionProgress {
  stage: ApplicationUninstallExecutionStage;
  currentApplicationId: string | null;
  completedApplications: ApplicationUninstallExecutionItemResult[];
  completedApplicationCount: number;
  totalApplicationCount: number;
  affectedApplicationCount: number;
  failedApplicationCount: number;
  releasedBytes: number;
  elapsedMs: number;
}

export type ApplicationUninstallActionStatus = 'previewed' | 'completed' | 'cancelled' | 'failed';
export type ApplicationUninstallActionReason =
  | 'applicationUnavailable'
  | 'applicationRunning'
  | 'processStateUnavailable'
  | 'catalogChanged'
  | 'componentUnavailable'
  | 'componentChanged'
  | 'unsupportedExecutor'
  | 'executionAborted'
  | 'externalUninstallerContinuing'
  | 'permanentDeleteFailed'
  | 'recoveryRequired'
  | 'nativeInstallerFailed'
  | 'verificationFailed';

export interface ApplicationUninstallActionResult {
  componentId: string;
  kind: ApplicationUninstallComponentKind;
  status: ApplicationUninstallActionStatus;
  reason: ApplicationUninstallActionReason | null;
  expectedBytes: number;
  releasedBytes: number;
}

export interface ApplicationUninstallResult {
  planId: string;
  applicationId: string;
  applicationName: string | null;
  expectedBytes: number;
  previewedBytes: number;
  releasedBytes: number;
  previewedItemCount: number;
  affectedItemCount: number;
  failedItemCount: number;
  releasedBytesIsEstimate: boolean;
  restartRequired: boolean;
  dryRun: boolean;
  actions: ApplicationUninstallActionResult[];
  historySaved: boolean;
}

export interface ApplicationUninstallBatchResult {
  batchId: string;
  expectedBytes: number;
  previewedBytes: number;
  releasedBytes: number;
  selectedApplicationCount: number;
  previewedApplicationCount: number;
  affectedApplicationCount: number;
  failedApplicationCount: number;
  previewedItemCount: number;
  affectedItemCount: number;
  failedItemCount: number;
  releasedBytesIsEstimate: boolean;
  restartRequired: boolean;
  dryRun: boolean;
  results: ApplicationUninstallResult[];
}
