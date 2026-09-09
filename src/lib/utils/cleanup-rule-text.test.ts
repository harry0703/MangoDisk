import { describe, expect, it } from 'vitest';

import { CLEANUP_RULE_IDS } from '@/lib/models/cleanup';
import type { CleanupResult, CleanupScanResult, ScanRuleResult } from '@/lib/models/cleanup';
import type { CleanupActionResult } from '@/lib/models/cleanup-action';
import * as CleanupRuleTextUtils from '@/lib/utils/cleanup-rule-text';
import { type CleanupRuleMessageResolver } from '@/lib/utils/cleanup-rule-text';

const RULE: ScanRuleResult = {
  ruleId: 'browser.chrome-cache',
  category: 'browser',
  group: 'userCache',
  risk: 'safe',
  defaultSelected: true,
  recommendedSelected: true,
  bytes: 1024,
  fileCount: 2,
  available: true,
  selectable: true,
  status: 'found',
  runningProcesses: [],
  requiresAppClose: false,
  sources: [],
  sourceCount: 0,
  sourcesTruncated: false,
  scanElapsedMs: 10,
};

const SNAPSHOT: CleanupScanResult = {
  schemaVersion: '1',
  customScanId: null,
  scannedAtMs: 1,
  disk: {
    name: 'Macintosh HD',
    mountPoint: '/',
    totalBytes: 100,
    availableBytes: 40,
    usedBytes: 60,
  },
  rules: [RULE],
  applicationIcons: [],
  warningCount: 0,
  safeBytes: 1024,
  reclaimableBytes: 0,
  applicabilityElapsedMs: 1,
  applicableRuleCount: 1,
  filteredRuleCount: 0,
  inventoryApplicationCount: 0,
  inventoryProcessCount: 0,
  elapsedMs: 10,
};

describe('CleanupRuleTextUtils', () => {
  it('builds a localized presentation without mutating backend facts', () => {
    const messages: Readonly<Record<string, string>> = {
      'cleanupRules.entries.browser.chrome-cache.name': 'Chrome rebuildable cache',
      'cleanupRules.entries.browser.chrome-cache.description': 'Localized description',
      'cleanupRules.entries.browser.chrome-cache.impact': 'Localized impact',
      'cleanupRules.categories.browser': 'Browsers',
    };
    const resolveMessage: CleanupRuleMessageResolver = key => messages[key];

    const result = CleanupRuleTextUtils.snapshot(SNAPSHOT, resolveMessage);

    expect(result).not.toBe(SNAPSHOT);
    expect(result.rules[0]).toMatchObject({
      ruleId: RULE.ruleId,
      category: RULE.category,
      categoryLabel: 'Browsers',
      name: 'Chrome rebuildable cache',
      description: 'Localized description',
      impact: 'Localized impact',
    });
  });

  it('keeps a newly contributed rule readable when translations are absent', () => {
    const rule: ScanRuleResult = {
      ...RULE,
      ruleId: 'dev.future-build-cache',
      category: 'development',
      risk: 'recoverable',
    };
    const messages: Readonly<Record<string, string>> = {
      'cleanupRules.categories.dev': 'Developer tools',
    };

    const result = CleanupRuleTextUtils.snapshot({ ...SNAPSHOT, rules: [rule] }, key => messages[key]);

    expect(result.rules[0]).toMatchObject({
      name: 'Future Build Cache',
      categoryLabel: 'Developer tools',
      description: '',
      impact: '',
    });
  });

  it('uses the backend category for cleanup cleaners', () => {
    const specialRule: ScanRuleResult = {
      ...RULE,
      ruleId: CLEANUP_RULE_IDS.dockerBuildCache,
      category: 'container',
      risk: 'recoverable',
    };
    const messages: Readonly<Record<string, string>> = {
      'cleanupRules.entries.special.docker-build-cache.name': 'Docker build cache',
      'cleanupRules.entries.special.docker-build-cache.description': 'Docker description',
      'cleanupRules.entries.special.docker-build-cache.impact': 'Docker impact',
      'cleanupRules.categories.container': 'Containers',
    };

    const result = CleanupRuleTextUtils.snapshot({ ...SNAPSHOT, rules: [specialRule] }, key => messages[key]);

    expect(result.rules[0]).toMatchObject({
      name: 'Docker build cache',
      category: 'container',
      categoryLabel: 'Containers',
    });
  });

  it('renders structured action reasons without backend messages', () => {
    const action: CleanupActionResult = {
      ruleId: CLEANUP_RULE_IDS.dockerBuildCache,
      actionKind: 'command',
      status: 'partial',
      reasonCode: 'verificationFailed',
      bytesExpected: 1024,
      releasedBytes: 0,
      affectedItemCount: 0,
      failedItemCount: 1,
      runningProcesses: [],
    };
    const record = {
      schemaVersion: 2,
      operationId: 'run-1',
      category: 'deepCleanup' as const,
      startedAtMs: 1,
      finishedAtMs: 2,
      outcome: 'completedWithWarnings' as const,
      dryRun: false,
      selectedItemCount: 1,
      expectedBytes: action.bytesExpected,
      releasedBytes: action.releasedBytes,
      releasedBytesIsEstimate: false,
      affectedItemCount: action.affectedItemCount,
      failedItemCount: action.failedItemCount,
      details: {
        type: 'deepCleanup' as const,
        payload: {
          cleanup: {
            selectedRuleIds: [action.ruleId],
            expectedBytes: action.bytesExpected,
            actions: [action],
          },
          applicationLeftovers: null,
        },
      },
    };
    const cleanupResult: CleanupResult = {
      planId: 'plan-1',
      planHash: 'hash',
      expectedBytes: action.bytesExpected,
      releasedBytes: action.releasedBytes,
      affectedItemCount: action.affectedItemCount,
      failedItemCount: action.failedItemCount,
      dryRun: false,
      actions: [action],
      record,
      historySaved: true,
    };
    const messages: Readonly<Record<string, string>> = {
      [`cleanupRules.entries.${CLEANUP_RULE_IDS.dockerBuildCache}.name`]: 'Docker build cache',
      'cleanupRules.actionReasons.verificationFailed':
        'The request completed, but its final state could not be verified.',
      'cleanupRules.actionMessages.partial': 'Some files were skipped.',
    };

    const result = CleanupRuleTextUtils.cleanupResult(cleanupResult, key => messages[key]);

    expect(result.actions[0]).toMatchObject({
      name: 'Docker build cache',
      message: messages['cleanupRules.actionReasons.verificationFailed'],
    });
    expect(result.record.details.payload.cleanup?.actions[0].message).toBe(result.actions[0].message);

    // Project cleanup must carry its process reason through both the result
    // dialog and persisted history, including mixed-success actions.
    for (const status of ['blocked', 'partial'] as const) {
      const guardedAction: CleanupActionResult = {
        ...action,
        ruleId: 'project.rust-build-artifacts',
        actionKind: 'delete',
        status,
        reasonCode: 'runningProcesses',
        runningProcesses: ['Codex', 'ChatGPT'],
      };
      const guardedResult = CleanupRuleTextUtils.cleanupResult(
        {
          ...cleanupResult,
          actions: [guardedAction],
          record: {
            ...record,
            details: {
              ...record.details,
              payload: {
                ...record.details.payload,
                cleanup: { ...record.details.payload.cleanup, actions: [guardedAction] },
              },
            },
          },
        },
        (key, values) =>
          key === 'cleanupRules.actionReasons.runningProcesses'
            ? `Close ${values?.processes} before retrying`
            : undefined
      );
      expect(guardedResult.actions[0]).toMatchObject({ status, message: 'Close Codex, ChatGPT before retrying' });
      expect(guardedResult.record.details.payload.cleanup?.actions[0].message).toBe(guardedResult.actions[0].message);
    }
  });
});
