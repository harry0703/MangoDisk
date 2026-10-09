import { CPU_TEMPERATURE_FRESHNESS_MS, type CpuTemperature, type MetricReading } from '@/lib/models/system-resources';

/** Require a fresh, identified observation; a missing reading never means zero. */
export function currentCpuTemperature(reading: MetricReading<CpuTemperature>, now: number): CpuTemperature | null {
  const value = reading.value;
  const sampled = reading.sampledAtMs;
  return reading.status === 'ready' &&
    sampled !== null &&
    Number.isFinite(sampled) &&
    Number.isFinite(now) &&
    now >= sampled &&
    now - sampled <= CPU_TEMPERATURE_FRESHNESS_MS &&
    value !== null &&
    Number.isFinite(value.celsius) &&
    value.celsius > 0 &&
    value.celsius <= 150 &&
    Number.isInteger(value.sensorCount) &&
    value.sensorCount > 0
    ? value
    : null;
}
