import { describe, expect, it } from 'vitest';

import { CLEANUP_RULE_IDS, type ScanRuleResult } from '@/lib/models/cleanup';

import { selectedCleanupCloseRequirement } from './cleanup-close-requirement';

const rule: ScanRuleResult = {
  ruleId: CLEANUP_RULE_IDS.macosUniversalBinaries,
  category: 'applicationOptimization',
  group: 'applicationOptimization',
  risk: 'recoverable',
  defaultSelected: false,
  recommendedSelected: false,
  bytes: 1_500,
  fileCount: 3,
  available: true,
  selectable: true,
  status: 'found',
  runningProcesses: ['Browser', 'Telegram'],
  requiresAppClose: true,
  sources: [
    {
      path: '/Applications/Browser.app',
      bytes: 500,
      fileCount: 1,
      modifiedAtMs: null,
      blockReason: 'requiresClose',
    },
    {
      path: '/Applications/Telegram.app',
      bytes: 600,
      fileCount: 1,
      modifiedAtMs: null,
      blockReason: 'requiresClose',
    },
    {
      path: '/Applications/Editor.app',
      bytes: 400,
      fileCount: 1,
      modifiedAtMs: null,
      blockReason: null,
    },
  ],
  sourceCount: 3,
  sourcesTruncated: false,
  scanElapsedMs: 1,
};

describe('cleanup close requirement', () => {
  it('offers app closure only when a protected project source is selected', () => {
    const projectRule: ScanRuleResult = {
      ...rule,
      category: 'project',
      requiresAppClose: true,
      runningProcesses: ['Codex', 'ChatGPT'],
    };
    expect(selectedCleanupCloseRequirement(projectRule, [rule.ruleId], [])).toEqual({
      requiresAppClose: true,
      runningProcesses: ['Codex', 'ChatGPT'],
    });
    for (const selection of [
      { mode: 'include' as const, paths: ['/Applications/Editor.app'] },
      { mode: 'exclude' as const, paths: ['/Applications/Browser.app', '/Applications/Telegram.app'] },
    ]) {
      expect(
        selectedCleanupCloseRequirement(projectRule, [rule.ruleId], [{ ruleId: rule.ruleId, ...selection }])
      ).toEqual({
        requiresAppClose: false,
        runningProcesses: [],
      });
    }
  });
  it('keeps the rule-level process list when every source is selected', () => {
    expect(selectedCleanupCloseRequirement(rule, [rule.ruleId], [])).toEqual({
      requiresAppClose: true,
      runningProcesses: ['Browser', 'Telegram'],
    });
  });

  it('includes only running applications selected by source', () => {
    expect(
      selectedCleanupCloseRequirement(
        rule,
        [rule.ruleId],
        [{ ruleId: rule.ruleId, mode: 'include', paths: ['/Applications/Telegram.app'] }]
      )
    ).toEqual({
      requiresAppClose: true,
      runningProcesses: ['Telegram'],
    });
  });

  it('does not request app closure when the selected source is not running', () => {
    expect(
      selectedCleanupCloseRequirement(
        rule,
        [rule.ruleId],
        [{ ruleId: rule.ruleId, mode: 'include', paths: ['/Applications/Editor.app'] }]
      )
    ).toEqual({
      requiresAppClose: false,
      runningProcesses: [],
    });
  });
});
