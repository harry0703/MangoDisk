import { expect, it } from 'vitest';
import { aiQuotaCooldownSeconds, formatAiQuotaResetAt, isAiServiceUnavailable } from './ai-quota';

it('calculates cooldown from monotonic elapsed time and tolerates invalid snapshots', () => {
  const quota = { serverTime: '2026-09-07T00:00:00Z', nextAllowedAt: '2026-09-07T00:01:00Z' };
  expect(aiQuotaCooldownSeconds(quota, 100, 100)).toBe(60);
  expect(aiQuotaCooldownSeconds(quota, 100, 60100)).toBe(0);
  expect(aiQuotaCooldownSeconds(quota, 100, 59101)).toBe(1);
  expect(aiQuotaCooldownSeconds(quota, 100, 0)).toBe(60);
  expect(aiQuotaCooldownSeconds(null, 0, 0)).toBe(0);
  expect(aiQuotaCooldownSeconds({ ...quota, serverTime: 'invalid' }, 0, 0)).toBe(0);
});

it.each(['AI_SERVICE_DISABLED', 'AI_GLOBAL_LIMIT_REACHED'])('recognizes service unavailability: %s', reason => {
  expect(isAiServiceUnavailable({ unavailableReason: reason })).toBe(true);
});
it.each(['AI_RATE_LIMITED', 'AI_CONCURRENCY_LIMITED', 'AI_DAILY_LIMIT_REACHED', null, 'unknown'])(
  'does not confuse quota restrictions with a service shutdown: %s',
  reason => {
    expect(isAiServiceUnavailable({ unavailableReason: reason })).toBe(false);
  }
);
it('formats the server reset instant in the explicit local timezone, not as tomorrow', () => {
  const reset = '2026-09-07T00:00:00Z';
  expect(formatAiQuotaResetAt(reset, 'zh-CN', 'Asia/Shanghai')).toBe('9/7 08:00');
  expect(formatAiQuotaResetAt(reset, 'en-US', 'America/Los_Angeles')).toBe('9/6, 05:00 PM');
  expect(formatAiQuotaResetAt('invalid', 'zh-CN', 'Asia/Shanghai')).toBeNull();
});
