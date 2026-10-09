import { describe, expect, it } from 'vitest';
import { currentCpuTemperature } from './cpu-temperature';
import type { CpuTemperature, MetricReading } from '@/lib/models/system-resources';
const reading = (): MetricReading<CpuTemperature> => ({
  status: 'ready',
  sampledAtMs: 1000,
  value: { celsius: 48.5, kind: 'coreAverage', source: 'appleSmc', sensorCount: 16 },
});
describe('current CPU temperature', () => {
  it('rejects stale status, expired age and backwards clocks independently', () => {
    expect(currentCpuTemperature(reading(), 11_000)?.celsius).toBe(48.5);
    expect(currentCpuTemperature(reading(), 11_001)).toBeNull();
    expect(currentCpuTemperature(reading(), 999)).toBeNull();
    for (const status of ['loading', 'stale', 'failed', 'unsupported', 'disconnected'] as const) {
      expect(currentCpuTemperature({ ...reading(), status }, 1000)).toBeNull();
    }
  });
  it('does not turn missing sensors, invalid numbers or placeholders into a measurement', () => {
    for (const celsius of [0, -1, 151, NaN, Infinity]) {
      const sample = reading();
      sample.value!.celsius = celsius;
      expect(currentCpuTemperature(sample, 1000)).toBeNull();
    }
    for (const sensorCount of [0, -1, 0.5, NaN]) {
      const sample = reading();
      sample.value!.sensorCount = sensorCount;
      expect(currentCpuTemperature(sample, 1000)).toBeNull();
    }
    expect(currentCpuTemperature({ ...reading(), value: null }, 1000)).toBeNull();
    expect(currentCpuTemperature({ ...reading(), sampledAtMs: null }, 1000)).toBeNull();
    expect(currentCpuTemperature(reading(), NaN)).toBeNull();
  });
});
