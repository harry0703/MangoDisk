import type { PrivacyItem, PrivacyTimeRange } from './privacy';
import type { ScanRuleResult } from './cleanup';
import type { StartupArtifact } from './startup';
import type { SystemSettingItem, SystemSettingTargetState } from './system-settings';
import type { SystemMaintenanceItem } from './system-maintenance';

export type AiReasoningMode = 'default' | 'disabled';

export const AI_ERROR_LABELS = {
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
  schemaVersion: 1;
  endpoint: string;
  model: string;
  hasKey: boolean;
  reasoning: AiReasoningMode;
}

export interface AiConfigurationUpdate {
  endpoint: string;
  model: string;
  apiKey: string | null;
  reasoning: AiReasoningMode;
}

/** Secret-bearing data is scoped to the settings editor, never the AI store. */
export interface AiConfiguration extends AiConfigurationUpdate {
  schemaVersion: 1;
  apiKey: string;
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
