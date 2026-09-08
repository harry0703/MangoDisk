import type { PrivacyItem, PrivacyTimeRange } from './privacy';
import type { ScanRuleResult } from './cleanup';
import type { StartupArtifact } from './startup';
import type { SystemSettingItem, SystemSettingTargetState } from './system-settings';
import type { SystemMaintenanceItem } from './system-maintenance';

export type AiReasoningMode = 'default' | 'disabled';
export type AiServiceMode = 'free' | 'custom';

export interface AiClientMetadata {
  installId: string;
  appVersion: string;
  locale: string;
  distribution: string;
  osVersion: string;
  timezone: string;
}

export interface AiQuota {
  available: boolean;
  unavailableReason: string | null;
  dailyLimit: number;
  remaining: number;
  cooldownSeconds: number;
  nextAllowedAt: string;
  resetAt: string;
  serverTime: string;
  activeRequests: number;
  maxConcurrentRequests: number;
  policyVersion: string;
  promptVersion: string;
}

export const AI_ERROR_LABELS = {
  freeUnavailable: 'ai.errors.freeUnavailable',
  freeConsentRequired: 'ai.errors.freeConsentRequired',
  freeDailyLimit: 'ai.errors.freeDailyLimit',
  freeRateLimited: 'ai.errors.freeRateLimited',
  freeConcurrent: 'ai.errors.freeConcurrent',
  freeClockSkew: 'ai.errors.freeClockSkew',
  freeSignatureInvalid: 'ai.errors.freeSignatureInvalid',
  freeRequestExists: 'ai.errors.freeRequestExists',
  freeArchiveUnavailable: 'ai.errors.freeArchiveUnavailable',
  invalidConfiguration: 'ai.errors.invalidConfiguration',
  invalidContext: 'ai.errors.invalidContext',
  notConfigured: 'ai.errors.notConfigured',
  configurationUnavailable: 'ai.errors.configurationUnavailable',
  busy: 'ai.errors.busy',
  cancelled: 'ai.errors.cancelled',
  unauthorized: 'ai.errors.unauthorized',
  quotaExceeded: 'ai.errors.quotaExceeded',
  modelUnavailable: 'ai.errors.modelUnavailable',
  providerRejected: 'ai.errors.providerRejected',
  connectionFailed: 'ai.errors.connectionFailed',
  timeout: 'ai.errors.timeout',
  invalidStream: 'ai.errors.invalidStream',
  incompleteStream: 'ai.errors.incompleteStream',
  outputLimit: 'ai.errors.outputLimit',
  emptyResponse: 'ai.errors.emptyResponse',
  responseTooLarge: 'ai.errors.responseTooLarge',
} as const;

export interface AiSettings {
  schemaVersion: 2;
  mode: AiServiceMode;
  freeConsent: boolean;
  freeAvailable: boolean;
  endpoint: string;
  model: string;
  hasKey: boolean;
  reasoning: AiReasoningMode;
  temperature?: number | null;
  maxTokens?: number | null;
}

export interface AiConfigurationUpdate {
  mode: AiServiceMode;
  freeConsent: boolean;
  endpoint: string;
  model: string;
  apiKey: string | null;
  reasoning: AiReasoningMode;
  temperature?: number | null;
  maxTokens?: number | null;
}

/** Secret-bearing data is scoped to the settings editor, never the AI store. */
export interface AiConfiguration extends AiConfigurationUpdate {
  schemaVersion: 2;
  apiKey: string;
}

/** One fresh editor read; this secret-bearing snapshot must not be cached. */
export interface AiEditorState {
  configuration: AiConfiguration | null;
  freeAvailable: boolean;
}

/** Descriptive metadata, including original startup locations; never executable actions. */
export interface AiContext {
  schemaVersion: 2;
  platform: 'macos' | 'windows' | 'unknown';
  title: string;
  description: string;
  subject: AiSubject;
}

export type AiStartupEntry = Pick<
  StartupArtifact,
  | 'sourceKind'
  | 'triggers'
  | 'configuredState'
  | 'runtimeState'
  | 'controlCapability'
  | 'diagnostics'
  | 'removalSupported'
> & {
  name: string;
  identity: {
    applicationName: string;
    publisher: string;
    executableName: string;
    executablePath: string;
    configurationPath: string;
    description: string;
    version: string;
    trust: StartupArtifact['trust'];
  };
};

/** Each module exposes only facts relevant to its explanation, not its full domain object. */
export type AiSubject =
  | {
      module: 'cleanup';
      impact: string;
      bytes: number;
      itemCount: number;
      requiresAppClose: boolean;
      scan: Pick<
        ScanRuleResult,
        | 'ruleId'
        | 'risk'
        | 'status'
        | 'available'
        | 'selectable'
        | 'runningProcesses'
        | 'sources'
        | 'sourceCount'
        | 'sourcesTruncated'
      >;
    }
  | ({ module: 'privacy'; timeRange: PrivacyTimeRange } & Pick<
      PrivacyItem,
      | 'kind'
      | 'impact'
      | 'capability'
      | 'recommendation'
      | 'itemCount'
      | 'estimatedBytes'
      | 'requiresBrowserClose'
      | 'synchronizationMayPropagate'
    >)
  | { module: 'startup'; entries: AiStartupEntry[]; omittedCount: number }
  | ({
      module: 'systemOptimization';
      pendingTarget: SystemSettingTargetState | null;
      hasRecordedOriginalValue: boolean;
    } & Pick<
      SystemSettingItem,
      'status' | 'riskLevel' | 'requiresRestart' | 'requiresElevation' | 'selectionKind' | 'diagnostic'
    >)
  | ({ module: 'systemMaintenance' } & Pick<
      SystemMaintenanceItem,
      | 'taskId'
      | 'status'
      | 'riskLevel'
      | 'requiresRestart'
      | 'requiresElevation'
      | 'estimatedDurationSeconds'
      | 'diagnostic'
    >);

/** Provider output channels remain distinct through streaming and caching. */
export type AiDelta = { kind: 'text' | 'reasoning'; text: string };

export interface AiUsage {
  promptTokens: number | null;
  completionTokens: number | null;
}

export const AI_ERROR_CODES = [
  'freeUnavailable',
  'freeConsentRequired',
  'freeDailyLimit',
  'freeRateLimited',
  'freeConcurrent',
  'freeClockSkew',
  'freeSignatureInvalid',
  'freeRequestExists',
  'freeArchiveUnavailable',
  'invalidConfiguration',
  'invalidContext',
  'notConfigured',
  'configurationUnavailable',
  'busy',
  'cancelled',
  'unauthorized',
  'quotaExceeded',
  'modelUnavailable',
  'providerRejected',
  'connectionFailed',
  'timeout',
  'invalidStream',
  'incompleteStream',
  'outputLimit',
  'emptyResponse',
  'responseTooLarge',
] as const;
export type AiErrorCode = (typeof AI_ERROR_CODES)[number];
