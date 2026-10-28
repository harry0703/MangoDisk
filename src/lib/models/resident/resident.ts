import type { MetricColorRule } from '@/lib/models/resident/resident-color-rules';
import type { MetricId, ResourceReadings } from '@/lib/models/system-resources/system-resources';

export type ResidentPanelPage = MetricId | 'overview' | 'power';

export interface ResidentReading extends ResourceReadings {
  revision: number;
}
export type ResidentDisplayMetricId =
  | MetricId
  | 'cpuFrequency'
  | 'cpuEfficiencyFrequency'
  | 'cpuPerformanceFrequency'
  | 'cpuSuperFrequency'
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

export type GpuDisplaySelection = { mode: 'automatic' } | { mode: 'selected'; adapterIds: string[] };

export interface ResidentPreferences {
  schemaVersion: 39;
  windowsCpuTemperatureEnabled: boolean;
  revision: number;
  enabled: boolean;
  showIcon: boolean;
  windowsDisplayMode: 'taskbar' | 'floating' | 'both';
  macosDisplayMode: 'menuBar' | 'floating' | 'both';
  linuxDisplayMode: 'tray' | 'floating' | 'both';
  floating: {
    layout: 'horizontal' | 'vertical';
    alwaysOnTop: boolean;
    locked: boolean;
    backgroundOpacity: number;
    snapToEdges: boolean;
    fontSize: 'standard' | 'larger' | 'large' | 'extraLarge';
    bold: boolean;
    dividers: boolean;
    clickThrough: boolean;
    hideOnFullscreen: boolean;
    background: 'default' | 'darkGray' | 'inkBlue' | 'warmWhite' | 'lightGray' | 'black' | 'white';
  };
  taskbarPosition: 'auto' | 'left' | 'right';
  taskbarBackground: boolean;
  taskbarCompact: boolean;
  menuBarCompact: boolean;
  usageColors: boolean;
  colorRules: MetricColorRule[];
  metrics: { id: ResidentDisplayMetricId; enabled: boolean }[];
  networkInterface: string | null;
  diskVolume: string | null;
  diskTemperatureDevice: string | null;
  gpuAdapter: string | null;
  gpuDisplay: GpuDisplaySelection;
  gpuTemperatureDisplay: GpuDisplaySelection;
  fanDisplay: 'highest' | 'average' | 'selected';
  fanIds: number[];
}

export type ResidentDestination = 'main' | 'cleanup' | 'applications' | 'settings' | 'displaySettings' | 'about';

export interface MemoryReleaseResult {
  schemaVersion: 1;
  status: 'completed' | 'cancelled' | 'unsupported' | 'failed' | 'busy';
  observedReductionBytes: number | null;
}

export type ApplicationQuitStatus = 'requested' | 'unavailable' | 'unsupported';

export type ResidentDisplayStatus = 'tray' | 'taskbar' | 'noSpace' | 'unsupportedLayout' | 'shellUnavailable';
