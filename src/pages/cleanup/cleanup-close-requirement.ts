import { CLEANUP_RULE_IDS, type CleanupSourceSelection, type ScanRuleResult } from '@/lib/models/cleanup';
import * as CleanupRuleSelectionUtils from '@/lib/utils/cleanup-rule-selection';
import * as PathUtils from '@/lib/utils/path';

export interface CleanupCloseRequirement {
  requiresAppClose: boolean;
  runningProcesses: string[];
}

/**
 * Narrows rule-level close requirements to selected sources, including
 * application bundles and protected project artifacts in mixed results.
 */
export function selectedCleanupCloseRequirement(
  rule: ScanRuleResult,
  selectedRuleIds: readonly string[],
  sourceSelections: readonly CleanupSourceSelection[]
): CleanupCloseRequirement {
  if (rule.category === 'project' && !rule.sourcesTruncated) {
    // A mixed rule may include ordinary projects. Only selected worktree
    // sources should offer app closure; ordinary projects must not add it.
    const needsClose = rule.sources.some(
      source =>
        source.blockReason === 'requiresClose' &&
        CleanupRuleSelectionUtils.sourceSelected(rule.ruleId, source.path, selectedRuleIds, sourceSelections)
    );
    return { requiresAppClose: needsClose, runningProcesses: needsClose ? rule.runningProcesses : [] };
  }
  if (rule.ruleId !== CLEANUP_RULE_IDS.macosUniversalBinaries) {
    return {
      requiresAppClose: rule.requiresAppClose,
      runningProcesses: rule.runningProcesses,
    };
  }

  const sourceSelection = sourceSelections.find(selection => selection.ruleId === rule.ruleId);
  if (!sourceSelection) {
    return {
      requiresAppClose: rule.requiresAppClose,
      runningProcesses: rule.runningProcesses,
    };
  }

  const runningProcesses = rule.sources
    .filter(
      source =>
        source.blockReason === 'requiresClose' &&
        CleanupRuleSelectionUtils.sourceSelected(rule.ruleId, source.path, selectedRuleIds, sourceSelections)
    )
    .map(source => PathUtils.fileName(source.path).replace(/\.app$/iu, ''));

  return {
    requiresAppClose: runningProcesses.length > 0,
    runningProcesses: [...new Set(runningProcesses)],
  };
}
