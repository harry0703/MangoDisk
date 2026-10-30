import { readingFixture as reading } from '@/tests/fixtures/resident';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { useTrayPanelStore } from './tray-panel-store';
import { PreferenceStorageService } from '@/lib/services/settings/preference-storage-service';
import { ResidentService } from '@/lib/services/resident/resident-service';
import type { MemoryReleaseResult, ResidentReading } from '@/lib/models/resident/resident';

vi.mock('@/lib/services/resident/resident-service', () => ({
  ResidentService: { reading: vi.fn(), refresh: vi.fn(), releaseMemory: vi.fn() },
}));
vi.mock('@/lib/services/settings/preference-storage-service', () => ({
  PreferenceStorageService: { loadResourceSort: vi.fn(), saveResourceSort: vi.fn(async () => {}) },
}));
vi.mock('@/lib/services/diagnostics/logger-service', () => ({ LoggerService: { warn: vi.fn() } }));

describe('tray panel snapshots', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('ignores late cached responses after a newer event and recovers from an error', async () => {
    const store = useTrayPanelStore();
    let finish!: (value: ResidentReading) => void;
    vi.mocked(ResidentService.reading).mockReturnValueOnce(
      new Promise(resolve => {
        finish = resolve;
      })
    );
    const pending = store.load();
    store.accept(reading(3));
    finish(reading(2));
    await pending;
    expect(store.reading.revision).toBe(3);
    store.accept({ ...reading(4), memory: { ...reading(4).memory, status: 'failed' } });
    expect(store.reading.memory.status).toBe('failed');
    expect(store.error).toBe(false);
    expect(store.reading.memory.value).not.toBeNull();
    store.accept(reading(5));
    expect(store.error).toBe(false);
  });

  it.each([35, 38, 99])('rejects schema %s without turning failures into zero readings', async schemaVersion => {
    const store = useTrayPanelStore();
    const future = reading(1);
    Object.assign(future, { schemaVersion });
    store.accept(future);
    expect(store.error).toBe(true);
    expect(store.reading.memory.value).toBeNull();
    vi.mocked(ResidentService.reading).mockRejectedValueOnce(new Error('unavailable'));
    await store.load();
    expect(store.reading.memory.value).toBeNull();
    expect(store.error).toBe(true);
  });

  it('rejects old nested memory snapshots and recovers with current capabilities', () => {
    const store = useTrayPanelStore();
    const legacy = reading(1);
    Object.assign(legacy.memory.value!, { schemaVersion: 4 });
    store.accept(legacy);
    expect(store.error).toBe(true);
    expect(store.reading.memory.value).toBeNull();
    store.accept(reading(2));
    expect(store.error).toBe(false);
    expect(store.reading.memory.value?.schemaVersion).toBe(5);
  });

  it('coalesces in-flight refresh requests and allows retry after failure', async () => {
    const store = useTrayPanelStore();
    let fail!: (error: Error) => void;
    vi.mocked(ResidentService.refresh).mockReturnValueOnce(
      new Promise((_, reject) => {
        fail = reject;
      })
    );
    const pending = store.refresh();
    await store.refresh();
    expect(ResidentService.refresh).toHaveBeenCalledOnce();
    fail(new Error('IPC unavailable'));
    await pending;
    expect(store.refreshing).toBe(false);
    expect(store.error).toBe(true);
    vi.mocked(ResidentService.refresh).mockResolvedValueOnce();
    vi.mocked(ResidentService.reading).mockResolvedValueOnce(reading(1));
    await store.refresh();
    store.accept(reading(1));
    expect(store.refreshing).toBe(false);
    expect(store.error).toBe(false);
  });
});

describe('explicit memory release', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    vi.mocked(ResidentService.reading).mockResolvedValue(reading(1));
  });

  it('coalesces repeated clicks and refreshes after completion', async () => {
    let finish!: (value: MemoryReleaseResult) => void;
    vi.mocked(ResidentService.releaseMemory).mockReturnValue(
      new Promise(resolve => {
        finish = resolve;
      })
    );
    const store = useTrayPanelStore();
    const pending = store.releaseMemory();
    await store.releaseMemory();
    expect(ResidentService.releaseMemory).toHaveBeenCalledOnce();
    expect(store.releasing).toBe(true);
    finish({ schemaVersion: 1, status: 'completed', observedReductionBytes: 10 });
    await pending;
    expect(store.releasing).toBe(false);
    expect(store.releaseResult?.observedReductionBytes).toBe(10);
    expect(ResidentService.refresh).toHaveBeenCalledOnce();
  });

  it.each(['cancelled', 'unsupported', 'failed', 'busy'] as const)(
    'preserves the %s result without claiming savings',
    async status => {
      vi.mocked(ResidentService.releaseMemory).mockResolvedValue({
        schemaVersion: 1,
        status,
        observedReductionBytes: null,
      });
      const store = useTrayPanelStore();
      await store.releaseMemory();
      expect(store.releaseResult?.status).toBe(status);
      expect(store.releaseResult?.observedReductionBytes).toBeNull();
      expect(store.releasing).toBe(false);
    }
  );

  it('allows retry after IPC failure and rejects an incompatible response', async () => {
    const store = useTrayPanelStore();
    vi.mocked(ResidentService.releaseMemory).mockRejectedValueOnce(new Error('IPC failed'));
    await store.releaseMemory();
    expect(store.releaseResult?.status).toBe('failed');
    expect(store.releasing).toBe(false);
    const future = { schemaVersion: 1, status: 'completed', observedReductionBytes: 5 } as const;
    vi.mocked(ResidentService.releaseMemory).mockResolvedValueOnce(
      Object.assign({}, future, { schemaVersion: 2 }) as unknown as MemoryReleaseResult
    );
    await store.releaseMemory();
    expect(store.releaseResult?.status).toBe('failed');
  });
});

describe('resource sort hydration', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('preserves the other metric preference when a user sorts during the initial read', async () => {
    let finish!: (value: unknown) => void;
    vi.mocked(PreferenceStorageService.loadResourceSort).mockReturnValueOnce(
      new Promise(resolve => {
        finish = resolve;
      })
    );
    const store = useTrayPanelStore();
    const loading = store.loadSort();
    const saving = store.sortResources('cpu', 'name');
    expect(store.sortPreferences.cpu).toEqual({ column: 'name', direction: 'ascending' });
    finish({
      schemaVersion: 1,
      cpu: { column: 'usage', direction: 'ascending' },
      memory: { column: 'name', direction: 'descending' },
    });
    await Promise.all([loading, saving]);
    expect(store.sortPreferences.memory).toEqual({ column: 'name', direction: 'descending' });
    expect(store.sortPreferences.cpu).toEqual({ column: 'name', direction: 'ascending' });
    expect(PreferenceStorageService.saveResourceSort).toHaveBeenLastCalledWith(store.sortPreferences);
    expect(PreferenceStorageService.loadResourceSort).toHaveBeenCalledOnce();
  });

  it('shares the initial read across rapid choices and saves the latest choices', async () => {
    let finish!: (value: unknown) => void;
    vi.mocked(PreferenceStorageService.loadResourceSort).mockReturnValueOnce(
      new Promise(resolve => {
        finish = resolve;
      })
    );
    const store = useTrayPanelStore();
    const first = store.sortResources('cpu', 'name');
    const second = store.sortResources('memory', 'name');
    const third = store.sortResources('cpu', 'name');
    expect(PreferenceStorageService.saveResourceSort).not.toHaveBeenCalled();
    finish(null);
    await Promise.all([first, second, third]);
    expect(store.sortPreferences.cpu).toEqual({ column: 'name', direction: 'descending' });
    expect(store.sortPreferences.memory).toEqual({ column: 'name', direction: 'ascending' });
    expect(PreferenceStorageService.loadResourceSort).toHaveBeenCalledOnce();
    expect(PreferenceStorageService.saveResourceSort).toHaveBeenLastCalledWith(store.sortPreferences);
  });

  it('allows a user choice to be saved when the initial read fails', async () => {
    vi.mocked(PreferenceStorageService.loadResourceSort).mockRejectedValueOnce(new Error('read failed'));
    const store = useTrayPanelStore();
    await store.sortResources('memory', 'name');
    expect(store.sortPreferences.memory).toEqual({ column: 'name', direction: 'ascending' });
    expect(PreferenceStorageService.saveResourceSort).toHaveBeenCalledWith(store.sortPreferences);
    expect(store.sortLoading).toBeNull();
  });
});
