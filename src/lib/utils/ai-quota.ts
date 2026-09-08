import type { AiQuota } from '@/lib/models/ai';

/** Use elapsed monotonic time so a changed local wall clock cannot extend cooldown. */
export function aiQuotaCooldownSeconds(
  quota: Pick<AiQuota, 'serverTime' | 'nextAllowedAt'> | null,
  readAt: number,
  now: number
): number {
  if (!quota) return 0;
  const remaining = Date.parse(quota.nextAllowedAt) - Date.parse(quota.serverTime) - Math.max(0, now - readAt);
  // An invalid snapshot must not disable retry forever; the server still enforces admission.
  return Number.isFinite(remaining) ? Math.max(0, Math.ceil(remaining / 1000)) : 0;
}

/** Availability also covers cooldown and concurrency; only service-wide reasons
 * belong to the temporary-unavailability message. */
export function isAiServiceUnavailable(quota: Pick<AiQuota, 'unavailableReason'> | null | undefined): boolean {
  return quota?.unavailableReason === 'AI_SERVICE_DISABLED' || quota?.unavailableReason === 'AI_GLOBAL_LIMIT_REACHED';
}

/** The server resets at UTC midnight, which may fall on the user's current day. */
export function formatAiQuotaResetAt(resetAt: string, locale: string, timeZone: string): string | null {
  const date = new Date(resetAt);
  if (!Number.isFinite(date.getTime())) return null;
  return new Intl.DateTimeFormat(locale, {
    timeZone,
    month: 'numeric',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  }).format(date);
}
