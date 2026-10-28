import type { FloatingEntry, FloatingMetricId } from '@/lib/models/resident/floating-monitor';
import type { ResidentPanelPage } from '@/lib/models/resident/resident';

export const FLOATING_LABEL_KEYS: Record<FloatingMetricId, string> = {
  cpu: 'floatingMonitor.labels.cpu',
  cpuFrequency: 'floatingMonitor.labels.cpuFrequency',
  cpuEfficiencyFrequency: 'floatingMonitor.labels.cpuEfficiencyFrequency',
  cpuPerformanceFrequency: 'floatingMonitor.labels.cpuPerformanceFrequency',
  cpuSuperFrequency: 'floatingMonitor.labels.cpuSuperFrequency',
  gpu: 'floatingMonitor.labels.gpu',
  memory: 'floatingMonitor.labels.memory',
  disk: 'floatingMonitor.labels.disk',
  cpuTemperature: 'floatingMonitor.labels.cpuTemperature',
  gpuTemperature: 'floatingMonitor.labels.gpuTemperature',
  diskTemperature: 'floatingMonitor.labels.diskTemperature',
  batteryTemperature: 'floatingMonitor.labels.batteryTemperature',
  chargingPower: 'floatingMonitor.labels.chargingPower',
  batteryPower: 'floatingMonitor.labels.batteryPower',
  cpuPower: 'floatingMonitor.labels.cpuPower',
  gpuPower: 'floatingMonitor.labels.gpuPower',
  systemPower: 'floatingMonitor.labels.systemPower',
  fanSpeed: 'floatingMonitor.labels.fanSpeed',
  upload: 'floatingMonitor.labels.upload',
  download: 'floatingMonitor.labels.download',
};

export function detailPage(id: FloatingMetricId): ResidentPanelPage {
  switch (id) {
    case 'cpu':
    case 'cpuPower':
    case 'cpuFrequency':
    case 'cpuEfficiencyFrequency':
    case 'cpuPerformanceFrequency':
    case 'cpuSuperFrequency':
    case 'cpuTemperature':
      return 'cpu';
    case 'batteryTemperature':
    case 'chargingPower':
    case 'batteryPower':
    case 'systemPower':
      return 'power';
    case 'gpu':
    case 'gpuPower':
    case 'gpuTemperature':
    case 'fanSpeed':
      return 'gpu';
    case 'memory':
      return 'memory';
    case 'disk':
    case 'diskTemperature':
      return 'disk';
    case 'upload':
    case 'download':
      return 'network';
  }
}

function floatingUnit(entry: FloatingEntry): string {
  if (entry.digits === '—' || (entry.frequency && entry.frequency.state !== 'active')) return '';
  switch (entry.id) {
    case 'cpuFrequency':
    case 'cpuEfficiencyFrequency':
    case 'cpuPerformanceFrequency':
    case 'cpuSuperFrequency':
      return 'GHz';
    case 'cpu':
    case 'gpu':
    case 'memory':
    case 'disk':
      return '%';
    case 'cpuTemperature':
    case 'gpuTemperature':
    case 'diskTemperature':
    case 'batteryTemperature':
      return '°C';
    case 'chargingPower':
    case 'batteryPower':
    case 'cpuPower':
    case 'gpuPower':
    case 'systemPower':
      return entry.marker;
    case 'fanSpeed':
      return '';
    case 'upload':
    case 'download': {
      // Native rate markers use a direction followed by B/K/M/G/T.
      const unit = entry.marker.slice(1);
      return unit === 'B' ? 'B/s' : `${unit}B/s`;
    }
  }
}

export function floatingValue(entry: FloatingEntry): string {
  if (entry.id === 'fanSpeed' && /^\d{5,}$/u.test(entry.digits)) {
    const rpm = Number(entry.digits);
    if (Number.isFinite(rpm)) return `${(rpm / 1000).toFixed(1).replace(/\.0$/u, '')}k`;
  }
  return entry.digits + floatingUnit(entry);
}
