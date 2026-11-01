import { beforeEach, describe, expect, it, vi } from 'vitest';

import { PreferenceStorageService } from '@/lib/services/settings/preference-storage-service';
import * as AppSettingsUtils from '@/lib/utils/settings/app-settings';
import { COLOR_THEME_IDS, LANGUAGE_IDS, THEME_IDS } from '@/lib/models/settings/settings';

const { delayedFirstWrite, loadMock, saveMock, values } = vi.hoisted(() => {
  const storedValues = new Map<string, unknown>();
  const delayState = { enabled: false };
  const save = vi.fn(async () => undefined);
  const store = {
    get: vi.fn(async (key: string) => storedValues.get(key)),
    set: vi.fn(async (key: string, value: unknown) => {
      if (
        delayState.enabled &&
        key === 'storageScopePreferences' &&
        typeof value === 'object' &&
        value !== null &&
        'selectedPaths' in value &&
        (value.selectedPaths as Record<string, unknown>).analysis === '/first'
      ) {
        await new Promise(resolve => setTimeout(resolve, 10));
      }
      storedValues.set(key, value);
    }),
    delete: vi.fn(async (key: string) => storedValues.delete(key)),
    save,
  };
  return {
    delayedFirstWrite: delayState,
    loadMock: vi.fn(async () => store),
    saveMock: save,
    values: storedValues,
  };
});

vi.mock('@tauri-apps/plugin-store', () => ({ load: loadMock }));

describe('PreferenceStorageService', () => {
  beforeEach(() => {
    values.clear();
    delayedFirstWrite.enabled = false;
    saveMock.mockReset().mockResolvedValue(undefined);
  });

  it('uses one unversioned settings file and persists values directly', async () => {
    const preferences = {
      selectedPaths: { analysis: '/workspace' },
      recentFolders: ['/workspace'],
    };
    await PreferenceStorageService.saveStorageScopePreferences(preferences);

    expect(loadMock).toHaveBeenCalledWith('settings.json', { autoSave: false });
    expect(values.get('storageScopePreferences')).toEqual(preferences);
    expect(saveMock).toHaveBeenCalledOnce();
  });

  it('serializes immutable export preferences without replacing other settings', async () => {
    const settings = AppSettingsUtils.defaults();
    await PreferenceStorageService.saveSettings(settings);
    const first = {
      schemaVersion: 1 as const,
      maxDepth: 6 as const,
      maxEntries: 0 as const,
      includeFullPaths: true,
      format: 'markdown' as const,
    };
    const initial = PreferenceStorageService.saveAnalysisExportPreferences(first);
    first.includeFullPaths = false;
    const latest = { ...first, maxDepth: 3 as const, maxEntries: 1000 as const };
    const followup = PreferenceStorageService.saveAnalysisExportPreferences(latest);
    latest.includeFullPaths = true;
    await Promise.all([initial, followup]);
    expect(await PreferenceStorageService.loadAnalysisExportPreferences()).toEqual({
      ...latest,
      includeFullPaths: false,
    });
    expect(await PreferenceStorageService.loadSettings()).toEqual(settings);
  });

  it('keeps settings domains as separate keys in the same store', async () => {
    const preferences = {
      selectedPaths: {
        analysis: '/Users/example/Downloads',
      },
      recentFolders: ['/Users/example/Downloads'],
    };

    await PreferenceStorageService.saveStorageScopePreferences(preferences);

    expect(await PreferenceStorageService.loadStorageScopePreferences()).toEqual(preferences);
    expect(await PreferenceStorageService.loadSettings()).toBeNull();
  });

  it('persists the read-failure alert choice with the app settings', async () => {
    const settings = { ...AppSettingsUtils.defaults(), hideCleanupReadFailureAlerts: true };
    await PreferenceStorageService.saveSettings(settings);
    expect(await PreferenceStorageService.loadSettings()).toEqual(settings);
    expect(saveMock).toHaveBeenCalledOnce();
  });

  it('patches the sidebar without replacing stored settings with startup fallback values', async () => {
    const saved = {
      ...AppSettingsUtils.defaults(LANGUAGE_IDS.jaJP),
      theme: THEME_IDS.dark,
      colorTheme: COLOR_THEME_IDS.warmGray,
    };
    values.set('settings', saved);

    await PreferenceStorageService.saveSidebarExpanded(true, AppSettingsUtils.defaults(LANGUAGE_IDS.enUS));

    expect(await PreferenceStorageService.loadSettings()).toEqual({ ...saved, sidebarExpanded: true });
    expect(saveMock).toHaveBeenCalledOnce();
  });

  it('creates a complete settings document when the first saved choice is the sidebar', async () => {
    const defaults = AppSettingsUtils.defaults();

    await PreferenceStorageService.saveSidebarExpanded(false, defaults);

    expect(await PreferenceStorageService.loadSettings()).toEqual({ ...defaults, sidebarExpanded: false });
    expect(defaults.sidebarExpanded).toBeNull();
  });

  it('preserves an unrecognized settings document instead of replacing it with sidebar defaults', async () => {
    values.set('settings', 'unrecognized');

    await expect(PreferenceStorageService.saveSidebarExpanded(true, AppSettingsUtils.defaults())).rejects.toThrow(
      'Invalid app settings document'
    );

    expect(values.get('settings')).toBe('unrecognized');
    expect(saveMock).not.toHaveBeenCalled();
  });

  it('persists shared scan exclusions without changing other preference domains', async () => {
    const storageScopePreferences = {
      selectedPaths: { 'large-files': '/workspace' },
      recentFolders: ['/workspace'],
    };
    const storageScanPreferences = {
      names: [],
      schemaVersion: 3 as const,
      folders: [{ path: '/workspace/cache', scopes: ['largeFiles' as const] }],
    };

    await PreferenceStorageService.saveStorageScopePreferences(storageScopePreferences);
    await PreferenceStorageService.saveScanExclusionPreferences(storageScanPreferences);

    expect(await PreferenceStorageService.loadScanExclusionPreferences()).toEqual(storageScanPreferences);
    expect(await PreferenceStorageService.loadStorageScopePreferences()).toEqual(storageScopePreferences);
    expect(values.get('scanExclusionPreferences')).toEqual(storageScanPreferences);
  });

  it('migrates legacy large-file exclusions without leaving two sources of truth', async () => {
    const preferences = {
      schemaVersion: 1 as const,
      excludedFolders: ['/workspace/cache'],
    };
    values.set('largeFilePreferences', preferences);

    expect(await PreferenceStorageService.loadLegacyLargeFilePreferences()).toEqual(preferences);
    await PreferenceStorageService.migrateScanExclusionPreferences({
      names: [],
      schemaVersion: 3,
      folders: [{ path: '/workspace/cache', scopes: ['largeFiles'] }],
    });

    expect(await PreferenceStorageService.loadScanExclusionPreferences()).toEqual({
      names: [],
      schemaVersion: 3,
      folders: [{ path: '/workspace/cache', scopes: ['largeFiles'] }],
    });
    expect(await PreferenceStorageService.loadLegacyLargeFilePreferences()).toBeNull();
    expect(saveMock).toHaveBeenCalledOnce();
  });

  it('deletes an invalid domain value without clearing other settings', async () => {
    values.set('settings', { invalid: true });
    await PreferenceStorageService.saveStorageScopePreferences({
      selectedPaths: {},
      recentFolders: [],
    });

    await PreferenceStorageService.clearSettings();

    expect(await PreferenceStorageService.loadSettings()).toBeNull();
    expect(await PreferenceStorageService.loadStorageScopePreferences()).toEqual({
      selectedPaths: {},
      recentFolders: [],
    });
  });

  it('serializes rapid writes so the latest preference reaches disk last', async () => {
    delayedFirstWrite.enabled = true;

    await Promise.all([
      PreferenceStorageService.saveStorageScopePreferences({
        selectedPaths: { analysis: '/first' },
        recentFolders: ['/first'],
      }),
      PreferenceStorageService.saveStorageScopePreferences({
        selectedPaths: { analysis: '/second' },
        recentFolders: ['/second'],
      }),
    ]);

    expect(values.get('storageScopePreferences')).toEqual({
      selectedPaths: { analysis: '/second' },
      recentFolders: ['/second'],
    });
  });

  it('coalesces rapid chart updates while retaining the last immutable snapshot', async () => {
    let release: () => void = () => undefined;
    saveMock.mockImplementationOnce(
      () =>
        new Promise(resolve => {
          release = () => resolve(undefined);
        })
    );
    const initial = { schemaVersion: 1 as const, viewMode: 'treemap' as const, treemapDepth: 1, sunburstDepth: 3 };
    const first = PreferenceStorageService.saveAnalysisViewPreferences(initial);
    await vi.waitFor(() => expect(saveMock).toHaveBeenCalledOnce());
    const writes = [];
    for (let index = 0; index < 100; index++) {
      writes.push(PreferenceStorageService.saveAnalysisViewPreferences({ ...initial, treemapDepth: (index % 6) + 1 }));
    }
    const latest = { ...initial, viewMode: 'sunburst' as const, treemapDepth: 2, sunburstDepth: 5 };
    writes.push(PreferenceStorageService.saveAnalysisViewPreferences(latest));
    latest.sunburstDepth = 6;
    release();
    await Promise.all([first, ...writes]);
    expect(saveMock).toHaveBeenCalledTimes(2);
    expect(await PreferenceStorageService.loadAnalysisViewPreferences()).toEqual({ ...latest, sunburstDepth: 5 });
  });

  it('isolates chart preferences and lets other settings complete before follow-up updates', async () => {
    let release: () => void = () => undefined;
    const savedKeys: string[][] = [];
    saveMock.mockImplementation(async () => {
      savedKeys.push([...values.keys()]);
    });
    saveMock.mockImplementationOnce(
      () =>
        new Promise(resolve => {
          release = () => resolve(undefined);
        })
    );
    const initial = { schemaVersion: 1 as const, viewMode: 'treemap' as const, treemapDepth: 1, sunburstDepth: 3 };
    const first = PreferenceStorageService.saveAnalysisViewPreferences(initial);
    await vi.waitFor(() => expect(saveMock).toHaveBeenCalledOnce());
    const settings = AppSettingsUtils.defaults();
    const otherDomain = PreferenceStorageService.saveSettings(settings);
    const followup = PreferenceStorageService.saveAnalysisViewPreferences({ ...initial, treemapDepth: 4 });
    release();
    await Promise.all([first, otherDomain, followup]);
    expect(saveMock).toHaveBeenCalledTimes(3);
    expect(savedKeys[0]).toContain('settings');
    expect(await PreferenceStorageService.loadSettings()).toEqual(settings);
    expect(await PreferenceStorageService.loadAnalysisViewPreferences()).toEqual({ ...initial, treemapDepth: 4 });
  });

  it('allows the latest chart update to save after a preceding write fails', async () => {
    let reject: (error: Error) => void = () => undefined;
    saveMock.mockImplementationOnce(
      () =>
        new Promise((_resolve, fail) => {
          reject = fail;
        })
    );
    const initial = { schemaVersion: 1 as const, viewMode: 'treemap' as const, treemapDepth: 1, sunburstDepth: 3 };
    const first = PreferenceStorageService.saveAnalysisViewPreferences(initial);
    const failure = expect(first).rejects.toThrow('disk unavailable');
    await vi.waitFor(() => expect(saveMock).toHaveBeenCalledOnce());
    const followup = PreferenceStorageService.saveAnalysisViewPreferences({ ...initial, treemapDepth: 6 });
    reject(new Error('disk unavailable'));
    await failure;
    await followup;
    expect(saveMock).toHaveBeenCalledTimes(2);
    expect(await PreferenceStorageService.loadAnalysisViewPreferences()).toEqual({ ...initial, treemapDepth: 6 });
  });
});
