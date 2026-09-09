import type { SystemMaintenanceExecutionItemResult } from '@/lib/models/system-maintenance';

type Translate = (key: string, values?: Record<string, string>) => string;

/** Keep the known cause visible even when an earlier step may already have changed state. */
export function maintenanceFailureFeedback(
  result: SystemMaintenanceExecutionItemResult,
  name: string,
  t: Translate
): { message: string } {
  const reason = result.failureReason ?? 'platformFailure';
  if (reason === 'platformFailure' && result.mutationState === 'mayHaveChanged') {
    return { message: t('systemMaintenance.feedback.mayHaveChanged', { name }) };
  }
  return {
    message: t(`systemMaintenance.feedback.failures.${reason}`, { name }),
  };
}
