import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useWindowsMonitoringStore } from './windows-monitoring-store';
import { WindowsMonitoringService } from '@/lib/services/system-resources/windows-monitoring-service';
import type {
  WindowsMonitoringObservation,
  WindowsMonitoringSetupResult,
} from '@/lib/models/system-resources/windows-monitoring';

vi.mock('@/lib/services/system-resources/windows-monitoring-service', () => ({
  WindowsMonitoringService: { observe: vi.fn(), install: vi.fn(), uninstall: vi.fn() },
}));
vi.mock('@/lib/services/diagnostics/logger-service', () => ({ LoggerService: { warn: vi.fn() } }));
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => {
    resolve = done;
  });
  return { promise, resolve };
}
const ready: WindowsMonitoringSetupResult = {
  outcome: 'ready',
  observation: { state: 'ready', driverAvailable: true, serviceInstalled: true },
};
describe('Windows monitoring setup state', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
  });
  it('does not install while observing and rejects a stale observation after setup', async () => {
    const pending = deferred<WindowsMonitoringObservation>();
    vi.mocked(WindowsMonitoringService.observe).mockReturnValue(pending.promise);
    vi.mocked(WindowsMonitoringService.install).mockResolvedValue(ready);
    const store = useWindowsMonitoringStore();
    const observation = store.load();
    expect(WindowsMonitoringService.install).not.toHaveBeenCalled();
    await store.setup();
    pending.resolve({ state: 'notInstalled', driverAvailable: false, serviceInstalled: false });
    await observation;
    expect(store.observation).toEqual(ready.observation);
    expect(store.loading).toBe(false);
  });
  it('serializes install and uninstall and releases the gate on failure', async () => {
    const pending = deferred<WindowsMonitoringSetupResult>();
    vi.mocked(WindowsMonitoringService.install).mockReturnValue(pending.promise);
    const store = useWindowsMonitoringStore();
    const installation = store.setup();
    expect(store.busy).toBe(true);
    expect(await store.setup(true)).toBe('busy');
    expect(WindowsMonitoringService.uninstall).not.toHaveBeenCalled();
    pending.resolve({
      outcome: 'userCancelled',
      observation: { state: 'notInstalled', driverAvailable: false, serviceInstalled: false },
    });
    expect(await installation).toBe('userCancelled');
    expect(store.busy).toBe(false);
    vi.mocked(WindowsMonitoringService.install).mockRejectedValue(new Error('transport failed'));
    expect(await store.setup()).toBe('installationFailed');
    expect(store.busy).toBe(false);
  });
  it('preserves the observation on failed transport and uses typed unavailability on failed refresh', async () => {
    const store = useWindowsMonitoringStore();
    store.observation = ready.observation;
    vi.mocked(WindowsMonitoringService.uninstall).mockRejectedValue(new Error('lost IPC'));
    expect(await store.setup(true)).toBe('removalFailed');
    expect(store.observation).toEqual(ready.observation);
    vi.mocked(WindowsMonitoringService.observe).mockRejectedValue(new Error('lost IPC'));
    await store.load();
    expect(store.observation).toEqual({ state: 'unavailable', driverAvailable: false, serviceInstalled: false });
    expect(store.loading).toBe(false);
  });
  it('preserves the native trust code and clears it before a later attempt', async () => {
    const store = useWindowsMonitoringStore();
    vi.mocked(WindowsMonitoringService.install).mockResolvedValue({
      outcome: 'certificateUntrusted',
      nativeCode: '0x800B0109',
      observation: ready.observation,
    });
    expect(await store.setup()).toBe('certificateUntrusted');
    expect(store.nativeCode).toBe('0x800B0109');
    const pending = deferred<WindowsMonitoringSetupResult>();
    vi.mocked(WindowsMonitoringService.install).mockReturnValue(pending.promise);
    const retry = store.setup();
    expect(store.nativeCode).toBeNull();
    pending.resolve(ready);
    expect(await retry).toBe('ready');
    expect(store.nativeCode).toBeNull();
  });
  it('clears a previous setup diagnostic before refreshing the current state', async () => {
    const store = useWindowsMonitoringStore();
    store.outcome = 'certificateUntrusted';
    store.nativeCode = '0x800B0109';
    const pending = deferred<WindowsMonitoringObservation>();
    vi.mocked(WindowsMonitoringService.observe).mockReturnValue(pending.promise);
    const refresh = store.load();
    expect(store.outcome).toBeNull();
    expect(store.nativeCode).toBeNull();
    pending.resolve(ready.observation);
    await refresh;
    expect(store.observation).toEqual(ready.observation);
  });
  it('deduplicates refresh and does not refresh during installation', async () => {
    const pending = deferred<WindowsMonitoringSetupResult>();
    vi.mocked(WindowsMonitoringService.install).mockReturnValue(pending.promise);
    const store = useWindowsMonitoringStore();
    const installation = store.setup();
    await store.load();
    expect(WindowsMonitoringService.observe).not.toHaveBeenCalled();
    pending.resolve(ready);
    await installation;
    const observed = deferred<WindowsMonitoringObservation>();
    vi.mocked(WindowsMonitoringService.observe).mockReturnValue(observed.promise);
    const first = store.load();
    await store.load();
    expect(WindowsMonitoringService.observe).toHaveBeenCalledOnce();
    observed.resolve(ready.observation);
    await first;
  });
  it('shares typed status labels between settings and the setup dialog', () => {
    const store = useWindowsMonitoringStore();
    expect(store.statusLabel(true)).toBeNull();
    store.observation = ready.observation;
    expect(store.statusLabel(true)).toBe('windowsMonitoring.states.ready');
    expect(store.statusLabel(false)).toBe('windowsMonitoring.states.notEnabled');
    store.observation = { state: 'unavailable', driverAvailable: false, serviceInstalled: true };
    expect(store.statusLabel(true)).toBe('windowsMonitoring.states.unavailable');
    expect(store.needsAttention).toBe(true);
  });
});
