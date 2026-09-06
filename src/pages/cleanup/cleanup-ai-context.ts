import type { AiContext } from '@/lib/models/ai';
import type { PresentedScanRuleResult } from '@/lib/models/cleanup';

export function cleanupAiContext(
  rule: PresentedScanRuleResult,
  platform: AiContext['platform'] = 'unknown'
): AiContext | null {
  // Custom rule names and descriptions may contain user-entered private paths.
  // The initial adapter only exposes built-in localized descriptions.
  if (rule.category === 'custom') return null;
  return {
    schemaVersion: 2,
    platform,
    title: rule.name,
    description: rule.description,
    subject: {
      module: 'cleanup',
      impact: rule.impact,
      bytes: rule.bytes,
      itemCount: rule.fileCount,
      requiresAppClose: rule.requiresAppClose,
      scan: {
        ruleId: rule.ruleId,
        risk: rule.risk,
        status: rule.status,
        available: rule.available,
        selectable: rule.selectable,
        runningProcesses: [...rule.runningProcesses],
        sources: rule.sources.map(source => ({
          path: source.path,
          bytes: source.bytes,
          fileCount: source.fileCount,
          modifiedAtMs: source.modifiedAtMs,
          blockReason: source.blockReason,
        })),
        sourceCount: rule.sourceCount,
        sourcesTruncated: rule.sourcesTruncated,
      },
    },
  };
}
