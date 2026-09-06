import type { AiContext } from '@/lib/models/ai';
import type { SystemMaintenanceItem } from '@/lib/models/system-maintenance';

export function systemMaintenanceAiContext(
  item: SystemMaintenanceItem,
  title: string,
  description: string,
  platform: AiContext['platform']
): AiContext {
  return {
    schemaVersion: 2,
    platform,
    title,
    description,
    subject: {
      module: 'systemMaintenance',
      taskId: item.taskId,
      status: item.status,
      riskLevel: item.riskLevel,
      requiresRestart: item.requiresRestart,
      requiresElevation: item.requiresElevation,
      estimatedDurationSeconds: item.estimatedDurationSeconds,
      diagnostic: item.diagnostic,
    },
  };
}
