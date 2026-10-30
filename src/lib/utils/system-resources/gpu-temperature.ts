import { cachedMetricValue, hasFreshValue } from './metric-reading';
import {
  GPU_TEMPERATURE_FRESHNESS_MS,
  TEMPERATURE_DIAGNOSTIC_RETENTION_MS,
  type GpuTemperature,
  type MetricReading,
} from '@/lib/models/system-resources/system-resources';

/** Require a fresh, identified observation; a missing reading never means zero. */
export function currentGpuTemperature(reading: MetricReading<GpuTemperature>, now: number): GpuTemperature | null {
  return hasFreshValue(reading, now, GPU_TEMPERATURE_FRESHNESS_MS) ? observedGpuTemperature(reading, now) : null;
}

/** Last valid identified observation for cached-first panel presentation. */
export function observedGpuTemperature(reading: MetricReading<GpuTemperature>, now: number): GpuTemperature | null {
  const value = cachedMetricValue(reading, now, TEMPERATURE_DIAGNOSTIC_RETENTION_MS);
  return value !== null &&
    Number.isFinite(value.celsius) &&
    value.celsius > 0 &&
    value.celsius <= 150 &&
    value.celsius !== 128 &&
    value.adapterId.length > 0 &&
    Number.isInteger(value.sensorCount) &&
    value.sensorCount > 0
    ? value
    : null;
}
