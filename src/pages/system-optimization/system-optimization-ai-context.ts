import type { AiContext } from '@/lib/models/ai';
import type { SystemSettingItem, SystemSettingTargetState } from '@/lib/models/system-settings';

export function systemOptimizationAiContext(
  item: SystemSettingItem,
  title: string,
  description: string,
  platform: AiContext['platform'],
  pendingTarget: SystemSettingTargetState | null
): AiContext {
  return {
    schemaVersion: 2,
    platform,
    title,
    description,
    subject: {
      module: 'systemOptimization',
      status: item.status,
      selectionKind: item.selectionKind,
      diagnostic: item.diagnostic,
      riskLevel: item.riskLevel,
      // Name the historical fact explicitly; it is not a future backup policy.
      hasRecordedOriginalValue: item.restoreAvailable,
      requiresRestart: item.requiresRestart,
      requiresElevation: item.requiresElevation,
      pendingTarget,
    },
  };
}
