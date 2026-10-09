import type { ResourceReadings, MetricReading } from '@/lib/models/system-resources';

export function emptyReadings(): ResourceReadings {
  const empty = <T>(): MetricReading<T> => ({ status: 'loading', sampledAtMs: null, value: null });
  return {
    schemaVersion: 16,
    observedAtMs: 0,
    cpu: empty(),
    cpuIdentity: null,
    cpuFrequency: empty(),
    cpuTemperature: empty(),
    cpuTemperatureHistory: [],
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
    diskIo: empty(),
    interfaces: [],
    volumes: [],
    gpuAdapters: [],
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
