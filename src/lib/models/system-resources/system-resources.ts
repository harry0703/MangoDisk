import type { BatteryDetails } from './power';
import type { GpuDetails } from '@/lib/models/system-resources/gpu-details';

export const RESOURCE_READINGS_SCHEMA_VERSION = 39;
export const TEMPERATURE_DIAGNOSTIC_RETENTION_MS = 30_000;
export const METRIC_IDS = ['cpu', 'gpu', 'memory', 'network', 'disk'] as const;
export type MetricId = (typeof METRIC_IDS)[number];
export type MetricStatus = 'loading' | 'ready' | 'stale' | 'disconnected' | 'unsupported' | 'failed';
export const METRIC_LABEL_KEYS: Record<MetricId, string> = {
  cpu: 'systemStatus.cpu',
  gpu: 'systemStatus.gpu',
  memory: 'systemStatus.memory',
  network: 'systemStatus.network',
  disk: 'systemStatus.disk',
};
export const METRIC_STATUS_KEYS: Record<MetricStatus, string> = {
  loading: 'systemStatus.loading',
  ready: 'systemStatus.ready',
  stale: 'systemStatus.stale',
  disconnected: 'systemStatus.disconnected',
  unsupported: 'systemStatus.unsupported',
  failed: 'systemStatus.failed',
};
export interface MetricReading<T> {
  status: MetricStatus;
  sampledAtMs: number | null;
  value: T | null;
}
export type MemoryPressure = 'unsupported' | 'unavailable' | 'normal' | 'warning' | 'critical';
export interface MemoryOverview {
  totalBytes: number;
  usedBytes: number;
  freeBytes: number;
  swapUsedBytes: number;
  pressure: MemoryPressure;
  usedPercent: number;
}
export interface ApplicationIdentity {
  id: string;
  name: string;
  processCount: number;
  iconPath: string | null;
  isBundle: boolean;
  canQuit: boolean;
  canForceQuit: boolean;
}
export interface ApplicationMemory extends ApplicationIdentity {
  usedBytes: number | null;
  readableProcessCount: number;
}
export interface ApplicationCpu extends ApplicationIdentity {
  processes?: { pid: number; startedAt: number; usedPercent: number }[];
  locationStatus?: 'available' | 'denied' | 'exited' | 'unavailable';
  pid: number;
  usedPercent: number;
}
export interface ProcessCpuSummary {
  usageScale: 'singleCore' | 'totalCapacity';
  applications: ApplicationCpu[];
  readableProcessCount: number;
  omittedProcessCount: number;
}
export interface ProcessMemorySummary {
  usageKind: 'physicalFootprint' | 'privateWorkingSet' | 'residentSet';
  applications: ApplicationMemory[];
  readableProcessCount: number;
  omittedProcessCount: number;
}
export interface SystemResourceSnapshot {
  schemaVersion: 5;
  sampledAtMs: number;
  memory: MemoryOverview;
  processes: ProcessMemorySummary | null;
}
export interface CpuUsage {
  usedPercent: number;
}
export type CpuFrequencyClass = 'average' | 'efficiency' | 'performance' | 'super';
export interface CpuIdentity {
  model: string | null;
  nominalFrequencyMhz: number | null;
  frequencyClasses: CpuFrequencyClass[];
}
export interface CpuFrequencyActivity {
  class: CpuFrequencyClass;
  activeRatio: number;
  idleRatio: number;
  downRatio: number;
  offRatio: number;
}
export interface CpuFrequency {
  averageMhz: number | null;
  efficiencyMhz: number | null;
  performanceMhz: number | null;
  superMhz: number | null;
  source: 'windowsPerformance' | 'applePerformanceStates';
  activity: CpuFrequencyActivity[];
}
export const CPU_TEMPERATURE_FRESHNESS_MS = 10_000;
export interface CpuTemperature {
  celsius: number;
  kind: 'coreAverage' | 'package' | 'coreMaximum' | 'diode' | 'proximity';
  source: 'appleSmc' | 'appleHid' | 'linuxHwmon' | 'intelDtsPawnIo' | 'amdZenPawnIo';
  sensorCount: number;
}
export const GPU_TEMPERATURE_FRESHNESS_MS = 10_000;
export interface GpuTemperature {
  adapterId: string;
  adapterName: string;
  celsius: number;
  kind: 'sensorAverage' | 'device';
  source: 'appleSmc' | 'appleHid' | 'driver';
  sensorCount: number;
}
export interface GpuUsage {
  usedPercent: number;
  adapterId: string;
  adapterName: string;
  details: GpuDetails | null;
}
export interface NetworkInterface {
  id: string;
  name: string;
  kind: 'ethernet' | 'wifi' | 'virtual' | 'other';
  connected: boolean;
  physical: boolean;
  defaultRouteMetric: number | null;
  details: {
    addresses: string[];
    hardwareAddress: string | null;
    wifi: {
      standard: string | null;
      channel: number | null;
      bandGhz: number | null;
      channelWidthMhz: number | null;
      signalDbm: number | null;
      noiseDbm: number | null;
    } | null;
  } | null;
}
export interface NetworkTrafficTotal {
  receivedBytes: string;
  transmittedBytes: string;
  startedAtMs: number;
}
export interface NetworkRate {
  cumulative: NetworkTrafficTotal | null;
  interface: NetworkInterface;
  receivedBytesPerSecond: number;
  transmittedBytesPerSecond: number;
  receivedBytesSinceStart: number;
  transmittedBytesSinceStart: number;
  receiveLinkBps: number | null;
  transmitLinkBps: number | null;
  selectionReason: 'manual' | 'defaultRoute' | 'physicalFallback' | 'disconnected';
}
export interface ResourceDisk {
  id: string;
  number: number | null;
  model: string | null;
}
export interface ResourceVolume {
  id: string;
  name: string;
  system: boolean;
  fileSystem: string | null;
  mountPoints: string[];
  devices: ResourceDisk[];
}
export interface DiskUsage {
  volume: ResourceVolume;
  totalBytes: number;
  usedBytes: number;
  availableBytes: number;
  usedPercent: number;
}
export interface TrendPoint {
  sampledAtMs: number;
  primary: number;
  secondary: number | null;
  gapBefore?: boolean;
}
export interface DiskIoRate {
  readBytesPerSecond: number;
  writtenBytesPerSecond: number;
  totalReadBytes: string;
  totalWrittenBytes: string;
}
export type BatterySource = 'appleSmc' | 'appleSmartBattery' | 'windowsBattery';
export interface BatteryTemperature {
  celsius: number;
  source: BatterySource;
}
export interface ChargingPower {
  watts: number;
  charging: boolean;
  externalConnected: boolean;
  source: BatterySource;
}
export type BatteryPowerState = 'charging' | 'discharging' | 'idle';
export interface BatteryPower {
  watts: number;
  state: BatteryPowerState;
  source: BatterySource;
}
export interface FanSpeed {
  id: number;
  rpm: number | null;
}
export interface FanSample {
  fans: FanSpeed[];
}
export interface ComponentPower {
  watts: number;
  source: 'ioReport' | 'appleSmc';
}
export interface GpuPower extends ComponentPower {
  adapterId: string;
}
export interface SystemPower {
  watts: number;
}
export interface DiskTemperature {
  volumeId: string;
  volumeName: string;
  celsius: number;
  source: 'nvmeSmart' | 'ataSmart' | 'windowsStorage';
}

export interface DiskHealth {
  volumeId: string;
  volumeName: string;
  status: 'healthy' | 'warning' | 'unhealthy' | 'unknown';
  source: DiskTemperature['source'];
  percentageUsed: number | null;
  totalWrittenBytes: string | null;
  totalReadBytes: string | null;
  powerCycles: number | null;
  availableSpare: number | null;
  criticalWarning: number | null;
  powerOnHours: number | null;
  mediaErrors: string | null;
  temperatureCelsius: number | null;
  ata: {
    model: string | null;
    firmware: string | null;
    profile: 'unknown' | 'samsungConsumer' | 'siliconMotionLegacy' | 'seagateBarracuda';
    revision: number;
    attributes: {
      id: number;
      flags: number;
      current: number;
      worst: number;
      raw: number[];
      reserved: number;
      decoded: {
        metric:
          | 'reallocatedSectors'
          | 'powerOnHours'
          | 'powerCycles'
          | 'remainingLife'
          | 'hostWrittenBytes'
          | 'hostReadBytes'
          | 'uncorrectableErrors'
          | 'spareBlocks'
          | 'initialBadBlocks'
          | 'totalEraseCount'
          | 'maximumEraseCount'
          | 'minimumEraseCount'
          | 'averageEraseCount'
          | 'ratedEraseCount'
          | 'runtimeInvalidBlocks'
          | 'programFailures'
          | 'eraseFailures'
          | 'temperature'
          | 'reallocationEvents'
          | 'pendingSectors'
          | 'offlineUncorrectableSectors'
          | 'interfaceCrcErrors'
          | 'tlcWrittenBytes'
          | 'slcWrittenBytes'
          | 'startStopCycles'
          | 'loadCycles'
          | 'headFlyingHours';
        value: string;
        unit: 'count' | 'hours' | 'bytes' | 'percent' | 'celsius';
      } | null;
    }[];
  } | null;
  nvme: {
    enduranceGroupCriticalWarning: number;
    availableSpareThreshold: number;
    dataUnitsRead: string;
    dataUnitsWritten: string;
    powerCycles: string;
    powerOnHours: string;
    hostReadCommands: string;
    hostWriteCommands: string;
    controllerBusyMinutes: string;
    unsafeShutdowns: string;
    errorInformationLogEntries: string;
    warningTemperatureMinutes: number;
    criticalTemperatureMinutes: number;
    temperatureSensorsCelsius: (number | null)[];
    thermalManagementTransitionCounts: number[];
    thermalManagementTotalSeconds: number[];
  } | null;
}

export interface GpuDeviceReading {
  adapterId: string;
  adapterName: string;
  usage: MetricReading<GpuUsage>;
  usageHistory: TrendPoint[];
  temperature: MetricReading<GpuTemperature>;
  temperatureHistory: TrendPoint[];
}

export interface ResourceReadings {
  schemaVersion: typeof RESOURCE_READINGS_SCHEMA_VERSION;
  observedAtMs: number;
  cpu: MetricReading<CpuUsage>;
  cpuIdentity: CpuIdentity | null;
  cpuFrequency: MetricReading<CpuFrequency>;
  cpuTemperature: MetricReading<CpuTemperature>;
  cpuTemperatureHistory: TrendPoint[];
  gpuTemperature: MetricReading<GpuTemperature>;
  gpuTemperatureHistory: TrendPoint[];
  batteryPresent: boolean | null;
  batteryDetails: MetricReading<BatteryDetails>;
  systemPowerHistory: TrendPoint[];
  batteryTemperature: MetricReading<BatteryTemperature>;
  chargingPower: MetricReading<ChargingPower>;
  batteryPower: MetricReading<BatteryPower>;
  cpuPower: MetricReading<ComponentPower>;
  gpuPower: MetricReading<GpuPower>;
  systemPower: MetricReading<SystemPower>;
  fanSpeed: MetricReading<FanSample>;
  gpu: MetricReading<GpuUsage>;
  gpuDetails: MetricReading<GpuUsage>;
  gpuDetailHistory: TrendPoint[];
  gpuDetailAdapterId: string | null;
  gpuRendererHistory: TrendPoint[];
  gpuTilerHistory: TrendPoint[];
  cpuProcesses: MetricReading<ProcessCpuSummary>;
  memoryProcesses: MetricReading<ProcessMemorySummary>;
  memory: MetricReading<SystemResourceSnapshot>;
  network: MetricReading<NetworkRate>;
  disk: MetricReading<DiskUsage>;
  diskTemperature: MetricReading<DiskTemperature>;
  diskDetailTemperature: MetricReading<DiskTemperature>;
  diskIo: MetricReading<DiskIoRate>;
  diskHealth: MetricReading<DiskHealth>;
  interfaces: NetworkInterface[];
  volumes: ResourceVolume[];
  gpuAdapters: { id: string; name: string }[];
  gpuDevices: GpuDeviceReading[];
  cpuHistory: TrendPoint[];
  gpuHistory: TrendPoint[];
  networkHistory: TrendPoint[];
  memoryHistory: TrendPoint[];
  diskIoHistory: TrendPoint[];
}
