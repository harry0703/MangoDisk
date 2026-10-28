import type { ResidentPreferences, ResidentDisplayMetricId } from '@/lib/models/resident/resident';

export type FloatingMetricId = Exclude<ResidentDisplayMetricId, 'network'> | 'upload' | 'download';
export interface FloatingEntry {
  frequency?: { idleLabel: string; state: 'active' | 'idle' | 'unavailable' };
  id: FloatingMetricId;
  tone: 'normal' | 'green' | 'yellow' | 'blue' | 'warning' | 'critical';
  digits: string;
  marker: string;
  text: string;
  tooltip: string;
  gpuDevice?: { id: string; name: string; label: string };
  fanRows?: { id: number; text: string; tone: FloatingEntry['tone'] }[];
}
export interface FloatingSnapshot {
  schemaVersion: 12;
  interactive: boolean;
  contextMenu: boolean;
  entries: FloatingEntry[];
  preferences: ResidentPreferences['floating'];
  columns: number[] | null;
}

export interface FloatingItemMeasurement {
  id: FloatingMetricId;
  gpuDeviceId: string | null;
  labelWidth: number;
  valueWidth: number;
}

export interface FloatingLayoutMeasurement {
  fontSize: ResidentPreferences['floating']['fontSize'];
  bold: boolean;
  items: FloatingItemMeasurement[] | null;
}
