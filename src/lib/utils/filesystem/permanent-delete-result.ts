import type { PermanentDeleteFailure, PermanentDeleteFailureReason } from '@/lib/models/filesystem/permanent-delete';

const REASONS: PermanentDeleteFailureReason[] = [
  'accessDenied',
  'resourceBusy',
  'itemChanged',
  'protected',
  'notFound',
  'other',
];

export function groupDeleteFailures(failures: PermanentDeleteFailure[]) {
  return REASONS.flatMap(reason => {
    const items = failures.filter(
      item => (REASONS.includes(item.reason ?? 'other') ? (item.reason ?? 'other') : 'other') === reason
    );
    return items.length ? [{ reason, items }] : [];
  });
}

export function deleteFailureDiagnostic(failures: PermanentDeleteFailure[]): string {
  return JSON.stringify(
    failures.map(item => ({
      path: item.path,
      reason: item.reason ?? 'other',
      stage: item.stage ?? null,
      nativeCode: item.nativeCode ?? null,
      partial: item.partial ?? null,
      remainingRestored: item.remainingRestored ?? null,
      diagnostic: item.message,
    })),
    null,
    2
  );
}

export function deleteFailureHintKey(failures: PermanentDeleteFailure[]): string {
  const reasons = failures.map(failure => failure.reason);
  if (reasons.includes('accessDenied') || reasons.includes('resourceBusy')) return 'fileDeletion.retryHint';
  if (reasons.includes('itemChanged') || reasons.includes('notFound')) return 'fileDeletion.hints.itemChanged';
  return reasons.includes('protected') ? 'fileDeletion.hints.protected' : 'fileDeletion.hints.other';
}
