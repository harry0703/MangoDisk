import { CPU_FREQUENCY_CLASSES, isCpuFrequencyDisplay, isIdleFrequency } from './cpu-frequency';
import type { ResidentDisplayMetricId, ResidentReading } from '@/lib/models/resident/resident';
import { METRIC_STATUS_KEYS, type MetricStatus } from '@/lib/models/system-resources/system-resources';

export type DisplayPlatform = 'macos' | 'windows' | 'linux';

/** Display availability never creates a sampling request. */
export function supportsDisplayMetric(id: ResidentDisplayMetricId, platform: DisplayPlatform): boolean {
  if (isCpuFrequencyDisplay(id)) return id === 'cpuFrequency' ? platform === 'windows' : platform === 'macos';
  if (id === 'systemPower' || id === 'cpuPower' || id === 'gpuPower') return platform === 'macos';
  return (
    platform !== 'linux' ||
    ![
      'gpu',
      'cpuTemperature',
      'gpuTemperature',
      'diskTemperature',
      'batteryTemperature',
      'chargingPower',
      'batteryPower',
      'fanSpeed',
    ].includes(id)
  );
}

/** Resolve typed observations without confusing absent hardware with a failed read. */
export function displayMetricStatusKey(
  id: ResidentDisplayMetricId,
  enabled: boolean,
  platform: DisplayPlatform,
  reading: ResidentReading | null,
  gpuTemperatureStatus?: MetricStatus
): string | null {
  if (!supportsDisplayMetric(id, platform)) return 'systemStatus.platformUnsupported';
  if (['batteryTemperature', 'chargingPower', 'batteryPower'].includes(id) && reading?.batteryPresent === false)
    return 'systemStatus.noBattery';
  if (isCpuFrequencyDisplay(id)) {
    const classes = reading?.cpuIdentity?.frequencyClasses;
    if (classes && !classes.includes(CPU_FREQUENCY_CLASSES[id])) return 'systemStatus.unsupported';
    const status = reading?.cpuFrequency.status ?? 'loading';
    if (!enabled && (status === 'loading' || status === 'stale')) return 'systemStatus.sampleWhenEnabled';
    if (status === 'ready' && enabled) {
      const fields = {
        average: 'averageMhz',
        efficiency: 'efficiencyMhz',
        performance: 'performanceMhz',
        super: 'superMhz',
      } as const;
      const field = fields[CPU_FREQUENCY_CLASSES[id]];
      if (reading?.cpuFrequency.value?.[field] == null) {
        const activity = reading?.cpuFrequency.value?.activity.find(value => value.class === CPU_FREQUENCY_CLASSES[id]);
        return isIdleFrequency(activity) ? 'systemStatus.frequencyIdle' : 'systemStatus.frequencyNoSample';
      }
    }
    return status === 'ready' ? null : METRIC_STATUS_KEYS[status];
  }
  const observation = reading?.[id];
  const status = (id === 'gpuTemperature' ? gpuTemperatureStatus : observation?.status) ?? 'loading';
  if (status === 'ready') {
    if (id !== 'gpuTemperature' && observation?.value === null) return 'systemStatus.readFailed';
    if (id === 'fanSpeed' && reading?.fanSpeed.value?.fans.length === 0) return 'systemStatus.noFans';
    return null;
  }
  if (status === 'failed') return 'systemStatus.readFailed';
  if ((status === 'loading' || status === 'stale') && !enabled) {
    return [
      'diskTemperature',
      'batteryTemperature',
      'chargingPower',
      'batteryPower',
      'systemPower',
      'fanSpeed',
    ].includes(id)
      ? 'systemStatus.sampleWhenEnabled'
      : METRIC_STATUS_KEYS[status];
  }
  return METRIC_STATUS_KEYS[status];
}
