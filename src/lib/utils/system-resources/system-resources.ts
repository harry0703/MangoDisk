import {
  RESOURCE_READINGS_SCHEMA_VERSION,
  type ResourceReadings,
  type MetricReading,
} from '@/lib/models/system-resources/system-resources';

export function emptyReadings(): ResourceReadings {
  const empty = <T>(): MetricReading<T> => ({ status: 'loading', sampledAtMs: null, value: null });
  return {
    schemaVersion: RESOURCE_READINGS_SCHEMA_VERSION,
    observedAtMs: 0,
    cpu: empty(),
    cpuIdentity: null,
    cpuFrequency: empty(),
    cpuTemperature: empty(),
    cpuTemperatureHistory: [],
    gpuTemperature: empty(),
    gpuTemperatureHistory: [],
    batteryPresent: null,
    batteryDetails: { status: 'loading', value: null, sampledAtMs: null },
    systemPowerHistory: [],
    batteryTemperature: empty(),
    chargingPower: empty(),
    batteryPower: empty(),
    cpuPower: empty(),
    gpuPower: empty(),
    systemPower: empty(),
    fanSpeed: empty(),
    gpu: empty(),
    gpuDetails: empty(),
    gpuDetailHistory: [],
    gpuDetailAdapterId: null,
    gpuRendererHistory: [],
    gpuTilerHistory: [],
    cpuProcesses: empty(),
    memoryProcesses: empty(),
    memory: empty(),
    network: empty(),
    disk: empty(),
    diskTemperature: empty(),
    diskDetailTemperature: empty(),
    diskIo: empty(),
    diskHealth: empty(),
    interfaces: [],
    volumes: [],
    gpuAdapters: [],
    gpuDevices: [],
    cpuHistory: [],
    gpuHistory: [],
    networkHistory: [],
    memoryHistory: [],
    diskIoHistory: [],
  };
}

/** Preserve unchanged sample values across unrelated metric publications. */
export function retainSampleValue<T>(previous: MetricReading<T>, incoming: MetricReading<T>): MetricReading<T> {
  return incoming.sampledAtMs !== null &&
    incoming.sampledAtMs === previous.sampledAtMs &&
    (incoming.value === null) === (previous.value === null)
    ? { ...incoming, value: previous.value }
    : incoming;
}
