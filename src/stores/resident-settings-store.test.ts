import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { useResidentSettingsStore } from './resident-settings-store';
import { ResidentService } from '@/lib/services/resident/resident-service';
import { preferencesFixture } from '@/tests/fixtures/resident';
import type { ResidentPreferences } from '@/lib/models/resident/resident';
import { watch } from 'vue';

vi.mock('@/lib/services/resident/resident-service', () => ({
  ResidentService: { preferences: vi.fn(), savePreferences: vi.fn(), defaultPreferences: vi.fn() },
}));
vi.mock('@/lib/services/diagnostics/logger-service', () => ({ LoggerService: { warn: vi.fn(), info: vi.fn() } }));

const conflict = {
  code: 'operationFailed',
  details: { operation: 'resident_save_preferences', reason: 'preferencesConflict' },
  retryable: true,
};

describe('resident preference transactions', () => {
  it.each(['default', 'darkGray'] as const)(
    'restores every option after a native conflict when the original background is %s',
    async background => {
      const defaults = preferencesFixture();
      const base: ResidentPreferences = {
        ...defaults,
        metrics: [...defaults.metrics].reverse(),
        floating: { ...defaults.floating, background },
      };
      const latest: ResidentPreferences = {
        ...base,
        revision: 1,
        floating: { ...base.floating, background: 'black' },
        enabled: false,
        macosDisplayMode: 'both',
      };
      vi.mocked(ResidentService.preferences).mockResolvedValueOnce(base).mockResolvedValue(latest);
      vi.mocked(ResidentService.savePreferences).mockRejectedValueOnce(conflict);
      const store = useResidentSettingsStore();
      await store.load();
      expect(await store.restoreDefaults()).toBe(true);
      expect(store.preferences).toEqual({
        ...defaults,
        revision: 2,
        enabled: latest.enabled,
        macosDisplayMode: latest.macosDisplayMode,
      });
    }
  );

  it('reports a failed restore even if a pending native refresh clears the shared error flag', async () => {
    const store = useResidentSettingsStore();
    await store.load();
    vi.mocked(ResidentService.savePreferences).mockRejectedValueOnce(new Error('disk full'));
    // Match the editor's pending native notification refresh after saving ends.
    const stop = watch(
      () => store.saving,
      saving => {
        if (!saving) void store.load();
      },
      { flush: 'post' }
    );
    try {
      expect(await store.restoreDefaults()).toBe(false);
    } finally {
      stop();
    }
  });

  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    vi.mocked(ResidentService.preferences).mockResolvedValue(preferencesFixture());
    vi.mocked(ResidentService.defaultPreferences).mockResolvedValue(preferencesFixture());
    vi.mocked(ResidentService.savePreferences).mockImplementation(async value => ({
      ...value,
      revision: value.revision + 1,
    }));
  });

  it.each([true, false])('restores native defaults while preserving destinations and enabled=%s', async enabled => {
    const defaults = preferencesFixture();
    const custom: ResidentPreferences = {
      ...defaults,
      revision: 17,
      enabled,
      windowsDisplayMode: 'both',
      taskbarMonitor: 'external-display',
      macosDisplayMode: 'floating',
      linuxDisplayMode: 'both',
      metrics: [...defaults.metrics].reverse().map(metric => ({ ...metric, enabled: true })),
      floating: { ...defaults.floating, layout: 'vertical', fontSize: 'extraLarge', background: 'black', locked: true },
      networkInterface: 'wifi',
      diskVolume: 'data',
      gpuDisplay: { mode: 'selected', adapterIds: ['gpu'] },
      usageColors: false,
      colorRules: defaults.colorRules.map(rule => ({ ...rule, enabled: false })),
    };
    vi.mocked(ResidentService.preferences).mockResolvedValue(custom);
    const store = useResidentSettingsStore();
    await store.load();
    expect(await store.restoreDefaults()).toBe(true);
    expect(store.preferences).toEqual({
      ...defaults,
      revision: 18,
      enabled,
      windowsDisplayMode: 'both',
      taskbarMonitor: 'external-display',
      macosDisplayMode: 'floating',
      linuxDisplayMode: 'both',
    });
    expect(ResidentService.savePreferences).toHaveBeenCalledTimes(1);
    expect(store.resetting).toBe(false);
  });

  it('keeps existing settings when defaults cannot be read and permits retry', async () => {
    const store = useResidentSettingsStore();
    await store.load();
    vi.mocked(ResidentService.defaultPreferences).mockRejectedValueOnce(new Error('IPC unavailable'));
    expect(await store.restoreDefaults()).toBe(false);
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
    expect(store.preferences).toEqual(preferencesFixture());
    expect(store.error).toBe(true);
    expect(store.resetting).toBe(false);
    expect(await store.restoreDefaults()).toBe(true);
    expect(store.error).toBe(false);
  });

  it('rolls back a rejected restore through the existing save transaction', async () => {
    const custom = { ...preferencesFixture(), revision: 9, usageColors: false };
    vi.mocked(ResidentService.preferences).mockResolvedValue(custom);
    const store = useResidentSettingsStore();
    await store.load();
    vi.mocked(ResidentService.savePreferences).mockRejectedValueOnce(new Error('disk full'));
    expect(await store.restoreDefaults()).toBe(false);
    expect(store.preferences).toEqual(custom);
    expect(store.draft).toEqual(custom);
    expect(store.error).toBe(true);
  });

  it('ignores duplicate resets and preserves mode changes made while defaults are loading', async () => {
    const store = useResidentSettingsStore();
    await store.load();
    let complete!: (value: ResidentPreferences) => void;
    vi.mocked(ResidentService.defaultPreferences).mockReturnValueOnce(new Promise(resolve => (complete = resolve)));
    const reset = store.restoreDefaults();
    expect(await store.restoreDefaults()).toBe(false);
    expect(ResidentService.defaultPreferences).toHaveBeenCalledTimes(1);
    await store.change({ macosDisplayMode: 'both' });
    complete(preferencesFixture());
    expect(await reset).toBe(true);
    expect(store.preferences?.macosDisplayMode).toBe('both');
    expect(store.preferences?.revision).toBe(2);
  });

  it('does not report success when a write starts while native defaults are loading', async () => {
    const store = useResidentSettingsStore();
    await store.load();
    let defaultsReady!: (value: ResidentPreferences) => void;
    let saved!: (value: ResidentPreferences) => void;
    vi.mocked(ResidentService.defaultPreferences).mockReturnValueOnce(
      new Promise(resolve => (defaultsReady = resolve))
    );
    const reset = store.restoreDefaults();
    vi.mocked(ResidentService.savePreferences).mockReturnValueOnce(new Promise(resolve => (saved = resolve)));
    const change = store.change({ macosDisplayMode: 'both' });
    defaultsReady(preferencesFixture());
    expect(await reset).toBe(false);
    expect(ResidentService.savePreferences).toHaveBeenCalledTimes(1);
    saved({ ...preferencesFixture(), revision: 1, macosDisplayMode: 'both' });
    await change;
    expect(store.preferences?.macosDisplayMode).toBe('both');
    expect(store.resetting).toBe(false);
  });

  it('coalesces fifty edits behind one in-flight write and uses its committed revision', async () => {
    const store = useResidentSettingsStore();
    await store.load();
    let complete!: (value: ResidentPreferences) => void;
    vi.mocked(ResidentService.savePreferences).mockReturnValueOnce(
      new Promise(resolve => {
        complete = resolve;
      })
    );
    const first = store.change({ showIcon: false });
    for (let index = 0; index < 50; index++) await store.change({ networkInterface: `device-${index}` });
    expect(ResidentService.savePreferences).toHaveBeenCalledTimes(1);
    expect(store.draft?.networkInterface).toBe('device-49');
    expect(store.preferences?.networkInterface).toBeNull();
    complete({ ...preferencesFixture(), showIcon: false, revision: 1 });
    await first;
    expect(ResidentService.savePreferences).toHaveBeenCalledTimes(2);
    expect(vi.mocked(ResidentService.savePreferences).mock.calls[1]?.[0]).toMatchObject({
      revision: 1,
      networkInterface: 'device-49',
    });
    expect(store.preferences).toMatchObject({ revision: 2, networkInterface: 'device-49', showIcon: false });
    expect(store.saving).toBe(false);
  });

  it('rolls preview back to committed state after a rejected native transaction', async () => {
    const store = useResidentSettingsStore();
    await store.load();
    vi.mocked(ResidentService.savePreferences).mockRejectedValueOnce(new Error('disk full'));
    await store.change({ showIcon: false, diskVolume: 'uncommitted' });
    expect(store.error).toBe(true);
    expect(store.draft).toEqual(preferencesFixture());
    expect(store.preferences).toEqual(preferencesFixture());
    await store.change({ showIcon: false });
    expect(store.error).toBe(false);
    expect(store.preferences?.showIcon).toBe(false);
  });

  it('retains known preferences if reconciliation also fails', async () => {
    const store = useResidentSettingsStore();
    await store.load();
    vi.mocked(ResidentService.savePreferences).mockRejectedValueOnce(new Error('native failure'));
    vi.mocked(ResidentService.preferences).mockRejectedValueOnce(new Error('IPC failure'));
    await store.change({ enabled: false });
    expect(store.draft?.enabled).toBe(true);
    expect(store.error).toBe(true);
  });

  it('recovers a stale window edit without overwriting another window GPU choice', async () => {
    const store = useResidentSettingsStore();
    const previous = { ...preferencesFixture(), revision: 41 };
    vi.mocked(ResidentService.preferences).mockResolvedValue(previous);
    await store.load();
    const latest = { ...previous, revision: 44, gpuAdapter: 'intel' };
    vi.mocked(ResidentService.preferences).mockResolvedValue(latest);
    vi.mocked(ResidentService.savePreferences).mockRejectedValueOnce(conflict);
    await store.change({ diskVolume: 'data' });
    expect(ResidentService.savePreferences).toHaveBeenCalledTimes(2);
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith({ ...latest, diskVolume: 'data' });
    expect(store.preferences).toMatchObject({ revision: 45, gpuAdapter: 'intel', diskVolume: 'data' });
    expect(store.draft).toEqual(store.preferences);
    expect(store.error).toBe(false);
  });

  it('keeps rapid local edits made while the conflicting write is in flight', async () => {
    const store = useResidentSettingsStore();
    await store.load();
    let reject!: (error: unknown) => void;
    vi.mocked(ResidentService.savePreferences).mockReturnValueOnce(new Promise((_, fail) => (reject = fail)));
    const saving = store.change({ diskVolume: 'data' });
    await store.change({ networkInterface: 'wifi' });
    vi.mocked(ResidentService.preferences).mockResolvedValue({
      ...preferencesFixture(),
      revision: 3,
      gpuAdapter: 'intel',
    });
    reject(conflict);
    await saving;
    expect(store.preferences).toMatchObject({
      revision: 4,
      diskVolume: 'data',
      networkInterface: 'wifi',
      gpuAdapter: 'intel',
    });
    expect(store.error).toBe(false);
  });

  it('does not retry when both windows changed the same field differently', async () => {
    const store = useResidentSettingsStore();
    await store.load();
    const latest = { ...preferencesFixture(), revision: 2, gpuAdapter: 'amd' };
    vi.mocked(ResidentService.preferences).mockResolvedValue(latest);
    vi.mocked(ResidentService.savePreferences).mockRejectedValueOnce(conflict);
    await store.change({ gpuAdapter: 'intel' });
    expect(ResidentService.savePreferences).toHaveBeenCalledTimes(1);
    expect(store.draft).toEqual(latest);
    expect(store.error).toBe(true);
  });

  it('bounds recovery to one retry even if another writer commits again', async () => {
    const store = useResidentSettingsStore();
    await store.load();
    vi.mocked(ResidentService.preferences)
      .mockResolvedValueOnce({ ...preferencesFixture(), revision: 2, gpuAdapter: 'intel' })
      .mockResolvedValueOnce({ ...preferencesFixture(), revision: 3, gpuAdapter: 'amd' });
    vi.mocked(ResidentService.savePreferences).mockRejectedValue(conflict);
    await store.change({ diskVolume: 'data' });
    expect(ResidentService.savePreferences).toHaveBeenCalledTimes(2);
    expect(store.draft).toMatchObject({ revision: 3, gpuAdapter: 'amd', diskVolume: null });
    expect(store.error).toBe(true);
  });

  it('does not retry storage failures even when another window advanced the revision', async () => {
    const store = useResidentSettingsStore();
    await store.load();
    vi.mocked(ResidentService.preferences).mockResolvedValue({ ...preferencesFixture(), revision: 2 });
    vi.mocked(ResidentService.savePreferences).mockRejectedValueOnce({
      ...conflict,
      details: { operation: 'resident_save_preferences' },
    });
    await store.change({ diskVolume: 'data' });
    expect(ResidentService.savePreferences).toHaveBeenCalledTimes(1);
    expect(store.error).toBe(true);
  });

  it.each(['resolve', 'reject'] as const)(
    'ignores a stale retry that completes with %s after a newer edit',
    async outcome => {
      const store = useResidentSettingsStore();
      await store.load();
      let resolve!: (value: ResidentPreferences) => void;
      let reject!: (error: Error) => void;
      vi.mocked(ResidentService.preferences).mockReturnValueOnce(
        new Promise((success, failure) => {
          resolve = success;
          reject = failure;
        })
      );
      const retry = store.load();
      await store.change({ showIcon: false });
      if (outcome === 'resolve') resolve(preferencesFixture());
      else reject(new Error('late read failure'));
      await retry;
      expect(store.preferences).toMatchObject({ revision: 1, showIcon: false });
      expect(store.draft).toEqual(store.preferences);
      expect(store.error).toBe(false);
      expect(store.loading).toBe(false);
    }
  );
});
