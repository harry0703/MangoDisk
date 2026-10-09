import type { ResidentPreferences, ResidentReading } from '@/lib/models/resident';
import { emptyReadings } from '@/lib/utils/system-resources';

export function preferencesFixture(): ResidentPreferences {
  return {
    schemaVersion: 11,
    revision: 0,
    enabled: true,
    showIcon: true,
    windowsDisplayMode: 'tray',
    taskbarPosition: 'right',
    taskbarBackground: true,
    taskbarCompact: false,
    menuBarCompact: false,
    usageColors: true,
    usageWarningPercent: 70,
    usageCriticalPercent: 90,
    metrics: [
      { id: 'cpu', enabled: false },
      { id: 'memory', enabled: true },
      { id: 'network', enabled: false },
      { id: 'disk', enabled: false },
      { id: 'gpu', enabled: false },
      { id: 'cpuTemperature', enabled: false },
    ],
    networkInterface: null,
    diskVolume: null,
    gpuAdapter: null,
  };
}
export function readingFixture(revision = 1): ResidentReading {
  return {
    revision,
    ...emptyReadings(),
    memory: {
      status: 'ready',
      sampledAtMs: revision,
      value: {
        schemaVersion: 4,
        sampledAtMs: revision,
        memory: {
          totalBytes: 100,
          usedBytes: 40,
          freeBytes: 60,
          usedPercent: 40,
          swapUsedBytes: 0,
          pressure: 'unsupported',
        },
        processes: null,
      },
    },
  };
}
