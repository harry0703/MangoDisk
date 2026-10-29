import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, it, expect, vi } from 'vitest';
import { flushPromises } from '@vue/test-utils';
import { NetworkAddressService } from '@/lib/services/system-resources/network-address-service';
import type { NetworkAddressSnapshot } from '@/lib/models/system-resources/network-addresses';
import { useNetworkAddressStore } from './network-address-store';
vi.mock('@/lib/services/system-resources/network-address-service', () => ({
  NetworkAddressService: { snapshot: vi.fn(), onChanged: vi.fn() },
}));
vi.mock('@/lib/services/diagnostics/logger-service', () => ({ LoggerService: { warn: vi.fn() } }));
function snapshot(revision = 1): NetworkAddressSnapshot {
  return {
    schemaVersion: 1,
    revision,
    probes: {
      ipv4: {
        address: { schemaVersion: 1, family: 4, ip: '203.0.113.1', countryCode: 'SG' },
        loading: false,
        failed: false,
      },
      ipv6: { address: null, loading: false, failed: true },
    },
    contextFailed: false,
    updatedAtMs: 1000,
  };
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(yes => {
    resolve = yes;
  });
  return { promise, resolve };
}
let notify: (value: NetworkAddressSnapshot) => void;
const stop = vi.fn<() => void>();
beforeEach(() => {
  setActivePinia(createPinia());
  vi.resetAllMocks();
  vi.mocked(NetworkAddressService.snapshot).mockResolvedValue(snapshot());
  vi.mocked(NetworkAddressService.onChanged).mockImplementation(async callback => {
    notify = callback;
    return stop;
  });
});
describe('native public address snapshots', () => {
  it('subscribes before opening and rejects an older command response after a newer event', async () => {
    const pending = deferred<NetworkAddressSnapshot>();
    vi.mocked(NetworkAddressService.snapshot).mockReturnValueOnce(pending.promise);
    const store = useNetworkAddressStore();
    const opening = store.setActive(true);
    await flushPromises();
    expect(NetworkAddressService.onChanged).toHaveBeenCalledTimes(1);
    const waiting = { ...snapshot(2), updatedAtMs: 2000 };
    waiting.probes.ipv4 = { address: null, loading: true, failed: false };
    waiting.probes.ipv6 = { address: null, loading: true, failed: false };
    notify(waiting);
    pending.resolve(snapshot(1));
    await opening;
    expect(store.revision).toBe(2);
    expect(store.updatedAt).toBe(2000);
    expect(store.busy).toBe(true);
  });
  it('reopens with the current background cache instead of a retained browser result', async () => {
    const store = useNetworkAddressStore();
    await store.setActive(true);
    await store.setActive(false);
    expect(stop).toHaveBeenCalledTimes(1);
    const fresh = snapshot(3);
    fresh.probes.ipv4.address!.ip = '203.0.113.3';
    vi.mocked(NetworkAddressService.snapshot).mockResolvedValue(fresh);
    await store.setActive(true);
    expect(store.addresses.ipv4?.ip).toBe('203.0.113.3');
    expect(vi.mocked(NetworkAddressService.snapshot).mock.calls.map(([request]) => request)).toEqual(['open', 'open']);
  });
  it('ignores late replies and listeners after disabling', async () => {
    const pending = deferred<NetworkAddressSnapshot>();
    vi.mocked(NetworkAddressService.snapshot).mockReturnValueOnce(pending.promise);
    const store = useNetworkAddressStore();
    const opening = store.setActive(true);
    await flushPromises();
    store.clear();
    notify(snapshot(4));
    pending.resolve(snapshot(3));
    await opening;
    expect(store.addresses.ipv4).toBeNull();
    expect(store.busy).toBe(false);
    await store.refresh();
    expect(NetworkAddressService.snapshot).toHaveBeenCalledTimes(1);
  });
  it('waits for an old read to settle before reopening without accepting its old data', async () => {
    const pending = deferred<NetworkAddressSnapshot>();
    vi.mocked(NetworkAddressService.snapshot).mockReturnValueOnce(pending.promise).mockResolvedValueOnce(snapshot(5));
    const store = useNetworkAddressStore();
    const opening = store.setActive(true);
    await flushPromises();
    await store.setActive(false);
    const reopening = store.setActive(true);
    await flushPromises();
    pending.resolve(snapshot(2));
    await Promise.all([opening, reopening]);
    expect(store.revision).toBe(5);
    expect(NetworkAddressService.snapshot).toHaveBeenCalledTimes(2);
  });
  it('coalesces observations and queues one explicit refresh during a pending read', async () => {
    const store = useNetworkAddressStore();
    await store.setActive(true);
    const pending = deferred<NetworkAddressSnapshot>();
    vi.mocked(NetworkAddressService.snapshot).mockReturnValueOnce(pending.promise);
    const observing = store.checkContext();
    void store.checkContext();
    void store.refresh();
    void store.refresh();
    pending.resolve(snapshot(2));
    await observing;
    await flushPromises();
    expect(vi.mocked(NetworkAddressService.snapshot).mock.calls.map(([request]) => request)).toEqual([
      'open',
      'observe',
      'refresh',
    ]);
  });
  it('keeps last known data on transport failure and recovers through observation', async () => {
    const store = useNetworkAddressStore();
    await store.setActive(true);
    vi.mocked(NetworkAddressService.snapshot).mockRejectedValueOnce(new Error('injected IPC failure'));
    await store.refresh();
    expect(store.contextFailed).toBe(true);
    expect(store.addresses.ipv4).not.toBeNull();
    vi.mocked(NetworkAddressService.snapshot).mockResolvedValue(snapshot(2));
    await store.checkContext();
    expect(store.contextFailed).toBe(false);
  });
  it('classifies actual proxy families and prefers fresh results over stale matching transports', async () => {
    const value = snapshot();
    value.probes.ipv4.failed = true;
    value.probes.ipv6 = {
      address: { schemaVersion: 1, ip: '203.0.113.2', family: 4, countryCode: null },
      loading: false,
      failed: false,
    };
    vi.mocked(NetworkAddressService.snapshot).mockResolvedValue(value);
    const store = useNetworkAddressStore();
    await store.setActive(true);
    expect(store.addresses.ipv4?.ip).toBe('203.0.113.2');
    expect(store.addresses.ipv6).toBeNull();
    expect(store.results.ipv4.stale).toBe(false);
  });
  it('rejects incompatible snapshots and still reads the cache if subscription fails', async () => {
    vi.mocked(NetworkAddressService.onChanged).mockRejectedValueOnce(new Error('listener unavailable'));
    vi.mocked(NetworkAddressService.snapshot).mockResolvedValueOnce({
      ...snapshot(),
      schemaVersion: 2,
    } as unknown as NetworkAddressSnapshot);
    const store = useNetworkAddressStore();
    await store.setActive(true);
    expect(store.contextFailed).toBe(true);
    expect(store.addresses.ipv4).toBeNull();
    await store.checkContext();
    expect(store.addresses.ipv4).not.toBeNull();
  });
  it('releases a subscription that finishes after the panel closes', async () => {
    const pending = deferred<() => void>();
    vi.mocked(NetworkAddressService.onChanged).mockReturnValueOnce(pending.promise);
    const store = useNetworkAddressStore();
    const opening = store.setActive(true);
    await store.setActive(false);
    pending.resolve(stop);
    await opening;
    expect(stop).toHaveBeenCalledTimes(1);
    expect(NetworkAddressService.snapshot).not.toHaveBeenCalled();
  });
});
