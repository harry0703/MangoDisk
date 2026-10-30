import { cachedMetricValue, hasFreshValue } from './metric-reading';
import {
  CPU_TEMPERATURE_FRESHNESS_MS,
  TEMPERATURE_DIAGNOSTIC_RETENTION_MS,
  type CpuTemperature,
  type MetricReading,
} from '@/lib/models/system-resources/system-resources';

/** Require a fresh, identified observation; a missing reading never means zero. */
export function currentCpuTemperature(reading: MetricReading<CpuTemperature>, now: number): CpuTemperature | null {
  return hasFreshValue(reading, now, CPU_TEMPERATURE_FRESHNESS_MS) ? observedCpuTemperature(reading, now) : null;
}

/** Last valid identified observation for cached-first panel presentation. */
export function observedCpuTemperature(reading: MetricReading<CpuTemperature>, now: number): CpuTemperature | null {
  const value = cachedMetricValue(reading, now, TEMPERATURE_DIAGNOSTIC_RETENTION_MS);
  return value !== null &&
    Number.isFinite(value.celsius) &&
    value.celsius > 0 &&
    value.celsius <= 150 &&
    Number.isInteger(value.sensorCount) &&
    value.sensorCount > 0
    ? value
    : null;
}
