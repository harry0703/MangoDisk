import type { MetricId } from '@/lib/models/system-resources/system-resources';

export type ColorMetricId =
  | Exclude<MetricId, 'network'>
  | 'cpuTemperature'
  | 'gpuTemperature'
  | 'diskTemperature'
  | 'batteryTemperature'
  | 'chargingPower'
  | 'batteryPower'
  | 'cpuPower'
  | 'gpuPower'
  | 'systemPower'
  | 'fanSpeed';
export type DisplayColor = 'default' | 'green' | 'yellow' | 'orange' | 'red' | 'blue';
export type ColorRule =
  | { condition: 'below'; value: number; color: DisplayColor }
  | { condition: 'range'; from: number; to: number; color: DisplayColor }
  | { condition: 'atLeast'; value: number; color: DisplayColor };
export interface MetricColorRule {
  id: ColorMetricId;
  enabled: boolean;
  rules: ColorRule[];
}
export const MAX_COLOR_RULES = 16;
export const DISPLAY_COLORS: DisplayColor[] = ['default', 'green', 'yellow', 'orange', 'red', 'blue'];
export function isTemperatureMetric(id: ColorMetricId): boolean {
  return id === 'cpuTemperature' || id === 'gpuTemperature' || id === 'diskTemperature' || id === 'batteryTemperature';
}
export function defaultColorRule(id: ColorMetricId): MetricColorRule {
  if (isPowerMetric(id)) return { id, enabled: false, rules: [] };
  const [middle, high] =
    id === 'fanSpeed'
      ? [2000, 4000]
      : id === 'diskTemperature'
        ? [50, 60]
        : id === 'batteryTemperature'
          ? [40, 45]
          : isTemperatureMetric(id)
            ? [60, 80]
            : [70, 90];
  return {
    id,
    enabled: true,
    rules: [
      { condition: 'below', value: middle!, color: 'green' },
      { condition: 'range', from: middle!, to: high!, color: 'orange' },
      { condition: 'atLeast', value: high!, color: 'red' },
    ],
  };
}
export function defaultColorRules(): MetricColorRule[] {
  return (
    [
      'cpu',
      'cpuTemperature',
      'gpu',
      'gpuTemperature',
      'memory',
      'disk',
      'diskTemperature',
      'batteryTemperature',
      'chargingPower',
      'batteryPower',
      'cpuPower',
      'gpuPower',
      'systemPower',
      'fanSpeed',
    ] as const
  ).map(defaultColorRule);
}

export function isPowerMetric(id: ColorMetricId): boolean {
  return (
    id === 'cpuPower' || id === 'gpuPower' || id === 'chargingPower' || id === 'batteryPower' || id === 'systemPower'
  );
}
export function colorMetricUnit(id: ColorMetricId): string {
  return id === 'fanSpeed' ? 'RPM' : isPowerMetric(id) ? 'W' : isTemperatureMetric(id) ? '°C' : '%';
}
export function colorMetricLimit(id: ColorMetricId): number {
  return id === 'fanSpeed'
    ? 30000
    : id === 'cpuPower' || id === 'gpuPower'
      ? 1000
      : id === 'systemPower'
        ? 3000
        : isPowerMetric(id)
          ? 300
          : id === 'batteryTemperature'
            ? 100
            : isTemperatureMetric(id)
              ? 150
              : 100;
}
