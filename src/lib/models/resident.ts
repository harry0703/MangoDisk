import type { MetricId, ResourceReadings } from '@/lib/models/system-resources';

export interface ResidentReading extends ResourceReadings {
  revision: number;
}
export interface ResidentPreferences {
  schemaVersion: 8;
  revision: number;
  enabled: boolean;
  showIcon: boolean;
  windowsDisplayMode: 'tray' | 'taskbar';
  taskbarPosition: 'auto' | 'left' | 'right';
  taskbarBackground: boolean;
  taskbarCompact: boolean;
  menuBarCompact: boolean;
  usageColors: boolean;
  usageWarningPercent: number;
  usageCriticalPercent: number;
  metrics: { id: MetricId; enabled: boolean }[];
  networkInterface: string | null;
  diskVolume: string | null;
}

export type ResidentDestination = 'main' | 'cleanup' | 'applications' | 'settings' | 'about';

export interface MemoryReleaseResult {
  schemaVersion: 1;
  status: 'completed' | 'cancelled' | 'unsupported' | 'failed' | 'busy';
  observedReductionBytes: number | null;
}

export type ApplicationQuitStatus = 'requested' | 'unavailable' | 'unsupported';

export type ResidentDisplayStatus = 'tray' | 'taskbar' | 'noSpace' | 'unsupportedLayout' | 'shellUnavailable';
