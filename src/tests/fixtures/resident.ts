import { defaultColorRules } from '@/lib/models/resident/resident-color-rules';
import type { ResidentPreferences, ResidentReading } from '@/lib/models/resident/resident';
import { emptyReadings } from '@/lib/utils/system-resources/system-resources';

export function preferencesFixture(): ResidentPreferences {
  return {
    schemaVersion: 42,
    windowsCpuTemperatureEnabled: false,
    revision: 0,
    enabled: true,
    showIcon: true,
    windowsDisplayMode: 'taskbar',
    macosDisplayMode: 'menuBar',
    linuxDisplayMode: 'tray',
    floating: {
      layout: 'horizontal',
      alwaysOnTop: true,
      locked: false,
      backgroundOpacity: 100,
      snapToEdges: true,
      fontSize: 'standard',
      bold: false,
      dividers: true,
      clickThrough: false,
      hideOnFullscreen: true,
      background: 'default',
    },
    taskbarMonitor: null,
    taskbarPosition: 'right',
    taskbarBackground: true,
    taskbarCompact: false,
    menuBarCompact: false,
    usageColors: true,
    colorRules: defaultColorRules(),
    metrics: [
      { id: 'cpu', enabled: false },
      { id: 'cpuFrequency', enabled: false },
      { id: 'cpuEfficiencyFrequency', enabled: false },
      { id: 'cpuPerformanceFrequency', enabled: false },
      { id: 'cpuSuperFrequency', enabled: false },
      { id: 'memory', enabled: true },
      { id: 'network', enabled: false },
      { id: 'disk', enabled: false },
      { id: 'diskTemperature', enabled: false },
      { id: 'gpu', enabled: false },
      { id: 'cpuTemperature', enabled: false },
      { id: 'gpuTemperature', enabled: false },
      { id: 'batteryTemperature', enabled: false },
      { id: 'chargingPower', enabled: false },
      { id: 'batteryPower', enabled: false },
      { id: 'cpuPower', enabled: false },
      { id: 'gpuPower', enabled: false },
      { id: 'systemPower', enabled: false },
      { id: 'fanSpeed', enabled: false },
    ],
    networkInterface: null,
    publicNetwork: { enabled: true },
    diskVolume: null,
    diskTemperatureDevice: null,
    gpuAdapter: null,
    gpuDisplay: { mode: 'automatic' },
    gpuTemperatureDisplay: { mode: 'automatic' },
    fanDisplay: 'highest',
    fanIds: [0],
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
        schemaVersion: 5,
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
