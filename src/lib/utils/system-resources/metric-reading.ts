import type { MetricReading } from '@/lib/models/system-resources/system-resources';

/** Check observation freshness with an explicit clock; metric-specific validation stays with its owner. */
export function hasFreshValue<T>(
  reading: MetricReading<T>,
  now: number,
  freshnessMs: number
): reading is MetricReading<T> & { value: T; sampledAtMs: number } {
  const sampled = reading.sampledAtMs;
  return (
    reading.status === 'ready' &&
    reading.value !== null &&
    sampled !== null &&
    Number.isFinite(sampled) &&
    Number.isFinite(now) &&
    now >= sampled &&
    now - sampled <= freshnessMs
  );
}

/** Display the last observation while refreshing, without promoting it to a live reading. */
export function cachedMetricValue<T>(reading: MetricReading<T>, now: number, maxAgeMs = Infinity): T | null {
  const sampled = reading.sampledAtMs;
  return (reading.status === 'ready' || reading.status === 'stale' || reading.status === 'failed') &&
    reading.value !== null &&
    sampled !== null &&
    Number.isFinite(sampled) &&
    sampled >= 0 &&
    Number.isFinite(now) &&
    now >= sampled &&
    now - sampled <= maxAgeMs
    ? reading.value
    : null;
}
