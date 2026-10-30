import { describe, expect, it } from 'vitest';
import type { GpuTemperature, MetricReading } from '@/lib/models/system-resources/system-resources';
import { currentGpuTemperature, observedGpuTemperature } from '@/lib/utils/system-resources/gpu-temperature';
function reading(celsius = 51): MetricReading<GpuTemperature> {
  return {
    status: 'ready',
    sampledAtMs: 1000,
    value: { adapterId: 'a', adapterName: 'GPU', celsius, kind: 'device', source: 'driver', sensorCount: 1 },
  };
}
describe('identified GPU temperatures', () => {
  it.each([0, -1, 128, 151, NaN, Infinity])('rejects invalid or sentinel temperature %s', celsius => {
    expect(currentGpuTemperature(reading(celsius), 1000)).toBeNull();
  });
  it('accepts only fresh, nonfuture, identified sensor observations', () => {
    expect(currentGpuTemperature(reading(), 11000)?.celsius).toBe(51);
    expect(currentGpuTemperature(reading(), 11001)).toBeNull();
    expect(currentGpuTemperature(reading(), 999)).toBeNull();
    const value = reading();
    value.value!.adapterId = '';
    expect(currentGpuTemperature(value, 1000)).toBeNull();
    value.value!.adapterId = 'a';
    value.value!.sensorCount = 1.5;
    expect(currentGpuTemperature(value, 1000)).toBeNull();
    value.value!.sensorCount = 1;
    value.status = 'failed';
    expect(currentGpuTemperature(value, 1000)).toBeNull();
  });
});

it('bounds retained temperature diagnostics without extending the native sample age', () => {
  const sample = { ...reading(), status: 'failed' as const };
  expect(observedGpuTemperature(sample, 31_000)).not.toBeNull();
  expect(observedGpuTemperature(sample, 31_001)).toBeNull();
  expect(observedGpuTemperature(sample, 999)).toBeNull();
});
